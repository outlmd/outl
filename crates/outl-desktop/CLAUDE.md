# CLAUDE.md — outl-desktop

Tauri 2 desktop client (macOS, Linux, Windows).
Solid + Tailwind frontend, Rust backend that **must stay thin** — every workspace operation delegates to `outl-actions`.

Feature-by-feature behaviour — what a surface shows, what a popup inserts, what a panel lists — is [`docs/desktop-ux.md`](../../docs/desktop-ux.md).
This file keeps the rules: why something is the way it is, and the regression each guard bought.

## Status

**Feature-complete v0**, with `desktop.yml` CI, signed bundles and the Homebrew cask on top.
Per-feature state is [`docs/client-parity.md`](../../docs/client-parity.md), which is generated.

## Layering

```text
outl-core      (CRDT, op log, storage trait)
outl-md        (.md parse/render, sidecar)
outl-actions   (workspace operations + SyncEngine, shared with TUI + mobile)
   ↑
outl-desktop   src-tauri/ (commands: parse args → outl-actions → render)
               src/       (Solid + Tailwind + @outl/shared)
```

## Hard rule

**This crate adds no business logic.**
Anything involving the workspace shape (edit, move, todo, journal render) delegates to `outl-actions` — root `CLAUDE.md` → "Shared logic" is the owner of that rule.
Same rule on the frontend: before writing a helper under `src/lib/`, check `@outl/shared` ([`crates/outl-frontend-shared/CLAUDE.md`](../outl-frontend-shared/CLAUDE.md)).
It owns the inline-token renderer, paste detection, ref autocomplete, the DTO types and the shared command wrappers.

**Do not hand-write a `#[tauri::command]` wrapper.**
Every purely-delegating command module here is a single macro invocation — `outl_tauri_shared::block_commands!(crate::state::AppState);` and friends — declared once in that crate's `wrappers/catalog.rs` so both clients get it.
Which stay hand-written (those needing more than `State<'_, AppState>`), why a client takes a whole module, and the parity test: [`outl-tauri-shared/CLAUDE.md`](../outl-tauri-shared/CLAUDE.md) → "One command surface, not two".

What this crate **does** own: path discovery, the desktop-only command surface, the Solid 3-pane frontend, and an `AppState` holding a `ProjectionWriter` for async `.md` + sidecar writes.
Plus the cross-platform `notify` FS watcher that signals the frontend when peer `ops-*.jsonl` files grow — what replaces the `NSMetadataQuery` dance iOS forces on mobile.
`run_code_block` is a **thin adapter**: the orchestration is `outl_actions::exec`, shared with mobile, so adding behaviour to `commands/exec.rs` is almost always a smell.

## Layout

The frontend / `src-tauri` file tree (which component owns what, where each `lib/*.ts` helper lives) is in [`docs/development.md`](../../docs/development.md#desktop-crate-layout).

Two parts of this client are reference rather than rule, and they live under `docs/`:

| Page | What it covers |
|---|---|
| [`docs/desktop-keybindings.md`](../../docs/desktop-keybindings.md) | How a keystroke resolves, group by group: the `action-handlers.ts` assembly, keyboard reach, undo / redo, inline markdown, paste, block-editor chords, vim parity |
| [`docs/desktop-page-surface.md`](../../docs/desktop-page-surface.md) | The `BlockRow.tsx` split, the sections below the outline and their loading rule, the cursor regions, the property editor, the history panel, `applyPageState`, the day clock, zoom, the task panel |

Read the matching one before editing those modules.

## First-run onboarding

`components/Onboarding.tsx` is the flow and `App.tsx` gates between it and `<AppShell />`; the two steps are in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#first-run).

The "has the user onboarded" flag is a **per-install UI flag in `localStorage`** (`outl.onboarded`), **not** workspace state — it deliberately does NOT go through the op log.
It is intentionally not in `settings.json` either, since `settings.last_workspace` is the only first-run signal the backend tracks.
The **copy** lives in `@outl/shared/onboarding`, identical to mobile, and pairing is **not** reimplemented — `Onboarding` renders the real `<SyncPanel />`.

## "This page isn't syncing" banner

`<PageAheadOfLogBanner client="desktop" />` (`@outl/shared/warnings`) renders above the outline when `PageView.md_ahead_of_log` comes back set.
`aheadOfLogNotice` owns the copy; `client="desktop"` is what makes it name `outl reconcile --ahead-of-log`.
What the condition means and why it has to reach the user: [`docs/clients.md` → Surfacing a page that stopped syncing](../../docs/clients.md#surfacing-a-page-that-stopped-syncing).

`appState.mdAheadOfLog` is sticky per page (`stickyAheadOfLog`), because only the open commands carry the flag — a mutation reply would otherwise clear the banner on the user's first edit, the very action it warns against.
Sticky stops at `PageView.md_ahead_of_log_checked`: a reply that ran the check is authoritative both ways, so the next open after a reconcile clears the banner instead of leaving a healthy page marked broken.

## What a launch waits for

**The loading screen paints today's journal, not a spinner.**
`App.tsx` subscribes to `onPagePreview` before any `await` (the backend emits it before it opens the op log, so a listener armed after the theme round-trip would miss it on a fast workspace) and renders `<PreviewOutline />` in place of "Opening your workspace…".
The spinner stays as the fallback for a launch with nothing to preview, and a small line above the content is what stops the read-only state reading as a frozen app.
Rules the preview carries: [`docs/boot-performance.md`](../../docs/boot-performance.md).

The boot opener publishes the workspace, emits `workspace-ready`, latches `WorkspaceReady`, and stops.
The filesystem watcher, the iroh transport, the orphan reconcile and the snapshot refresh are registered on `outl_tauri_shared::FirstPaintGate` and start when the frontend reports `first_content`.

`workspace_open::spawn_interactive_phase` is where that registration lives, and [`docs/boot-performance.md`](../../docs/boot-performance.md) owns why each one waits.

Four things in this crate that are easy to undo:

- **`setup` arms the gate's deadline (4 s), and must keep doing so.**
  The painted signal comes over IPC; a webview that fails to load leaves it unsent, and without the deadline this window would sync with nobody and say nothing.
- **Deferring the watcher opens a window, and the fix is to ask the log, not to start earlier.**
  A watcher only reports what happens after it starts, so the `fs_watcher` job asks `ops_watermark::read_needed` once it is up and emits a single `peer-ops-changed` when a peer appended in between.
- **`App.tsx` waits instead of polling.**
  The 500 ms `setInterval` is gone; `waitWorkspaceReady` resolves when the opener publishes. The loop around it keeps the old ~20 s budget and still calls `refresh()`, because a rejected wait (an older backend) has to fall back to asking.
- **`window_paint::window_background` repaints the native window from the theme on disk.**
  `tauri.conf.json`'s static `backgroundColor` covers the brand default; this is what stops a light-theme user opening outl with a dark rectangle. It **cannot** resolve `mode = "auto"` (the OS appearance setting is the webview's question), so `auto` takes the dark side and `index.html`'s cached-palette script corrects it one frame later.
  **`tauri.windows.conf.json` mirrors the window entry**, and `webview_boot`'s parity test fails when it does not.

The four marks the frontend reports (`html_parsed` in `index.tsx`, `theme_applied` after `installTheme`, `first_content` off whichever cold-start target won, `interactive` at the end of `AppShell`'s mount) are `@outl/shared/boot`'s.
`first_content` rides the deep-link and open-file paths too, and fires even when the target **failed**: the shell is mounted and the error is on screen, so holding the mark back would hold sync behind a state nothing resolves on its own.

## Theme tokens

`applyPaletteToRoot` (`@outl/shared/theme`) writes only the canonical `--color-outl-*` namespace, and `src/styles.css`'s `@theme` block declares only that one.
The legacy `--color-ios-*` / `--color-iosd-*` set is gone, which is what stops the OS appearance setting from changing markdown block elevation through Tailwind's `dark:` variant (invariant 13, RFC 0022).
Details: [`docs/theming.md`](../../docs/theming.md#desktop-css-custom-property-namespaces).
`commands/theme.rs` is a thin wrapper over `outl_tauri_shared::commands::theme`, so mobile registers the identical two commands rather than hardcoding palette values.

## Running and tests

Dev + bundle commands, why Vite runs on port 1421, and what a terminal-launched dev build logs: [`docs/development.md`](../../docs/development.md#desktop-tauri-2).
Test layers, tooling and the frontend suites: [`docs/development.md`](../../docs/development.md#per-client-test-suites).

## Reminders (`remind::`)

`<RemindersPanel />` lists them, `set_block_remind` authors a rule, `<AppShell />` polls `deliver_due_reminders` every 30s.
Panel wiring, the chord choices and the `[reminders] enabled` default: [`docs/reminders.md`](../../docs/reminders.md#desktop-outl-desktop).
The one rule here: **nothing about when a reminder fires is computed in the frontend** — the instants come from `outl_actions::reminders`, the labels from `@outl/shared`.

## Trust and polish

- The saved mark: `savedMarks` in `lib/store.ts`, written when `edit_block` resolves and rendered by `BlockRow` as a fading dot.
  Do not mark before the command resolves — the mark means "the op is on disk".
- Focus mode is **not an `Action` yet**: `support.rs` and `defaults.rs` sit at their file-size baseline, so a chord waits for that split.
- Motion, focus mode and the first-run tips come from `@outl/shared/polish`: [`docs/desktop-ux.md`](../../docs/desktop-ux.md#motion-focus-mode-first-run-tips).

## Plugins

The effect layer, the toolbar and the view surfaces: [`docs/plugins.md`](../../docs/plugins.md) → "Desktop plugin surface".

## Logging

`run()` in `src-tauri/src/lib.rs` ignores `SIGPIPE`, probes the Windows webview runtime, calls `outl_telemetry::init`, then installs a `tracing_subscriber` fmt subscriber.

**The Windows probe comes before `init` on purpose**, so the crash reporter is not running on the path that exists to avoid a report.
`src-tauri/src/webview_boot.rs` owns both Windows boot failures: a missing WebView2 runtime, and a present runtime that times out handing Tauri a controller (OUTL-4).
Those, the sentences the user gets, and the rules that are easy to undo: [`docs/development.md`](../../docs/development.md#why-the-desktop-crate-has-windows-only-boot-code).
Both decisions are taken against traits, not `tauri::AppHandle` — the only reason either is testable off Windows, so do not inline them back.

The subscriber writes through `outl_telemetry::lossy_stderr` with `log_internal_errors(false)`.
A closed stderr — a desktop launched with no terminal — must not panic, and `eprintln!` turns `EPIPE` into `failed printing to stderr`, including in the fmt layer's own error report.
The `EnvFilter` defaults to `info,outl_sync_iroh=debug,iroh=info` and honors `RUST_LOG`; what a terminal-launched dev build shows is in [`docs/development.md`](../../docs/development.md#desktop-tauri-2).
Init uses `set_global_default`, not `.try_init()`: `try_init` also claims the `log` slot, which `tauri-plugin-log` needs.

## Settings

Stored through `outl-config` at the shared global config path; the flat JSON DTO `settings.rs::Settings` hands the frontend, field by field, is in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#settings-modal).

`settings.rs::restore_unmodeled_sections` lists only fields the modal does not own.
**A new unmodeled config field omitted there will get silently dropped on the next modal save.**
`backlinks_order` is the same pattern from the other side: it rides the DTO but `save` restores it from disk, so the modal cannot clobber the dedicated `set_backlinks_order` command's write.
`managed` is one of those: it belongs to whoever installed the file (Nix/home-manager), so `restore_unmodeled_sections` restores it from disk and `update_settings` pins the DTO field to `outl_config::managed()` before storing — the client's `managed` value is display-only and can never reach the file. When set, `outl_config::save` is itself a no-op, `settings::load` carries the flag, and `SettingsModal` shows a notice and disables Save (the theme picker still previews live). See [`docs/nix.md`](../../docs/nix.md#declarative-config).

### Privacy: the crash-reporting toggle writes outside Save

`SettingsModal`'s **Privacy** section (RFC 0377) is the one row that does not live in `draft()`.
`[telemetry] enabled` is not in the flat `Settings`, and `restore_unmodeled_sections` puts the section back from disk on every save.
So routing it through Save would make the toggle inert, and dropping the restore would flip a user's opt-out back on with every unrelated save, because the default is `true`.
It reads `getTelemetryEnabled()` on every open and writes `setTelemetryEnabled()` on change, the same split the backlinks direction button has.
On open rather than `onMount`, because `AppShell` mounts it for the app's lifetime and another process can write the flag while the window runs (`outl telemetry off` in a terminal).
The switch moves only on the command's reply: an optimistic flip that fails to save leaves a checkbox reading "off" over a reporter that is still on.
The words are `CRASH_REPORTS_SETTING` (`@outl/shared/onboarding`), never written here.

**This section is where a user *looks*; it is not where they are *told*.**
Reporting is on by default, so a disclosure that lives only here reaches only the curious.
The proactive half is `<CrashReportsNotice />` (`@outl/shared/polish`, RFC 0377 §6), mounted at the **shell**, not above the outline, because per page it would re-run its dismissed check on every navigation.

## Peers

`commands/peers.rs` carries no business logic: `outl_sync_iroh::PeersStore` owns the store, and `peers.json`'s per-graph location is in [`docs/desktop-ux.md`](../../docs/desktop-ux.md#paired-devices).
`outl_sync_now()` just calls `Arc<dyn SyncTransport>`'s trait method — the trigger behind Refresh ([panel](../../docs/desktop-ux.md#sync-panel)).
What that pass actually does: [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "Force-sync trigger (`sync_now`)".

The always-visible capsule dot ([states](../../docs/desktop-ux.md#sync-status-dot)) and the panel's derive reachability from the same `peersOnline` (`@outl/shared/peers`) mobile uses.
**Do not add a second reachability path**; `peersOnline` is the one owner.
The same module owns the state built on top of it — `syncState` (checking / online / offline / syncing), `syncStateLabel`, `syncStateColor` and the `<SyncDot>` glyph — because it did not, and the two clients had already drifted: this one started dim on `null` and mobile started orange on `false`, so a phone claimed "offline" over a probe that had not answered yet ([#575](https://github.com/outlmd/outl/issues/575)).
`<SyncIndicator />` keeps only the chrome: the button, the click-to-sync pass, the right-click into Settings, and the "last synced" half of the tooltip.
**The button's `aria-label` says what pressing it does, never what the dot shows** — `<SyncDot>` is the live region that announces the state, and putting it in both reads it out twice on every change.
When adding a control to the top-right capsule, keep it in the same order as mobile's top bar.

### When this window does not hold the device endpoint

A device binds **one** iroh endpoint, and a lease decides which process gets it — not being the GUI ([`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "One endpoint per identity, elected not assigned").
So the desktop can run with `iroh_transport` / `iroh_pairing` empty while sync works fine through the shared `ops/` dir; a co-resident `outl mcp serve` that started first is the ordinary case, since Claude Desktop launches it at login.

`iroh_sync::no_endpoint_reason()` separates why those slots can be empty, because the cases deserve opposite answers.
`P2pDisabled` is the user's opt-out, `HeldByAnotherProcess` lost the election, `Unavailable(why)` could not arbitrate the lease or failed to build the transport.
It is rewritten on every `wire_iroh_transport`, so a workspace swap that wins the endpoint clears the previous warning.
Only `P2pDisabled` refuses to pair; collapsing the others into it told a user whose transport failed to build to switch on a setting that was already on.

- **Pairing** falls back to the CLI's one-shot `host_pairing` / `join_pairing`, which bind their own endpoint and close it before returning.
  For those seconds it **does** take the relay route from the lease holder.
  Accepted deliberately: pairing is rare, user-initiated and short, the holder recovers when the one-shot closes, and the alternative is a user who cannot add a device at all.
  It is not a precedent — no other path in the GUI may bind an endpoint.
  With P2P simply **off** (`transport = "file"`) pairing is **refused** instead, because binding there would override the setting the user picked.
- **Refresh** returns an error naming the reason rather than an `Ok(())` that did nothing.
  The silent no-op was the actual defect: the dot stays orange, Refresh appears to work, nothing says why.
  `transport = "file"` stays a quiet no-op — the user's own choice is not a degraded state.
  It also **re-runs the election** first (`retry_endpoint`): the recorded reason is a snapshot of login, nothing here notices the holder exiting, so without that the window would explain a dead process all session.
  It re-contends only from `HeldByAnotherProcess` / `Unavailable` — `P2pDisabled` is a setting rather than a race, and a `None` reason with no transport means the boot opener has not wired yet and a second pass would race it for the same lease.

**Still missing:** the status dot cannot tell "every device is offline" from "another local process holds the endpoint".
Teaching it that state means changing `PeerStatusDto` in [`outl-tauri-shared`](../outl-tauri-shared/CLAUDE.md), not a desktop-local patch, because `peersOnline` is the one owner of reachability on both clients.
Until then the Refresh error is the only surface that explains it.
Note this is **not** what kept the dot orange on a window that did hold the endpoint: that was the responder recording nothing, fixed in `outl-sync-iroh` ([#575](https://github.com/outlmd/outl/issues/575)).

### `reload_workspace` offloads the replay off the IPC thread

The desktop command is a thin caller of `outl_tauri_shared::reload_workspace_into`.
That owns the offload, the publish-only-what-it-cannot-have-dropped retry, and why both exist: [that crate's CLAUDE.md](../outl-tauri-shared/CLAUDE.md) → "A reload publishes only what it cannot have dropped".
Only the cheap tail — history invalidation, the `Mutex` swap, the reconcile spawn — runs on the command thread here.

## "Open With → outl" (`.md` / `.txt` from the OS)

`src-tauri/src/open_with.rs` is the plumbing and all this crate adds, alongside `bundle.fileAssociations`' deliberate `role: "Viewer"` + `rank: "Alternate"`.
The namespace, the re-open rule, the refusals and the three delivery paths belong to `outl_actions::open_with` and [`docs/client-outside-the-app.md`](../../docs/client-outside-the-app.md#opening-a-file-from-the-os-open-with--outl).

Three things a contributor will trip on:

- **`RunEvent::Opened` is also where `outl://` deep links arrive on macOS.**
  `file_path_from` keeps only `file://`; without that filter a deep link would be navigated twice — once by the `deep-link` plugin, once here.
- **"Is the frontend listening" is a flag the frontend sets, never inferred, and it comes back down.**
  `setup` creates the main window inside `build()`, before the event loop starts, so *every* `RunEvent::Opened` sees a window.
  An earlier version read that as "warm" and emitted every cold-start file into a webview that had not mounted; the emit was lost and the app silently opened today's journal instead of the file the user double-clicked (issue #98).
  `take_pending_open_file` marks the flag and `AppShell` subscribes before calling it; both paths go through `route`, warm ones included, since a slow boot keeps the loading screen up long after the process is "running".
  `on_page_load` clears it again on `PageLoadEvent::Started`: a reload takes the listener with it while the process keeps running, and a latch that only goes up tells the same lie one layer in.
  `onMount` drains the open-file and deep-link buffers together — the deep link wins when both are set, but the other drains anyway, so an unread target cannot replay later.
- **`PendingOpenFile` is managed on the `Builder` (`open_with::install`), never inside `setup`, and every lookup goes through `open_with::pending`.**
  macOS can deliver `RunEvent::Opened` before `setup` finishes, and `Manager::state` panics on an unmanaged type (OUTL-5).
  `pending` uses `try_state` and logs, so an OS event costs one file at worst.
  Net: `a_file_routed_before_setup_runs_is_buffered_not_a_panic`.
- **`run()` is `.build(…).run(|app, event| …)` now, not `.run(generate_context!())`.**
  The event callback is the only place `RunEvent::Opened` is reachable.
  Do not collapse it back.

## Application menu

`src-tauri/src/menu.rs` builds the native menu on **macOS only** (`menu::install`, a no-op elsewhere).
Tauri's default menu is macOS-only, so Windows and Linux had no menu bar before; installing one there would be new chrome.
On macOS the default's File menu had only "Close Window" while Settings pointed users at "File → Switch Workspace…".
Windows and Linux switch from the **Switch…** button in `SettingsModal.tsx`, which calls the same `switchWorkspace`.
Behaviour and the frontend half: [`docs/desktop-ux.md`](../../docs/desktop-ux.md#switch-workspace--file-menu-macos-and-settings-button).

- **The Rust side only emits `menu://switch-workspace`.**
  The dialog, `set_workspace` and the store reset are `lib/switch-workspace.ts`, so no workspace logic sits behind a menu click or a button.
- **`build` and `on_event` are `cfg(target_os = "macos")`; `event_for` and the constants are not**, so Linux CI still runs `menu.rs`'s tests.
  Do not add a Windows / Linux branch to `build`.
- **It replaces the default menu, so it has to carry what the default carried.**
  The Edit items are what route `Cmd+C` / `Cmd+V` / `Cmd+A` to the webview; dropping them breaks text editing.
  A new predefined item belongs in `build`, not in a second menu.
- **The label is written twice** (`SWITCH_WORKSPACE_LABEL` and `switchWorkspaceHint` in `lib/switch-workspace.ts`); `menu.rs`'s test pins the Rust side.
  The hint must not mention a File menu off macOS.
- **No chord, no palette row.**
  Either needs an `Action`, and an `Action` needs a verdict from every client in `outl_shortcuts::support`.
- **A click resolves to a `MenuAction` and nothing else happens in `menu.rs`.**
  `Emit` hands off to the frontend, `ShowAbout` to `about.rs`, `OpenUrl` to the opener plugin.
  `action_for`'s tests are what stop a new item shipping as an enabled row that does nothing.

### About window and the Help menu

Both macOS-only; what they show is [`docs/desktop-ux.md`](../../docs/desktop-ux.md#about-window-and-the-help-menu-macos), and the gap on the other clients is `Capability::AboutAndSupport`.

- **`src-tauri/src/links.rs` owns the project's outbound links** (id, label, URL).
  The Help items are built from it, a click resolves back through it, and `src/lib/links.ts` is a mirror pinned by `the_frontend_mirror_matches_this_table`.
  Never write a URL or a label in a component: About and Help are two surfaces for one fact.
- **About is a window, not a modal, and not `PredefinedMenuItem::about`.**
  The predefined item opens AppKit's panel, where no link is clickable.
  A modal is the smaller change and the wrong one: `<AppShell />` mounts only once a workspace is open, so About would be dead during the boot replay and on the first-run picker.
- **`src/index.tsx` branches on `?view=about` first, and reports no boot mark.**
  The four marks measure the app's cold start; a second window reporting `html_parsed` lands in that budget as a launch nobody made.
- **`capabilities/about.json` is scoped to the window by label**, narrower than the main window's on purpose.
  A label that stops matching `ABOUT_WINDOW_LABEL` is silent: every command the window calls is denied, which reads as an About screen stuck loading.
  `the_capability_file_scopes_this_window` is the net.

## Capture from outside the app (RFC 0363)

`drain_capture_inbox` plus `AppShell`'s `installForegroundRefresh` (`@outl/shared/capture`) land a line `outl capture` left in `<root>/.outl/captures/`, on mount and on every return to the foreground.
That is the whole story: outl registers **no** Service, menu bar item or global hotkey that writes one, which is why `CaptureOutsideTheApp` is `Partial`.

**The refresh is skipped while the user is in a block**, for the reason `onPeerChange` defers a peer reload: re-applying a view re-materializes the outline under the cursor.
The capture is in the op log either way, so the next refresh shows it ([`UX.md` → Never block a keystroke](../../UX.md#2-never-block-a-keystroke)).

## Deep links (`outl://`)

The scheme contract, the shared `outl_actions::parse_deep_link` parser, and this client's warm / cold wiring live in [`docs/deep-links.md`](../../docs/deep-links.md#desktop-wiring-outl-desktop).
The cold-start buffer, the parse and the warm dispatch are `src-tauri/src/deep_link.rs` (lifted out of `lib.rs` unchanged when that file crossed the size ratchet).
One thing bites every time, and it applies to "Open With" too: **testing on macOS needs a bundled, installed app**.
LaunchServices only registers `outl://` and reads `CFBundleDocumentTypes` from the built bundle, so `cargo tauri dev` never sees either.

## When you're done

1. `cargo fmt && cargo clippy -p outl-desktop --all-targets -- -D warnings && cargo test -p outl-desktop`
2. `bun --filter outl-desktop test` (Vitest)
3. `cd crates/outl-desktop && cargo tauri dev` — smoke open in a real window, click the parts you touched.
4. If you touched `@outl/shared`, also run `bun --filter outl-mobile test` to confirm paridade.
5. If you changed what a surface *does*, update [`docs/desktop-ux.md`](../../docs/desktop-ux.md) in the same change.
