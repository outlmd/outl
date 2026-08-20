//! outl-mobile — Tauri 2 mobile companion app.
//!
//! Thin glue layer:
//!
//! - **Storage:** `outl_core::JsonlStorage` writes the op log into the
//!   `ops/` directory of a workspace folder **the user picks** — local
//!   by default, optionally inside an iCloud container (see
//!   `workspace_open` and `workspace_picker`). Each device only writes
//!   to its own `ops-<actor>.jsonl`; iroh P2P is the primary sync, and
//!   iCloud (when the chosen folder lives there) syncs the files for
//!   free on top.
//! - **Actions:** delegated wholesale to `outl-actions` so the TUI,
//!   the desktop, and this mobile app all share the same semantics
//!   for edit / indent / outdent / TODO / delete / move / journal /
//!   page / backlinks.
//! - **Tauri commands:** lightweight wrappers split across
//!   `commands::{workspace, page, block, exec}` that parse `String`
//!   ids, call into `outl-actions`, and return the new outline so the
//!   Solid frontend renders in a single round-trip. The split mirrors
//!   `outl-desktop` 1:1 — same module names, same shape — so a
//!   contributor reading either crate immediately knows the other.
//!
//! ## Async startup
//!
//! The Tauri `setup` callback returns immediately so the WebView
//! starts painting right away. Opening the workspace (filesystem
//! reads + op-log replay) runs on a background thread (see
//! `workspace_open::spawn_workspace_opener`); commands that need
//! the workspace return a `workspace_loading` error until it's ready,
//! and the frontend retries on a short interval. As soon as the
//! workspace lands, Tauri emits a `workspace-ready` event the frontend
//! can listen for to refresh proactively.

// Android-only: primes the JVM-backed globals iroh's TLS verifier + DNS reader
// need, via a JNI entry point `MainActivity.onCreate` calls before boot.
#[cfg(target_os = "android")]
mod android_jni;
mod bg_sync;
mod commands;
mod iroh_sync;
mod plugin_service;
mod state;
mod workspace_open;
mod workspace_picker;

use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;
use tauri::{Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;
use tracing::info;

use crate::commands::{
    add_block, attach_asset, copy_block_markdown, copy_markdown, create_block, date_title,
    delete_block, delete_page, edit_block, exec, import_asset_file, indent_block,
    instantiate_template_at, list_all_pages, list_outline, list_templates_cmd, move_block_after,
    move_block_down, move_block_up, next_day, open_asset, open_journal_for, open_page_by_slug,
    open_ref, open_today_journal, outdent_block, outl_emoji_search, outl_peer_list,
    outl_peer_pair_host, outl_peer_pair_join, outl_peer_remove, outl_peer_status, outl_sync_now,
    page_backlinks, paste_block_after, paste_markdown_at, paste_plain_at, plugin_config_set,
    plugin_install_official, plugin_list, plugin_registry_list, plugin_run, plugin_secret_remove,
    plugin_secret_set, plugin_set_enabled, plugin_settings_describe, plugin_sync_hooks,
    plugin_toolbar, plugin_transform, plugin_transformers, plugin_uninstall, previous_day,
    read_asset_data_url, reload_workspace, resolve_page_labels, resolve_ref, search_blocks,
    search_pages, search_persons, set_backlinks_order, set_block_collapsed, split_block,
    today_slug_cmd, toggle_quote, toggle_todo, workspace_stats,
};
use crate::commands::{
    clear_reminder_snooze, deliver_due_reminders, known_property_keys, list_reminders,
    mark_block_done, reminder_settings, set_block_property, set_block_remind, set_page_property,
    set_reminder_settings, snooze_presets, snooze_reminder,
};
use crate::plugin_service::spawn_plugin_service;
use crate::state::AppState;
use crate::workspace_open::{load_or_create_actor, resolve_storage_root, spawn_workspace_opener};

/// A deep link that arrived during cold start, before the frontend
/// mounted its `deep-link://navigate` listener. The frontend drains it
/// once on boot via [`take_pending_deep_link`] (issue #98). Mirrors the
/// desktop buffer; only the cold-start launch URL populates it.
struct PendingDeepLink(Mutex<Option<serde_json::Value>>);

/// Frontend command: take (and clear) the deep link buffered during cold
/// start. Returns `null` when the app launched normally.
#[tauri::command]
fn take_pending_deep_link(pending: tauri::State<'_, PendingDeepLink>) -> Option<serde_json::Value> {
    pending.0.lock().take()
}

/// Parse an `outl://` URL via the shared `outl_actions` parser into the
/// `{kind, …}` payload the frontend maps onto its `open*` commands.
///
/// A malformed URL is logged at `warn` and returns `None` — never a
/// crash, never a stray page (issue #98). The parser is shared with
/// `outl-desktop` so the two clients can't drift on the scheme contract.
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
    // `info!`/`warn!`/`debug!` lines reach the device console (on iOS, stderr
    // surfaces in idevicesyslog / Xcode). Without this the transport runs
    // blind. `RUST_LOG` overrides the default.
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

    // rustls 0.23 needs a process-wide CryptoProvider installed before any
    // rustls consumer builds a client. iroh pulls rustls with
    // `default-features = false`, which drops the feature-selected default
    // provider for the whole workspace — so reqwest (Tauri's asset protocol,
    // serving the webview) panics in `ClientBuilder::build()` at boot. Install
    // `ring` (the provider in our dep graph) explicitly so every rustls user
    // shares it. Ignore the error: a second call just means it's already set.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let builder = tauri::Builder::default()
        // Unified logging (React-Native-style): the frontend logs via
        // `@tauri-apps/plugin-log` + `attachConsole()`, and everything lands on
        // three targets — Stdout (idevicesyslog / terminal), the Webview console,
        // and a rotating file in the OS log dir (`~/Library/Logs/<bundle>/` on
        // macOS; the app's log dir on iOS) so a device build is debuggable
        // without a cable. This owns the `log` crate slot (see the tracing note
        // above); our own `tracing::` events still go to stderr via the
        // subscriber.
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        // OS banners for `remind::` rules. Registering the plugin does
        // not prompt for permission — the frontend asks only when the
        // user turns reminders on.
        .plugin(tauri_plugin_notification::init());

    // Camera/QR scanning is the device-pairing entry point and only
    // compiles on the mobile targets (Android + iOS). Gate it behind
    // `cfg(mobile)` so the desktop/host build of this crate stays clean.
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());

    builder
        .setup(|app| {
            let local_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("app data dir: {e}"))?;
            std::fs::create_dir_all(&local_dir)?;

            let actor = load_or_create_actor(&local_dir)?;
            // Storage is a local folder synced by iroh (no iCloud).
            // `resolve_storage_root` reopens the previously chosen path
            // (`WorkspaceCfg.last`) when present, otherwise defaults to the
            // app-local `<app-data-dir>/outl/`. Change detection is the iroh
            // reload signal — there is no filesystem watcher.
            let global_cfg = outl_config::load();
            let persisted = global_cfg.workspace.last;
            // Resolve the journal/clock timezone before the workspace opener
            // renders today's journal (#107). No `[calendar] timezone` → OS
            // local, as before.
            outl_actions::clock::init(global_cfg.calendar.timezone.as_deref());
            let storage_root = resolve_storage_root(&local_dir, persisted.as_deref());
            std::fs::create_dir_all(&storage_root)?;
            info!("workspace root {}", storage_root.display());
            let hlc = HlcGenerator::new(actor);

            let workspace: Arc<Mutex<Option<Workspace>>> = Arc::new(Mutex::new(None));

            spawn_workspace_opener(
                workspace.clone(),
                storage_root.clone(),
                hlc.clone(),
                app.handle().clone(),
            );

            let registry = Arc::new(RuntimeRegistry::with_builtins());

            // Wire iroh P2P sync. Driven by the `[sync]` section of the
            // global config: iroh is the default on mobile (P2P is the
            // companion app's whole point), and we only fall back to the
            // iCloud file transport when the config explicitly selects
            // `transport = "file"`. A bind/identity failure logs and
            // returns `None` — the app still runs on the native iCloud
            // watcher, so iroh trouble never blocks startup.
            let iroh =
                iroh_sync::wire_iroh_transport(&app.handle().clone(), storage_root.clone(), actor);

            // Plugin host runs on its own thread (Boa `Context` is
            // `!Send`, so it can't live in `AppState`). It shares the same
            // `workspace` Arc every command locks, plus an owned clone of
            // the (fixed-for-the-process) storage root and the per-device
            // HLC, and loads plugins from `<root>/.outl/plugins/` on its
            // first request once the workspace is open. See
            // `plugin_service.rs`.
            let plugins =
                spawn_plugin_service(workspace.clone(), storage_root.clone(), hlc.clone());

            // Background projection writer: mutations queue their page
            // here so the `.md` + sidecar write happens off the command
            // thread (async-writes default). `storage_root` is a fixed
            // `PathBuf` here (folder swap is a relaunch), which is itself
            // a `StorageRootProvider`.
            let projection_writer =
                outl_tauri_shared::ProjectionWriter::spawn(workspace.clone(), storage_root.clone());

            app.manage(AppState {
                workspace,
                hlc,
                storage_root,
                registry,
                iroh,
                backlink_index: Arc::new(Mutex::new(None)),
                projection_writer,
            });
            app.manage(plugins);
            app.manage(PendingDeepLink(Mutex::new(None)));

            // `outl://` deep links (issue #98). iOS routes the URL to the
            // running app via the registered `CFBundleURLTypes` scheme;
            // no single-instance plugin is needed (iOS is single-instance
            // by construction).
            //
            // Warm path: a URL opened while the app already runs. The
            // frontend listener is up, so emit straight to it.
            let dl_handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    dispatch_deep_link(&dl_handle, url.as_str());
                }
            });
            // Cold start: an `outl://` URL that *launched* the app. The
            // frontend hasn't mounted its listener yet, so buffer the
            // target; `take_pending_deep_link` drains it once `Journal`
            // mounts. Only the first URL is kept.
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                if let Some(payload) = urls.first().and_then(|u| deep_link_payload(u.as_str())) {
                    *app.state::<PendingDeepLink>().0.lock() = Some(payload);
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            // Properties (key catalogue + page-level writer)
            known_property_keys,
            set_page_property,
            deliver_due_reminders,
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
            workspace_stats,
            resolve_ref,
            delete_page,
            page_backlinks,
            set_backlinks_order,
            resolve_page_labels,
            // Structural templates
            list_templates_cmd,
            instantiate_template_at,
            // Mutations
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
            set_block_collapsed,
            paste_markdown_at,
            paste_block_after,
            paste_plain_at,
            copy_markdown,
            copy_block_markdown,
            // Assets (open uploaded file / import a file as a block /
            // read bytes as a data URL for inline render)
            open_asset,
            read_asset_data_url,
            attach_asset,
            import_asset_file,
            reload_workspace,
            // Peer / device management
            outl_peer_list,
            outl_peer_remove,
            outl_peer_status,
            outl_sync_now,
            outl_peer_pair_host,
            outl_peer_pair_join,
            // Plugins (JS host on a dedicated thread — Boa is !Send)
            plugin_list,
            plugin_run,
            plugin_sync_hooks,
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
            // Workspace folder selection (choose where the workspace lives)
            workspace_picker::set_workspace,
            // Code execution
            exec::run_code_block,
            // Legacy
            list_outline,
            add_block,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
