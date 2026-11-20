# Clients and shared logic

outl has multiple clients today (TUI, mobile, desktop) and more coming (plugins).
They all sit on top of the same workspace and the same op log.
To keep them honest, we route every workspace operation through one shared crate: **`outl-actions`**.
The TS+Solid frontends share `@outl/shared` (`crates/outl-frontend-shared`) for everything pure (DTO types, `<MarkdownInline />`, paste helpers, copy wrappers, autocomplete).

## Where each per-feature contract lives

This page owns what every client shares.
The per-feature contracts are one page per question:

| Page | The question it answers |
|---|---|
| [`client-editing.md`](client-editing.md) | What an authoring gesture does on each client: code fences, templates, copy / paste, attachments, task state, blockquotes, zoom, the mobile keyboard bar |
| [`client-backlinks.md`](client-backlinks.md) | What the "Linked from" panel is, what you can do to a row, in what order, and what makes it cheap |
| [`client-page-lifecycle.md`](client-page-lifecycle.md) | History, restoring a deleted block, reopening a deleted page, rename and merge |
| [`client-sync.md`](client-sync.md) | When the `.md` is written, when a re-projection is withheld, backups, and two devices on one folder |
| [`client-outside-the-app.md`](client-outside-the-app.md) | Capture gestures, a file handed over by another app, and reminder delivery |

## The stack

```text
┌──────────────────────────────────────────────────────────────┐
│ Clients                                                       │
│   outl-cli  outl-tui  outl-mobile  outl-desktop  …plugins    │
└──────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│ outl-actions                                                  │
│   block · tree · todo · journal · outline · page · backlinks  │
│   history (bounded undo/redo stacks + .md snapshot restore)   │
│   sync (SyncEngine: reload workspace, reproject page,         │
│         snapshot peer jsonls, scan for orphan .md)            │
└──────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│ outl-md          (.md parse/render, sidecar, matching,       │
│                   inline tokens, outline_ops)                │
└──────────────────────────────────────────────────────────────┘
                          │
                          ▼
┌──────────────────────────────────────────────────────────────┐
│ outl-core        (CRDT, op log, storage trait)               │
└──────────────────────────────────────────────────────────────┘
```

## What lives where

| Concern                              | Crate                           |
|--------------------------------------|---------------------------------|
| Op log, tree CRDT, storage trait     | `outl-core`                     |
| `.md` parse / render, sidecar        | `outl-md`                       |
| Workspace mutations (edit, indent, todo, delete, journal render) | `outl-actions` |
| Committed-mutation undo / redo (snapshot stacks + `.md` restore via reconcile) | `outl-actions::history` |
| Code-block execution (runtimes + orchestration) | `outl-exec`            |
| Cross-client "run a fence" glue (`run_code_block`) | `outl-actions::exec` |
| Tauri command bodies **and the one declaration of the command surface** (`wrappers/catalog.rs`), wire DTOs, plugin thread (Boa `!Send`), `AppHost` / `StorageRootProvider` traits — shared by `outl-desktop` and `outl-mobile` src-tauri. A client's `commands/<module>.rs` is one macro invocation; a client takes a whole module or declares the gap, pinned by `tests/command_parity.rs` | `outl-tauri-shared` |
| The post-mutation commit sequence — undo snapshot, backlink invalidation, peer announce, `.md` projection — as `commit_page` + the `CommitHooks` trait, callable from any client holding a `&mut Workspace` | `outl-actions` |
| TUI: keymaps, modes, overlays, in-flight AST manipulation | `outl-tui`         |
| Desktop: FS watcher, settings IO, Solid frontend (3-pane, OS-standard shortcuts) | `outl-desktop` |
| Mobile: iCloud container resolution, iOS-native bridges (`NSMetadataQuery`, `BGTaskScheduler`), Solid frontend | `outl-mobile` |
| CLI subcommands                      | `outl-cli`                      |
| Chord catalog, and **which client performs which action** | `outl-shortcuts` |

## Workspace locks on open

Every client that opens a workspace takes the same two advisory locks, through the same `outl-core` API.
No client implements the protocol itself.

| Lock | Scope | Taken by |
|---|---|---|
| `<root>/.outl/.lock` | **shared** — every well-behaved opener piles on | `outl_core::WorkspaceLock::acquire` |
| `<root>/ops/.lock-<actor>` | **exclusive** — one process per actor id | `outl_core::ActorWriteLock::try_acquire` |

The shared lock is not mutual exclusion; it is a presence signal.
`outl compact --apply` answers *"is anyone in this workspace?"* by taking that same file **exclusively**, because compaction renames each `ops-<actor>.jsonl` and a live client holds in-memory byte offsets into the pre-compaction layout.
A client that holds nothing lets that gate pass, and every later index-driven read in that session seeks into a renumbered file.
The exclusive per-actor lock is the stated precondition of `JsonlStorage::append_ops`: one writer per `ops-<actor>.jsonl`, ever.

The CLI and TUI resolve theirs through `outl_ws::open`; the desktop and mobile clients resolve theirs through `outl_tauri_shared::workspace_open::open_workspace_at`, which is the **single writer** of the client's `workspace_guards` slot.
It installs the new guards only after the open succeeds, which is also what releases the previous workspace's.
A client never acquires or drops one of these itself.

Both locks are `flock`-based: advisory, machine-local, and released by the OS if a process dies.
They arbitrate between processes on one machine and can never arbitrate between devices — the per-device actor binding (`outl-core`'s `DeviceStore`) is what handles that.

### What happens when the actor lock is contended

A CLI command that appends, and the TUI, mint a fresh ephemeral actor and write to a brand-new `ops-<ephemeral>.jsonl` (`outl_core::resolve_write_actor`).
A CLI command that only reads opens through `outl_ws::open_read` and takes no per-actor lock, so contention does not mint an actor for it.
**The GUI clients refuse the open instead**, and the difference is not an oversight.

A GUI's `HlcGenerator` is built in `setup()`, before a workspace is picked, and every op it stamps carries that generator's actor for the life of the process.
Swapping only the *storage* actor would write ops stamped with the device actor into the ephemeral file and leave two live generators sharing one actor id — two generators that tick in the same millisecond emit the identical `(time, counter, actor)` triple, which *is* op identity, so `Workspace::apply`'s dedup silently drops one of the two ops.
A fallback that loses ops is worse than the collision it dodges.

In practice the refusal is a safety net rather than a path users hit: the desktop is single-instanced by `tauri-plugin-single-instance`, mobile is single-instanced by the OS, and the CLI / TUI resolve a *different* actor for the same workspace (`DeviceStore::device_actor` reads `<config>/actor`, `DeviceStore::actor_for_instance` reads `<config>/actors/<key>`), so they never contend with a GUI.

## When a client can't do the thing

Not every client performs every action, and that is fine.
What is not fine is the user pressing a key and getting nothing, with no way to tell a gap from a bug.

`outl_shortcuts::support(action)` is the single owner of that fact — one exhaustive `match`, so a new `Action` variant does not compile until all three clients have declared what they do with it.
The lesser states (`Partial`, `Missing`, `NotApplicable`) each carry the sentence shown to the user, written **in the catalog** so a client cannot invent its own wording.

The desktop wires it in `lib/shortcuts.ts`: a chord with no handler falls through without `preventDefault` (the textarea or the OS still gets the key) *and* surfaces the catalog's sentence in the status line.
It used to emit `console.warn` instead, in a comment that called DevTools output something "the user sees".

Full table, generated and test-pinned: [`client-parity.md`](client-parity.md).

> **Why this needed an owner at all.**
> The fact had three homes and they disagreed: `shortcuts.md` listed `y r` and `:` as desktop chords (neither has a handler), and mobile undo / redo as "toolbar" (mobile has neither — [#14](https://github.com/outlmd/outl/issues/14)).
> Three hand-maintained copies, each stale in a different direction, and nothing that could fail.
> This is [invariant 12](../CLAUDE.md#critical-invariants-never-violate): **when you add a capability, enumerate who does not have it.**


## When to put logic in `outl-actions`

Yes if any of these are true:

- Two or more clients (today or in the next quarter) need the same op.
- The function takes only `Workspace + HlcGenerator` and returns `Result<_, ActionError>`.
- It produces ops by way of `Workspace::apply` — no direct storage writes, no filesystem touches outside `journal::write_md_atomic`.

No if:

- It manipulates client UI state (selection, modes, toasts, focus, keymaps).
- It manipulates an in-flight `Vec<OutlineNode>` that hasn't been parsed back into a workspace yet.
  Those helpers live in `outl-md::outline_ops`, re-exported through a one-liner shim at `outl-tui/src/outline_ops.rs` because the mobile client needs them too.
  They're workspace-free pure AST manipulation, so they sit in `outl-md` rather than `outl-actions`.
- It's storage-backend-specific (iCloud watcher, future ChronDB) — those implement `outl_core::Storage` in the binary that needs them.

## Surfacing parser warnings on every client

A user can drop a `.md` into the workspace by hand, paste an exported Roam/Logseq tree, or edit a file in vim before outl ever saw it.
When that file doesn't match the outl dialect (e.g. starts with `# heading`, contains a free paragraph, or imports a markdown table), the parser **does not** drop content.
It preserves the line as a regular block and records the recovery in `ParsedPage.warnings: Vec<outl_md::ParseWarning>`.

### Two families, not one

`ParseWarningKind` has six members and they do not mean the same thing.
Read `warning.kind` before wording anything.

| Kind | What actually happened |
|---|---|
| `unrecognized_block_marker` | A line outside the dialect, kept verbatim as a block. A **table** is no longer one of these — it is modelled ([Tables](markdown-format.md#tables)); a table-looking run with no delimiter row still is. |
| `remind_missing_anchor` | `remind:: every 1h` — a repeat with nothing to repeat from. |
| `remind_invalid_time` | `remind:: 25:00` — not a wall-clock time the dialect recognises. |
| `remind_invalid_interval` | `remind:: 10am every 30s` — below the 1min floor, or a bad unit. |
| `remind_invalid_stop` | `remind:: 10am until yesterday` — unusable stop clause, dropped. |
| `remind_max_clamped` | `remind:: 10am max 50` — clamped down to the 10-fire ceiling. |

For a `remind_*` warning the line **is** in the dialect and **was not** preserved as a block: it is a valid `remind::` property whose *rule* the scheduler could not take at face value.
The reminder either did not schedule or scheduled differently than written, which is the part a user needs told — "a line outside the dialect" describes neither.

**This is open work.** Every surface currently renders the single fixed sentence `N line(s) outside outl dialect — preserved as blocks`, with no reader of `kind` anywhere:
`@outl/shared`'s `ParseWarningsBanner.tsx`, the TUI's `view/warnings_banner.rs` and its status chip in `actions/lifecycle/loading.rs`, and the sample output in [`docs/tui.md`](tui.md).
The wording is wrong for all five `remind_*` kinds, and was invisible for as long as it was because `types.ts` modelled only one of the six — closed now by `outl-tauri-shared/tests/wire_enums.rs`.
The fix is to split the count (`N line(s) outside outl dialect`, `N reminder rule(s) outl could not schedule as written`) and give each row a per-kind sentence, in the shared banner and the TUI together so the two cannot drift.

Every client surfaces these warnings to the user instead of pretending the file is clean:

| Client | Surface |
|--------|---------|
| TUI | Banner at the top of the outline + chip in the status line; `?` opens the help overlay with the full list (line number + first 60 chars of `raw`). |
| Mobile / Desktop | `<ParseWarningsBanner>` from `@outl/shared` renders above the outline. Tap a row to scroll to the offending line in the raw view. |
| CLI | `outl doctor` lists every page with warnings and writes a structured row per warning to `.outl/orphans.log`. |

The shared entry point that bundles outline + warnings in one trip is `outl_actions::outline::read_page_outline` (and the workspace-aware variant `read_page_outline_with_workspace`) returning `PageOutline { nodes, warnings, sidecar_identity }`.
The third field reports whether the sidecar those ids came from describes **this** page — reported, never enforced, since an `Err` on the page-open path is the deleting half of invariant 8.
Tauri commands on mobile + desktop expose this directly; the TUI calls it via `lifecycle::load_current`.

The contract is intentionally non-blocking: a file with warnings is still editable, still saves cleanly (render normalises it to `- <raw>` on the next write), and never refuses to load.
Users decide when to clean up; outl never deletes content on their behalf.

## Surfacing a page that stopped syncing

A parser warning says "outl kept a line that doesn't match the dialect".
This is a different, louder condition: **the page is no longer converging with your other devices.**

It happens when a page's `.md` holds content that exists in no op.
A [YAML frontmatter fence](markdown-frontmatter.md) the op log has never seen counts as such content, and reaches the user through this same refusal and the same recovery.
outl refuses to overwrite such a file (root `CLAUDE.md` invariant 8, [RFC 0210](rfcs/0210-md-content-outside-op-log.md)), because that write deletes the content for good.
The cost of refusing is that the page is frozen in both directions until `outl reconcile --ahead-of-log` runs: those lines never reach another device, and a peer's edits never reach this `.md`.

This table is generated from `outl_actions::refusal` (RFC 0255), the single owner of "which surface tells the user about which refusal" — the same mechanism `outl_shortcuts::support` uses for chords.
**MCP** is a fifth surface here, deliberately not part of `outl_shortcuts::Client` (three members: TUI, desktop, mobile — see `outl-shortcuts/CLAUDE.md`).
A refusal is owed to whoever asked for the write, whether or not that asker draws an outline.

<!-- BEGIN GENERATED: refusal-matrix -->

**`PageMarkdownAheadOfLog`**

| Client | Surface |
|---|---|
| CLI | Every write subcommand that touches an existing page (`page update`, `block append`, `template apply`/`run`, ...) returns the same structured `PAGE_MARKDOWN_AHEAD_OF_LOG` JSON error the MCP does, with `--json`. `outl doctor` names the page, the line count and one sample outside any write attempt. `outl serve`'s `tree → .md` sweep discovers it with nobody asking: it names the page and the recovery on the first sweep that sees it and on every change to the set, never once per 30s tick, and says so once more when the set clears. `outl reconcile --ahead-of-log` is the recovery on all three. |
| Desktop | `<PageAheadOfLogBanner client="desktop" />` above the outline, from `PageView.md_ahead_of_log`. Names the command to run in the workspace folder. |
| Mobile | Same banner, `client="mobile"`. **There is no `outl` binary on iOS**, so the copy says to open the workspace on a computer instead of pointing at a terminal that doesn't exist. |
| TUI | A persistent banner above the outline, from `App::md_ahead_of_log`: every page open runs `apply_page_md_with_sidecar_if_stale` (`load_current_no_autorun`), so a page that drifted ahead of the log is named on open, with no write attempt in between. Not a status-line message — the page stays frozen until the recovery runs, and a line that scrolls away is the same silence with a delay. Plus a status-line message wherever a TUI-initiated write re-projects the page (template apply, `call:` code-block exec, mention-creation autocomplete) and a toast when a peer-sync reload's re-projection declines (`SyncEngine::reproject_page`, `reload_workspace_from_disk`) or when a trash restore lands on a page the user is **not** looking at (`actions/trash`): the banner can only speak for the page on screen, so a write that freezes a different one has no banner to appear on. A restore into the open page stays silent, because `load_current` re-derives that banner. |
| MCP | A structured tool refusal — `PAGE_MARKDOWN_AHEAD_OF_LOG` — naming the page, line count, sample, and the recovery command, instead of a generic failure. |
<!-- END GENERATED: refusal-matrix -->

Regenerate with `OUTL_UPDATE_CLIENTS_DOC=1 cargo test -p outl-actions refusal::`, pinned by `outl_actions::refusal::tests::the_clients_doc_matches_the_refusal_matrix` — never hand-edit the region between the markers.

The wording has two owners and one pin between them, because the clients are split across a language boundary.
`outl_actions::refusal::ahead_of_log_notice` is the Rust one — the TUI banner reads it, and it carries the per-surface recovery sentence (a phone has no `outl` binary; everything else is sitting in a shell).
`@outl/shared/warnings::aheadOfLogNotice` is a **declared mirror** for the two GUI clients: a Solid component cannot call into that crate, and shipping seven prose strings over the IPC on every page open to avoid retyping two of them is not a trade worth making.
What must not drift is pinned rather than trusted — `outl_actions::refusal::tests::the_shared_frontend_copy_says_the_same_thing` reads the TypeScript and compares the two facts a user acts on: the headline, and the command.
Neither is ever written inline in a client.
The MCP's wording is a third shape, and deliberately — see the module doc on `crates/outl-cli/src/mcp/tools/dispatch.rs`: it forwards `ActionError::PageMarkdownAheadOfLog`'s own `Display` verbatim rather than writing a second sentence.
Both banners also warn against editing the page in the meantime: a local edit is safe: `ProjectionWriter` routes through `apply_page_md_with_sidecar_guarded`, which refuses to project over unlogged content exactly as the open path does. The `.md` simply stays behind until the lines are recorded.
A refusal discovered after the mutation is persisted is not returned as a command failure: synchronous paths attach it to the successful `PageView`, while `ProjectionWriter` emits `projection-write-failed` with the same structured notice.
Desktop and mobile feed either route into the existing sticky banner; other projection failures retain their original message and use the existing status/toast surface.

The page **still opens** and still shows what is on disk — the guard withheld a write, not the page.
Before this banner existed the refusal only reached a backend log line, so the page appeared to freeze with nothing said.

**When the banner clears.**
Only the open commands run the check, so a mutation reply can never carry the notice; a client that read it off every reply would clear the banner on the user's first edit, which is the action the banner warns against.
`PageView.md_ahead_of_log_checked` marks the replies that *did* run it, and those are authoritative in both directions: the first checked reply with no notice (the next open or refresh of the page, after `outl reconcile --ahead-of-log` fixed it) clears the banner on both clients.
The TUI needs no such flag — the check and the render are the same process, so `App::md_ahead_of_log` is rewritten by every load and a load whose re-projection succeeds clears it.
A banner that outlives the condition is the mirror of the silence this section exists to end.

**Focus mode does not hide it (TUI).**
`z f` drops the header, the sidebar, the status bar and the parse-warning banner, because all four are chrome.
This banner is not: asking for a distraction-free outline is not consent to be told nothing about a page that is saving nowhere else.

## Opening a page from a user-typed ref

When a click on `[[avelino/outl]]`, `#code-review`, `[[2026-06-04]]`
or a picker field hands a client a string, the client must not split
the "journal vs page" decision between a frontend regex and a
backend parser.
The two will drift.
They already did:
`[[2026-13-01]]` matched the mobile frontend's `^\d{4}-\d{2}-\d{2}$`
shape regex, the command then fed `2026-13-01` into the strict date
parser, and the user got an `invalid date slug` toast for what
should have been a regular page.

The canonical entry point is
`outl_actions::resolve::open_or_create_by_ref(target)` (re-exported at the crate root).
It runs the whole decision tree in one place:

1. Date-shaped target → journal (semantic validator, not the regex
   shape — `2026-13-01` falls through).
2. Literal slug match → existing page (clean slug from the picker).
3. Slugified slug match → existing page (`[[avelino/outl]]` finds
   `pages/avelino-outl.md` even if the ref was typed before the
   page existed).
4. **`@`-prefixed mention sugar** (`[[@avelino]]`, `[[@Thiago Avelino]]`) — strip the `@` and resolve via steps 1-3 against the bare name.
   When nothing matches, create the page and set `type:: person` automatically so the next `@` mention surfaces it in the autocomplete popup without any property editing.
   The `@` is purely the link affordance; page identity does not carry it.
5. Case-insensitive title match → existing page.
6. Fallback: create a fresh page via `open_or_create_by_name` (slugifies disk path, keeps the typed string as title).

### `@` mention autocomplete

Every client surfaces a person picker on a word-initial `@`.
The popup is filtered to pages where `type:: person` is set, ranked through the shared `outl_actions::search_persons(query)` helper owned by the `person` module.
The TUI calls it directly; desktop and mobile expose it as the `search_persons` Tauri command.
Accepting a candidate inserts `[[@<title>]]`, a regular wikilink whose target carries the `@` as a visual prefix only.
Mentions ride every existing page-ref code path (render, roundtrip, navigation), so adding the trigger costs no new render/matching/reconcile branch.

A person's backlinks panel surfaces both forms (`[[avelino]]` and `[[@avelino]]`).
`backlinks_for_page` scans the `@`-prefixed alias when `meta.page_type == Some("person")`.
Plain pages do not, so a stray `[[@projeto]]` in a non-person page's backlinks does not trigger false positives.

Every client that turns a tap on a ref / tag / picker entry into a page view should wrap this single helper.
There is no client-side discrimination to maintain.
The `open_or_create_by_name(name, kind)` variant stays for callers that already know they want a regular page (no date branch).

## Deep links (`outl://`)

The `outl://` scheme, its one parser (`outl_actions::parse_deep_link`), and each client's wiring live in [deep-links.md](deep-links.md).

## Adding a new client

The pattern is small:

1. Take a dependency on `outl-core`, `outl-md`, `outl-actions`.
2. Open a `JsonlStorage` rooted at `<workspace>/ops/`, or bring your own `Storage` impl.
3. Open a `Workspace` with that storage; hold one `HlcGenerator` per device.
4. Call into `outl-actions` for every user-visible mutation.
5. Call `outl_actions::apply_journal_md` (or the per-page equivalent when we add it) if you want the `.md` projection on disk.

What you write in your client crate: command surface (Tauri, keyboard, HTTP, …), UI state, navigation, animations.
Nothing else.
