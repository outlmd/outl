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
//! - `webview_boot` — Windows WebView2 checks: exit on missing runtime, retry a window that fails to create (OUTL-4).
//! - `display_boot` — Linux: exit with a message when GTK cannot open a display (OUTL-6).
//! - `open_with` — the OS "Open With → outl" gesture (macOS
//!   `RunEvent::Opened`, `argv` elsewhere) turned into one import.
//! - `workspace_open` — open / reconcile / boot opener primitives.
//! - `commands` — Tauri command surface split by responsibility
//!   (`workspace`, `page`, `block`).

mod about;
mod commands;
mod deep_link;
mod display_boot;
mod fs_watcher;
mod helpers;
mod iroh_sync;
mod links;
mod log_target;
mod menu;
mod open_with;
mod plugin_service;
mod quick_capture;
mod settings;
mod state;
mod webview_boot;
mod window_paint;
mod workspace_open;

use std::path::PathBuf;
use std::sync::Arc;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_exec::RuntimeRegistry;
use parking_lot::Mutex;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;

use crate::commands::{
    attach_asset, backlink_row_op, boot_mark, clear_reminder_snooze, copy_block_embed,
    copy_block_markdown, copy_block_ref, copy_markdown, create_backlink_sibling, create_block,
    current_workspace, cut_block, date_title, delete_block, delete_page, deliver_due_reminders,
    device_activity, drain_capture_inbox, edit_backlink_block, edit_block, export_markdown,
    format_remind_rule, get_settings, get_telemetry_enabled, get_theme, get_theme_config,
    import_asset_file, indent_block, instantiate_template_at, journal_days_with_content,
    known_property_keys, list_action_support, list_all_pages, list_insert_commands,
    list_open_tasks, list_reminders, list_shortcut_bindings, list_templates_cmd, list_themes,
    list_trash, mark_block_done, merge_pages, merged_edits, move_block_after, move_block_down,
    move_block_up, next_day, open_asset, open_external_file, open_journal_for, open_page_by_slug,
    open_ref, open_today_journal, outdent_block, outl_emoji_search, outl_peer_list,
    outl_peer_pair_host, outl_peer_pair_join, outl_peer_remove, outl_peer_status, outl_sync_now,
    page_backlinks, page_timeline, parse_remind_rule, paste_block_after, paste_markdown_at,
    paste_plain_at, plugin_config_set, plugin_install_official, plugin_keybindings, plugin_list,
    plugin_registry_list, plugin_run, plugin_secret_remove, plugin_secret_set, plugin_set_enabled,
    plugin_settings_describe, plugin_sync_hooks, plugin_toolbar, plugin_transform,
    plugin_transformers, plugin_uninstall, previous_day, read_asset_data_url, redo_page,
    reload_workspace, reminder_settings, rename_page, resolve_embeds, resolve_insert_command,
    resolve_page_embeds, resolve_page_labels, resolve_ref, restore_trash_entry,
    run_auto_run_blocks, run_code_block, search_blocks, search_pages, search_persons,
    set_backlinks_order, set_block_collapsed, set_block_property, set_block_remind,
    set_page_property, set_reminder_settings, set_telemetry_enabled, set_workspace, snooze_presets,
    snooze_reminder, split_block, today_slug_cmd, toggle_pin, toggle_quote, toggle_todo,
    undo_merged_edit, undo_page, unfold_ancestors, update_settings, wait_workspace_ready,
    workspace_stats,
};
use crate::deep_link::{
    deep_link_payload, dispatch_deep_link, take_pending_deep_link, PendingDeepLink,
};
use crate::plugin_service::spawn_plugin_service;
use crate::state::AppState;
use outl_tauri_shared::workspace_open::WorkspaceGuards;

use crate::workspace_open::{load_or_create_actor, spawn_workspace_opener};

/// Repeat Rust's `SIG_IGN` for `SIGPIPE` so a library cannot restore the
/// default and kill a GUI on the next write to a closed pipe.
#[cfg(unix)]
fn ignore_sigpipe() {
    // SAFETY: `signal` is async-signal-safe. `SIG_IGN` is a valid disposition
    // and does not run user code. The return value is the previous disposition;
    // `SIG_ERR` is the only failure, and leaving the runtime's existing
    // `SIG_IGN` in place is the outcome we already wanted.
    unsafe {
        let _ = libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

#[cfg(not(unix))]
fn ignore_sigpipe() {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Before anything that writes. Rust already ignores SIGPIPE; a library
    // that restores the default disposition would turn the next write to a
    // closed pipe into a kill, which a GUI launched with no terminal hits
    // the moment its log pipe goes away. The write path still has to tolerate
    // EPIPE — that is `outl_telemetry::lossy_stderr`, below.
    ignore_sigpipe();

    // Windows only, before the crash reporter. A failed `webview_version()`
    // is the setup panic; here it is a message and an exit.
    #[cfg(windows)]
    webview_boot::exit_if_runtime_missing(tauri::webview_version().map_err(|err| err.to_string()));

    // Linux only, same place for the same reason: no display is tao's
    // `Failed to initialize gtk backend!` panic (OUTL-6), not a crash.
    #[cfg(target_os = "linux")]
    display_boot::exit_if_display_unavailable();

    // Crash reporting (RFC 0377) goes up next, before the subscriber and
    // before the crypto provider, because the crashes worth the most are the
    // ones during boot — the Android app that closed on open is what motivated
    // this, and a reporter installed after the failing call reports nothing.
    //
    // `init` returns `None` unless all four conditions hold. A build without
    // `--features sentry` links no SDK, so this is a no-op in every build but
    // the release pipeline's.
    //
    // Named binding, never `let _ =`: the guard flushes pending events when
    // dropped, and `let _ =` would drop it on this line.
    let _telemetry = outl_telemetry::init("desktop", outl_config::telemetry_enabled());

    // Install a `tracing` subscriber before anything that logs, so the iroh P2P transport's
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
        // `std::io::stderr` plus the fmt layer's default `log_internal_errors`
        // panics when the pipe is closed: the layer reports the failed write
        // with `eprintln!`, and that macro turns EPIPE into
        // `failed printing to stderr`. A desktop with no terminal hits this
        // on the first log line.
        .with_writer(outl_telemetry::lossy_stderr)
        .log_internal_errors(false)
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);

    // rustls 0.23 needs a process-wide CryptoProvider before any rustls
    // consumer builds a client. iroh pulls rustls with `default-features =
    // false`, dropping the workspace-wide default provider — so reqwest
    // (Tauri's webview asset protocol) panics in `ClientBuilder::build()` at
    // boot. Install `ring` (the provider in our dep graph) explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // `PendingOpenFile` is managed here, not in `setup`: on macOS the
    // "open documents" Apple Event can reach `RunEvent::Opened` before
    // `setup` has run (and `setup` can bail on a `?`), and a state that
    // is not managed makes `Manager::state` panic.
    //
    // `menu::install` is a no-op off macOS, see `menu.rs`.
    menu::install(open_with::install(tauri::Builder::default()))
        // Single-instance MUST be the first plugin. Its `deep-link`
        // feature forwards an `outl://` URL opened while the app is
        // already running to `on_open_url` (Linux/Windows); the callback
        // only needs to surface the existing window. macOS routes the
        // URL to the running instance natively via Apple Event.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // Linux / Windows warm path for "Open With": the second
            // process's `argv` carries the file. macOS never routes a
            // file this way — it sends an Apple Event, which arrives as
            // `RunEvent::Opened` below — so this is additive, not a
            // second owner of the same delivery.
            //
            // `route`, not `dispatch`: "warm" means this process is
            // running, not that its webview has mounted. Open a file
            // during a slow workspace boot and the loading screen is
            // still up, with no listener to receive the emit.
            for path in open_with::file_paths_from_argv(argv) {
                open_with::route(app, &path);
            }
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.set_focus();
            }
        }))
        // Unified logging (React-Native-style): the frontend logs via
        // `@tauri-apps/plugin-log` + `attachConsole()`, and everything lands on
        // three targets — stdout (the terminal running `cargo tauri dev`; a
        // closed pipe is ignored, see `log_target`), the Webview console,
        // and a rotating file in the OS log dir
        // (`~/Library/Logs/<bundle>/` on macOS, `%APPDATA%/<bundle>/` on
        // Windows, `~/.config/<bundle>/` on Linux). This owns the `log` crate
        // slot (see the tracing note above); our own `tracing::` events still go
        // to stderr via the subscriber. Registered after single-instance, which
        // must stay first.
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    log_target::stdout_target(),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
                    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                        file_name: None,
                    }),
                ])
                .level(log::LevelFilter::Info)
                .build(),
        )
        // A page load tears the frontend's `open-file://import` listener
        // down with it. Clear the flag so a file opened mid-reload is
        // buffered for the remount instead of emitted into nothing —
        // the cold-start loss this buffer exists to prevent, arriving
        // through a different door.
        .on_page_load(|webview, payload| {
            if payload.event() == tauri::webview::PageLoadEvent::Started
                && webview.label() == "main"
            {
                if let Some(pending) = open_with::pending(webview.app_handle()) {
                    pending.listener_gone();
                }
            }
        })
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_deep_link::init())
        // OS banners for `remind::` rules. Registering the plugin does
        // not prompt for permission — the frontend asks only when the
        // user turns reminders on in Settings.
        .plugin(tauri_plugin_notification::init())
        .plugin(quick_capture::plugin())
        .setup(|app| -> Result<(), Box<dyn std::error::Error>> {
            #[cfg(windows)]
            webview_boot::open_main_window(app.handle());
            quick_capture::register(app.handle());
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
            // `[calendar] week_start` cached for `workspace_stats` (issue #94).
            // `None` keeps the desktop's historical Monday; restart-to-apply,
            // matching the timezone line above and the TUI.
            let calendar_week_start = global_cfg.calendar.week_start;

            // Repaint the native window from the theme on disk, before
            // the webview draws. See `window_paint`.
            window_paint::repaint_windows(app.handle(), &global_cfg.theme);

            let workspace: Arc<Mutex<Option<Workspace>>> = Arc::new(Mutex::new(None));
            let storage_root: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
            // Cross-process workspace locks, held for as long as a
            // workspace is open. Written only by `open_workspace_at`.
            let workspace_guards: Arc<Mutex<Option<WorkspaceGuards>>> = Arc::new(Mutex::new(None));
            let backup_root = workspace_open::start_backup(&global_cfg.backup);

            let registry = Arc::new(RuntimeRegistry::with_builtins());
            let fs_watcher = Arc::new(Mutex::new(None));
            let iroh_transport: Arc<Mutex<Option<Arc<dyn outl_actions::SyncTransport>>>> =
                Arc::new(Mutex::new(None));
            let iroh_pairing: Arc<Mutex<Option<outl_sync_iroh::IrohSyncTransport>>> =
                Arc::new(Mutex::new(None));
            let (first_paint, workspace_ready) = workspace_open::start_boot_phase();

            if let Some(path) = last_workspace {
                spawn_workspace_opener(
                    workspace.clone(),
                    storage_root.clone(),
                    workspace_guards.clone(),
                    backup_root.clone(),
                    fs_watcher.clone(),
                    iroh_transport.clone(),
                    iroh_pairing.clone(),
                    path,
                    hlc.clone(),
                    app.handle().clone(),
                    first_paint.clone(),
                    workspace_ready.clone(),
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

            let projection_writer =
                workspace_open::spawn_projection_writer(&workspace, &storage_root, app.handle());

            app.manage(AppState {
                workspace,
                storage_root,
                workspace_guards,
                backup_root,
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
                first_paint,
                workspace_ready,
                calendar_week_start,
            });
            app.manage(plugins);
            app.manage(PendingDeepLink(parking_lot::Mutex::new(None)));

            // Cold start on Linux / Windows: the file the user chose
            // "Open With → outl" for is an argument. Buffer it — the
            // frontend drains it in `AppShell`'s `onMount`, the same
            // way it drains a cold-start deep link, because an emit
            // here would land before any listener exists.
            if let Some(pending) = open_with::pending(app.handle()) {
                for path in open_with::file_paths_from_argv(std::env::args()) {
                    pending.offer(path);
                }
            }

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
            boot_mark,
            wait_workspace_ready,
            reload_workspace,
            // Settings
            get_settings,
            update_settings,
            // Privacy (crash reporting) — its own pair, deliberately not
            // part of `update_settings`: the flat `Settings` does not
            // model the telemetry section and restores it from disk on
            // save. (A comment in this list must carry no closing square
            // bracket: `command_parity.rs` ends the list at the first one
            // it finds, so a bracket here hides every command after it.)
            get_telemetry_enabled,
            set_telemetry_enabled,
            // Theme
            list_insert_commands,
            list_themes,
            resolve_insert_command,
            get_theme,
            get_theme_config,
            // Shortcuts
            list_shortcut_bindings,
            list_action_support,
            // Reminders (`remind::`)
            list_reminders,
            reminder_settings,
            set_reminder_settings,
            snooze_reminder,
            snooze_presets,
            clear_reminder_snooze,
            set_block_remind,
            parse_remind_rule,
            format_remind_rule,
            mark_block_done,
            set_block_property,
            deliver_due_reminders,
            // Page history, read out of the op log (read-only)
            page_timeline,
            // Trash: what was deleted, and putting one back
            list_trash,
            restore_trash_entry,
            device_activity,
            export_markdown,
            merged_edits,
            undo_merged_edit,
            // Capture inbox: lines `outl capture` left for this app to land
            drain_capture_inbox,
            // Properties (`key:: value`) — catalogue + page-level writer
            known_property_keys,
            set_page_property,
            // Page / journal navigation
            list_all_pages,
            search_pages,
            search_persons,
            search_blocks,
            list_open_tasks,
            journal_days_with_content,
            outl_emoji_search,
            open_today_journal,
            open_journal_for,
            open_page_by_slug,
            open_ref,
            open_external_file,
            take_pending_deep_link,
            crate::open_with::take_pending_open_file,
            previous_day,
            next_day,
            today_slug_cmd,
            date_title,
            resolve_ref,
            delete_page,
            rename_page,
            merge_pages,
            page_backlinks,
            set_backlinks_order,
            resolve_page_labels,
            // Structural templates
            list_templates_cmd,
            instantiate_template_at,
            // Block mutations
            create_block,
            edit_block,
            edit_backlink_block,
            // Cross-page: the same gestures aimed at a backlink row's
            // own page, where a stale id is refused instead of
            // re-targeted (`commands/backlink_edit.rs`).
            backlink_row_op,
            create_backlink_sibling,
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
            unfold_ancestors,
            paste_markdown_at,
            paste_plain_at,
            copy_markdown,
            cut_block,
            copy_block_ref,
            copy_block_embed,
            // Pages
            toggle_pin,
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
            resolve_page_embeds,
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
        .build(tauri::generate_context!())
        .expect("error while building outl-desktop application")
        .run(|app, event| {
            // macOS delivers "Open With" as an Apple Event, which Tauri
            // surfaces here — for a cold launch *and* for a running
            // app, so this one arm covers both. `file_path_from`
            // filters to `file://`, which is what keeps an `outl://`
            // deep link (same event) from being navigated twice.
            //
            // `route` decides buffer-vs-emit from whether the frontend
            // has actually announced itself. It must not be inferred
            // from the window existing: `setup` creates that before the
            // event loop starts, so every cold-start file looked warm
            // and was emitted into a webview that had not mounted.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                for url in urls {
                    crate::open_with::route(app, url.as_str());
                }
            }
            let _ = (app, event);
        });
}
