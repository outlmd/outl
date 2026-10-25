---
version: alpha
name: outl
description: Local-first outliner. What happens, to whom, under which circumstances, and what we say when we can't.
users:
  - id: outliner-refugee
    from: Roam, Logseq
    knows: bidirectional links, daily journal, block refs, indent-as-structure
    wants: the same feel without the cloud lock-in or the `id::` lines
    leaves-when: sync loses work, or the markdown stops being theirs
  - id: terminal-first
    from: vim, tmux, ssh
    knows: modal editing, chords, grep over a notes folder
    wants: a first-class TUI, not a fallback
    leaves-when: the keyboard path is second-class to the mouse path
  - id: multi-device
    devices: laptop + phone, sometimes a second machine
    knows: has lost work to a bad merge at least once
    wants: two devices editing offline to converge, deterministically
    leaves-when: a conflict resolves silently in either direction
contexts:
  - offline-and-partitioned
  - two-devices-edited-the-same-block
  - asleep (quiet hours)
  - terminal-with-no-OS-appearance-API
  - phone-with-no-`outl`-binary-and-no-git
  - soft-keyboard-covering-half-the-screen
  - an-OS-that-kills-the-app-if-the-main-thread-stalls
  - a-workspace-of-2500-pages-and-64k-blocks
  - an-external-editor-touched-the-file
  - the-process-died-between-the-op-and-the-projection
principles:
  - journal-first
  - never block a keystroke
  - silence is the defect
  - refuse rather than delete
  - view state is local, disagreement converges
  - interruption is always opt-in
  - structure on demand, prose always
  - one owner per user-visible fact
surfaces:
  - tui
  - desktop
  - mobile
  - cli
  - mcp
---

# outl — UX specification

## Overview

`DESIGN.md` answers *what it looks like*.
This file answers **what happens, to whom, under which circumstances, and what we say when we can't**.

It is written as context rather than as a report — the
[UX-context design](https://www.nngroup.com/articles/ux-context-design/) framing: research findings become
constraints, interaction patterns become rules, and the domain vocabulary is written down so three surfaces
stop inventing three names for one operation.
The audience is a contributor or an agent about to build something user-visible, and the test of every
sentence here is whether it decides a question that would otherwise be decided by whoever types first.

The one rule that outranks every interaction preference:

> **A user-visible fact has exactly one owner, and every surface reads it from there.**
> Root [`CLAUDE.md`](CLAUDE.md) invariants 12 and 13.

That owner is a Rust function whenever one can hold it — `outl_shortcuts::support` for "can I do this here",
`outl_actions::refusal` for "who tells the user we declined", `outl_actions::reminders::next_fire_at` for
"when does this fire".
A second opinion in TypeScript or Swift is not a convenience; it is drift that reaches the user before it
reaches a test.

### The boundary with `DESIGN.md`

| The question | Owner |
|---|---|
| What colour is this, how big, how round, how far apart | [`DESIGN.md`](DESIGN.md) |
| What happens when I press it, and what if it can't happen | this file |
| Which chord reaches which action | [`docs/shortcuts.md`](docs/shortcuts.md) |
| Which client performs which action, verbatim | [`docs/client-parity.md`](docs/client-parity.md) — generated |
| How a client is wired to the shared crates | [`docs/clients.md`](docs/clients.md) |
| How mobile behaves, feature by feature | [`docs/mobile-ux.md`](docs/mobile-ux.md) |
| Why a design was chosen over its alternatives | [`docs/rfcs/`](docs/rfcs/README.md) |

The two files overlap in exactly one place, deliberately: an affordance has a look *and* a behaviour.
When that happens the visual half stays in `DESIGN.md` and this file links to it — the hidden-until-hover
fold chevron is specified as a layout rule there, and appears here only as the reach question it raises.

## Who this is for

### The reference user

One person, thinking in nested bullets, on more than one device, who already lost notes once.

They are comfortable with a keyboard-driven tool and with the filesystem.
They expect to be able to `grep` their notes, open one in vim, and have outl notice.
They are not a team: there is no shared workspace, no permissions model, no "who changed this" question
that resolves to another human — [`docs/why-outl.md`](docs/why-outl.md) owns that boundary.

The workspace this product is actually daily-driven against is ~2,500 pages and ~64,000 blocks,
migrated out of Roam.
Every performance and scale decision in the product is calibrated to that, not to a fresh workspace —
see [RFC 0128](docs/rfcs/0128-boot-and-memory-at-scale.md) and
[RFC 0137](docs/rfcs/0137-storage-scale.md).
A feature that is pleasant at 50 pages and unusable at 2,500 has not shipped.

### What they already know

They arrive fluent in a vocabulary we did not invent, and we do not rename it:
`[[ref]]`, `#tag`, the daily journal, `Tab` / `Shift+Tab` to indent, `((block-ref))`, zoom-into-a-block.
Roam and Logseq taught them these, and a gratuitous rename costs a returning user more than a better name
earns them.
Where we differ from their muscle memory, the difference is load-bearing and stated —
`id::` lines are gone because the markdown is theirs
(root [`CLAUDE.md`](CLAUDE.md) invariant 2), not because a different syntax looked nicer.

### What makes them leave

In order of how fast it ends the relationship:

1. **Work disappeared.** Not "is hidden", not "is on the other device" — gone.
2. **Two devices disagree and nothing says so.** A silent divergence is discovered weeks later, by which
   time there is no memory of which side was right.
3. **The markdown stopped being theirs.** Metadata in the file, a proprietary re-encoding, an unreadable
   diff.
4. **The tool interrupted them.** A modal on launch, a buzz they did not ask for, a keystroke that
   blocked on a disk write.
5. **A key did nothing.** Not an error — nothing. Indistinguishable from a bug in the tool, a bug in the
   terminal, or a bug in their own fingers.

Every principle below traces to one of those five.

## Where they are when they use it

Context is not decoration around the interaction; it is what decides whether the interaction is correct.
Each row below changes what a correct answer *is*.

| Context | What it changes |
|---|---|
| **Offline, partitioned, for days** | There is no server to ask, no "sync now" that can succeed. Every operation must be complete locally, and convergence is something that happens later without asking. |
| **Two devices edited the same page offline** | The merge cannot be a dialog — nobody is there to answer it. It resolves deterministically through the tree CRDT ([`docs/crdt.md`](docs/crdt.md)), and a move that would create a cycle is a no-op on the tree while staying in the log. |
| **Asleep** | A fire landing inside quiet hours is pushed to the end of the window, never dropped. You asked for it; you get it, just not at 3am ([`docs/reminders.md`](docs/reminders.md#quiet-hours)). |
| **A terminal, possibly over ssh, possibly inside tmux** | There is no OS appearance API, no notification centre worth the name, no background presence. `mode = "auto"` resolves to the dark side and says so; a reminder due with the TUI closed is lost to that client, and that is recorded rather than papered over. |
| **A phone** | There is no `outl` binary and no `git`. Recovery copy that says "run `outl reconcile`" is wrong here, so mobile's banner says to open the workspace on a computer. Automatic backups do not exist on iOS for the same reason. |
| **A soft keyboard covering half the screen** | Chords are not available. Every action a phone user needs is reachable by touch, or it is declared missing with a nudge saying what to do instead. |
| **An OS watching the main thread with a stopwatch** | iOS allows a scene update 10 wall-clock seconds before `FRONTBOARD` sends `SIGKILL`. A slow answer here is not a stall, it is no app — and there is no error, no dialog, no crash screen, just a white one. "Do it synchronously and accept the wait" is therefore survivable on the desktop and fatal on the phone, and the thing that kills you is usually a *poll* rather than a one-off: 342 ms of work, reached by a 5s timer plus the per-peer catch-up plus the ingest, came to ~100 calls inside one 10 s window — 33 s of CPU billed against a frame that never arrived. |
| **~2,500 pages, ~64,000 blocks** | Any list with no natural bound needs one: a namespace's mentions are capped at 50 with the real total beside it, because 3,221 of them is an unreadable panel and ~292 KB on the IPC per page open. |
| **An external editor touched the file** | vim, Obsidian, a script, a peer still on an older binary. The parser never drops a line it does not understand; it keeps it as a block and records a warning. A page that cannot be read at all is never overwritten. |
| **The process died mid-write** | The op log is written synchronously, the `.md` after. A crash between them leaves the file briefly behind the log — recoverable by construction, because the log is the source of truth and the `.md` is a projection. |
| **The user is in the middle of a sentence** | Nothing that is not the user's own keystroke may take focus, steal the cursor, or block the frame. Hot-reload refuses to clobber an in-flight edit and writes to the status line instead. |

## Principles

These are the tie-breakers.
When two reasonable designs disagree, the one that satisfies the earlier principle wins.

### 1. Journal-first

The product opens on today.
Not a dashboard, not a file tree, not a "welcome back" screen — the journal for today's date, cursor ready.
A page you have to navigate to is a page you write in less.

### 2. Never block a keystroke

Every client is async-on-write: the op log write is synchronous, the `.md` projection is not, and no
keystroke, command reply, or frame waits on a render, an fsync, or a backlink rebuild
([`docs/clients.md` → Async projection writes](docs/clients.md#async-projection-writes-performance)).

The same rule governs everything that is not the user: automatic backups run on a background thread after a
startup delay, and never on the edit path, the quit path, or an idle hook the render loop waits on.
Plugin `onOp` hooks are fire-and-forget so a slow plugin cannot stall the next keystroke.

The user-visible consequence is deliberate and worth stating: **pressing `Esc` does not mean the bytes are
on disk yet.**
It means the op is in the log, which is the thing that survives.
The TUI drains the write the moment you pause, forces it if you keep a burst going, and always flushes
before quitting, `Ctrl+S`, navigation, or a peer reload — [`docs/tui.md`](docs/tui.md#behavior-worth-knowing)
owns the timings.

**It covers reads too, and that half is where it breaks.**
Work the user did not ask for, and cannot usefully wait on, loads *behind* the page.
The page paints, and the only element allowed to show a pending state is the section whose own content is
missing.
A load state that encloses the editor is the same defect as a blocking write, because what it takes away is
the input the user still had.

The desktop regressed exactly there.
Backlinks are an `O(blocks-in-workspace)` scan, and the page carried the load state for it, so on a large
workspace every navigation took the outline, the caret and the keystrokes with it.
The missing piece was never the background fetch, which has been off the page-open path since the "Esc is
slow" fix.
It was a home for "this part is still loading" that was smaller than the page.
`trackBacklinks` (`outl-desktop/src/lib/store.ts`) is the single owner of that flag for all three fetch sites,
`<InlineBacklinks />` is its only reader, and `InlineBacklinks.test.tsx` → "backlinks load behind the page"
pins that the editor keeps focus while the reply is in flight.
The backend half is the same sentence one layer down: a synchronous `#[tauri::command]` runs on the IPC
thread, so whatever holds the workspace lock holds the window, and `compute_backlinks_offloaded` takes it for
the lookup alone.

**On a phone that sentence does not describe a stall, it describes the end of the app.**
Under `wry` the IPC thread *is* the UI main thread, and the desktop's worst case — a window that stops taking
keystrokes until the reply lands — is on iOS a `SIGKILL` from the scene-update watchdog.
`peer_status` was a synchronous command, polled every 5s by both clients, and it opened the whole JSONL store
(`JsonlStorage::open` → every actor index → every sidecar) to answer one column.
Nine crash reports off one phone, one per launch, every one `EXC_CRASH` / `FRONTBOARD` `0x8BADF00D`, all with
the identical `com.apple.main-thread` stack bottoming out in `NodeIndex::insert`, with 33 s of CPU inside a
10 s window.
**What the user saw was a white screen** — not an error, not a crash dialog, nothing, which is principle 3
with the process gone before it could speak.
`crates/outl-tauri-shared/src/offload.rs` is the single owner of the fix: `offload::run` is the one
`spawn_blocking` site, and the async command count in `wrappers/catalog.rs` went from 2 to 22.
Making the read cheap was the other half and is not a substitute for it — without the offload the watchdog
stops firing and the phone just cooks instead.

**Holding the lock is the second way to block the user, and it is independent of which thread you are on.**
Moving a body to the pool makes the stall concurrent, not free: the UI still waits on the same workspace
mutex, and `parking_lot` has no timeout, so a command that arrives mid-hold simply stops.
Measured on a clone of a real 2,923-page / 20-actor / 366 MB workspace, release: `open_today_journal`'s own
work is **1.8 ms**, and it was observed waiting **374–1,056 ms** on the mutex alone.
The holder was the background reconcile sweep, which starts on `workspace-ready` — exactly when the frontend
asks for today's journal, so every launch booted into that window.
On a first launch mobile held it **continuously for 2,557 ms** — back-to-back `reconcile_md` calls on a
blocking `lock()` with no yield, inside the first second, against 407 ms on the desktop.
After: two windows of ≤234 ms, nine seconds in, which is out of the launch window rather than shorter.
`crates/outl-tauri-shared/src/reconcile.rs` owns the sweep now and both clients' `workspace_open.rs` are
adapters over it; every lock is `try_lock` + retry, never `lock()`, and a busy unit is retried rather than
skipped, because a `continue` steps over a page for good.

Two phases are deliberately **not** chunked, and the reason is a UX reason: **a wrong repair is worse than a
slow one.**
`repair_namespaced_titles` builds a mention index over every block in the workspace precisely so it can
*refuse* a slug two spellings claim, and a partial index is exactly how one spelling is seen when two exist.
Its write is then permanent, because the pass skips any page that already has a title, so it can never
correct itself — chunking it would trade a 185 ms wait for a wrong title nobody can see is wrong.
`scan_for_desynced_projections` was not chunkable from that crate without re-implementing a predicate that is
private to `outl-actions`, which is a parallel implementation (principle 8).
Both simply wait out the launch window instead, which is the honest answer: both are idempotent, both ran on
every previous boot, and both will run on the next one.

Before `reconcile.rs` existed the two clients had **drifted** in the shape this file exists to prevent: the
desktop had no desync-recovery pass at all, mobile had one with a blocking lock and no yield, and mobile's
comment claimed it "mirrors the desktop".
Nothing could fail.

**Nothing heavy goes in front of the first paint, and "after the await" is not after the paint.**
Mobile fired `syncNow()` at mount, ahead of the journal load, on the argument that dialling early starts
punching the NAT path sooner — true, and paid for with the only thing the user is waiting for, because
`outl_sync_now` dials every known peer from the thread that has to draw today's journal.
Moving it behind the paint took the date and the week strip from **+2,158 ms to +39 ms**, and the outline from
+2,158 ms to +1,261 ms.
`runBootSequence` (`crates/outl-mobile/src/components/Journal.boot.ts`) is the single written-down order, and
it waits on a real presented frame rather than on `load` resolving: `load` resolving means the signals are
set, so a command issued in that continuation still lands on the main thread ahead of the pixels.

The load gate was already scoped to the outline `<section>` rather than the page, and still behaved
page-wide, because everything around it read `view()` — which arrives in the same reply.
So the chrome was given sources that need no workspace: the week strip paints immediately from
`today_slug_cmd` (pure date math, no lock), and the header says "Journal / Today" from the first frame instead
of falling back to the label "Page" over an empty title, which is not a placeholder but a wrong answer held
for as long as the open takes.
Day navigation cannot re-enter the load state, and the retry path clears the gate only when there is nothing
on screen to lose; both are pinned by tests.

**The floor is honest, and a future contributor will want to "fix" it.**
The outline itself stays gated on the workspace being open.
There is no block id before `create_block` replies, and an editable line not backed by a real id either loses
the user's keystrokes or writes them against an id the op log never issued.
A fake editor is the one thing not to ship here — it converts a wait into deleted work, which is principle 4
read forwards: refuse to show an editor you cannot stand behind rather than take input you cannot keep.

So before shipping anything that lands after the first paint, answer three questions: **what is the user doing
while it runs, which element says it is running, and what is it holding while it holds it?**
When the answer to the second is "the page", it is the wrong element.
When the answer to the third is "the workspace mutex, for the whole pass", the pass is wrong wherever its
thread happens to be.

### 3. Silence is the defect

A refusal that reaches only a log line is the bug, not the fix.

outl declines writes on purpose in several places — that is principle 4 — and every one of those refusals
is owed to whoever asked for the write.
`outl_actions::refusal` is an exhaustive `match` over (refusal, surface) for exactly this reason: a new
surface does not compile until every refusal declares what it does there, and the table in
[`docs/clients.md`](docs/clients.md#surfacing-a-page-that-stopped-syncing) is generated from it.
The surfaces are **five** — TUI, desktop, mobile, CLI, MCP — because a refusal is owed to a program
calling the MCP just as much as to a human, and a program will retry where a human would notice
([RFC 0255](docs/rfcs/0255-operation-vocabulary.md)).

A banner that outlives its condition is the same defect mirrored.
Only the open commands run the ahead-of-log check, so the first checked reply with no notice clears the
banner — never a mutation reply, which would clear it on the user's first edit, the exact action the
banner warns against.

**And a third shape, which is neither a refusal nor a banner: a best-effort call that silently takes
something else down with it.**
`listenForFileDrop` is explicitly best-effort — the OS rarely delivers a webview drop on a phone, and a
failed registration just leaves the long-press "Attach file" as the import path.
But `installFileDrop` reached `getCurrentWebview()` *before* returning a promise, so the trailing `.catch`
was not attached yet when that threw, and the throw took the rest of `onMount` with it — which contained the
journal load.
The app painted its chrome and never loaded the page, and said nothing, because the only thing that was
allowed to fail quietly was not the thing that failed.
"Best-effort" is a claim about one call's own outcome, never a licence for it to decide anything about the
calls after it; the fix is one line, and the cost of being wrong about which environments can reach it is the
whole journal.

### 4. Refuse rather than delete

When outl cannot tell whether a write would destroy content, it declines and says so.

The canonical case: a page's `.md` holds lines that exist in no op.
Overwriting it projects the tree over content the log never saw, and those bytes are gone.
So the write is withheld, the page still opens showing what is on disk, and the user is told — the cost
being that the page is frozen in both directions until `outl reconcile --ahead-of-log` runs
(root [`CLAUDE.md`](CLAUDE.md) invariant 8, [RFC 0210](docs/rfcs/0210-md-content-outside-op-log.md)).

The trade is stated because it is not free: **a stale view is recoverable, deleted bytes are not.**
The same reasoning gives the TUI its `cannot read <path> … editing disabled` state — a failed read parses
as an empty document, and without the guard the next commit renders that emptiness back over the page and
sends every block to the trash on every device.

Deletion itself follows the rule: a deleted block is moved to `TRASH_ROOT`, never physically removed.

### 5. View state is local; disagreement converges

Before adding state, ask whether two devices can disagree about it and whether they should reconcile.

- **Yes → it is an `Op`.** Fold state is `Op::SetCollapsed`; a snooze is `Op::SnoozeRemind`.
- **No → it never leaves the device.** Zoom is pure frontend state that never reaches Rust. Theme
  choice, backlinks order, quiet hours and the enabled flag live in device-local config. "This device
  already fired that reminder" lives in a workspace dotfile that no transport replicates.

The reminders split is the clearest statement of the rule: snoozing on the phone must silence the laptop,
and the phone having buzzed must not stop the laptop from buzzing.
Getting this backwards produces either a note that syncs your scroll position or a snooze that only
worked on the device you were holding.

### 6. Interruption is always opt-in

A `[[date]]` alone never schedules anything.
People use dates for backlinking, and the moment a link becomes a buzz, the linking stops.
Notification permission is asked at the first actual fire, not when a switch is flipped.

The same instinct governs the rest of the product: no required tags, no mandatory daily-review modal, no
onboarding that must be dismissed before the first bullet.

### 7. Structure on demand, prose always

Chrome that explains the outline — fold chevrons, gutters — is quiet at rest and appears on hover, focus
or selection.
Chrome that *is* the outline — the bullet, the indent guides — is always visible.
The specification, including why an indent guide must not fade and why `:focus-within` is load-bearing
rather than decorative, is in [`DESIGN.md` → Layout](DESIGN.md#layout).

### 8. One owner per user-visible fact

Covered above, and the whole reason this file exists as a separate document rather than as prose spread
across three client crates.

## Interaction patterns

### What we say, and where it lands

Five channels, and picking the wrong one is how a real problem becomes invisible.

| Channel | Use it for | Lives for | Wording owner |
|---|---|---|---|
| **Status line** (TUI, desktop) | The thing you just tried did something other than what you expected — a gap, a partial, a refused save | Until the next message | `outl_shortcuts::support` for gaps; the `ActionError` for refusals |
| **Toast** | A transient outcome you did not initiate — a peer reload declined, a backup failed | Seconds | The error's own `Display` |
| **Sticky banner** | A condition that persists and blocks convergence — a page ahead of the log, parse warnings on the open page | Until the condition clears, verified by a checked reply | `@outl/shared/warnings` |
| **Structured error** (CLI `--json`, MCP) | A program asked and a program must be able to tell "this page stopped syncing" from "that failed" | The reply | `ActionError`'s `Display`, forwarded verbatim |
| **Nudge** | The user reached for something this client does not do | Until the next message | the catalog, never the client |

Two rules cut across all five.
**One condition gets one sentence** — the MCP forwards `ActionError`'s own `Display` rather than writing
a second sentence for the same thing, because two owners of one sentence is two sentences that drift.
And **a console is not a surface**: the desktop dispatcher once logged `console.warn` for an unhandled
action, in a comment that called DevTools output something "the user sees".

### When a client cannot do the thing

Not every client does everything, and that is fine.
What is not fine is a key that does nothing, because the user cannot tell a gap from a bug.

`outl_shortcuts::support(action)` is the single owner — an exhaustive `match` over every action in the
catalog × every client — with `outl_shortcuts::capability_support` doing the same for the capabilities that
have no chord at all (page history, the plugin marketplace, the calendar, assets, peer pairing…).
A new variant of either does not compile until every client declares its verdict.

The five verdicts, and what each one promises the user:

| Verdict | Means | Example |
|---|---|---|
| `Full` | The user can do it here. Chord, button or gesture — the distinction is not the user's problem | — |
| `Native` | The platform does it, outl has no handler and needs none | `Backspace` on an empty textarea |
| `Partial` | Reachable, not with the full semantics the name implies | "No character cursor on the desktop, so `a` behaves like `i`." |
| `Missing` | Should exist here, doesn't yet. Says what to do instead | "Pinning a page isn't on the desktop yet — pin it from the TUI with `g P`." |
| `NotApplicable` | Cannot exist here by construction. Says why | "Mobile is single-pane — the page switcher replaces the sidebar." |

`Native` is its own state rather than a boolean's rounding error: "reachable" and "has a handler" are
different questions, and a client test asserting "every `Full` action has a handler" must not demand one
where there is nothing to hand off to.

A chord with no handler falls through without `preventDefault` — the textarea or the OS still gets the
key — *and* surfaces the nudge.
Swallowing the key and saying nothing is the worst of both.

### Confirmation and destruction

Confirm what cannot be undone from inside the product, and nothing else.

Destructive intent is signalled visually with its own colour role (`destructive`, never `warn` — see
[`DESIGN.md` → Roles](DESIGN.md#roles)), and destructive gestures are deliberately harder than their
neighbours: deleting a page from the mobile switcher is a sustained long-press that cancels if the finger
moves, then a confirm.
Journals are excluded from that path entirely.

Everything reversible skips the dialog.
Block deletion is a move to the trash, so it is undo, not a question.

### Undo, history, and recovery are three different questions

Users conflate these; the product must not.

| The question | The answer | Scope |
|---|---|---|
| "Undo that" | `outl_actions::history` — bounded snapshot stacks, each remembering selection and cursor | This session |
| "What did this page say last Tuesday" | `outl_actions::timeline` → `outl page history` / the desktop `⏱` panel. **Read-only on every surface** | The op log |
| "A truncating edit ate my text" | `outl recover`, restricted to a provably-additive rule | One narrow case |
| "This `.md` holds lines the log never saw" | `outl reconcile --ahead-of-log` | One page |
| "Give me the whole folder as it was" | `outl backup restore` — device-local git snapshots outside the workspace | The workspace |

History is read-only on purpose: a general "put this back" button needs its own safety argument, and does
not have one yet.
A deleted block stays in its page's history with the text it held when it went — a history that omits
deletions answers "what changed" with everything except the change people opened it to find.
And not every op is an event: folds, snoozes, and re-emitted ops that changed nothing are skipped, because
a reconcile produces them in volume and a real edit was once buried under six of them.

### Going somewhere that may not exist yet

Tapping `[[avelino/outl]]`, `#code-review`, `[[2026-06-04]]` or a picker row must never split the
journal-vs-page decision between a frontend regex and a backend parser.
`outl_actions::open_or_create_by_ref` runs the whole decision tree in one place, and the behaviour it
guarantees is that **a ref always lands somewhere** — an existing page, a journal, or a page created on
the spot with the typed string kept as its title.

This is not a hypothetical split.
`[[2026-13-01]]` once matched a frontend `^\d{4}-\d{2}-\d{2}$` shape check, hit the strict date parser,
and produced an `invalid date slug` toast for what should obviously have been a regular page.

The `@` mention is the same path with one visual affordance: a word-initial `@` opens a picker over pages
with `type:: person`, accepting one inserts `[[@name]]`, and the `@` is presentation only — page identity
never carries it.
A person page that does not exist yet is created with `type:: person` set, so the next `@` finds it
without anyone editing a property.

### Zoom, and gestures that must not collide

Zoom makes one block the outline root.
It is local view state (principle 5), resolved against the live outline on every render — so an edit
inside the zoom stays reflected, and a target that was deleted or moved away falls back to the full page
instead of showing an empty screen.
Switching pages drops it.

On touch the gesture budget is small and every tap target is spoken for: the plain bullet dot zooms, the
TODO checkbox cycles state, the collapse triangle folds.
Adding a gesture means finding one that is free, or moving an existing one somewhere still reachable —
zoom took the plain dot's old mark-as-TODO tap, and TODO moved to the long-press menu rather than
disappearing.

Horizontal swipes have two owners on a phone, so each gets its own band of the screen.
A row owns its own swipe (delete, indent), so the outline never changes day under a finger that meant
to act on a block.
Changing day by swipe lives only on the journal header and the week strip above the outline.
A swipe there has to be mostly sideways, at least 56 px long, quicker than 700 ms, and must not start
within 24 px of a screen edge, where iOS keeps its own back gesture.

Choosing a block from search zooms into it when it has children, on both the phone and the desktop
palette, so the hit is the whole view instead of one row on a long page.
A leaf block opens in its page with the cursor on it.

### Getting around on a phone

The bottom bar is the phone's map: Journal, Search, Tasks and Reminders, with a quick capture button in the middle.
Search is the one door to pages, tasks and blocks, so there is no separate Pages tab.
It sits within thumb reach and hides while the keyboard is up, because the keyboard toolbar owns that
space during an edit.

Quick capture always writes to today's journal, wherever the user is standing.
On today it appends a block and focuses it in place.
On any other page it opens a small sheet that adds the line to today without moving the user, and says
so with a toast.
Sending a thought somewhere else is a second step, not a question asked before the thought is down.

The week strip under the journal header shows the current week with a dot on each day that holds text.
Opening a day creates its page, so the dot means "a block has text", not "the file exists".

Coming back to the app after 30 minutes, or on a new calendar day, lands on today's journal.
It never moves the user while a block is being edited, and it does nothing when today is already on
screen.

Tasks lists the workspace's open TODO and DOING, grouped by page, with DOING first — the first 200,
with the uncapped count beside them.
Tapping one opens its page zoomed into that task.

The header's gear opens the one surface on this client that shows **preferences** rather than workspace
content, and it opens on **Devices and sync** — which is how pairing is reachable *by name*.
Press-and-hold on the sync dot gets there faster and is still there, but a feature whose only entrance is a
gesture reads as absent to anyone who went looking for it, which is a key that does nothing wearing a
different disguise.
Feature-by-feature behaviour, including what that sheet's two toolbar controls do, is in
[`docs/mobile-ux.md` → Locking the order](docs/mobile-ux.md#locking-the-order-settingssheet).

### Making a block and typing in it

A tap that creates a line and puts the cursor in it has to bring the keyboard with it, and on a phone that
is a rule about *when* the focus happens rather than about whether it happened.

The reported symptom was "the + button doesn't let me type".
WKWebView raises the software keyboard for a programmatic `.focus()` only while the user-gesture window
opened by the tap is still open, and creating a block is two awaits deep — commit whatever draft is open,
then `create_block` over IPC.
By the time the new textarea mounted and focused itself, the gesture context was gone.
The element really was focused (the accessibility tree said so); the keyboard simply never came, which is
the worst available failure because the UI looks correct and the user is wrong about their own phone.

**So the keyboard is armed synchronously, inside the tap, before the first `await`.**
`crates/outl-mobile/src/lib/keyboard-keeper.ts` is the single owner: a real, laid-out, 1×1 `opacity: 0`
textarea on `document.body` takes focus inside the gesture, which raises the keyboard; focus then moves to
the real block's textarea when it mounts, and iOS does not dismiss a keyboard for a move between two text
inputs.

Two consequences are the actual rule, and both are about not lying to the user:

- **Keystrokes typed during the handoff are carried, not dropped.** They land in the keeper and are seeded
  into the new block's draft, reaching the op log through the ordinary commit as an edit against the id
  `create_block` returned. There is no optimistic block and no invented id (principle 4, and the floor in
  principle 2).
- **A failed create gives the keyboard back** rather than leaving it up over a field that is not there.

Every create-and-type affordance goes through this: "+" on today, "+" elsewhere (the capture sheet), "Tap to
start writing", "Add a block", the first-run tips, and long-press → "New block above" — that last one arms
before its `queueMicrotask`, because a microtask does not carry the gesture.
`handleCreateAfter` (Return inside a block, the bar's "New line") deliberately does not: the keyboard is
already up, and stealing focus from the open textarea would commit its draft.
The per-path table is owned by
[`docs/mobile-ux.md` → the keyboard handoff](docs/mobile-ux.md#making-a-block-and-typing-in-it-the-keyboard-handoff);
what belongs here is that this is how the next one gets written, on the client where most writing happens.

### Optimistic by default

The reply a client renders is built from the in-memory tree, not from a re-read of the file it just
queued.
Reminders are scanned from the tree for the same reason: reading the projection would have been stale by
construction, and the user who just pressed "remind me" would open the list to find nothing.

**The rule has a boundary, and it is not "be optimistic wherever it feels faster".**
Optimism here means rendering from state the op log already holds, ahead of the projection catching up.
It never means rendering state the op log does not hold — a block with a client-side id, a line the user can
type into before `create_block` has answered, a count assumed rather than read.
The first is a view that is early; the second is input with nowhere to go, and it resolves either by losing
the keystrokes or by writing them against an id no op ever issued.
That is why the mobile outline waits on the workspace while the chrome around it does not (principle 2), and
why the keyboard handoff carries the typed characters instead of pre-creating the block they belong to.

## Voice

User-facing strings are English today (i18n is later, and is not a reason to write English badly).
The rules below are enforced by tests, not by review.

**Say what the user can do instead.**
A nudge is not an apology and not a status report.
`Missing` and `NotApplicable` states carry a sentence that names the alternative or the reason.

**Never leak developer vocabulary.**
`unimplemented`, `not implemented`, `todo`, `fixme`, `no handler`, `handler`, `dispatcher` — banned by
`DEV_WORDS`, checked against every nudge in the catalog.
A gap explained in the implementation's words is the same failure with a nicer transport.

**Be long enough to help.**
A nudge under 20 characters fails its test.
"Not supported" tells the user nothing they had not already worked out by pressing the key.

**Name the platform's actual reality.**
The ahead-of-log banner says "run `outl reconcile --ahead-of-log` in the workspace folder" on desktop and
"open the workspace on a computer" on mobile, because there is no `outl` binary on iOS.
Copy that points at a terminal that does not exist is worse than no copy.

**One condition, one sentence, one owner.**
`@outl/shared/warnings::aheadOfLogNotice` owns the GUI wording; the MCP forwards the Rust `Display`
verbatim rather than growing a second copy.

## Glossary

The domain words, as the user means them.
Definitions live in [`docs/concepts.md`](docs/concepts.md); this is the list a feature author needs so a
new surface does not invent a synonym.

**Workspace** — a directory. One `outl` process is bound to one; there is no workspace switcher.
**Page** — one `.md` file holding one outline. Named by `title::`, filed by [slug](docs/concepts.md#slugs).
**Journal** — a page keyed by date. The default landing surface.
**Block** — one bullet. The unit of reference, movement, undo and sync.
**Property** — `key:: value` on a page (top of file) or a block (nested under it).
**Ref / tag** — `[[page]]` and `#page` resolve to the same file and both count as backlinks; the
difference is presentational.
**Mention** — `[[@name]]`, a ref whose `@` is an affordance, resolving to a `type:: person` page.
**Namespace** — a `/` in a page *title*. `os` lists and collects `os/linux`; the slug on disk is flat.
**Backlink** — a block that points here. Distinct from **nested pages**, which are pages that live under
here; the two come from different sources and a client can ship one without the other.
**Sidecar** — the `.outl` JSON next to each `.md`, holding block ids. Never visible in the markdown.
**Op log** — the append-only truth. One file per device.
**Zoom / focus** — one block as the outline root. Local, never synced.
**Actor** — one device's write identity. Users never see the word; they see "this device".

### Three vocabularies, on purpose

The same operation is spelled three ways, and this is settled, not pending:

| Surface | Convention | Example |
|---|---|---|
| MCP tools | `noun_verb` | `outl_page_delete` |
| CLI subcommands | `noun verb` | `outl page delete` |
| `outl_shortcuts::Action` | `VerbNoun` | `DeletePage` |

Each is internally consistent; renaming any of them breaks scripts and agent configs to make guessable
names guessable ([RFC 0255](docs/rfcs/0255-operation-vocabulary.md) → Why not the alternatives).
What is *not* optional is that a new operation follows its surface's convention.

### Words we don't use

- **"Note"** for a block or a page — the user has pages and blocks; "note" collapses the distinction the
  whole product is built on.
- **"Save"** as a user action — writes are automatic (principle 2). `Ctrl+S` exists to force a flush, not
  to be the way work persists.
- **"Sync now"** as a promise — there is no server to ask. Peers converge; a button that implies a
  request/response is lying about the architecture.
- **"Conflict"** for anything the CRDT resolved — a converged move is not a conflict the user must
  adjudicate. Reserve the word for the ahead-of-log case, which genuinely needs them.
- **"Delete"** for a block — it moved to the trash. Say so if the distinction matters in context.

## What each client can do

The verbatim answer is generated and test-pinned in
[`docs/client-parity.md`](docs/client-parity.md) — every action and every capability across TUI, desktop
and mobile, regenerated with:

```sh
OUTL_UPDATE_PARITY_DOC=1 cargo test -p outl-shortcuts
```

The shape of the divergence, and why each one exists rather than being a backlog item:

| Divergence | Why it is not a gap |
|---|---|
| Mobile binds no chords | Touch and a soft keyboard. `docs/shortcuts.md` leaves the column blank rather than inventing a row. |
| The desktop has no character cursor inside a block | Its vim mode holds a selected block id, not a caret. Ten char-level ops surface one shared nudge written once in the catalog. |
| `mode = "auto"` resolves dark on the TUI, always | A terminal has no OS appearance API, so the gap is declared rather than guessed at. [`DESIGN.md` → Theming](DESIGN.md#theming--light-dark-and-who-resolves-it) owns the reasoning. |
| No automatic backups on iOS | There is no `git` binary. |
| The TUI can miss a reminder | A terminal session has no background presence. Recorded rather than papered over. |
| Only the desktop resolves keystrokes through `outl_shortcuts::lookup()` | The TUI still dispatches Normal mode from its own `match`. This one *is* open work rather than a settled divergence, and it is why a chord added to the catalog does not automatically reach the TUI. |

A capability that exists on no client is not in the catalog at all — the catalog answers "who does not
have this", not "what could we build".

## Accessibility is a reach question

The visual half — contrast obligations, reduced motion, selection never being colour-only — is specified
in [`DESIGN.md` → Accessibility](DESIGN.md#accessibility).
What belongs here is whether an affordance can be *reached*:

- **Hidden chrome must stay keyboard-reachable.** Chrome revealed on hover is also revealed on
  `:focus-within`; without that, "quieter at rest" silently means "mouse-only".
- **Every icon-only control is labelled with what it will do to what** — `Delete page "{name}"`, not
  `Delete` — because a label read out of context is the only context a screen-reader user gets.
- **Decorative structure is hidden** from assistive tech: an indent guide is `aria-hidden`, so nesting
  comes from the DOM tree rather than from a column of spacers.
- **A message must land somewhere the user is looking.** The status line and the banner qualify; a
  console does not.
- **Touch targets are sized for a fingertip and gestures never overlap** — see Zoom above.

## Findings that became constraints

Research synthesis, in the only form that survives: incidents with numbers, and the behaviour each one
changed.

| What happened | What it changed |
|---|---|
| A fix for "tree ahead of `.md`" (#166) re-projected whenever a hash matched. On a 2,560-page workspace that deleted 233 pages holding 1,426 lines that existed in no op — and `doctor --repair` printed `708 fixed` while doing it | Principle 4. A hash match proves outl wrote the file last, not that the log holds its content ([RFC 0210](docs/rfcs/0210-md-content-outside-op-log.md)) |
| That refusal then reached only a backend log line — the page appeared to freeze with nothing said | Principle 3, and the five-surface refusal matrix ([RFC 0255](docs/rfcs/0255-operation-vocabulary.md)) |
| "Does the desktop bind `y r`?" had three answers in three docs, all stale in different directions. Both `y r` and `:` were dead keys logging to a console | `outl_shortcuts::support`, the generated parity table, and the nudge ([RFC 0253](docs/rfcs/0253-client-capability-catalog.md)) |
| `#os/linux` parsed, resolved, and did nothing a hierarchy implies. When the listing shipped it read `title::` — and a real workspace had 14 titles across 2,575 pages, so it rendered empty everywhere while the backlink half credited 3,221 blocks under one namespace | `repair_namespaced_titles`, plus the 50-item cap on namespace mentions ([#275](https://github.com/outlmd/outl/issues/275)) |
| "Only the GUI binds the iroh endpoint" was a policy, so a headless `outl mcp serve` machine synced with nobody, silently | Root [`CLAUDE.md`](CLAUDE.md) invariant 10 — when you change who decides, enumerate who was standing on the old decision ([#220](https://github.com/outlmd/outl/issues/220)) |
| `[[2026-13-01]]` matched a frontend shape regex and produced `invalid date slug` | `open_or_create_by_ref` as the single decision tree |
| A block's real edit was buried under six non-events in its own history on a 64k-block workspace | `timeline`'s "not every op is an event" rule |
| A synchronous `peer_status`, polled every 5s, opened the whole JSONL store on the iOS main thread to answer one column. The app was killed on **every launch** — nine crash reports, one per launch, all `EXC_CRASH` / `0x8BADF00D` with the same `com.apple.main-thread` stack, 33 s of CPU in a 10 s window. The user saw a white screen and no message | Principle 2 covers the kill, not just the stall; `offload::run` as the one `spawn_blocking` site (async commands 2 → 22), and the platform watchdog as a context row above |
| `open_today_journal` is 1.8 ms of work and was waiting **374–1,056 ms** on the workspace mutex, held by the boot sweep that starts at the exact moment the frontend asks for today — 2,557 ms unbroken on a first mobile launch. The two clients had meanwhile drifted: no desync pass on the desktop, a blocking one on mobile, and a comment claiming they mirrored | `reconcile.rs` as the single owner, `try_lock` + retry everywhere, and the two un-chunkable phases waiting out the launch window instead — a wrong repair is worse than a slow one |
| One unlayered `button { color: … }` in mobile's stylesheet outranked every layered Tailwind utility regardless of specificity, so **no text colour set on a button on that client did anything**. Reported as one symptom: the week strip's selected day painting an accent tile with near-black text in it | The visual half is [`DESIGN.md`](DESIGN.md)'s. The finding here is that a one-surface bug report was a whole *class* of surfaces failing silently, so "where else does this rule apply" is part of reading a report, not part of the fix |

Two shapes account for all of them.
Most are **a fact with more than one owner, and nothing that could fail when the copies disagreed.**
The rest are **a cost nobody attributed to a surface** — the thread it runs on, the lock it holds, the
cascade layer it lands in — where the symptom surfaces a long way from the line that caused it, and the
report names one instance of something that was failing everywhere.

## Do's and don'ts

**Do** land a new user-visible fact in a Rust function that every surface reads.
**Don't** mirror it in TypeScript "just for this client" — that is how a schedule fires at 3am on one
device.

**Do** declare what every client does with a new action or capability.
**Don't** ship a chord with no handler. A key that does nothing is indistinguishable from a bug.

**Do** write the nudge in the catalog, saying what the user can do instead.
**Don't** write it in the client, and don't write it in the implementation's vocabulary.

**Do** refuse a write you cannot prove is safe, and tell every surface that asked.
**Don't** let a refusal reach only a log line, and don't leave the banner up once the condition clears.

**Do** ask whether two devices can disagree about a piece of state.
**Don't** put converging state in a file with last-write-wins semantics, or local view state in an `Op`.

**Do** make interruption opt-in, once, explicitly.
**Don't** turn a link into a notification.

**Do** keep the write off the input path.
**Don't** block a keystroke, a reply, or a quit on a projection.

**Do** ask which thread a command body runs on and what it holds while it runs.
**Don't** assume a background thread is invisible — it shares the workspace mutex, and on iOS a busy main
thread is a `SIGKILL`, not a slow frame.

**Do** let a best-effort call fail on its own.
**Don't** let it decide anything about the calls after it.

**Do** test a feature against ~2,500 pages before calling it done.
**Don't** let a list grow without a bound and a stated total.

## Adding something user-visible, end to end

1. **Name it in the user's vocabulary**, following the surface's convention (Glossary above). Check the
   word is not already taken by a different concept.
2. **Decide whether it converges.** Two devices disagreeing → `Op`. Otherwise device-local, and say which
   file holds it and what cleans it up (root [`CLAUDE.md`](CLAUDE.md) invariant 9).
3. **Put the logic in `outl-actions`** if more than one client needs it — before its first use, not after
   the second client copies it ([`docs/clients.md`](docs/clients.md#when-to-put-logic-in-outl-actions)).
4. **Declare the verdict for all three clients** in `support.rs` (it has a chord) or
   `capability_support.rs` (it does not). The lesser states need a sentence that passes the vocabulary
   and length tests.
5. **Decide what it says when it declines**, and on which of the five surfaces. If it is a new refusal,
   it is a row in `outl_actions::refusal`, and the CLI and MCP are surfaces too.
6. **Keep the write off the input path**, and name the thread it runs on, the lock it takes, and the element
   that says it is running. A new `#[tauri::command]` is `async` and goes through `offload::run` unless you
   can say why its body can never be slow.
7. **Regenerate the tables** (`OUTL_UPDATE_PARITY_DOC=1`, `OUTL_UPDATE_CLIENTS_DOC=1`) and never hand-edit
   a generated region.
8. **Run `/check`** — both halves. Two of the invariants behind this file are enforced by TypeScript
   parity tests.

If the feature is visual, [`DESIGN.md` → Adding a token or a preset](DESIGN.md#adding-a-token-or-a-preset-end-to-end)
is the other half of this checklist.
