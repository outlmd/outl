# How each mobile surface is wired

Surface by surface: onboarding, the page switcher, properties, settings, reminders, plugins, peers, the sync dot, theming, the log, and the polish layer.

Moved out of [`crates/outl-mobile/CLAUDE.md`](../crates/outl-mobile/CLAUDE.md), which is loaded into the model's context on every session in that crate.
These sections are reference: you read one when you touch that component.
The rules that hold whatever you are editing stayed in `CLAUDE.md`.

What the user experiences, feature by feature, is [`docs/mobile-ux.md`](mobile-ux.md).
The task-oriented user guide is [`docs/mobile.md`](mobile.md), with [capture](mobile-capture.md) and [troubleshooting](mobile-troubleshooting.md) beside it.
When a section here changes what a user sees or presses, update the guide in the same change.

## First-run onboarding

`components/Onboarding.tsx` is the first-run flow.
`App.tsx` gates it on a **per-install `localStorage` flag** (`outl.onboarded`) — pure UI state, **never** an Op (it must not converge across devices; each device onboards once).
Mobile has no "is a workspace chosen?" backend gate (a fresh install always resolves *a* root — the local default), so this flag is the only signal that distinguishes a brand-new install from a returning one.

Two honest steps, no filler:

1. **Storage** — one button, "Keep on this device".
   It only advances; no `setWorkspace` call, because the workspace already opened against the app-local default.
   There is no iCloud option and no folder picker today: `Onboarding.tsx` offers the local root and nothing else, and an earlier version of this section described a "Store in iCloud" button that does not exist in the code.
2. **Sync (optional)** — the shared `SYNC_STEP` copy (`@outl/shared/onboarding`) + a button that opens the existing `<DevicesSheet />` (set its `open` prop — internals untouched).
   Fully skippable.

The onboarding **copy** lives in `@outl/shared/onboarding` (identical to desktop); the bottom-sheet chrome + haptics stay here.

**A third first-run surface sits after both steps**: `<GuideSheet />`, which names this client's own chrome — the bottom bar, the `+`, the week strip, the long-press ([#574](https://github.com/outlmd/outl/issues/574)).
It is not the 30-second outline tour (`firstRunTips`), which only ever draws on an empty page and names no buttons, so it carries its own dismissal key (`CHROME_TOUR_KEY`) rather than sharing `TIPS_KEY`.
Sharing one would hide it from the user whose journal already has content, which is exactly who needs it.
The rows are `lib/chrome-tour.ts` (mobile-only copy: every one names a control only this client has); the shared half is the "show it once, then remember" pair, which never becomes an Op.
Pairing is **not** reimplemented — `Onboarding` opens the real `<DevicesSheet />`.

**DEFERRED — native folder picker + security-scoped bookmark.**
The native `UIDocumentPickerViewController` (folder mode) bridge is **not** implemented (and is not faked).
Two real blockers: Tauri 2's iOS folder picker is incomplete (tauri-apps/plugins-workspace#3030), and a folder *outside* the app sandbox needs an `NSURL` security-scoped **bookmark** to be reopenable across launches.
Storing just the string path in `WorkspaceCfg.last` only works for the sandbox and the local default.
The follow-up adds an `objc2` bridge that presents the picker, serialises a bookmark, persists it next to `actor`, and resolves it on boot before `resolve_storage_root`.
Until then, `set_workspace` works for any path the frontend can already reach without a scoped bookmark, and the local default is the only root a fresh install opens.

## Page switcher: rename and merge (RFC 0557)

A long-press on a **page** row (never a journal) opens the same menu component the block long-press uses, with three rows: **Rename page...**, **Merge into...**, **Delete page**.

**Rename has a second door, and it is the page title.**
`Journal.rename.tsx` owns it: a tap on a non-journal title calls `armKeyboard()` inside the gesture window and opens the same `PageNameSheet`, over the same `@outl/shared/pages/rename-merge` flow.
`JournalHeader.tsx` stays pure render, so the pencil glyph and the tap arrive as props.
The tap shares the title region's `consumedClick()` guard with the long-press, read **once** per click, because `lib/long-press.ts` clears the flag as it answers.
Merge keeps only this menu: the header has no list to turn into a target picker.
`PageSwitcher.tsx` keeps the search list and the wiring; everything one page can be asked to do lives in `PageSwitcher.page-actions.tsx` (`createPageActions`, the long-press recogniser, `<MergeBanner />`, `<PageActionOverlays />`), split out when the switcher would otherwise have crossed the 600-line ratchet.

- **The flow is not written here.** `renameOrMerge` / `mergeInto` and every sentence (`nameTakenMessage`, `mergeConfirmMessage`, `renameSummary`, `mergeSummary`) come from `@outl/shared/pages/rename-merge`, so the desktop runs the same steps and says the same words.
  This crate owns only the chrome: `PageNameSheet.tsx` (a prefilled one-line sheet, 48pt targets) for the name and `ConfirmDialog` for the question.
- **A merge is never reached without a confirmation.** "Merge into..." turns the switcher's own list into a target picker (pages only, the source left out, no pin toggles, no long-press) and a tap on a row asks first.
  Cancelling that question stays in the picker so another target can be chosen; the banner's Cancel leaves it.
  A rename onto a taken name asks the same question, and any answer but "Merge" writes nothing.
- **On success** the switcher navigates to the resulting page through `onPick` (delete does the same) and hands the summary to `onNotice`, which `Journal.tsx` routes into its toast.
  A summary with several lines (a merge with property conflicts) makes `Toast.tsx` show every line and stay 12 s, because the conflict line is the one the user has to read.
- **The `+` beside the search box starts a page**, name sheet → `openRef`, which is also the only reason `PageNameSheet` grew a `submitUnchanged` prop (a rename refuses a no-op name; a create prefilled from the search box must not).
  Before it, a non-journal page could only be started by typing `[[` into a block ([#573](https://github.com/outlmd/outl/issues/573)) — the mechanism was fine, the discoverability was the bug.
  `openRef` decides what a name means, so an existing name **opens** rather than duplicating; see [`docs/mobile-ux.md`](mobile-ux.md#search-sheet--starting-a-page-that-is-not-a-daily-note).
- Not done on mobile: a keyboard shortcut (there is no chord surface) and the optional alias-conflict warning.
- Pinned by `PageSwitcher.page-actions.test.tsx` (the shared flow runs for real over stubbed `api/page-merge` commands) and `sheets.bounded.test.ts` (the name sheet declares its bound).

## Properties are fields, not tags

A pill is a **tag**: a flat label you tap to filter.
A `key:: value` property is a **field**, a named attribute with a value, and this client drew one as the other — so two different things read as the same thing while behaving differently, and the tap a pill promises was not the tap it performed.

The outline now draws **one attribute line** per block: key dim, value a step darker, `·` between, truncated, never more than a line, and past two fields the rest is `+N more`.
Both the block row (`BlockBody.tsx`) and the page strip (`Journal.tsx`) are the same `<BlockProperties />` now — the strip was a second hand-written copy, which is how the same dead `text-[11px]` came to be written twice.
`@outl/shared/markdown` owns the shape; what is left here is where the line sits and what a tap means.

**Colour is spent only on a field that acts on the user.**
`remind::` gets the accent and a bell because it will interrupt you; `auto-run::` runs code and stays quiet, because it does not. That criterion is *interruption*, not importance, and it is what stops the line from becoming decoration again.

**Editing left the outline.**
The line is one tap target that opens `<PropertiesSheet />`, which is a two-column field list (`PropertyFieldList.tsx`) in the shape of iOS Settings or Notion's property panel. A row that looks like a form row opens a form; a row that looked like a button did not.

An importer's stamps (`created`, `edited`, `roam-uid`) render nowhere in an outline and are still listed in the sheet, dimmer and sorted last — you have to see one to delete it. The policy is `outl_actions::property::is_import_stamp`, mirrored in TS.

## Properties (`key:: value`) — the sheet (issue #13)

Creating a property was impossible in either GUI client: the chips
rendered what a block already carried and edited a value in place, so
the *first* one had to come from the TUI or a hand-edited `.md`. Page
properties (`icon::`, `type::`) were not even visible.

**`components/PropertiesSheet.tsx`** is mobile's answer. The block
long-press menu's "Properties…" action opens it on that block; the
page's own chips (rendered under the header in `Journal.tsx`) open it
on the page, and a Block/Page switch inside crosses between the two.

Two design facts, both measured on a real 66k-block graph, decide the
shape — and both are the reason this is a sheet and not a `+` on the
chip row:

1. **Keys repeat.** A graph reuses about a dozen (`icon`, `related`,
   `status`, `oura-date`). So the Add step paints the workspace's own
   keys as **tappable chips**, ranked by `known_property_keys`, with
   "Other…" as the keyboard escape. Two taps, nothing typed — that is
   ~87% of the value of this feature on a phone.
2. **87% of values are a `[[page]]` or a `#tag`.** Typing `[[` in the
   value field opens the page picker (`searchPages` + the shared
   `detectRefContext` / `applySuggestion` / `refReplacement` — the
   accept rule is **not** re-implemented), and a "Link a page…" button
   types the `[[` for you.

Deleting is explicit in two shapes: swipe a row (`<SwipeRow>`) and a
Delete button inside the value editor. Both write an **empty value**,
which is how `set_property` spells "remove" — the user never has to
discover that.

Wire commands, both thin wrappers over shared bodies:

- `known_property_keys` (`commands/property.rs` → the catalogue; the
  ranking itself is `outl_actions::known_keys`, one owner, three
  clients).
- `set_page_property` (`commands/reminders.rs`, next to the existing
  `set_block_property` — same shared module owns both bodies). It
  refuses `page-slug` / `page-kind`: those *are* the page's identity,
  and renaming is `page_rename`, which moves the projection too.

`PageView.page_properties` is what makes the page side possible at all.

Mobile-only pieces, and why they are not in `@outl/shared` yet:
`lib/properties.ts` (`normalizeKey` — strips the `::` a user copies —
plus `suggestedKeys` / `editableProperties`) and the sheet itself are
phone-keyboard answers; the desktop's affordance for the same
catalogue is inline completion with no cap and no "Other…" escape.
`knownPropertyKeys` / `setPageProperty` live in
`@outl/shared/api/commands` beside `setBlockProperty` — both clients
register those commands, so the wrappers have one owner; `lib/api.ts`
only re-exports them so existing imports keep resolving.
The sheet's deny-list delegates its bookkeeping half to
`@outl/shared/markdown::isInternalKey` (the same predicate
`propertyChips` hides those keys with) and keeps only the
page-identity pair (`page-slug` / `page-kind`) local.

Covered by `PropertiesSheet.test.tsx` (chips exclude keys already set,
two-tap add, `::` normalisation, edit, both delete affordances, page
scope, the `[[` picker) and `lib/properties.test.ts`.

## Settings (`SettingsSheet.tsx`) — the first preferences surface

The header capsule's gear opens `components/SettingsSheet.tsx`.
It is the client's **only** preferences screen: every other sheet here (`DevicesSheet`, `TemplateSheet`, `RemindersSheet`, `PropertiesSheet`) shows workspace content, which is why the toolbar lock ([#271](https://github.com/outlmd/outl/issues/271)) had nowhere to live before it.
Its first three rows are the exception, and all are navigation rather than preferences.
**Devices and sync** hands over to `<DevicesSheet />`, closing this sheet first since this one is `z-[55]` and that one `z-50`.
**Trash** hands over to `<TrashSheet />` the same way, the way back for a block deleted on any device ([#455](https://github.com/outlmd/outl/issues/455)).
**What's what** re-opens `<GuideSheet />`, the chrome walkthrough that otherwise only appears once per device ([#574](https://github.com/outlmd/outl/issues/574)).
It is here because pairing otherwise had no name anywhere on the phone — the sync dot's press-and-hold is a gesture nothing announces, and a user looking for P2P looks where the desktop keeps it, in Settings.
The gesture stays as the shortcut.

Two rules govern what may go in it:

- **Per-device UI state, not workspace data, unless the process itself has to read it.** Nothing here goes through the op log: two devices disagreeing about their own toolbar layout is not a conflict to reconcile (root `CLAUDE.md` invariant 7). The toolbar lock stays in `localStorage`. `editor.diagrams` is per-device too but lives in `~/.config/outl/config.toml`, so the desktop on this machine sees the same choice; the sheet reads and writes that key through `get_editor_diagrams` / `set_editor_diagrams`. A preference that *is* workspace data belongs in `outl.toml`, not here.
- **The setting's logic lives in `@outl/shared` or in a shared command, not in the sheet.** The toolbar lock is `@outl/shared/toolbar`'s `lock.ts`, the diagram switch is `@outl/shared/diagram`'s `diagramsMode`; this file only renders the row and calls them. A preference implemented inside the sheet is a preference the other bar can't honour.

**Crash reports goes further than either, because it is consent rather than a preference** (RFC 0377).
`localStorage` cannot hold it: `outl-telemetry` reads `[telemetry] enabled` out of the global config at **process start**, before a workspace or a webview exists.
It is a property of the person, which is why it belongs in a config file that follows them to the next device.
And the phone has to offer it at all: reporting is on by default and there is no `outl` binary on iOS, so this row is a mobile user's only opt-out (`Capability::CrashReportingOptOut`).
The writer is `outl_tauri_shared::commands::telemetry::set_telemetry_enabled`, the wrappers are `@outl/shared/api/telemetry`, and the words are `CRASH_REPORTS_SETTING` in `@outl/shared/onboarding` — identical to the desktop's, because the same claim goes to Apple and Google.
The switch moves only on the command's reply, like `RemindersSheet`'s; a read or write that fails says so in the row instead of falling back to the default, since the default is *on* and a guess would tell someone who opted out that they had not.
This row is the opt-out, not the disclosure: the strip that tells an unasking user at all is `<CrashReportsNotice />` from `@outl/shared/polish`, mounted by `App.tsx` **inside the onboarded branch** so it never stacks on the first-run flow, and lifted clear of `BottomBar` by `--outl-notice-offset` in this client's `styles.css` (RFC 0377 section 6).

**The sheet is mounted for the app's lifetime, not per open** — `<Show>` gates its markup, and `Journal` renders it unconditionally. So state it reads out of a store goes in a `createEffect` guarded on `props.open`, the way `TemplateSheet` and `RemindersSheet` already do. A plain `createSignal(readSomething())` in the body reads once, at app start, and then quietly shows a stale value forever.

**A preference this sheet writes has to be readable by whoever acts on it.** The toolbar lock's first version stored the counts it derives from in `UserDefaults` on the iOS side, where this sheet — which is web on both platforms — could not see them: Lock froze a cold-start row instead of the user's, and Reset was a no-op. Before adding a setting, check that the surface which consumes it reads the same store this sheet writes.

Toolbar behaviour it controls (lock, reset, and why the row now only reshuffles between editing sessions): [`docs/mobile-ux.md` → Keyboard accessory bar](mobile-ux.md#keyboard-accessory-bar-android-web-bar--ios-native-bar).

## Reminders (`remind::`)

The bottom bar's Reminders tab opens `<RemindersSheet />` (the header has no bell any more), a block long-press authors a rule through `<RemindSheet />` → `set_block_remind`, and `Journal.tsx` polls `deliver_due_reminders` every 30s.
Sheet wiring and why both device-local settings live in the list sheet: [`docs/reminders.md`](reminders.md#mobile-outl-mobile).

**Two rules, and neither is about the sheet's markup.**

- **The schedule is never computed in TS** — `groupReminders` / `formatNextFire` only format what `outl_actions::reminders` decided.
- **The rule text is never written in TS either.**
  `<RemindSheet />` reads a rule with `parseRemindRule` and writes one with `formatRemindRule`, two commands over `outl_md::remind`'s `parse_remind` / `format_remind` pair, twice per session rather than per keystroke.
  Authoring was a `window.prompt` for the grammar until [#572](https://github.com/outlmd/outl/issues/572).
  The sheet keeps the whole grammar reachable through its **Edit as text** field, which is what stops a picker from quietly narrowing what a user can say.
  `Journal.remind.ts` owns the target signal and the write, the way `Journal.embeds.ts` owns its own — the component holds no state for it.
  Per-control decisions (a repeat the list does not offer, a seeded `until`) are `lib/remind-fields.ts`, unit-tested.

### Actionable banners (mobile only)

A reminder banner carries **Snooze 1h** and **Done**, and a plain tap lands on the block rather than the journal.
Three pieces, and the split between them is the point:

- `commands/reminders.rs` owns the ids (`REMINDER_CATEGORY` / `ACTION_SNOOZE_1H` / `ACTION_DONE`), stamps the category onto every banner via `action_type_id`, and attaches `blockId` / `pageSlug` as extras so the handler knows the subject.
  `reminder_action_catalog` hands the same constants to the frontend, with a declared `kind` (`snooze` / `done`) so the handler dispatches on meaning instead of pattern-matching the id.
- `lib/reminder-actions.ts` is the decision table (which button → which call), pure and unit-tested, including the tap identifiers both platforms send and the "banner with no subject" case an upgrade can produce.
- `lib/reminder-notifications.ts` is the IPC + listener wiring, which can only run on a device.

**Why the frontend registers something Rust owns:** the plugin's `ActionType` / `Action` are `#[cfg(mobile)]` structs with private fields, no constructor and no `Deserialize`, so `Notification::register_action_types` is unreachable from a consumer crate.
`registerActionTypes()` in JS reaches the same command through IPC, so that is the only door.

**Never move this into `@outl/shared`.** The command, `registerActionTypes` and the `actionPerformed` event are mobile-only; the desktop plugin registers no such command and emits no such event, so a shared wrapper would hand the desktop an IPC call that fails.
The per-client verdict is declared once, in `outl_shortcuts::capability_support` (`Capability::ReminderNotificationActions`), and pinned here by `capability_parity.rs`.

## Plugins

JS plugins (`outl_plugins::PluginHost`) run on mobile; the design is the desktop's.
Read [`docs/plugins.md` → Desktop plugin surface](plugins.md#desktop-plugin-surface) for the shared rationale (`!Send` Boa host on a dedicated thread, `PluginService` in `AppState`, re-projection via `apply_all_pages_md`).
Boa is pure-Rust (no JIT), so it ships under iOS's dynamic-code ban (same as `lang-js`).

**The one divergence from desktop:** mobile's `storage_root` is an owned `PathBuf` (folder swap is a relaunch), absorbed by the shared `StorageRootProvider` trait — a fixed root never triggers the "re-load on root swap" branch.
The host loads plugins once, lazily, from `<root>/.outl/plugins/` on the first request after the workspace opens (`ensure_loaded` + `mark_synced`).

Capabilities honored: `slash-command` + `op-hook` + `ui-render` + `toolbar-button` + `content-transformer:text` + `content-transformer:rich` (no `keybinding` — no chord surface on mobile).
Each must be declared in `client_capabilities()` (`plugin_service.rs`); the host gates contributions on the client∩plugin intersection.
Dropping `ToolbarButton` silently empties `toolbar_buttons("mobile")`; dropping either transformer cap silently filters `transformers()` (a custom-language fence then renders as plain code).
Tauri commands in `commands/plugin.rs` have the **identical shape to desktop** — the full command table is in [`docs/plugin-architecture.md`](plugin-architecture.md#client-tauri-command-surface-desktop--mobile).

Op-hooks fire at a single post-mutation point: `Journal.tsx`'s `commitEdit` calls `pluginSyncHooks(pid)` after an edit lands.
One call dispatches every op since the last sweep, so it also catches structural ops (indent / move / delete).
The hook-driven `applyView` guards on `!editingId()` so it never resets the textarea.

Frontend: the plugin DTOs + wrappers (`pluginList` / `pluginToolbar` / `pluginRun` / `pluginSyncHooks`, …) live in `@outl/shared/api`.
The stacked-squares header glyph opens `components/PluginSheet.tsx` — a bottom sheet that lists + runs commands and pipes `notify` / errors to the toast.
Toolbar buttons are inline header glyphs.
`Journal.tsx` loads `pluginToolbar()` in `onMount` into a `toolbarButtons()` signal and renders one `<button>` per entry next to the sheet glyph.
`runToolbarButton` → `pluginRun(...)` reuses the sheet's toast / `showPluginViews` / `applyView` path.

### `ui-render` views (the confetti path)

A `ui-render` plugin emits HTML/JS via `ctx.ui.render(html)`; the core gates it onto `PluginRun::views`, propagated as `views` on both `PluginRunReply` (command path) and `PluginSyncReply` (`onOp` hook path).

`components/PluginViewOverlay.tsx` paints each in a **sandboxed, ephemeral `<iframe>`**.
The frame is `sandbox="allow-scripts"` **WITHOUT `allow-same-origin`** — load-bearing.
Plugin JS is untrusted; the missing flag forces an opaque origin so the frame can't reach the app DOM / Tauri bridge — the two flags together defeat the sandbox, so **never** add it.
Content is `srcdoc={html}` (no network), fullscreen, `pointer-events: none`, auto-removed after ~6s.

The overlay exposes an imperative `push(html)` via its `bind` prop.
`Journal.tsx` holds it as `pushPluginView` and feeds it from `showPluginViews(views)` at every source: `PluginSheet`'s `onViews`, `commitEdit`'s `pluginSyncHooks` reply, and `runToolbarButton`.
**End-to-end:** block → DONE → `commitEdit` → `plugin_sync_hooks` → `onOp` emits HTML → `showPluginViews` → iframe overlay.

The sandbox attrs + auto-removal are pinned by `PluginViewOverlay.test.ts`; `plugin_service.rs` unit-tests the host surface.
A real plugin load + iframe overlay only exercise under `cargo tauri ios dev`.

### Content transformers (custom-language fences)

A transformer claims a fence language and turns the body into a `{kind, content}` descriptor.
The registry + `(block id, body)` cache glue is shared with the desktop in `@outl/shared/plugins/transformer-registry` (see [`docs/plugins.md` → Desktop plugin surface](plugins.md#desktop-plugin-surface)).
Mobile just wires it: `Journal.tsx`'s `onMount` calls `loadTransformers()`, and `BlockRow`'s fence branch renders `<PluginFence />` on a `transformerFor(lang)` match (else plain `<HighlightedCode />`).
`rich` output lands in an inline sandboxed `<iframe>` — `allow-scripts`, never `allow-same-origin`, same posture as `<PluginViewOverlay />`.

## Peer / device management (`outl_peer_list` / `outl_peer_remove`)

`commands/peers.rs` wraps the same three Tauri commands `docs/sync.md` § Managing peers documents (`outl_peer_list`, `outl_peer_remove`, `outl_sync_now`) over `<workspace>/.outl/peers.json` (`outl_sync_iroh::PeersStore`).
Mobile-specific: the peer list is per-**graph** (`outl_sync_iroh::workspace_peers_path`), the device `identity.key` is per-**install** ([`iroh_sync::iroh_dir`]).
Each command runs `migrate_global_peers_if_absent` first (legacy global `~/.outl/peers.json` copied in once).
These are the **only** commands that touch `peers.json` directly instead of the workspace lock — graph-scoped sync-transport state reads `storage_root` without going through `outl-actions`.

## Sync dot + refresh (iroh-driven)

The header `<SyncDot>` and `PullToRefresh` reflect and drive the **iroh P2P transport** (outl's default sync), not the iCloud-era `navigator.onLine` signal they started on.

- **Dot state.**
  The PRIMARY input is iroh peer health, polled via `peerStatus()` into a `peers` signal (`PeerStatusDto[] | null`, `null` until the first probe answers — the absence of a measurement, which `[]` is not).
  The poll runs every 3s, and after each `peer-ops-changed` (the native ops bridge) plus after a force-sync.
  It is **armed** in the component body but **started** at the end of `onMount`, after the boot load settles (`Journal.polls.ts`): probing during render put a full workspace load in front of the first paint.
  `peer_status` is `async` now, so that load is off the iOS main thread, but it still reopens the op log per call — deferring the first probe stays right.
  **The derivation is not written here**: `JournalChrome` hands the list, `navigator.onLine` and the two in-flight flags to `syncState` (`@outl/shared/peers`), which answers `syncing` / `checking` / `online` / `offline` for this client and the desktop alike, and `<SyncDot>` paints it.
  A pass in flight wins; no radio floors to offline; `null` is **checking** (dim), which is the bug the shared owner closed — this client used to start at `false` and claim offline on launch over a probe nobody had taken ([#575](https://github.com/outlmd/outl/issues/575)).
  `navigator.onLine` stays only as a secondary floor (truly no radio → orange regardless).
  Zero paired peers reads as offline — there's nothing to sync with.
- **Refresh.**
  There is no separate refresh button: the sync dot is the button. A tap fires `handleRefresh`, press-and-hold opens the devices sheet (`createLongPress`, the same recogniser the block row and title use) — a shortcut for the named row in Settings, never the only way in.
  Taps while a pass is in flight (`syncing` or `refreshing`) are dropped, and `handleRefresh` itself refuses to start a second pass, so the dot, the pull and the auto-sync can never stack dials.
  `handleRefresh` (the dot **and** `PullToRefresh`) calls `syncNow()` (force a P2P pull — dial every peer now instead of waiting for the 8s catch-up tick) THEN `reloadWorkspace()` (re-render with whatever landed).
  Both calls are wrapped in `withError` (toast on failure, never wedge the local reload), and the `syncing` spinner brackets the whole pass.
- **Auto-sync (no button).**
  `Journal.tsx` shares the refresh core as `pullAndReload()`, fired on `onMount`, on `visibilitychange` → visible (iOS froze JS in the background), and on the 3s poll tick.
  The mobile side initiating the dial is NAT-friendly — waiting for the desktop to reach an iPhone behind carrier NAT is not — so this is what makes a desktop edit show up without the user touching refresh.
  The `workspace-ready` reload skips while a block is being edited (guarded by `editingId()`) so it never resets the textarea mid-edit.
  It also routes through `pullAndReload` (not a raw `openJournalFor` + `applyView`) so it inherits its guards.
  **Reload-ordering guard:** each `pullAndReload` captures a monotonic `reloadGen` at entry and applies only when still the latest, so a slow reload can't flip the page back to an older state.
  (The split-content flicker's real cause was a backend duplicate-slug-root bug, fixed in `outl-actions::merge_duplicate_slug_roots`; this is the frontend belt-and-suspenders.)

`syncNow()`, `peersOnline()`, `syncState()` and `<SyncDot>` all live in `@outl/shared` so mobile and desktop derive the dot + drive the refresh identically.
There is no `SyncDot.tsx` in this crate any more; the sync button's `aria-label` names the action only, because the shared dot is the live region that announces the state.
See [`outl-sync-iroh/CLAUDE.md`](../crates/outl-sync-iroh/CLAUDE.md) → "Force-sync trigger (`sync_now`)".

## Theming (RFC 0022)

`@outl/shared/theme::installTheme` fetches both pair sides via the shared `get_theme` command, holds both `Palette`s in memory, and repaints through `applyPaletteToRoot` on a `prefers-color-scheme` flip (no backend round-trip mid-repaint).
`App.tsx` reads `outl_config::ThemeCfg`; a fresh config already resolves to the brand pair `outl-light` / `outl`, while older configs with only `preset` keep that preset on both sides.
Field reference: [`docs/config.md` § `[theme]`](config.md#theme).

**A text colour belongs on the element that holds the text, and `src/styles.css` must stay out of its way.**
An unlayered rule outranks every layered one regardless of specificity, and Tailwind's utilities live in `@layer utilities` — so the `button, input, textarea { color: inherit }` this file used to carry unlayered silently killed **every** `text-*` class on a `<button>`.
The week strip's selected day painted its accent tile and then rendered near-black text inside it, because the colour resolved to nothing and the spans inherited `fg` from `#root`; the same failure was live on every primary accent button here.
The `color` line is gone (Tailwind's preflight already sets it in `@layer base`, which beats the UA sheet and loses to a utility, which is the order you want).
`font-family` / `font-size` are still unlayered there and still beat `text-[16px]` on a button — if you need a button's font size to take, that is why.
Paint a fill with `bg-(--color-outl-accent)` and its text with `text-(--color-outl-accent-fg)`, never with `text-(--color-outl-bg)` (root `CLAUDE.md` invariant 13, and [`DESIGN.md`](../DESIGN.md) owns the pairing).

**So the property line sizes the row, not the chip inside it.**
Both `key:: value` strips here carried a `text-[11px]` on a `<button>` that resolved to nothing: they painted at the body's 17px, centred (a `<button>` centres by UA default) and wrapping, so a Roam-imported block showed nine lines of grey chrome above one line of its own text.
The size moved to the container — where `inherit` resolves — and `property-line.test.ts` pins both call sites, asserting the unlayered rule is still in `styles.css` so the pin retires with the trap rather than outliving it.
**24 other `<button>` / `<input>` tags in this client still carry a dead `text-*`**, measured; clearing those means deleting the declaration and re-reading every sheet, which is the typography change the comment above defers.

## Logging (device console)

`run()` in `src-tauri/src/lib.rs` ignores `SIGPIPE`, then calls `outl_telemetry::init`, then installs a `tracing_subscriber` fmt subscriber.
The subscriber writes through `outl_telemetry::lossy_stderr` with `log_internal_errors(false)`, for the same reason as the desktop: a closed stderr must not become `failed printing to stderr`.
The `EnvFilter` defaults to `info,outl_sync_iroh=debug,iroh=info` and honors `RUST_LOG`.
On iOS stderr reaches nothing at all, which is what `backend_log` exists for — see "The backend's log, and why it needed one" below for the measurement.
So the iroh P2P transport's `info!`/`warn!`/`debug!` lines (endpoint bound + node id, each connect attempt's target + outcome, "delta sync received N ops") are read out of that file while debugging device↔device sync, never off stderr.
Init uses `set_global_default`, not `.try_init()`: `try_init` also claims the `log` slot, which `tauri-plugin-log` needs.
See [`outl-sync-iroh/CLAUDE.md`](../crates/outl-sync-iroh/CLAUDE.md) for what the transport logs.

## Trust surfaces

- **Page history**: `components/HistorySheet.tsx`, opened from the page properties sheet (long-press the title, then "Page history"). Same `restorePlan` and `DiffText` as the desktop `TimelinePanel`; restores are new ops.
- **Devices sheet** shows per-device op counts (`device_activity`), the session's merged edits with an undo, and the Markdown export (`export_markdown`).
  The export lands in the **parent of the workspace folder**, which on the default local root is inside the app's own storage.
  `Info.plist` sets neither `UIFileSharingEnabled` nor `LSSupportsOpeningDocumentsInPlace`, so the Files app does not list it.
  The old line here said it shows up in Files through iCloud Drive; that only held when the workspace lived in an iCloud container, which no build offers now.
- **Trash**: `components/TrashSheet.tsx`, opened from Settings → Trash and the search palette's "Open trash".
  The restore rules come from `@outl/shared/trust`'s `createTrashRestore`, shared with the desktop `TrashPanel`.
- **Saved mark**: `lib/saved.ts`; `Journal`'s commit marks a block only after `edit_block` resolves, `BlockRow` renders the dot.

## Polish: motion, focus mode, first-run tips

- `BlockRow` wraps the row (not its children) in `outl-row` so focus mode dims rows without compounding, and slides children in with `outl-unfold` only on an unfold. `SyncDot` pulses once when a pass ends.
- Focus mode is the capsule button between the sync dot and Settings, same as desktop. Header and `BottomBar` carry `outl-focus-hide` and fade to 0.18, never to 0, so the button stays tappable.
- `PageSections` gets `blank` from `Journal` and shows `<FirstRunTips client="mobile" />`; `JournalEmpty` shows it on the start and empty-page states. The tip names the indent keys, never a swipe: a swipe deletes a row here.
- Refs, tags and links in block text and backlinks use `MarkdownInline` `variant="inline"`, the same underlined look as desktop, not pills. The 44pt touch target comes from `.outl-ref::after` in `src/styles.css`; keep it if you touch either. The TODO box in `BlockMarkers` is a square (`rounded-[4px]`) inside a 44pt button, matching desktop.

## The backend's log, and why it needed one

`src-tauri/src/lib.rs` installs a `tracing` subscriber writing to stderr, with a comment saying that on iOS stderr "surfaces in idevicesyslog / Xcode".
It does not.
A release build on the simulator emits **nothing** to `simctl launch --console`, `--console-pty` or `log stream`; the only lines that arrive come from dylibs using `NSLog`, which goes through `os_log`.
Rust's `io::stderr()` is not wired to anything on either simulator or device.

So every `info!` / `warn!` the backend produced — the sync transport, the workspace opener, every boot repair, every projection refusal — was invisible on the one platform where the app is hardest to debug and where there is no `outl doctor` to fall back on.
A blank-screen report could only be narrowed by elimination, which is exactly as slow as it sounds.

`backend_log` is the sink: `$HOME/Library/Logs/outl-backend.log` inside the app sandbox, **truncated on every launch**, still mirrored to stderr so a desktop-style run is unchanged.
Truncating rather than rotating is the right trade on a phone: what you want from a backend log there is "what did *this* launch do", and a file holding one launch cannot grow without bound on a device nobody prunes.

`tauri_plugin_log` owns the `log` crate's slot and writes the **frontend's** lines to its own rotating file; our `tracing::` events deliberately bypass it (see `lib.rs` on why the two logger slots are kept apart), which is why they need a sink of their own.

Read it with:

```sh
cat "$(xcrun simctl get_app_container <udid> app.outl.mobile-app data)/Library/Logs/outl-backend.log"
```

`outl_core::boot_timing` is what makes that worth reading on a launch: one line per workspace open, naming each phase plus the delta size and resident block count that explain them.
