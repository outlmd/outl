# CLAUDE.md — outl-tauri-shared

Shared Tauri backend for the two GUI clients (`outl-desktop/src-tauri`, `outl-mobile/src-tauri`).
Before this crate existed, both clients kept near-identical copies of the same nine files and every fix was ported by hand twice — that drift is the bug this crate deletes.

## What lives here

| Module | Owns |
|---|---|
| `state.rs` | Wire DTOs every command returns: `PageView` (includes `backlinks_order: outl_config::BacklinksOrder`, serialized `"newest"`/`"oldest"`, so a client knows the current direction without a separate settings read; **`backlinks` now always comes back empty from the open commands** — see `BacklinksReply`; **`md_ahead_of_log: Option<MdAheadOfLog>`** + **`md_ahead_of_log_checked: bool`** — see "A page that stopped syncing" below), `MdAheadOfLog` (`path` + `lines` + `sample` — why a page stopped syncing, in the shape a client renders), `BacklinksReply` (the lazy `page_backlinks` reply: `backlinks` + `backlinks_order`, fetched off the page-open path because `backlinks_for_page` is an O(blocks) scan that used to block the first journal paint), `CreateBlockReply`, `WorkspaceSummary`, `BlockHit` (a `((…))` block-ref autocomplete hit — `handle` + `text` + `source_slug`), `ERR_LOADING` |
| `host.rs` | `AppHost` + `StorageRootProvider` — the two traits that absorb the one real client divergence (desktop storage root is `Arc<Mutex<Option<PathBuf>>>`, mobile is a plain `PathBuf`). `AppHost::backlink_index() -> Option<Arc<Mutex<Option<BacklinkIndex>>>>` is the client's pre-computed backlinks index slot (default `None`); `Some` lets `page_backlinks` serve `O(refs)` lookups and rebuild the `O(blocks)` index only when it's stale instead of re-scanning the workspace every navigation. `AppHost::projection_writer() -> Option<&ProjectionWriter>` (default `None`) is the client's off-thread `.md`+sidecar writer slot — see `projection.rs` below and "Async projection writes" |
| `helpers.rs` | `parse_node_id`, `parse_date`, `with_ws*`, **`reproject_stale_md(ws, root, page_id, context) -> ReprojectOutcome`** (the one owner of "refresh the `.md` before reading a view off it" — every open path calls it; classifies the failure instead of logging it, see "A page that stopped syncing"), `build_page_view` (**does NOT compute backlinks** — it's on the first-paint AND post-mutation path, and `backlinks_for_page` is O(blocks); returns `backlinks: []`), `build_page_view_from_tree(workspace, page_id) -> Result<PageView, ActionError>` (projects the view straight from the in-memory tree via `outl_actions::project_outline`, no disk read; `warnings` always empty — feeds the async-projection commit path, see below), `invalidate_backlink_index` (drops the host's cached index so the next `page_backlinks` rebuilds it — called from `finish_in_page*` after every local mutation), `finish_in_page*` (see "Async projection writes" below), `storage_root_or_err`. There is no `compute_backlinks` here anymore — building the index from the in-memory `Workspace` (materializing every block's text under the workspace lock) was the freeze; the rebuild now happens in `commands/page.rs::compute_backlinks_offloaded`, straight from disk. |
| `projection.rs` | `ProjectionWriter` — a single background worker thread that serializes every `.md`+sidecar projection write and coalesces bursts (drains its queue into a dedup set, re-renders each queued page from the current tree via `apply_page_md_with_sidecar_guarded` (post-mutation write that refuses to delete unlogged content)). `spawn<R: StorageRootProvider>(workspace, root, report_failure) -> Self` starts the thread; `queue(&self, page: NodeId)` enqueues a page (best-effort, never blocks the caller), and `flush() -> Result<(), String>` reports whether every queued projection ahead of its barrier succeeded. Every write happens under the workspace lock, same as every synchronous projection path, so `.md` and sidecar can never interleave with another writer — no torn pair, no sync corruption. A crash with queued writes leaves the `.md` briefly behind the op log, never a data loss: the op log is truth, next boot re-projects via `apply_page_md_with_sidecar_if_stale` + the orphan scanner, and peers sync ops over iroh, never the `.md`. Exported at the crate root as `outl_tauri_shared::ProjectionWriter`. |
| `commands/` | The command *bodies* (`asset`, `block`, `history`, `page`, `peers`, `plugin`, `exec`, `shortcuts`, `theme`) — generic over `S: AppHost` except `theme` (pure functions, no workspace access). `commands/history.rs` owns `undo_page(page_id)` / `redo_page(page_id)` (RFC 0254 phase 1): moved here from `outl-desktop/src-tauri/src/commands/history.rs` — the stacks live in whatever `AppHost::history()` slot the client wires, so a host without one (the trait's own default) gets `"undo is not supported on this client"` instead of a panic, rather than the desktop-only registration mobile had before. `commands/theme.rs` owns `list_themes()` / `get_theme(name)` (RFC 0022): thin wraps over `outl_theme::PRESETS` / `outl_theme::by_name` / `outl_theme::default`, moved here from the desktop crate so mobile can register the identical two commands instead of hardcoding palette values. `commands/asset.rs` owns **`open_asset(url)`** (resolves an `assets/…` link to an absolute path under `<root>/assets/` via `outl_actions::resolve_asset_path`, rejecting traversal / external schemes, then `open::that` launches the OS default app — outl never renders the file; read-only, no workspace lock; `Ok(None)` → "asset not found on this device yet"), **`read_asset_data_url(url)`** (the **inline image render** path — resolves the same `assets/…` link through `resolve_asset_path`, serves only a regular file (a FIFO / device node is rejected, since `metadata().len()` lies for those), reads it with a structural `Take` bound of 25 MB so a giant file can't be base64'd into the webview, guesses the MIME from the extension, and returns a `data:<mime>;base64,<…>` string the webview loads directly as an `<img src>`; the frontend only calls it for image tokens, non-image assets stay click-to-open via `open_asset`; the Tauri asset protocol is deliberately avoided since the workspace root is runtime-picked and can't be statically scoped; read-only, no workspace lock; `Ok(None)` → "asset not found on this device yet") and **`attach_asset(source_path, page_id, after_block_id?)`** (imports a file via `outl_actions::import_asset` — content-addressed copy into `<root>/assets/`, size-capped by `outl_config` `[assets] max_bytes` — then inserts a block carrying its markdown link through the shared `finish_in_page` commit path, after `after_block_id` or at the page end; returns the refreshed `PageView`). `commands/block.rs` also owns `split_block(page_id, id, char_offset)` (splits a block at the caret via `outl_actions::split_block`; `char_offset` is a codepoint offset the client converts from the textarea's UTF-16 `selectionStart`; tolerates a stale anchor exactly like `create_block`, degrading to an empty sibling via `create_after_or_append`). `commands/page.rs` owns page navigation, search, `delete_page` (calls `outl_actions::delete_page` + `remove_page_projection`, returns today's-journal `PageView` so the caller navigates away from the deleted slug), `page_backlinks(slug)` (the lazy backlinks fetch the frontend fires after the outline paints: `compute_backlinks_offloaded` runs three phases off the IPC thread — a brief workspace lock for `list_pages` + this page's meta, an `O(blocks)` rebuild via `outl_actions::build_backlink_index_from_disk` when the host's `backlink_index()` slot is stale (reads the `.md` projection, touches no `Workspace`, holds **no** lock; when the host has no slot at all this is a one-shot from-disk build instead of the old direct workspace scan), then an `O(refs)` lookup under a brief lock, shipping each hit through `Backlink::into_shallow` to trim the IPC payload; returns `BacklinksReply`), and `set_backlinks_order(order, slug)` (persists `[display] backlinks_order` via `outl_config::save` and returns the re-sorted `BacklinksReply`, issue #142). `commands/exec.rs` owns `resolve_embeds(handles)` and its `EmbedContent` DTO (`handle`, `text`, `page_slug`, `status`, **`children: Vec<outl_actions::OutlineNode>`** — the source block's subtree projected with tokens via `outl_actions::project_parsed_subtree`, empty for a leaf, capped at `EMBED_SUBTREE_MAX_DEPTH = 4` to match the client render depth so deeper levels don't ride the IPC for nothing); one command serves **both** `((…))` inline refs (uses `text`) and `!((…))` embeds (uses `text` + `children`), so the desktop resolves refs and expands embed subtrees off one round-trip (issue #147) |
| `reminder_runtime.rs` | `take_due(state) -> Vec<ReminderDto>` — a thin DTO wrapper over `outl_actions::take_due`. The fired log and the due-scan live in `outl-actions`, not here: the TUI delivers too (OSC 9) and cannot depend on this crate, so keeping them here would have made the Tauri clients the owners of something every client needs. What's left is pulling the config + workspace off the `AppHost` and mapping to `ReminderDto`. |
| `commands/reminders.rs` | `list_reminders` / `reminder_settings` / `set_reminder_settings` / `snooze_reminder` / `clear_reminder_snooze` / `set_block_remind` / `mark_block_done` + the `ReminderDto` / `ReminderSettingsDto` wire shapes. `mark_block_done` sets DONE outright via `outl_actions::todo::set_todo`, never `toggle_todo`: a rule can sit on a block with no marker, and one toggle there lands on `TODO`, so the reminders list's "mark done (cancels the reminder)" button used to arm the nag instead of cancelling it. `set_reminder_settings` exists because mobile has no settings screen: it reads `config.toml` and writes back only the two reminder keys, so it can't clobber a hand-set timezone or relay URL. Times cross the bridge as ISO-8601 **local** strings, not epoch numbers — the backend already resolved them in the configured timezone (`outl_actions::clock`), and re-deriving a local time from an epoch in JS reintroduces exactly the bug that module exists to fix. `snooze_reminder` takes no page id because it touches no `.md`: the snooze lives only in the op log. |
| `commands/shortcuts.rs` | `list_shortcut_bindings()` / `list_action_support()` + the `SupportDto` / `ActionSupportDto` wire shapes — the `(chord, action)` catalog and the per-client support matrix (root `CLAUDE.md` invariant 12). Moved here from `outl-desktop` so mobile registers the same two commands: a client can only tell the user *where* an action exists if it can read the matrix on the device asking. Pure functions over `outl_shortcuts`, no workspace access |
| `commands/timeline.rs` | `page_timeline` + the `PageTimelineDto` / `TimelineEventDto` wire shapes — a page's history, read out of the op log (issue #241). **Read-only**; there is no restore counterpart, on purpose (see `outl_actions::timeline`). `TimelineEventDto` is deliberately **flat** with `change` as a string tag saying which optional fields are meaningful. It is not a discriminated union and nothing narrows on it, but a reader switches on one field instead of unwrapping a nested enum, and `@outl/shared` renders it directly. `total` is the count **before** the limit, never `events.len()` — a capped list that reports its own length as the total reads as the whole history. `limit: Some(0)` is read as the default rather than as "no events", so a client that forgets the field gets a usable panel instead of an empty one. Registered by **both** clients now (`timeline_commands!`); mobile has the command and no timeline UI yet, which is why `Capability::PageHistory`'s mobile column stays `Missing` — see "One command surface, not two". |
| `workspace_open.rs` | `open_workspace_at` / `reconcile_orphan_md` primitives, plus **`WorkspaceGuards`** (the shared `<root>/.outl/.lock` + exclusive `<root>/ops/.lock-<actor>` flocks, held for as long as the workspace is open — see "Workspace locks" below) and **`load_or_create_actor(local_dir)`** — a thin wrapper over `outl_core::DeviceStore::device_actor`. Both GUI clients keep a **device-wide** actor (`<local_dir>/actor`: `~/.config/outl` on desktop, the app sandbox's data dir on mobile) rather than the per-workspace one the CLI / TUI resolve, because the `HlcGenerator` is bound at app start, before a workspace is picked. That is safe precisely because `local_dir` is outside every workspace — it never rides the file-sync surface, so the cross-device actor collision described in `outl-core/CLAUDE.md` → "Actor id is device-local" cannot reach it. **Never move this file into the workspace.** |
| `iroh_sync.rs` | `start_with_reload_bridge` — bridges a started transport's two signals to Tauri events. Building one belongs to `outl_sync_iroh::build_transport` (config gate, identity, peers, relay, **and the device endpoint lease** — one endpoint per device, first process in wins); each client calls it with its own identity path and handles `EndpointBusy` / `Disabled` by staying on its watcher. |
| `plugin_service.rs` + `plugin_thread.rs` | `PluginService` — the dedicated plugin thread (Boa `Context` is `!Send`), parametrized by client id + capability set + `StorageRootProvider` |
| `plugin_dto.rs` | Plugin wire shapes (`PluginCommandDto`, `ToolbarButtonDto`, …) |
| `wrappers/` | The one declaration of the Tauri command *surface*. `wrappers/mod.rs` holds the `tauri_commands!` generator; `wrappers/catalog.rs` holds one `*_commands!` macro per command module. A client's `commands/<module>.rs` is now a single macro invocation (`outl_tauri_shared::block_commands!(crate::state::AppState);`). **A client takes a whole module or none of it** — see "One command surface, not two" below |

## Workspace locks

The Tauri clients used to take **neither** workspace lock, which made a running GUI invisible to every other `outl` process on the machine.
`outl compact --apply` answers *"is anyone in this workspace?"* by taking `<root>/.outl/.lock` exclusively; a GUI holding nothing let that gate pass, and compaction then renamed a rewritten `ops-<actor>.jsonl` under a live client still holding in-memory byte offsets into the pre-compaction layout — "a silently dropped op on every index-driven read", in compaction's own words.
`JsonlStorage::append_ops`'s stated precondition ("the SINGLE writer for its own actor file, guarded by `ActorWriteLock`") was false for the same reason.

`open_workspace_at` now takes both, through `outl_core::lock`, and is the **single writer** of the client's `workspace_guards` slot:

- shared `WorkspaceLock` first, then exclusive `ActorWriteLock` — the order `outl_ws::open_with` uses and the order compaction checks them in;
- the guards are installed only **after** the open succeeds, so a failed open never parks a lock on a workspace nobody has open (compaction would then refuse forever with nothing running);
- installing them is what drops the previous workspace's, so switching roots releases the old one with no client-side sequencing.

**Never acquire or drop one of these from a client crate.** A second opinion about who holds the workspace is the defect this replaced.

**Lock ordering.** Both acquisitions are non-blocking `try_lock_*`, and both happen at open time — outside the process-wide `workspace` `Mutex`, and outside `ProjectionLock` (the *blocking* `flock` on `pages/.<name>.md.lock` that `apply_page_md_with_sidecar_guarded` takes under the workspace mutex). They add no wait-for edge to the existing `workspace` → `ProjectionLock` path. Keep it that way: a blocking cross-process lock taken above the workspace mutex would turn today's unbounded stall into a deadlock.

**A contended actor lock refuses the open; it does not fall back to an ephemeral actor.**
`outl_core::resolve_write_actor` is right for the CLI and TUI and unavailable here: a GUI's `HlcGenerator` is built in `setup()`, before a workspace is picked, so swapping only the *storage* actor would leave two live generators on one actor id — identical `(time, counter, actor)` triples, which is op identity, so `Workspace::apply`'s dedup silently drops one of the two ops.
Making the fallback available means making the client's `HlcGenerator` swappable at workspace-open time (a plain field on both `AppState`s today, read from ~56 call sites); that is client-side work, tracked separately.

Re-picking the workspace already open is handled inside `acquire_guards`, not by the caller: a POSIX `flock` belongs to an open file description, so a second `open` + `LOCK_EX|LOCK_NB` of `ops/.lock-<actor>` fails *inside the process that already holds it*.

**The regression net** is `tests/workspace_locks.rs` — compaction refuses while a GUI is open and runs once it closes, a contended actor refuses rather than sharing the file, a refused open strands no lock, a re-pick does not refuse itself, a switch releases the old root, and a running GUI does **not** lock the TUI or MCP server out (the workspace lock is shared on purpose).

## Background passes yield, they do not race

A batch pass on a worker thread is **not** automatically invisible. The UI reads the same disk to paint, so a pass that saturates I/O freezes the app whatever thread it runs on. outl's premise is that it opens fast and is ready for input, and that premise is about the *device*, not about thread count.

Measured, on the boot after a `CURRENT_PIPELINE_VERSION` bump: 2,827 files, **24.7 seconds at 8% CPU**. All of it `write_atomic`'s two `fsync`s per sidecar, 5,656 of them, for 44 ops of real content.

Two rules came out of that:

- **Yield in proportion to the work.** `BackgroundPace::COOPERATIVE` sleeps as long as the page took, so the pass holds about half the device and a slow disk makes it yield more rather than stutter more. Sleep *outside* the lock; sleeping while holding it is the same stall with extra steps.
- **Take the lock with `try_lock`, and retry the page — never skip it.** `lock()` is FIFO, which hands the frontend every other turn and keeps the disk busy in between. And a `continue` on a busy lock steps over the page permanently, which is the silent-skip class of bug this whole area exists to close.

What is **not** the answer: dropping the sidecar `fsync`. It takes 24.7s to 0.3s and trades the one failure the project cannot afford — a rename landing before its data leaves a sidecar of garbage, read as missing, minting a fresh ULID per block, duplicating the page and breaking every `((blk-…))` handle.

## Async projection writes

`finish_in_page_with` (the tail every mutating command calls to build its reply) branches on `state.projection_writer()`:

- **`Some(writer)` (async path, both GUI clients today):** `writer.queue(page)` hands the page off to the background thread.
  The reply's `PageView` is built straight from the tree via `helpers::build_page_view_from_tree` — no disk read, no render on the IPC thread.
- **`None` (sync fallback):** the page is projected inline via `apply_page_md_with_sidecar_guarded`; a refusal annotates the successful tree-built `PageView` instead of falsely turning an already-persisted mutation into an error.

Either way, the undo snapshot (`HistoryStacks`) still renders the pre/post `.md` under the workspace lock — that render is needed for the diff regardless of who writes the projection to disk.

**`commands/history.rs`'s `step_history` (`undo_page` / `redo_page`) calls `ProjectionWriter::flush()` before it does anything else, whenever one is wired, and aborts without moving the history stack if any queued projection was refused.** Its restore (`outl_actions::restore_page_md`) writes the snapshot straight to the page's `.md` and reconciles it against whatever sidecar is *currently on disk* — and `finish_in_page_with`'s async path only ever queues that sidecar's write, never waits for it. Undo right after an edit, with nothing forcing the queue to drain first, let `reconcile_md` match against a stale or entirely absent sidecar and create a **duplicate block** instead of replacing the edited one — a real content-corruption bug a user could hit with an ordinary fast `Cmd+Z`, caught by `outl-tauri-shared/tests/history_command.rs`'s `undo_immediately_after_an_edit_does_not_duplicate_the_block` (RFC 0254 phase 1, fix round 3). The call happens *before* `with_ws_mut` takes the workspace lock — `flush()` blocks until the worker acks, and the worker needs that same lock to drain a page write queued ahead of the flush, so calling it from inside the lock would deadlock the two against each other. Cost: `flush()` drains the **whole** queue, not just the page being restored, so an undo issued right after a batch that dirtied many pages (a large paste, a plugin's `sync_hooks` sweep) waits for all of them, not one — for the ordinary one-edit-then-undo case this is one extra render+hash+write, the same order of cost `restore_page_md` already pays a line later.

A client that wires `AppHost::projection_writer()` to `Some` **must** spawn the `ProjectionWriter` at boot with the same `Arc<Mutex<Option<Workspace>>>` every command locks, or the queued writes race a different workspace instance.
`tests/projection_view.rs` asserts the tree-built view and the `.md`-built view agree — if you change either path, keep both in sync.
The shared page lock serializes cooperating outl writers around each `.md` + sidecar transaction. Because external editors do not honour advisory locks and pathnames have no portable atomic compare-and-swap, guarded writers also re-read the `.md` immediately before replacement and refuse if its bytes changed after authorization; the remaining read-to-rename interval is an unavoidable filesystem limitation, not something the lock is claimed to close.

## A page that stopped syncing

Every open path (`open_today_journal`, `open_journal_for`, `open_page_by_slug`, `open_ref`) refreshes the page's `.md` from the tree before reading a view off it (issue #166).
That refresh can be **refused**: root `CLAUDE.md` invariant 8 forbids overwriting a `.md` holding content that exists in no op, so `apply_page_md_with_sidecar_if_stale` returns `ActionError::PageMarkdownAheadOfLog`.

The refusal is correct and it freezes the page in *both* directions — those lines never reach another device, and a peer's edits never reach this `.md` — until `outl reconcile --ahead-of-log` runs.
It used to die in a `tracing::warn!`, so the user saw a page that had silently stopped updating with nothing on screen to say why, and on iOS no binary to fix it with.

`helpers::reproject_stale_md` is the single owner of that verdict now:

- `ActionError::PageMarkdownAheadOfLog` → `ReprojectOutcome.ahead_of_log`, which the open command hangs on `PageView.md_ahead_of_log` (via `PageView::with_ahead_of_log_check`, which also sets `md_ahead_of_log_checked` so the clients know this reply is authoritative and can clear a banner the reconcile fixed) and `@outl/shared`'s `<PageAheadOfLogBanner />` renders.
  The copy — including the fact that a mobile user has to go to a computer — lives in `@outl/shared/warnings::aheadOfLogNotice`, one owner for both clients.
- everything else → `ReprojectOutcome.other_error`, a local retryable condition.
  Only `open_ref` surfaces it, through its existing `ref-projection-failed` event; a disappearing toast is the wrong surface for a page that stays broken until a command runs.

**The open must never fail because of this.**
The guard withheld a *write*; the `.md` on disk is still readable, so the page opens showing exactly what is on disk.
Turning the refusal into an `Err` would trade a stale page for no page at all on the hottest path in the app.
`tests/ahead_of_log_view.rs` pins all three properties (the notice reaches the reply, the page still opens with its content, a healthy page stays quiet).

**Closed in the same change that opened this note.** A local mutation used to overwrite those lines: `finish_in_page_with` queued an unconditional `apply_page_md_with_sidecar` on the `ProjectionWriter`, so the write the read path refuses happened on the next keystroke commit. Every GUI write path now routes through `outl_actions::apply_page_md_with_sidecar_guarded` (`projection.rs`, `commands/block.rs`, `commands/template.rs`), which asks the one question that matters and skips the projection instead of deleting. The user's edit is never at risk either way — it went through `Workspace::apply`.
Post-mutation projection failures never turn that durable edit into an `Err`: synchronous paths annotate the successful `PageView` (`md_ahead_of_log` or `projection_error`), and the worker emits `projection-write-failed` with `ProjectionWriteFailed` after an async refusal.
Both clients route the structured refusal to the sticky banner and preserve unrelated failures on their existing status/toast surface.

## One command surface, not two

The bodies always lived here. The **wrappers** did not: each client
hand-wrote its own `#[tauri::command]` shim per command, 3,033 lines
across the two of them to register 72 functions.

The boilerplate was not the problem. The divergence was, and it arrived
by omission: `commands/asset.rs` and `commands/theme.rs` were
byte-identical between the clients, while `commands/history.rs` was 183
lines on the desktop and 22 on mobile, and `commands/exec.rs` was 3
commands against 1. Nobody decided mobile should lack `page_timeline`,
`run_auto_run_blocks`, `resolve_embeds`, `set_page_property`,
`list_shortcut_bindings` or `list_action_support` — the wrappers were
never typed, and nothing could fail. Root `CLAUDE.md` invariant 12 makes
a missing *action* a compile error, but `outl_shortcuts::capability_support`
cannot see a *command* that was never registered: an unregistered command
leaves no trace in any exhaustive `match`.

So the list lives in [`wrappers/catalog.rs`](src/wrappers/catalog.rs),
once, and:

- **A client takes a whole `*_commands!` module or none of it.**
  Registering a command whose frontend does not call it yet costs a
  symbol; not registering it costs a feature that silently does not
  exist on that client.
- **A gap is allowed — it just has to be written down.**
  `tests/command_parity.rs` walks both clients' `commands/` directories
  and fails when either skips a module, unless the pair is listed in that
  test's `DECLARED_GAPS` with a reason. The table is empty today.
- **Registering the backend command is not shipping the feature.**
  Mobile registers `page_timeline` now and still has no timeline UI, so
  `Capability::PageHistory`'s mobile column stays `Missing`. The catalog
  answers "what can the user do here", not "what does the IPC accept".

Anything that needs more from Tauri than `State<'_, AppState>` — an
`AppHandle` to emit an event, a second `State` for the plugin thread — is
not boilerplate and is **not** generated: `open_ref`, `outl_sync_now`,
`deliver_due_reminders`, the pairing commands and the whole `plugin`
module stay hand-written in the client, where the extra dependency is
visible.

A shared body takes `String`, never `&str`, even when it only reads it.
Tauri hands the wrapper an owned value, so a borrowed parameter buys
nothing and puts an `&` in every generated call site.

## The commit pipeline lives in `outl-actions`

`finish_in_page_with` is still the tail every mutating command calls, but
the *sequence* it runs is now `outl_actions::commit::commit_page`:

1. snapshot the pre-mutation `.md` (only when the host has undo stacks,
   and only kept when the render actually changed);
2. run the mutation — the only step that can fail the commit;
3. drop the cached backlinks index;
4. announce the new ops to peers;
5. project `.md` + sidecar (queued off-thread, or inline).

What stayed here is the Tauri half: `TauriCommitHooks` implements
`outl_actions::CommitHooks` against `AppHost`, and `finish_in_page_with`
builds the `PageView` afterwards. The pipeline moved down because it was
unreachable from the TUI and the CLI — both hold a plain `Workspace`,
neither can satisfy a trait that wants `&Mutex<Option<Workspace>>` — so
each re-derived the parts it thought applied ([#264](https://github.com/outlmd/outl/issues/264)).

`AppHost` deliberately **did not** move. It is shaped by Tauri's managed
state (`Mutex<Option<Workspace>>`, `Arc<RuntimeRegistry>`), and relocating
it would drag `parking_lot` and the lock shape into the UI-agnostic crate
— relocating the problem instead of removing it, which is the question
root `CLAUDE.md` invariant 9 exists to ask. The lock stays in the client;
the sequence moved.

## What this crate does NOT own

- The `AppState` structs (fields differ per client) — each client implements `AppHost` on its own state.
- `#[tauri::command]` fns — Tauri's `generate_handler!` needs concrete fns in the app crate, so each client registers 1–3 line wrappers that delegate here.
  **The body always lives here; the wrapper never grows logic.**
- Client-specific surface: desktop `settings.rs` / `fs_watcher.rs`; mobile `bg_sync.rs` / `workspace_picker.rs`.
  Undo-history invalidation across a peer reload (`helpers::invalidate_changed_history`) moved here in RFC 0254 phase 1 — both clients now record snapshots, so both call it from their own `reload_workspace`.
- Business logic.
  Everything that mutates the workspace shape delegates to `outl-actions` — same hard rule as the client crates.

## Rules

- Adding a Tauri command that both clients need: body here (generic over `AppHost`), one line in the matching `*_commands!` list in `wrappers/catalog.rs`, and an `invoke_handler!` entry in **both** clients.
  Do not hand-write a wrapper — a command registered in only one client is drift, and `tests/command_parity.rs` exists to make that drift fail rather than ship.
- A client that wires `AppHost::backlink_index()` must call `helpers::invalidate_backlink_index` after **every** path that can change what a page's backlinks are.
  That's local mutation (`finish_in_page*` already does this), a peer/workspace reload (`reload_workspace`, desktop's `set_workspace`), and a plugin run that applied ops (`commands/plugin.rs::run` / `sync_hooks`, guarded on `applied > 0`).
  Missing one of these serves stale backlinks until the next unrelated invalidation happens to fire.
- **Lock order is `workspace` → everything else.**
  The workspace `Mutex` is the outermost lock; the backlinks index, the undo `history` map and the desktop's `storage_root` slot are only ever taken *inside* it (or on their own).
  `finish_in_page_with` holds the workspace lock for a whole commit and drops the cached index from in there, so any thread that takes the index first and then waits on the workspace is an ABBA deadlock — `parking_lot::Mutex` has no timeout, so the app freezes until the user force-quits it.
  `compute_backlinks_offloaded` did exactly that and pasting was the reliable way in: a paste commits twice (draft flush + the paste) and every commit refreshes the panel, so the two collided within a keystroke.
  Pinned by `tests/backlinks_commit_deadlock.rs`, which stress-runs both paths against a watchdog — a re-inverted order fails the test instead of hanging the app.
- Never change a DTO shape without checking the TS side — the frontends depend on the wire format, and the mirror is hand-written on purpose.
  Three test binaries make that safe, and a change belongs in whichever one matches its shape.
  `tests/wire_types.rs` pins a struct's key set.
  `tests/wire_enums.rs` pins an enum's **variant** set, plus each tagged variant's fields.
  `tests/wire_mirrors.rs` pins mirrors declared outside `@outl/shared/api/types.ts`.
  The comparisons live once in `tests/wire_pin/`, the TypeScript reader in `tests/ts_parser/`.
- Client identity (the `CLIENT` str + capability set) stays in each client's `plugin_service.rs` shim, never here.

## The wire mirror is hand-written, so the pin is the safety

Two gaps are worth remembering, because both were invisible in the same way.
Neither was a hard problem — they were a reader that never looked.

**An enum could not be pinned at all.**
`wire_keys` panics on anything that is not a JSON object, and a `serde` enum is a string.
So twelve enums had zero coverage.
`outl_md::ParseWarningKind` shipped **one of its six variants** to TypeScript while the comment above that union claimed variants "land here in lockstep".
A `serde` attribute is never assumed: every pin serializes a real value, so `rename_all`, `rename`, `tag` and `content` are accounted for by construction.
Exhaustiveness is the other half.
`wire_pin::wire_variants!` generates a `match`, so a variant added in Rust stops the pin file compiling, in the one place that also names the TypeScript union.

**The coverage gate walked one file.**
It reported 26 of 34 declarations pinned and the rest exempted, which was true of a universe it had chosen and silent about fourteen mirrors in three other files.
`ts_parser::MIRROR_FILES` is the list now, and the gate prints the intersection of *declared* and *pinned*, not its own call count.
A gate that overstates its reach is worse than no gate: the number it prints is what stops the next person from looking.

Two consequences for anything new:

- **A `serde_json::json!` event payload cannot be pinned**, because there is nothing to serialize.
  `ref-projection-failed` used to be one and is now `state::RefProjectionFailed`, with identical JSON.
  The deep-link payload still is one — built in both clients' `lib.rs`, out of this crate's reach — and `wire_types.rs`'s `UNTYPED_EVENTS` records that rather than letting it stay quiet.
- **A hand-written tag needs the real producer.**
  `TimelineEventDto.change`, `ThemeConfigDto.mode` and `SupportDto.kind` are `String`s a match writes, so `commands::timeline::to_dto` and `commands::theme::theme_config_dto` are `pub`.
  A table retyped in a test is a second owner of the fact, which is the thing being prevented.
