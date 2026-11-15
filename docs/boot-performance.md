# Boot performance

What every client does between launch and a usable screen, in what order, and who is allowed to wait for what.

This is the owner of that fact.
A per-crate `CLAUDE.md` links here instead of restating the order, because the order is one sequence across five surfaces and five copies of it drift.

## The product rule

**"Content visible" must stop depending on vault size.**
Only "editable" may grow with it.

A launch has three moments, and they are different questions:

| Moment | What it means | What may delay it |
|---|---|---|
| Themed first frame | The window is the user's own colours, not a white or brand-dark flash | Nothing that reads the workspace |
| Real content visible | The user's own text is on screen | Reading one page |
| Editable | A keystroke lands in the op log | Opening the workspace |

Everything else (sync, the filesystem watcher, the plugin host, backups, the orphan sweep, the boot cache) belongs after all three.

## Budgets

Measured on a synthetic workspace of 10k pages / 70k nodes / 160k ops, release build, warm cache.
`100k` is the same shape at ten times the size.

| Client | Themed first frame | Real content visible | Editable |
|---|---|---|---|
| Desktop (10k / 100k) | < 100 ms | < 250 ms / < 250 ms | < 400 ms / < 1 s |
| Mobile (10k / 100k) | native frame | < 500 ms / < 500 ms | < 1 s / < 2 s |
| TUI | < 50 ms | < 100 ms / < 150 ms | same |
| CLI `page get` | n/a | < 50 ms / < 80 ms | n/a |

These are targets, not assertions.
What is enforced today is the CI trend line (below); a hard gate needs the instruction-count harness, which is open work.

## The phases, and who owns each

### 1. Before the webview (GUI clients)

The native window is filled by the OS before any HTML exists.
With no `backgroundColor` that is white, so a dark-theme user opened outl with a white rectangle.

- `tauri.conf.json`'s `backgroundColor` covers the brand default.
- `outl-desktop`'s `window_paint::window_background` repaints it from the theme on disk, so a light-theme user gets a light window.
  It cannot resolve `mode = "auto"`: the OS appearance setting is a question the webview answers, so `auto` takes the dark side here and the cache below corrects it one frame later.

### 2. The first HTML frame

The palette comes over IPC, which is two round-trips after the document parses.
So the last installed palette is replayed from `localStorage` by an inline `<head>` script, before anything paints.

- `applyPaletteToRoot` (`@outl/shared/theme`) is the **only** writer of that cache, beside the only writer of the tokens, so the two cannot disagree about what the theme is.
- `paletteDeclarations` builds the string; `BOOT_STYLE_KEY` (`@outl/shared/boot`) is the key.
- The inline script is deliberately not a module (a module is deferred until after parsing, which is the frame this exists to fix) and holds no knowledge of colours: it assigns one string to one element.

Mobile's splash is dismissed by the app, not by a timer.
The CSS timer cost a **620 ms floor on every launch** (380 ms delay plus a 240 ms fade), paid whether the journal was ready in 180 ms or in four seconds.
`dismissBootSplash` (`outl-mobile/src/lib/splash.ts`) starts the fade when the first view is applied, including on failure, because the failure state carries a Retry button and a splash over it is a dead end.
The 4 s failsafe animation in `index.html` is a backstop for a launch that never gets there, not the mechanism.

### 3. Today's journal, before the workspace

The screen a user waits on says "Opening your workspace…", and what it is waiting for is the op log: the index sidecars, the snapshot decode, the delta replay, the boot repairs.
None of it is needed to *show* today's journal.

The `.md` on disk is the projection of exactly that page and the sidecar beside it carries the block ids, so the opener reads the pair **before** it opens the op log and emits it as `page-preview`.
Measured on a real 730k-op workspace, against its largest journal (1,006 lines, 18 blocks): **0.6-1.3 ms**, against 163 ms of snapshot boot plus the index load.
That cost does not move with the size of the graph, which is the product rule at the top of this file.

`outl_tauri_shared::page_preview` owns it. Four rules:

- **The preview is never editable.**
  Every mutation needs the workspace, which by definition is not open yet, so a keystroke let through here would be writing into `ERR_LOADING`.
  Nothing on the wire enforces that and nothing should: the payload is a plain `PageView`, and the guarantee lives in the renderer, which has no interaction in it at all.
  A "do not edit me" flag beside the data would be a second, weaker statement of the same thing, and one a client can ignore.
- **It is best-effort and silent.**
  A fresh workspace, a day with no journal yet, an unreadable file: all of them mean no preview, never an error.
  The open behind it is the real answer and reports its own failures.
- **It is not authoritative about anything the op log knows and the file does not.**
  A peer's edit that arrived as ops, or a collapsed flag (`Op::SetCollapsed` lives in the log, so every node renders expanded).
  `md_ahead_of_log_checked` is `false`, so a client's sticky banner is never cleared by a reply that did not ask the question.
- **The page id is derived, not looked up.**
  `NodeId::from_slug` is deterministic, so the preview carries the id the real view will carry and the swap does not remount the outline under the user.

The client half is `@outl/shared/boot`'s `onPagePreview` and `<PreviewOutline />`, a **second** outline renderer with no interaction in it at all.
That is deliberate: the clients' real outlines are not separable from editing (callbacks per mutation, a draft signal, long-press, taps that route into commands), and threading a "nothing works yet" flag through all of it is wrong in exactly one direction — the day it leaks, a keystroke goes to a workspace that cannot take it.
A renderer with nothing to make editable cannot drift into being editable.

### 4. Waiting for the workspace, without polling

Both clients used to ask repeatedly: the desktop polled `workspace_stats` every 500 ms, the phone retried `open_today_journal` 50 times with a 200 ms backoff.
Neither can resolve faster than its own interval, so a workspace that became ready 10 ms into a tick waited out the rest of it, every launch.

`outl_tauri_shared::WorkspaceReady` is a latch with a blocking wait, and `wait_workspace_ready` is the command over it.
`false` means **not yet, ask again**; it is never a failure, because a cold 700k-op replay on a phone legitimately takes seconds.

The event (`workspace-ready`) stays, and the fallback it needed stays reachable: an event can be emitted before the frontend subscribed or lost to a webview reload mid-boot, which is why the poll existed at all.
What is gone is the interval.

### 5. The interactive phase

`outl_tauri_shared::FirstPaintGate` holds back everything a launch does not need on screen, and releases it when the frontend reports `first_content`:

| Job | Why it waits |
|---|---|
| Filesystem watcher (desktop) | Cheap, but it closes its own window: see below |
| Snapshot refresh | A multi-MB encode plus an fsync, read only by the *next* launch |
| iroh transport | Binds an endpoint and dials every peer; on a phone the dial and the open both peak in the hundreds of MB |
| Orphan `.md` reconcile | Two phases cannot be chunked and hold the workspace lock for 407 ms on desktop, **2,557 ms** on mobile |

Three properties of the gate that are easy to undo:

- **The deadline is not a nicety.**
  The painted signal comes over IPC from the webview.
  A webview that failed to load, a frontend built before the signal existed, or a `boot_mark` lost to a reload leaves it unsent, and without a deadline that is an app that syncs with nobody, silently, forever.
  Both clients arm one (desktop 4 s, mobile 6 s).
- **Deferring the watcher opens a window, and the window is closed by asking the log.**
  A watcher only reports what happens after it starts, so "did a peer append while nobody was watching" has to be asked of `ops/`, not of `notify`.
  `ops_watermark::read_needed` is that question, and the boot already recorded the answer once, so a `Some` means a peer appended in between and the desktop emits one `peer-ops-changed`.
- **Callbacks run on their own thread.**
  `painted()` arrives on the IPC thread; the jobs take the workspace lock and touch the network stack.

Mobile's existing `open_done` channel is **not** replaced by the gate.
It is a memory gate rather than a readiness one, and it is a channel precisely because a channel cannot be missed.
What changed is that the signal now goes out from inside the gate, so the first dial does not compete with the first paint either.

### 6. The TUI

The order was inverted: raw mode, the alternate screen and a skeleton frame come first, then the workspace open.
Everything the open costs used to be spent against the user's shell prompt, so a slow open was indistinguishable from a hang.

`supports_keyboard_enhancement()` moved after the first frame.
It costs **2.4 s** on a terminal that does not answer, and it cannot move to a thread, because it reads stdin: a thread doing that races the event loop for the user's keystrokes.
Those 2.4 s are still input-dead wherever they happen; closing that needs a way to skip the query outright, which is a config key and a decision about its default.
`crates/outl-tui/src/runtime/boot.rs` carries the reasoning.

A failed open now happens **inside** the alternate screen, so `restore_terminal` is called on that path too.
Returning `?` straight out would hand the user back a shell in raw mode with no prompt.

### 7. The CLI

A writing open persists the boot cache; a read does not.

Short-lived openers were excluded wholesale, on the grounds that a write would race the long-lived GUI.
That race was real and was fixed in the writer (`snapshot::write_to_disk` uses a per-write scratch and publishes by `rename`), so the cost no longer argues against the design.
What remains true is that writing one is a multi-MB encode plus an fsync, and a 200 ms command paying it is paying for the next process.

So the opt-in sits where the cost is attributable: `outl_cli::ws::open` (which is already encoding and fsyncing ops) turns it on, `open_read` does not.
`Workspace::apply` only spawns the write when the threshold is crossed, which after a full replay means "there was no usable snapshot", exactly the case that needs one.
`WsCtx::drop` waits the worker out so a short command cannot exit mid-fsync.

On a terminal-only machine this is what stops every invocation from full-replaying forever: 330-357 ms for `outl page get` without a snapshot against 132-166 ms with one.

## Measuring it

### The marks

`outl_core::boot_timing` emits one `boot phases:` line per workspace open, naming each phase.
That covers the Rust half.
The frontend half is invisible to it, so a client reports four milestones through `boot_mark`:

| Mark | Meaning |
|---|---|
| `html_parsed` | The document exists; the app has not started |
| `theme_applied` | The authoritative palette is on `<html>` |
| `first_content` | Real workspace content is on screen. **This releases the interactive phase** |
| `interactive` | Every listener is armed and every ticker is running. Emits the summary line |

The names are a wire contract: the log, a CI budget checker and the simulator job all match on the string.
`outl_tauri_shared::commands::boot::BOOT_MARKS` is the owner and `@outl/shared/boot`'s `BOOT_MARKS` is its declared mirror.

Reading them on a device:

```sh
# iOS simulator
xcrun simctl spawn <udid> log stream --predicate 'eventMessage CONTAINS "boot mark"'
# the mobile backend log, which survives the launch
cat "$(xcrun simctl get_app_container <udid> app.outl.mobile-app data)/Library/Logs/outl-backend.log"
# a desktop dev build
RUST_LOG=info cargo tauri dev 2>&1 | grep -E "boot (phases|marks?)"
```

### The CI trend line

`bench.yml` triggered on `crates/outl-md/**` alone, so `crates/outl-core/benches/boot.rs` never ran on a PR: a change to the op log, the snapshot or the workspace open could not trip the workflow at all.

It now also triggers on `outl-core`, `outl-actions` and `outl-tauri-shared`, runs the `10k` tier of `boot` on every PR, and the `50k` + `218k` tiers on the weekly cron.
The reference workspace is 217,811 ops across 20 actors, so `218k` is production rather than a stress test.

**The PR tier is a trend line, not a gate.**
Criterion's wall clock on a shared 4-vCPU runner moves by double digits between identical runs.
A real gate needs instruction counts (iai-callgrind or cachegrind) and absolute budgets checked with hyperfine on top of the existing `bench-cli-xlarge` JSON.
Both are open work.
Until they land, a regression is caught by reading the trend rather than by a red check.

## What is still open

Named here so none of it reads as done:

- **A first-screen cache** of the last UI state (open view, sidebar, pins, calendar marks), so the chrome paints complete on the first frame.
- **A persistent incremental backlink index.** `build_backlink_index_from_disk` is 213-256 ms, rebuilt whole and sequentially, and invalidated by every commit.
- **The full-replay cliffs.** `reboot_with_all_storages` ignores the snapshot with per-page shards, and a late peer op below the snapshot's HLC cut forces a full replay.
- **A zero-copy snapshot** via mmap, replacing the `fs::read` plus full postcard decode.
  Needs an RFC first.
- **Lazy-loading** Settings, Marketplace, Sync, Onboarding and Timeline off the entry bundle.
- **Hard regression gates**: the instruction-count benches, the absolute budget checker, a TUI pty harness that measures first frame, and the desktop/mobile jobs that read the marks.
