//! outl-desktop — Tauri 2 desktop client.
//!
//! Thin glue layer between the Solid frontend and `outl-actions`.
//! Same architectural rule as `outl-mobile` and `outl-tui`: this
//! crate adds zero business logic — every workspace mutation is a
//! shim that delegates to `outl-actions`.
//!
//! ## Lifecycle
//!
//! 1. Boot: load (or generate) the device's `ActorId`, load
//!    `settings.json`, start the Tauri runtime.
//! 2. If `settings.last_workspace` points at a valid directory, open
//!    it on a background thread and emit `workspace-ready` when done.
//! 3. Otherwise the frontend renders the `WorkspacePicker` and the
//!    user picks a directory; the resulting Tauri call to
//!    `set_workspace` opens the workspace, persists the path in
//!    settings, and emits `workspace-ready`.
//! 4. After `workspace-ready` the frontend calls `open_today_journal`
//!    and renders the outline.
//!
//! The `ActorId` is generated once per device and lives in
//! `<app_config_dir>/actor` — switching workspaces does *not* rotate
//! the actor (actors identify devices, not workspaces). The op log
//! within each workspace tracks `ops-<actor>.jsonl` per device.
//!
//! ## Module map
//!
//! - `settings` — JSON IO for user preferences + `last_workspace`.
//! - `state` — `AppState`, wire types (`PageView`, `WorkspaceSummary`).
//! - `helpers` — argument parsers, workspace-lock acquisition,
//!   `finish_in_page` (the mutation funnel).
//! - `workspace_open` — open / reconcile / boot opener primitives.
//! - `commands` — Tauri command surface split by responsibility
//!   (`workspace`, `page`, `block`).

mod commands;
mod fs_watcher;
mod helpers;
mod iroh_sync;
mod plugin_service;
mod settings;
mod state;
mod workspace_open;

use std::path::PathBuf;
use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

use crate::commands::{
    attach_asset, clear_reminder_snooze, copy_block_markdown, copy_markdown, create_block,
    current_workspace, date_title, delete_block, delete_page, deliver_due_reminders, edit_block,
    get_settings, get_theme, import_asset_file, indent_block, instantiate_template_at,
    known_property_keys, list_all_pages, list_reminders, list_shortcut_bindings,
    list_templates_cmd, list_themes, mark_block_done, move_block_after, move_block_down,
    move_block_up, next_day, open_asset, open_journal_for, open_page_by_slug, open_ref,
    open_today_journal, outdent_block, outl_emoji_search, outl_peer_list, outl_peer_pair_host,
    outl_peer_pair_join, outl_peer_remove, outl_peer_status, outl_sync_now, page_backlinks,
    paste_block_after, paste_markdown_at, paste_plain_at, plugin_config_set,
    plugin_install_official, plugin_keybindings, plugin_list, plugin_registry_list, plugin_run,
    plugin_secret_remove, plugin_secret_set, plugin_set_enabled, plugin_settings_describe,
    plugin_sync_hooks, plugin_toolbar, plugin_transform, plugin_transformers, plugin_uninstall,
    previous_day, read_asset_data_url, redo_page, reload_workspace, reminder_settings,
    resolve_embeds, resolve_page_labels, resolve_ref, run_auto_run_blocks, run_code_block,
    search_blocks, search_pages, search_persons, set_backlinks_order, set_block_collapsed,
    set_block_property, set_block_remind, set_page_property, set_reminder_settings, set_workspace,
    snooze_presets, snooze_reminder, split_block, today_slug_cmd, toggle_quote, toggle_todo,
    undo_page, update_settings, workspace_stats,
};
use crate::plugin_service::spawn_plugin_service;
use crate::state::AppState;
use crate::workspace_open::{load_or_create_actor, spawn_workspace_opener};

/// A deep link that arrived during cold start, before the frontend
/// mounted its `deep-link://navigate` listener. The frontend drains it
/// once on boot via [`take_pending_deep_link`] (issue #98).
///
/// Only the **cold-start** path (the launch URL) populates this. The
/// warm path (`on_open_url` while the app runs) emits the event directly
/// — the listener is already up — and never touches the buffer, so a
/// stale target can't replay on the next plain launch.
struct PendingDeepLink(parking_lot::Mutex<Option<serde_json::Value>>);

/// Frontend command: take (and clear) the deep link buffered during cold
/// start. Returns `null` when the app launched normally.
#[tauri::command]
fn take_pending_deep_link(pending: tauri::State<'_, PendingDeepLink>) -> Option<serde_json::Value> {
    pending.0.lock().take()
}

/// Parse an `outl://` URL via the shared `outl_actions` parser into the
/// `{kind, …}` payload the frontend maps onto its `open*` commands.
///
/// A malformed URL (wrong scheme, unknown kind, bad date, traversal
/// slug) is logged at `warn` and returns `None` — never a crash, never a
/// stray page (issue #98). The parser is shared with `outl-mobile` so
/// the two clients can't drift on the scheme contract.
fn deep_link_payload(raw: &str) -> Option<serde_json::Value> {
    use outl_actions::DeepLinkTarget;

    match outl_actions::parse_deep_link(raw) {
        Ok(DeepLinkTarget::Today) => Some(serde_json::json!({ "kind": "today" })),
        Ok(DeepLinkTarget::Daily(date)) => Some(serde_json::json!({
            "kind": "daily",
            "date": date.format("%Y-%m-%d").to_string(),
        })),
        Ok(DeepLinkTarget::Page(slug)) => Some(serde_json::json!({
            "kind": "page",
            "slug": slug,
        })),
        Err(err) => {
            tracing::warn!("deep link ignored ({raw}): {err}");
            None
        }
    }
}

/// Warm path: an `outl://` URL opened while the app is running. Emit the
/// navigate event (the frontend listener is up) and focus the window.
fn dispatch_deep_link(app: &tauri::AppHandle, raw: &str) {
    let Some(payload) = deep_link_payload(raw) else {
        return;
    };
    if let Err(err) = app.emit("deep-link://navigate", payload) {
        tracing::warn!("deep link: failed to emit navigate event: {err}");
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install a `tracing` subscriber FIRST so the iroh P2P transport's
    // `info!`/`warn!`/`debug!` lines are visible in the terminal running the
    // app (stderr). Without this the transport runs blind. `RUST_LOG` overrides
    // the default.
    //
    // We use `set_global_default` (NOT `.try_init()`) on purpose: `try_init`
    // also installs a `log`→`tracing` bridge (`LogTracer`) that claims the
    // global `log` logger slot, which would collide with `tauri-plugin-log`
    // below (it installs ITS logger in the same slot). Splitting them keeps
    // `tracing` as the backend for our own `tracing::` events (sync/iroh) and
    // leaves the `log` crate's slot free for the plugin, which owns the
    // frontend + file/webview targets. A double-init just returns `Err`, so
    // `.ok()` keeps it a no-op.
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,outl_sync_iroh=debug,iroh=info")
            }),
        )
        .with_writer(std::io::stderr)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    // rustls 0.23 needs a process-wide CryptoProvider before any rustls
    // consumer builds a client. iroh pulls rustls with `default-features =
    // false`, dropping the workspace-wide default provider — so reqwest
    // (Tauri's webview asset protocol) panics in `ClientBuilder::build()` at
    // boot. Install `ring` (the provider in our dep graph) explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();

    tauri::Builder::default()
        // Single-instance MUST be the first plugin. Its `deep-link`
        // feature forwards an `outl://` URL opened while the app is
        // already running to `on_open_url` (Linux/Windows); the callback
        // only needs to surface the existing window. macOS routes the
        // URL to the running instance natively via Apple Event.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_focus();
            }
        }))
        // Unified logging (React-Native-style): the frontend logs via
        // `@tauri-apps/plugin-log` + `attachConsole()`, and everything lands on
        // three targets — Stdout (the terminal running `cargo tauri dev`), the
        // Webview console, and a rotating file in the OS log dir
        // (`~/Library/Logs/<bundle>/` on macOS, `%APPDATA%/<bundle>/` on
        // Windows, `~/.config/<bundle>/` on Linux). This owns the `log` crate
        // slot (see the tracing note above); our own `tracing::` events still go
        // to stderr via the subscriber. Registered after single-instance, which
        // must stay first.
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_deep_link::init())
        // OS banners for `remind::` rules. Registering the plugin does
        // not prompt for permission — the frontend asks only when the
        // user turns reminders on in Settings.
        .plugin(tauri_plugin_notification::init())
        .setup(|app| -> Result<(), Box<dyn std::error::Error>> {
            // Local-only state (the per-device `actor` ULID and the
            // `config.toml`) lives at `~/.config/outl/` — the XDG
            // path the TUI also reads, so the two clients share a
            // single source of truth. Tauri's
            // `app.path().app_config_dir()` (which would resolve to
            // `~/Library/Application Support/app.outl.desktop/` on
            // macOS) is deliberately ignored here.
            let app_config_dir = outl_config::config_dir();
            std::fs::create_dir_all(&app_config_dir)?;

            let actor = load_or_create_actor(&app_config_dir)?;
            let hlc = HlcGenerator::new(actor);

            let settings = settings::load(&app_config_dir);
            let last_workspace = settings.last_workspace.clone();

            // The flat desktop `Settings` drops `[sync]` (see `settings.rs`)
            // and doesn't carry `[calendar]`, so read both straight from
            // `outl_config` here, in one load. Default transport (`File`)
            // leaves iroh off and the `notify` watcher handles detection on
            // its own.
            let global_cfg = outl_config::load();
            // Resolve the journal/clock timezone once, before any workspace
            // opens and renders today's journal (#107). No `[calendar]
            // timezone` → OS local, as before.
            outl_actions::clock::init(global_cfg.calendar.timezone.as_deref());

            let workspace: Arc<Mutex<Option<Workspace>>> = Arc::new(Mutex::new(None));
            let storage_root: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));

            let registry = Arc::new(RuntimeRegistry::with_builtins());
            let fs_watcher = Arc::new(Mutex::new(None));
            let iroh_transport: Arc<Mutex<Option<Arc<dyn outl_actions::SyncTransport>>>> =
                Arc::new(Mutex::new(None));
            let iroh_pairing: Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>> =
                Arc::new(Mutex::new(None));

            if let Some(path) = last_workspace {
                spawn_workspace_opener(
                    workspace.clone(),
                    storage_root.clone(),
                    fs_watcher.clone(),
                    iroh_transport.clone(),
                    iroh_pairing.clone(),
                    path,
                    hlc.clone(),
                    app.handle().clone(),
                );
            }

            // Plugin host runs on its own thread (Boa `Context` is
            // `!Send`, so it can't live in `AppState`). It shares the same
            // `workspace` / `storage_root` Arcs every command locks, plus
            // the per-device HLC, and loads plugins from
            // `<root>/.outl/plugins/` on its first request once the
            // workspace is open. See `plugin_service.rs`.
            let plugins =
                spawn_plugin_service(workspace.clone(), storage_root.clone(), hlc.clone());

            // Background projection writer: mutations queue their page
            // here so the `.md` + sidecar write happens off the IPC
            // thread (async-writes default). Shares the same workspace
            // slot + swap-capable storage root the commands lock.
            let projection_writer =
                outl_tauri_shared::ProjectionWriter::spawn(workspace.clone(), storage_root.clone());

            app.manage(AppState {
                workspace,
                storage_root,
                hlc,
                settings: Arc::new(Mutex::new(settings)),
                app_config_dir,
                registry,
                fs_watcher,
                iroh_transport,
                iroh_pairing,
                history: Mutex::new(std::collections::HashMap::new()),
                backlink_index: Arc::new(Mutex::new(None)),
                projection_writer,
            });
            app.manage(plugins);
            app.manage(PendingDeepLink(parking_lot::Mutex::new(None)));

            // `outl://` deep links (issue #98). On Linux (and Windows in
            // dev) the scheme is registered at runtime; bundled macOS /
            // Windows builds get it from the `CFBundleURLTypes` /
            // registry entry the plugin writes from tauri.conf.json.
            #[cfg(any(target_os = "linux", all(debug_assertions, windows)))]
            {
                if let Err(err) = app.deep_link().register_all() {
                    tracing::warn!("deep link: register_all failed: {err}");
                }
            }
            // Warm path: a URL opened while the app already runs. The
            // frontend listener is up, so emit straight to it.
            let dl_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    dispatch_deep_link(&dl_handle, url.as_str());
                }
            });
            // Cold start: an `outl://` URL that *launched* the app. The
            // frontend hasn't mounted its listener yet, so an emit here
            // would be lost (the app would just open today's journal).
            // Buffer the target instead; `take_pending_deep_link` drains
            // it once the AppShell mounts. Only the first URL is kept.
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                if let Some(payload) = urls.first().and_then(|u| deep_link_payload(u.as_str())) {
                    *app.state::<PendingDeepLink>().0.lock() = Some(payload);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Workspace lifecycle
            set_workspace,
            current_workspace,
            workspace_stats,
            reload_workspace,
            // Settings
            get_settings,
            update_settings,
            // Theme
            list_themes,
            get_theme,
            // Shortcuts
            list_shortcut_bindings,
            // Reminders (`remind::`)
            list_reminders,
            reminder_settings,
            set_reminder_settings,
            snooze_reminder,
            snooze_presets,
            clear_reminder_snooze,
            set_block_remind,
            mark_block_done,
            set_block_property,
            deliver_due_reminders,
            // Properties (`key:: value`) — catalogue + page-level writer
            known_property_keys,
            set_page_property,
            // Page / journal navigation
            list_all_pages,
            search_pages,
            search_persons,
            search_blocks,
            outl_emoji_search,
            open_today_journal,
            open_journal_for,
            open_page_by_slug,
            open_ref,
            take_pending_deep_link,
            previous_day,
            next_day,
            today_slug_cmd,
            date_title,
            resolve_ref,
            delete_page,
            page_backlinks,
            set_backlinks_order,
            resolve_page_labels,
            // Structural templates
            list_templates_cmd,
            instantiate_template_at,
            // Block mutations
            create_block,
            edit_block,
            split_block,
            toggle_todo,
            toggle_quote,
            delete_block,
            indent_block,
            outdent_block,
            move_block_up,
            move_block_down,
            move_block_after,
            copy_block_markdown,
            paste_block_after,
            set_block_collapsed,
            paste_markdown_at,
            paste_plain_at,
            copy_markdown,
            // Assets (open uploaded file / import a file as a block /
            // read bytes as a data URL for inline render)
            open_asset,
            read_asset_data_url,
            attach_asset,
            import_asset_file,
            // Undo / redo
            undo_page,
            redo_page,
            // Code execution
            run_code_block,
            run_auto_run_blocks,
            resolve_embeds,
            // Peer / device management
            outl_peer_list,
            outl_peer_remove,
            outl_peer_status,
            outl_sync_now,
            outl_peer_pair_host,
            outl_peer_pair_join,
            // Plugins
            plugin_list,
            plugin_run,
            plugin_sync_hooks,
            plugin_keybindings,
            plugin_toolbar,
            plugin_transformers,
            plugin_transform,
            // Marketplace
            plugin_registry_list,
            plugin_install_official,
            plugin_set_enabled,
            plugin_uninstall,
            // Plugin settings (config + secrets)
            plugin_settings_describe,
            plugin_config_set,
            plugin_secret_set,
            plugin_secret_remove,
        ])
        .run(tauri::generate_context!())
        .expect("error while running outl-desktop application");
}
