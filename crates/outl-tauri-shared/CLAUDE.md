# CLAUDE.md — outl-tauri-shared

Shared Tauri backend for the two GUI clients (`outl-desktop/src-tauri`, `outl-mobile/src-tauri`).
Before this crate existed, both clients kept near-identical copies of the same nine files and every fix was ported by hand twice — that drift is the bug this crate deletes.

## What lives here

| Module | Owns |
|---|---|
| `state.rs` | Wire DTOs every command returns: `PageView` (includes `backlinks_order: outl_config::BacklinksOrder`, serialized `"newest"`/`"oldest"`, so a client knows the current direction without a separate settings read; **`backlinks` now always comes back empty from the open commands** — see `BacklinksReply`; **`md_ahead_of_log: Option<MdAheadOfLog>`** + **`md_ahead_of_log_checked: bool`** — see "A page that stopped syncing" below), `MdAheadOfLog` (`path` + `lines` + `sample` — why a page stopped syncing, in the shape a client renders), **`DroppedEdit`** (`block` + `kept_as?` + `message` — a keystroke whose target block had left the tree; the two constructors own the wording, never a client), `BacklinksReply` (the lazy `page_backlinks` reply: `backlinks` + `backlinks_order`, fetched off the page-open path because `backlinks_for_page` is an O(blocks) scan that used to block the first journal paint), `CreateBlockReply`, **`PasteReply`** (`view` + the host block's new `host_text` / `host_caret` — see "A paste rewrites the block being edited" below — + `new_id`, the last **root** a view-mode paste created), `WorkspaceSummary`, `BlockHit` (a `((…))` block-ref autocomplete hit — `handle` + `text` + `source_slug`), `ERR_LOADING` |
| `host.rs` | `AppHost` + `StorageRootProvider` — the two traits that absorb the one real client divergence (desktop storage root is `Arc<Mutex<Option<PathBuf>>>`, mobile is a plain `PathBuf`). `AppHost::backlink_index() -> Option<Arc<Mutex<Option<BacklinkIndex>>>>` is the client's pre-computed backlinks index slot (default `None`); `Some` lets `page_backlinks` serve `O(refs)` lookups and rebuild the `O(blocks)` index only when it's stale instead of re-scanning the workspace every navigation. `AppHost::projection_writer() -> Option<&ProjectionWriter>` (default `None`) is the client's off-thread `.md`+sidecar writer slot — see `projection.rs` below and "Async projection writes" |
| `helpers.rs` | `parse_node_id`, `parse_date`, `with_ws*`, **`reproject_stale_md(ws, root, page_id, context) -> ReprojectOutcome`** (every GUI open path calls it before reading a view off the `.md`; a thin DTO + log-line wrapper over `outl_actions::reproject_stale_md`, which owns both the refresh and the classification — see "A page that stopped syncing"), `build_page_view` (**does NOT compute backlinks** — it's on the first-paint AND post-mutation path, and `backlinks_for_page` is O(blocks); returns `backlinks: []`), `build_page_view_from_tree(workspace, page_id) -> Result<PageView, ActionError>` (projects the view straight from the in-memory tree via `outl_actions::project_outline`, no disk read; `warnings` always empty — feeds the async-projection commit path, see below), `storage_root_or_err`. The commit tail is **not** here any more — `finish_in_page*`, `TauriCommitHooks`, the backlink-index policy and the peer announce moved to `commit.rs` when this file crossed the size ratchet. There is no `compute_backlinks` here anymore — building the index from the in-memory `Workspace` (materializing every block's text under the workspace lock) was the freeze; the rebuild now happens in `commands/page_backlinks.rs::compute_backlinks_offloaded`, straight from disk. |
| `offload.rs` | `run` + `locked` / `locked_mut` — the one owner of **getting a command body off the IPC thread**, and of why a non-O(1) body must be `async` (its module doc carries the `peer_status` crash that bought the rule). `locked*` are `with_ws*` against the slot's `Arc` rather than an `AppHost`, for a body already on the pool; `helpers::with_ws*` delegate to them |
| `projection.rs` | `ProjectionWriter` — a single background worker thread that serializes every `.md`+sidecar projection write and coalesces bursts (drains its queue into a dedup set, re-renders each queued page from the current tree via `apply_page_md_with_sidecar_guarded` (post-mutation write that refuses to delete unlogged content)). `spawn<R: StorageRootProvider>(workspace, root, report_failure) -> Self` starts the thread; `queue(&self, page: NodeId)` enqueues a page (best-effort, never blocks the caller), and `flush() -> Result<(), String>` reports whether every queued projection ahead of its barrier succeeded. Every write happens under the workspace lock, same as every synchronous projection path, so `.md` and sidecar can never interleave with another writer — no torn pair, no sync corruption. A crash with queued writes leaves the `.md` briefly behind the op log, never a data loss: the op log is truth, next boot re-projects via `apply_page_md_with_sidecar_if_stale` + the orphan scanner, and peers sync ops over iroh, never the `.md`. Exported at the crate root as `outl_tauri_shared::ProjectionWriter`. |
| `commit.rs` | What happens **around** a page mutation on the Tauri side: `finish_in_page` / `finish_in_page_with` / `finish_in_page_with_refresh`, the private `TauriCommitHooks` (this crate's `outl_actions::CommitHooks` impl), `BacklinkRefresh`, `invalidate_backlink_index`, **`reindex_backlink_page(state, workspace, page)`** (`BacklinkIndex::reindex_page_from_disk` on the host's slot, `O(one page)`, the GUI twin of the TUI's `reindex_backlinks_for_slug` — **reads disk, so the page's `.md` must already be projected**) and `announce_after_commit`. Split out of `helpers.rs`, which is argument parsing and lock acquisition and had no reason to own a pipeline. The refresh policy lives **with** the hooks rather than in a module of its own because it *is* steps 4 and 5 of that pipeline — see "One page's backlinks, not the whole index" below |
| `surface.rs` | `SURFACE` — which `outl_actions::refusal::Surface` this build words a refusal for, a `cfg` on the target rather than an `AppHost` method (the fact is a property of the build, not of the host's runtime state). Its own module because two bodies ask it — `commands/trash.rs`'s listing and `commands/backlink_edit.rs`'s ahead-of-log refusal — and a second `cfg` pair would be a second owner of what this build is, the one thing a `cfg` can never be caught disagreeing about |
| `commands/` | The command *bodies* (`asset`, `block`, `backlink_edit`, `history`, `page`, `page_open`, `peers`, `plugin`, `exec`, `shortcuts`, `theme`) — generic over `S: AppHost` except `theme` (pure functions, no workspace access). **`backlink_edit` is apart from `block` on a behavioural line, not a size one**: every body in `block.rs` acts on the page the user is looking at and may therefore *tolerate* a stale block id by re-targeting, and `edit_backlink_block` aims at a page the client is not showing and must **refuse** instead — one file is how a later reader merges the two. `commands/history.rs` owns `undo_page(page_id)` / `redo_page(page_id)` (RFC 0254 phase 1): moved here from `outl-desktop/src-tauri/src/commands/history.rs` — the stacks live in whatever `AppHost::history()` slot the client wires, so a host without one (the trait's own default) gets `"undo is not supported on this client"` instead of a panic, rather than the desktop-only registration mobile had before. `commands/theme.rs` owns `list_themes()` / `get_theme(name)` (RFC 0022): thin wraps over an `outl_theme::Registry` built from the built-ins plus `outl_config::theme_dir()`, so a user's own `.toml` theme reaches both GUI clients, moved here from the desktop crate so mobile can register the identical two commands instead of hardcoding palette values. `commands/asset.rs` owns **`open_asset(url)`** (resolves an `assets/…` link to an absolute path under `<root>/assets/` via `outl_actions::resolve_asset_path`, rejecting traversal / external schemes, then `open::that` launches the OS default app — outl never renders the file; read-only, no workspace lock; `Ok(None)` → "asset not found on this device yet"), **`read_asset_data_url(url)`** (the **inline image render** path — resolves the same `assets/…` link through `resolve_asset_path`, serves only a regular file (a FIFO / device node is rejected, since `metadata().len()` lies for those), reads it with a structural `Take` bound of 25 MB so a giant file can't be base64'd into the webview, guesses the MIME from the extension, and returns a `data:<mime>;base64,<…>` string the webview loads directly as an `<img src>`; the frontend only calls it for image tokens, non-image assets stay click-to-open via `open_asset`; the Tauri asset protocol is deliberately avoided since the workspace root is runtime-picked and can't be statically scoped; read-only, no workspace lock; `Ok(None)` → "asset not found on this device yet") and **`attach_asset(source_path, page_id, after_block_id?)`** (imports a file via `outl_actions::import_asset` — content-addressed copy into `<root>/assets/`, size-capped by `outl_config` `[assets] max_bytes` — then inserts a block carrying its markdown link through the shared `finish_in_page` commit path, after `after_block_id` or at the page end; returns the refreshed `PageView`). `commands/block.rs` owns **`copy_block_ref(id)`** and **`copy_block_embed(id)`**, which are one private `block_pointer` with a different writer: both resolve the handle the `WorkspaceIndex` **assigned** (never `derive_ref_handle`, which yields the deterministic 6-char form rather than the one collision expansion lengthened, so a derived token resolves to a *different block*), and both spell it through `outl_md::inline`'s `block_ref_token` / `block_embed_token` rather than a local `format!` — the `!` is the user's whole reason for picking one row over the other, and a client that built the embed by prefixing the ref would be a second owner of the syntax. Async because that index build reads and parses every `.md` + sidecar in the workspace. Each returns a **string** and the client writes it to the OS clipboard (the desktop through `src/lib/os-clipboard.ts`, over the clipboard-manager plugin). `commands/block.rs` also owns `split_block(page_id, id, char_offset)` (splits a block at the caret via `outl_actions::split_block`; `char_offset` is a codepoint offset the client converts from the textarea's UTF-16 `selectionStart`; tolerates a stale anchor exactly like `create_block`, degrading to an empty sibling via `create_after_or_append`). **`edit_block` reads the same `NotInTree` and does two things with it** — it re-targets through `create_after_or_append` to keep the text, **and** reports it as `PageView.dropped_edit` ([#473](https://github.com/outlmd/outl/issues/473)). Returning the error instead parks mobile on a Retry that can never succeed and stops the desktop ever applying the refreshed view; keeping the text *silently* is a new silence, since the end of a journal day is below the fold. An empty edit writes nothing and says nothing. When the **page** is trashed too nothing is written at all: `create_after_or_append` refuses before its `Op::Create`, which is what stops a junk empty block landing under a trashed parent on every keypress. **`commands/backlink_edit.rs` owns the whole cross-page family — `edit_backlink_block` plus `backlink_row_op` (the seven chords) and `create_backlink_sibling` (`o` / `O`) — and every one of them differs from its `block.rs` twin only in how it recovers.** `edit_backlink_block` propagates the `NotInTree` instead of re-targeting, because a backlink's `block_id` can be a transient `NodeId` and the append would move the user's text to the end of a page they are not looking at; it also refuses a source page ahead of its log, before anything is written, in `outl_actions::refusal::ahead_of_log_notice`'s own words — because **both GUI clients seed the draft from `link.source_block`, which `build_backlink_index_from_disk` built from the `.md`**, so on a frozen page the user typed over bytes the log does not hold. **`backlink_row_op` asks the same question for `ToggleTodo` and `MarkDone` and for nothing else**, and the criterion is `outl_actions::CrossPageWrite::computed_from_the_logged_text` — *is this op's result computed from the block's logged text*, never *is it structural*: those two read `block_text`, cycle a marker and write it back, so they have the text edit's corruption shape while the five `Op::Move`s name a node and converge whatever the local `.md` says. The criterion lives in `outl-actions` because the TUI gates on it too, and it was that same method implemented in both crates until it moved down — a drift between the two copies means one client refusing an operation the other performs, which costs user text. `BacklinkRowOp::shape` is this crate's half: the exhaustive mapping from a row op onto one of the three shapes, pinned by `each_row_op_declares_the_shape_of_the_write_it_performs` beside the `match`. `CrossPageWrite::TypedText` has no arm there on purpose — a text edit is `edit_backlink_block`, not a row op. The TUI's text edit deliberately does **not** ask at all, because `enter_insert_backlink` seeds its buffer from the op log (`outl_tui::actions::block::backlink_gesture` owns that side). The other nine took that recovery from the `block.rs` side until they were routed here: `create_block` resolves its anchor through `create_after_or_append`, which appended an empty block at the source page's end and answered `Ok`; six reported a stale id as `ActionError`'s `Display` (a ULID) while `outl_actions::refusal::stale_backlink_block_unchanged_refusal`, written for exactly that event, was called only by the TUI; and `move_up` / `move_down` / `indent` answer `Ok(())` when they find no sibling, so a stale id was a silent success — which is why the liveness gate is the module's own (`stale_row_block`), rather than each action's error. Liveness there means live **and under the row's page** (`row_is_live_on`: `outl_actions::is_live` plus `enclosing_page_id` equal to the row's page), because a child row names a block that a peer can move to another page while it stays live; the wording of the stale-row refusal is unchanged (`tests/backlink_row_ownership.rs`). It is asked **twice**: once before the ahead-of-log check (which *writes* on a merely-stale page, so a chord aimed at a deleted block used to rewrite a `.md` and then refuse) and again inside the commit, where a peer's delete can have landed in between and the workspace is in hand — `refuse_a_dead_row` owns that ordering and the TUI's argument for it. `BacklinkRowOp` is an exhaustive `match`, so a tenth chord cannot ship un-worded; `tests/backlink_row_ops.rs` is the net. Keep `edit_block` exactly as it is: unifying the two puts that append back on the cross-page path, which `tests/backlink_edit_strict.rs` pins in both directions — on a writer-wired host as well as the synchronous one, because `Some` is the only `projection_writer()` either client ships. It is a separate module for that reason and not only for the line count: every body in `block.rs` acts on the open page and may be tolerant of a stale id, and this one must not be. `commands/page_open.rs` owns the three **open** commands (`async` via `offload::run`; its module doc says why). `commands/page.rs` owns navigation, search, `delete_page` (calls `outl_actions::delete_page` + `remove_page_projection`, returns today's-journal `PageView` so the caller navigates away from the deleted slug), `page_backlinks(slug)` (the lazy backlinks fetch the frontend fires after the outline paints; the body lives in the sibling `commands/page_backlinks.rs`, which is a concurrency problem rather than page navigation: `compute_backlinks_offloaded` runs three phases off the IPC thread — a brief workspace lock for `list_pages` + this page's meta, an `O(blocks)` rebuild via `outl_actions::build_backlink_index_from_disk` when the host's `backlink_index()` slot is stale (reads the `.md` projection, touches no `Workspace`, holds **no** lock; when the host has no slot at all this is a one-shot from-disk build instead of the old direct workspace scan), then an `O(refs)` lookup under a brief lock (`look_up`), which also collects each direct hit's `Arc` subtree from `BacklinkIndex::subtree` (refcount bumps only) before the lock drops; `shape_reply`, after the locks drop, does the one deep copy per direct hit through `Backlink::with_children` **before** sorting, so a direct hit ships with its children (capped at `outl_actions::SUBTREE_MAX_DEPTH`), while namespaced hits still go through `Backlink::into_shallow`; returns `BacklinksReply`, which also carries the page's nested-pages rows — `namespace_children`, issue #275, derived from the page list phase 1 already read), and `set_backlinks_order(order, slug)` (persists `[display] backlinks_order` via `outl_config::save` and returns the re-sorted `BacklinksReply`, issue #142). `commands/exec.rs` owns `resolve_embeds(handles)` and its `EmbedContent` DTO (`handle`, `text`, `page_slug`, `status`, **`children: Vec<outl_actions::OutlineNode>`** — the source block's subtree projected with tokens via `outl_actions::project_parsed_subtree`, empty for a leaf, capped at `outl_actions::SUBTREE_MAX_DEPTH` (4) to match the client render depth so deeper levels don't ride the IPC for nothing); one command serves **both** `((…))` inline refs (uses `text`) and `!((…))` embeds (uses `text` + `children`), so the desktop resolves refs and expands embed subtrees off one round-trip (issue #147); plus **`resolve_page_embeds(names)`** + `PageEmbedContent` (`slug`, `title`, `blocks`), its page-level twin for `![[page]]` (issue #190) — same `project_parsed_subtree` call and same `SUBTREE_MAX_DEPTH`, so an embed renders what the source page renders. Both are `async` with **no workspace lock** (index and page come off the `.md` projection) and are keyed by the **name/handle as written**, never the slug, because the renderer holds `token.value`; an unresolvable one is **omitted**, so the client falls back to the raw token instead of drawing an empty page. Name → page is `page_for_name`: the read-only, non-creating rungs of `outl_actions::resolve::resolve_or_create_by_name`'s ladder (literal slug → `slugify(name)` → `by_title`, which owns title-beats-alias) |
| `commands/open_with.rs` | `open_external_file(source_path)` — the OS "Open With → outl" import. A path + lock adapter only: normalise the string (`file://` URL or plain path, via `helpers::normalize_picker_path`), read the file **before** taking the workspace lock (the bytes are not workspace state), then run create + import as one `finish_in_page_with` commit. An import dirties **two** pages (the new one, and today's journal which gains the `[[ref]]`), so the command projects the journal itself afterwards — the same shape `move_block_after` uses for a cross-page move, and for the same reason: `commit_page` is scoped to one page. Every policy question — the `open-in/` namespace, the `source::` rule that makes re-opening navigate instead of duplicating, the extension / size / UTF-8 refusals — is `outl_actions::open_with`'s. Registered by **both** clients (`open_with_commands!`); only the desktop has an OS association delivering paths, which is `Capability::OpenExternalFile` = `Missing` on mobile |
| `capture_inbox.rs` | `drain_capture_inbox` — the only path from a capture inbox (RFC 0363) into the workspace, over **both** inboxes: the one a mobile shell exported in `OUTL_CAPTURE_INBOX` (iOS App Group container, Android `filesDir`) and the open workspace's own `.outl/captures`, which is what `outl capture` writes. A client registers it once and gets whichever of the two its platform has — iOS only the first, the desktop only the second, Android both, and `mobile_inbox` is the `#[cfg]` that makes that true rather than aspirational (the env arm used to run everywhere, so on a desktop anything able to set the app's launch environment could aim the drain at a folder the user never chose). Policy (which day, which order, idempotency) is `outl_actions::capture`'s; what lives here is the page projection and the `CaptureDrainDto` the frontend refreshes from |
| `reminder_runtime.rs` | `take_due(state) -> Vec<ReminderDto>` — a thin DTO wrapper over `outl_actions::take_due`. The fired log and the due-scan live in `outl-actions`, not here: the TUI delivers too (OSC 9) and cannot depend on this crate, so keeping them here would have made the Tauri clients the owners of something every client needs. What's left is pulling the config + workspace off the `AppHost` and mapping to `ReminderDto`. |
| `commands/reminders.rs` | `list_reminders` / `reminder_settings` / `set_reminder_settings` / `snooze_reminder` / `clear_reminder_snooze` / `set_block_remind` / `mark_block_done` / `parse_remind_rule` / `format_remind_rule` + the `ReminderDto` / `ReminderSettingsDto` / `RemindFieldsDto` wire shapes. The last pair is an **authoring** surface, not a second grammar: `RemindFieldsDto` is a `remind::` rule flattened into what a form binds to, and the two commands are `outl_md::remind`'s `parse_remind` / `format_remind` across the bridge — a webview that read or wrote the rule text itself would be the second owner. `format_remind_rule` **refuses** a field set the grammar cannot express rather than correcting it, and re-parses its own output before returning, because a silently rewritten rule is a reminder that fires at a time the user did not pick. Registered through `wrappers/reminders.rs` (its own file now, the `trash.rs` reason: `catalog.rs` was at its ceiling). `mark_block_done` sets DONE outright via `outl_actions::todo::set_todo`, never `toggle_todo`: a rule can sit on a block with no marker, and one toggle there lands on `TODO`, so the reminders list's "mark done (cancels the reminder)" button used to arm the nag instead of cancelling it. `set_reminder_settings` exists because mobile has no settings screen: it reads `config.toml` and writes back only the two reminder keys, so it can't clobber a hand-set timezone or relay URL. Times cross the bridge as ISO-8601 **local** strings, not epoch numbers — the backend already resolved them in the configured timezone (`outl_actions::clock`), and re-deriving a local time from an epoch in JS reintroduces exactly the bug that module exists to fix. `snooze_reminder` takes no page id because it touches no `.md`: the snooze lives only in the op log. |
| `commands/telemetry.rs` | `get_telemetry_enabled` / `set_telemetry_enabled` — the crash-reporting opt-out (RFC 0377). Pure config I/O, and the **only** writer of `[telemetry] enabled`: the desktop's flat `Settings` restores that section from disk on every modal save, the same split `backlinks_order` has from `set_backlinks_order`. The setter returns the **saved** value, so a client moves its switch only after the write lands. Both clients register it, which here is not a convenience — reporting is on by default, so a client without the pair has no opt-out (`Capability::CrashReportingOptOut`). Its module doc carries why there are no unit tests |
| `commands/insert.rs` | `list_insert_commands()` / `resolve_insert_command(name, args)` + `InsertCommandDto` — the text-inserting slash commands (`/date-today`, `/time-now`, `/week-num`). Pure over `outl_actions::insert`, no workspace and no lock, same shape as `theme`. **The clock is read here, not in the client**: the instant has to resolve in the configured `[calendar] timezone`, and a webview's `new Date()` is the browser's zone — the same mistake the reminder DTOs avoid by crossing the bridge as local strings. The error is the catalog's own wording |
| `commands/shortcuts.rs` | `list_shortcut_bindings()` / `list_action_support()` + the `SupportDto` / `ActionSupportDto` wire shapes — the `(chord, action)` catalog and the per-client support matrix (root `CLAUDE.md` invariant 12). Moved here from `outl-desktop` so mobile registers the same two commands: a client can only tell the user *where* an action exists if it can read the matrix on the device asking. Pure functions over `outl_shortcuts`, no workspace access |
| `commands/timeline.rs` | `page_timeline` + the `PageTimelineDto` / `TimelineEventDto` wire shapes — a page's history, read out of the op log (issue #241). **Read-only**; there is no restore counterpart, on purpose (see `outl_actions::timeline`). `TimelineEventDto` is deliberately **flat** with `change` as a string tag saying which optional fields are meaningful. It is not a discriminated union and nothing narrows on it, but a reader switches on one field instead of unwrapping a nested enum, and `@outl/shared` renders it directly. `total` is the count **before** the limit, never `events.len()` — a capped list that reports its own length as the total reads as the whole history. `limit: Some(0)` is read as the default rather than as "no events", so a client that forgets the field gets a usable panel instead of an empty one. Registered by **both** clients now (`timeline_commands!`); mobile has the command and no timeline UI yet, which is why `Capability::PageHistory`'s mobile column stays `Missing` — see "One command surface, not two". |
| `commands/trash.rs` | `list_trash` / `restore_trash_entry` + `TrashEntryDto` / `RestoreTrashReply` — reading a deletion back (issue #287). Every verdict is `outl_actions::trash`'s: `refusal` rides each row so a listing cannot offer a restore the mutation declines, and the mutation re-asks `refusal_for` under its own lock because that verdict reads the live tree and can go stale. Both answers are the same `ActionError::to_string()`, so a client renders them and writes neither. `list_trash` is `async` (it folds every deletion's parent trail); `restore_trash_entry` resolves the landing page *before* mutating, since `commit_page` is scoped to one page and a trashed node's ancestor walk reaches the trash root. Registered by **both** clients (`trash_commands!` in `wrappers/trash.rs`, its own file because `catalog.rs` is full); mobile has no sheet, which is `Capability::Trash` |
| `commands/page_merge.rs` | `rename_page` / `merge_pages` + `RenamePageReply` (tagged on `status`: `renamed` or `exists`) and `MergePagesReply`. The policy is `outl_actions::rename_page` / `merge_pages` (RFC 0557); this is the IPC tail, in `outl page rename`'s order: **every** slug in `report.touched_pages` re-projected, then the stale projection removed **last** and only once the landing page projected. A taken name is a typed `Exists` reply with nothing written, never an `Err`. Not `commit_page`: it carries one page, and a rename or merge touches N; the module doc says which steps run once and which per page, and why undo stacks are cleared rather than recorded. Registered by both clients through `page_merge_commands!` (`wrappers/page_merge.rs`, its own file because `catalog.rs` is full); TS mirror in `@outl/shared/api/page-merge`, pinned in `wire_mirrors.rs` and `wire_enums.rs` |
| `commands/tasks.rs` | `list_open_tasks` — the `AppHost` lookup plus the offload, and nothing else. The selection, the ordering and the `OpenTasks` / `TaskHit` shapes moved to [`outl_actions::tasks`] so the TUI could reach them ([#454](https://github.com/outlmd/outl/issues/454)); this module re-exports them, which is what keeps `tests/wire_mirrors.rs` resolving. `async` because the index build reads and parses every `.md` in the workspace |
| `workspace_open.rs` | `open_workspace_at` / `reconcile_orphan_md` primitives, plus **`WorkspaceGuards`** (the shared `<root>/.outl/.lock` + exclusive `<root>/ops/.lock-<actor>` flocks, held for as long as the workspace is open — see "Workspace locks" below) and **`load_or_create_actor(local_dir)`** — a thin wrapper over `outl_core::DeviceStore::device_actor`. Both GUI clients keep a **device-wide** actor (`<local_dir>/actor`: `~/.config/outl` on desktop, the app sandbox's data dir on mobile) rather than the per-workspace one the CLI / TUI resolve, because the `HlcGenerator` is bound at app start, before a workspace is picked. That is safe precisely because `local_dir` is outside every workspace — it never rides the file-sync surface, so the cross-device actor collision described in `outl-core/CLAUDE.md` → "Actor id is device-local" cannot reach it. **Never move this file into the workspace.** Its `boot` no longer writes the snapshot: `snapshot_after_first_paint` does, off the launch path, and it asks `Workspace::snapshot_refresh_due` **at that point** rather than carrying a verdict from `boot` — sync ingest appends without going through `Workspace::apply`, so the delta can grow in between. Asking at all is what the local `!booted_from_snapshot()` used to get wrong: it covered only the full-replay half, so an adopted snapshot was never refreshed and the delta the next open replays grew without bound (488,651 ops on a real 722,943-op workspace: a 9.1s open here, a jetsam kill on iOS). See `outl-core/CLAUDE.md` → "`Workspace::snapshot_refresh_due` is the single owner", and [The interactive phase](#the-interactive-phase) in this file. |
| `page_preview.rs` | `emit_today_journal_preview(app, root)` — today's journal read straight off disk **through `outl_actions::read_page_outline_readonly`**, because this runs before `open_workspace_at` installs the workspace guards and the ordinary read renames a legacy sidecar, **before** the workspace opens, emitted as `page-preview`. The `.md` is the projection of exactly the page the user is waiting for and the sidecar carries the block ids, so the pair costs 0.6-1.3 ms whatever the graph holds. Four rules, each of which was the alternative: the preview is never editable, and nothing on the wire says so because nothing on the wire could — the guarantee is that `@outl/shared/boot`'s `<PreviewOutline />` has no interaction in it at all, where a flag is a claim a client can ignore; it is best-effort and silent (a fresh workspace and a day with no journal are `None`, never an error); `md_ahead_of_log_checked` is `false` so it cannot clear a client's sticky banner; and the page id is **derived** via `NodeId::from_slug`, so swapping the preview for the real view does not remount the outline under the user. The payload is a plain `PageView`, not a wrapper: a `{ view, editable: false }` shape would have been a second, weaker statement of what the renderer already guarantees by having no interaction in it |
| `first_paint.rs` | `FirstPaintGate` — what waits for the first frame, and what is allowed to. `on_open(label, job)` queues (or runs, once open); `painted()` opens it; **`arm_deadline(d)` is the host's obligation, not an option** — the painted signal comes over IPC, so a webview that fails to report would otherwise be an app that syncs with nobody, silently. Jobs run in registration order on one thread, off the caller's, because `painted()` arrives on the IPC thread. See "The interactive phase" below |
| `boot_ready.rs` | `WorkspaceReady` — a latch with a blocking wait, so "is the workspace open yet" is asked once and waited on instead of polled. `publish()` is the opener's, after the slot is filled. `wait(timeout)` blocks, so a command calling it belongs on the blocking pool. It runs the opposite direction from `FirstPaintGate` and is deliberately not folded into it |
| `commands/boot.rs` | `boot_mark(name)` + `wait_workspace_ready(timeout_ms)`, plus `BOOT_MARKS` — the four launch milestone names, a **wire contract** with `@outl/shared/boot` (pinned by `tests/boot_marks.rs`) and with CI. `boot_mark` does two jobs on purpose (record the milestone, release the gate on `first_content`) because they are the same fact and splitting them gives the backend two opinions about when a launch finished |
| `ops_watermark.rs` | Whether the op log on disk has moved since this process last read it — a `(file name, length)` pair per `ops-*.jsonl`, **excluding this device's own**. `read_needed` returns an opaque `Watermark` or `None`; `note` records it after the read lands. Append-only means a landed op always grows its actor's file, so a set of unchanged lengths means nothing was written. See "A reload with nothing to read is free" below |
| `workspace_reload.rs` | `reload_workspace_into` — the reload **both** clients run (`replay_from_disk` off a blocking pool thread, then publish only a tree that is not missing a local op), plus `publish_replayed` (the retry/compare/swap core, split out so a test can supply a replay that lands an edit inside the window) and `RELOAD_ATTEMPTS`. See "A reload publishes only what it cannot have dropped" below |
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
- the client's `BackupRoot` is pointed at the new root at that same moment, for the same reason: a failed open must not aim the automatic backup at a workspace nobody has.

**Never acquire or drop one of these from a client crate.** A second opinion about who holds the workspace is the defect this replaced.

**Lock ordering.** Both acquisitions are non-blocking `try_lock_*`, and both happen at open time — outside the process-wide `workspace` `Mutex`, and outside `ProjectionLock` (the *blocking* `flock` on `pages/.<name>.md.lock` that `apply_page_md_with_sidecar_guarded` takes under the workspace mutex). They add no wait-for edge to the existing `workspace` → `ProjectionLock` path. Keep it that way: a blocking cross-process lock taken above the workspace mutex would turn today's unbounded stall into a deadlock.

**A contended actor lock refuses the open; it does not fall back to an ephemeral actor.**
`outl_core::resolve_write_actor` is right for the CLI and TUI and unavailable here: a GUI's `HlcGenerator` is built in `setup()`, before a workspace is picked, so swapping only the *storage* actor would leave two live generators on one actor id — identical `(time, counter, actor)` triples, which is op identity, so `Workspace::apply`'s dedup silently drops one of the two ops.
Making the fallback available means making the client's `HlcGenerator` swappable at workspace-open time (a plain field on both `AppState`s today, read from ~56 call sites); that is client-side work, tracked separately.

Re-picking the workspace already open is handled inside `acquire_guards`, not by the caller: a POSIX `flock` belongs to an open file description, so a second `open` + `LOCK_EX|LOCK_NB` of `ops/.lock-<actor>` fails *inside the process that already holds it*.
It answers that case with `GuardClaim::Retained` — the locks this process already holds *are* the locks the re-pick needs, so nothing is released and nothing is retaken.
**Not** release-then-retake, which is what it used to do: every step after the claim can fail, and on failure the caller keeps the old `Workspace` published (`set_workspace` returns early) with its locks already gone — compaction then passes its gate and rewrites `ops-<actor>.jsonl` under a live client holding byte offsets into the old layout, which is the corruption the guards exist to prevent.
Pinned by `a_failed_re_pick_keeps_the_locks_it_could_not_replace`.

**The regression net** is `tests/workspace_locks.rs` — compaction refuses while a GUI is open and runs once it closes, a contended actor refuses rather than sharing the file, a refused open strands no lock, a re-pick does not refuse itself, a switch releases the old root, and a running GUI does **not** lock the TUI or MCP server out (the workspace lock is shared on purpose).

## Background passes yield, they do not race

A batch pass on a worker thread is **not** automatically invisible.
The UI reads the same disk to paint and waits on the same workspace mutex, so a pass that saturates either freezes the app whatever thread it runs on.
The premise is about the *device*, not thread count.

[`reconcile.rs`](src/reconcile.rs) owns the boot sweep both clients run; its module doc carries the per-phase numbers, the two phases that cannot be chunked, and why.
Three rules:

- **Yield in proportion to the work**, *outside* the lock (`workspace_open::yield_to_user`) — sleeping while holding it is the same stall with extra steps.
- **Take the lock with `try_lock`, and retry the unit — never skip it.**
  `parking_lot` is only *eventually* fair, so `lock()` can hand a background pass a mutex a command waited longer for; a `continue` on a busy lock steps over the page for good.
- **A phase that cannot be chunked waits out the launch window** rather than being chunked anyway (one unbroken 407ms hold on desktop, **2,557ms** on mobile, inside the first second → two windows of ≤234ms, nine seconds in).

What is **not** the answer: dropping the sidecar `fsync`.
On a `CURRENT_PIPELINE_VERSION` bump that is 2,827 files, **24.7s at 8% CPU**; dropping it reaches 0.3s and trades the one failure the project cannot afford — a rename landing before its data leaves a sidecar of garbage, read as missing, minting a fresh ULID per block, duplicating the page and breaking every `((blk-…))` handle.

## Async projection writes

`finish_in_page_with` (the tail every mutating command calls to build its reply) branches on `state.projection_writer()`:

- **`Some(writer)` (async path, both GUI clients today):** `writer.queue(page)` hands the page off to the background thread.
  The reply's `PageView` is built straight from the tree via `helpers::build_page_view_from_tree` (`commit.rs` is the caller) — no disk read, no render on the IPC thread.
  **"No disk read" is this branch's property, not the commit's**, and the two bullets are not a complete census:
  `BacklinkRefresh::PatchPage` projects inline even here, because the backlink patch reads the `.md` that write produces (see "One page's backlinks, not the whole index").
- **`None` (sync fallback):** the page is projected inline via `apply_page_md_with_sidecar_guarded` and the reply is read back **off the `.md`** via `build_page_view`, not from the tree.
  Either way a refusal annotates that successful `PageView` instead of falsely turning an already-persisted mutation into an error — the annotation is asked outside the branch, so the one path that can fail inline on a writer host still reports.

Either way, the undo snapshot (`HistoryStacks`) still renders the pre/post `.md` under the workspace lock — that render is needed for the diff regardless of who writes the projection to disk.

**`commands/history.rs`'s `step_history` (`undo_page` / `redo_page`) calls `ProjectionWriter::flush()` before it does anything else, whenever one is wired, and aborts without moving the history stack if any queued projection was refused.** Its restore (`outl_actions::restore_page_md`) writes the snapshot straight to the page's `.md` and reconciles it against whatever sidecar is *currently on disk* — and `finish_in_page_with`'s async path only ever queues that sidecar's write, never waits for it. Undo right after an edit, with nothing forcing the queue to drain first, let `reconcile_md` match against a stale or entirely absent sidecar and create a **duplicate block** instead of replacing the edited one — a real content-corruption bug a user could hit with an ordinary fast `Cmd+Z`, caught by `outl-tauri-shared/tests/history_command.rs`'s `undo_immediately_after_an_edit_does_not_duplicate_the_block` (RFC 0254 phase 1, fix round 3). The call happens *before* `with_ws_mut` takes the workspace lock — `flush()` blocks until the worker acks, and the worker needs that same lock to drain a page write queued ahead of the flush, so calling it from inside the lock would deadlock the two against each other. Cost: `flush()` drains the **whole** queue, not just the page being restored, so an undo issued right after a batch that dirtied many pages (a large paste, a plugin's `sync_hooks` sweep) waits for all of them, not one — for the ordinary one-edit-then-undo case this is one extra render+hash+write, the same order of cost `restore_page_md` already pays a line later.

A client that wires `AppHost::projection_writer()` to `Some` **must** spawn the `ProjectionWriter` at boot with the same `Arc<Mutex<Option<Workspace>>>` every command locks, or the queued writes race a different workspace instance.
`tests/projection_view.rs` asserts the tree-built view and the `.md`-built view agree — if you change either path, keep both in sync.
The shared page lock serializes cooperating outl writers around each `.md` + sidecar transaction. Because external editors do not honour advisory locks and pathnames have no portable atomic compare-and-swap, guarded writers also re-read the `.md` immediately before replacement and refuse if its bytes changed after authorization; the remaining read-to-rename interval is an unavoidable filesystem limitation, not something the lock is claimed to close.

## One page's backlinks, not the whole index

Every commit used to call `invalidate_backlink_index`, which drops the host's whole cached index, so the next `page_backlinks` re-reads and re-parses **every** `.md` in the workspace.
That is the right policy for a mutation whose reach this crate cannot bound (a paste, a cross-page move, a template, a plugin sweep)
and the wrong one for editing text through a backlink row, which is the daily `#fup` flow and dirties exactly one page.
On a 2,500-page graph that is root [`CLAUDE.md`](../../CLAUDE.md#critical-invariants-never-violate) invariant 15's third question asked of a path about to get much hotter.

`BacklinkRefresh` is which of the two a commit takes, and `finish_in_page_with_refresh` is where it is said.
`DropAll` is the default and every existing caller keeps it; `PatchPage` keeps the index and patches the one page through `reindex_backlink_page`.

Three things about `PatchPage` that are not preferences:

- **It is taken at the end of `project`, not in `invalidate_backlinks`.**
  The pipeline's backlink step is step 4 and the projection is step 5, and the patch reads the `.md` that step writes — so patching in step 4 would file the text from *before* the mutation, silently.
- **It forces a synchronous projection**, even on a host wired for async writes, for the same reason.
  The alternative is `ProjectionWriter::flush()`, which costs the same one-page write **plus** whatever else is in the queue, so this is the cheaper of the two ways to have an `.md` on disk.
  A cross-page commit therefore pays one page's render + write on the calling thread and saves the next `page_backlinks` an `O(blocks-in-workspace)` rebuild.
- **A refused projection falls back to dropping the index.**
  There is nothing to patch from, and an index describing a page this process could not read is worse than no index.

## The interactive phase

`boot` no longer writes the snapshot, and the opener no longer starts sync, the watcher or the orphan sweep.
All four wait for the frontend's `first_content` mark, through `FirstPaintGate`.

`workspace_open::snapshot_after_first_paint` is the snapshot half: it asks `Workspace::refresh_snapshot_in_background` **at that point**, not with a verdict carried from `boot`, because sync ingest appends to `ops-*.jsonl` without going through `Workspace::apply`, so the delta `snapshot_refresh_due` measures can grow in between.

Why each one waits, what deferring the watcher costs and how that window is closed, and why mobile's `open_done` channel is *not* replaced by the gate: [`docs/boot-performance.md`](../../docs/boot-performance.md).

Two rules a client has to keep:

- **A host returning `Some` from `first_paint_gate()` must have armed the deadline.**
  The gate cannot do it for them: it is constructed before the client knows its own timings, and a gate that armed itself would make the number invisible at the call site where it is a judgement.
- **A job registered on the gate is a `spawn`, not the work.**
  They run in sequence on one thread, so a job that blocks delays the ones behind it.

## A reload with nothing to read is free

A reload exists to pick up ops somebody **else** wrote, and it is O(whole log).
On a real workspace that is a 48 MB snapshot decode, a delta replay and 169,037 nodes rehydrated: 271-317 ms, measured on the running app.

Mobile's poll (`Journal.polls.ts`) runs `syncNow` then `reloadWorkspace` on a timer and has no way to know whether the pass delivered anything.
On a device whose peers were unreachable, that full rebuild therefore ran **every tick, forever, while the user typed** — three of them inside the first seven seconds of a launch, with `delta_ops` identical across all three.
Nothing had arrived. That is the journal that never stops looking like it is loading.

The guard is in the backend, not in the poll, because a reload with nothing to read should be free for **every** caller.
`ops_watermark::read_needed` is the question; `note` is the acknowledgement.

Three properties that are the point of it, and are easy to undo:

- **This device's own file is excluded.**
  Our appends are already in the resident log, so replaying for them is waste — and it *races*: `publish_replayed` refuses to publish a tree missing a local op and retries, so a steadily typing user could keep a reload spinning against their own keystrokes.
- **`Watermark` is opaque, and the only way to call `note`.**
  The state recorded has to be the one sampled *before* the read. A reload takes hundreds of milliseconds, a peer can append inside that window, and noting the later sample would record ops as seen that nothing replayed — the one way this guard could **hide** an edit rather than merely delay one.
- **A wrong answer costs latency in one direction only.**
  A fingerprint read from disk cannot call a file that grew "unchanged", so an arriving op is never hidden; at worst a reload is skipped and the next tick takes it. An unreadable directory answers "changed", so the guard degrades to the behaviour it replaced.

The frontend comment that made the poll unconditional is still true and still the reason this is not a frontend fix: relying on the `workspace-ready` event alone *"left the ops on disk without re-rendering — the symptom was 'only shows after I hit sync'"*.
The poll still asks on every tick. Asking is now what is cheap.

## A reload publishes only what it cannot have dropped

The replay runs on a blocking pool thread — a synchronous one holds the Tauri IPC thread through an O(all ops) rebuild and, on iOS, trips the scene-update watchdog.
It therefore runs **outside** the workspace mutex, and that is a window: a local edit can be applied (and appended to `ops-<actor>.jsonl`) after `JsonlStorage::open` scanned the log and before the fresh workspace is swapped in.
The op survives on disk and disappears from the published tree — and the next projection renders that tree over the page's `.md`.
Invariant 8's guard does not catch it: the sidecar still lists the block, so every line on disk reads as "known to the log".
A dropped op becomes deleted text.

The fix is the *publish*, not a longer lock (holding the mutex across the replay reinstates the stall the offload exists to remove).
`publish_replayed` reads `workspace.log().len()` under the lock before the replay and compares it under the lock at swap time, **in the same critical section as the swap** — splitting the two, which is what both clients did, reopens the window on a smaller scale.
The marker is exact for the question: `Workspace::apply` appends to the resident log exactly when it also persists, the log is never pruned (`apply_lru_cap` bounds the op *cache*), and a `WorkspaceBatch` cannot outlive the critical section that reads it.
Peer ops are deliberately not covered — sync ingest never touches the live workspace, so a replay that misses one is no worse than the tree already on screen.

A lost race retries (`RELOAD_ATTEMPTS`, bounded so a continuously-typing user cannot hold a reload in a loop); exhausting it **refuses** and leaves the workspace untouched rather than publishing a lossy tree.
Both clients call `reload_workspace_into`; `tests/reload_race.rs` pins the behaviour and pins that neither client replays on its own.

## A page that stopped syncing

Every open path (`open_today_journal`, `open_journal_for`, `open_page_by_slug`, `open_ref`) refreshes the page's `.md` from the tree before reading a view off it (issue #166).
That refresh can be **refused**: root `CLAUDE.md` invariant 8 forbids overwriting a `.md` holding content that exists in no op, so `apply_page_md_with_sidecar_if_stale` returns `ActionError::PageMarkdownAheadOfLog`.

The refusal is correct and it freezes the page in *both* directions — those lines never reach another device, and a peer's edits never reach this `.md` — until `outl reconcile --ahead-of-log` runs.
It used to die in a `tracing::warn!`, so the user saw a page that had silently stopped updating with nothing on screen to say why, and on iOS no binary to fix it with.

`helpers::reproject_stale_md` is where a GUI open path asks, and the verdict itself is `outl_actions::ProjectionRefusal`.
It moved down in [#292](https://github.com/outlmd/outl/issues/292): the TUI needs the same classification on its own open path and cannot depend on this crate, so the client with the hardest failure mode had no verdict at all and opened a frozen page silently.
What is left here is the DTO and the log line:

- `ActionError::PageMarkdownAheadOfLog` → `ReprojectOutcome.ahead_of_log`, which the open command hangs on `PageView.md_ahead_of_log` (via `PageView::with_ahead_of_log_check`, which also sets `md_ahead_of_log_checked` so the clients know this reply is authoritative and can clear a banner the reconcile fixed) and `@outl/shared`'s `<PageAheadOfLogBanner />` renders.
  The copy — including the fact that a mobile user has to go to a computer — lives in `@outl/shared/warnings::aheadOfLogNotice`, one owner for both clients and now a **declared mirror** of `outl_actions::refusal::ahead_of_log_notice`, which every Rust surface reads.
  The headline and the command are pinned across the boundary by `outl_actions::refusal::tests::the_shared_frontend_copy_says_the_same_thing`; the banner's extra fields have no terminal equivalent and are deliberately not mirrored.
- everything else → `ReprojectOutcome.other_error`, a local retryable condition.
  Only `open_ref` surfaces it, through its existing `ref-projection-failed` event; a disappearing toast is the wrong surface for a page that stays broken until a command runs.

**The open must never fail because of this.**
The guard withheld a *write*; the `.md` on disk is still readable, so the page opens showing exactly what is on disk.
Turning the refusal into an `Err` would trade a stale page for no page at all on the hottest path in the app.
`tests/ahead_of_log_view.rs` pins all three properties (the notice reaches the reply, the page still opens with its content, a healthy page stays quiet).

**Closed in the same change that opened this note.** A local mutation used to overwrite those lines: `finish_in_page_with` queued an unconditional `apply_page_md_with_sidecar` on the `ProjectionWriter`, so the write the read path refuses happened on the next keystroke commit. Every GUI write path now routes through `outl_actions::apply_page_md_with_sidecar_guarded` (`projection.rs`, `commands/block.rs`, `commands/template.rs`), which asks the one question that matters and skips the projection instead of deleting. The user's edit is never at risk either way — it went through `Workspace::apply`.
Post-mutation projection failures never turn that durable edit into an `Err`: synchronous paths annotate the successful `PageView` (`md_ahead_of_log` or `projection_error`), and the worker emits `projection-write-failed` with `ProjectionWriteFailed` after an async refusal.
Both clients route the structured refusal to the sticky banner and preserve unrelated failures on their existing status/toast surface.

## A paste rewrites the block being edited

`paste_markdown_at` / `paste_plain_at` are the only mutations that
rewrite the block the user is **currently typing in**, so they return
`PasteReply` rather than a bare `PageView`: `view`, plus the host
block's new `host_text` and the `host_caret` inside it (a **codepoint**
offset, like the `caret` argument).

Both GUI clients hold the block being edited as a draft seeded once, so
a reply carrying only the view left the editor on the pre-paste string —
which the next Esc / blur committed straight back over the paste. The
pasted link vanished from the block and from the `.md`, silently
([issue #488](https://github.com/outlmd/outl/issues/488)).

Two rules:

- **Neither field is computed here.** `outl_actions::PasteOutcome` built
  the string, and `commands::paste::paste_reply` only maps it — one
  function for every paste command, because a second copy is how one of
  them starts forgetting to carry the caret.
- **The caret is not "the end of the host text".** A plain splice leaves
  the user's tail to the right of it. `@outl/shared/paste`'s
  `pasteEditorUpdate` is the clients' one reader.

`commands/paste.rs` also owns the view-mode `paste_block_after(page_id, after_id?, parent_id?, text)`.
Its anchor is resolved **once, inside the commit, before any op**: a `parent_id` live on the page (the zoom root), else an `after_id` live on the page, else the page itself, else `NotInTree` with nothing written.
"Live on the page" is `backlink_edit::row_is_live_on`, promoted rather than copied, because a client captures both ids before awaiting the OS clipboard and a peer can trash or move either in between.
`tests/paste_block_anchor.rs` pins each rung, the `new_id` landing and the zero-op refusal.

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

- **Both clients register `boot_commands!`, and here that is not a convenience.**
  A client without `boot_mark` never reports a painted frame, so its deferred boot work waits out the gate's whole deadline on every launch instead of starting when the journal appears.
  It still works, which is what the deadline is for; it is just slower, with nothing on screen to say so.
- Adding a Tauri command that both clients need: body here (generic over `AppHost`), one line in the matching `*_commands!` list in `wrappers/catalog.rs`, and an `invoke_handler!` entry in **both** clients.
  Do not hand-write a wrapper — a command registered in only one client is drift, and `tests/command_parity.rs` exists to make that drift fail rather than ship.
- A client that wires `AppHost::backlink_index()` must call `commit::invalidate_backlink_index` after **every** path that can change what a page's backlinks are.
  That's local mutation (`finish_in_page*` already does this), a peer/workspace reload (`reload_workspace`, desktop's `set_workspace`), and a plugin run that applied ops (`commands/plugin.rs::run` / `sync_hooks`, guarded on `applied > 0`).
  Missing one of these serves stale backlinks until the next unrelated invalidation happens to fire.
- **Lock order is `workspace` → everything else.**
  The workspace `Mutex` is the outermost lock; the backlinks index, the undo `history` map and the desktop's `storage_root` slot are only ever taken *inside* it (or on their own).
  `finish_in_page_with` holds the workspace lock for a whole commit and drops the cached index from in there, so any thread that takes the index first and then waits on the workspace is an ABBA deadlock — `parking_lot::Mutex` has no timeout, so the app freezes until the user force-quits it.
  `compute_backlinks_offloaded` did exactly that and pasting was the reliable way in: a paste commits twice (draft flush + the paste) and every commit refreshes the panel, so the two collided within a keystroke.
  Pinned by `tests/backlinks_commit_deadlock.rs`, which stress-runs both paths against a watchdog — a re-inverted order fails the test instead of hanging the app.
- **Do not ask a command for an expensive field nobody is rendering.**
  `peer_status(with_pending)` is the worked example, and the third half of a fix whose first two halves are recorded on that command's own doc comment.
  Moving it to the blocking pool stopped the iOS watchdog killing the app; making the read cheap was `outl-sync-iroh`'s side.
  Neither noticed that `pending_ops` is the only field here that reads the op log, that `pendingLabel` renders it in exactly one place (the Sync panel's device list), and that the always-visible status dot — which reads `online` alone — polls this every 8 s **and** once per `peer-ops-changed`.
  So during an active sync the dot re-counted every op in the workspace several times a second for a number that was not on screen.
  `pending_ops` comes back `None` for a caller that did not ask, which is what it already means before a peer's first exchange, so no renderer needed changing.
  The general rule is root [`CLAUDE.md`](../../CLAUDE.md#critical-invariants-never-violate) invariant 15; [`docs/performance.md`](../../docs/performance.md) has the numbers.
- **The reply a mutation returns is on the hot path too, and `tests/backlink_commit_cost.rs` is the census that says how often it runs.**
  `build_page_view_from_tree` is two lines over `outl_actions::project_outline`, which read the workspace-wide property map once per block: **2,588 ms** to build the reply for one 688-block page, on every mutation, on both clients.
  Reported as "saving a backlink edit is slow", and the cross-page path was not the cause — `edit_block` on the same page cost the same, and the four extra `.md` touches a cross-page commit pays sum to ~85 ms.
  The fix was `outl_actions::tree::group_properties_of`, in that crate; what is left here is the count.
  A cross-page commit walks its page five times (the pre-check, the undo snapshot and its comparison, the inline projection `PatchPage` forces, the reply) and no signature says so, which is why the census is a test with a declared reason per call.
  [`docs/performance.md`](../../docs/performance.md) has the table and the procedure.
- **A command body that is not O(1) is `async`.**
  Not O(1) means it builds a `WorkspaceIndex`, walks the op log, renders markdown, copies files, runs a user's code, or holds the workspace mutex for more than a lookup.
  `offload.rs` owns the mechanics and the crashes that bought the rule, including the one set that *cannot* move off the IPC thread yet (the mutations behind `finish_in_page*`) and why.
- **Hold the workspace lock for the lookup, and nothing else.**
  A synchronous `#[tauri::command]` runs on the IPC thread (only `async fn` entries in `wrappers/catalog.rs` get the threadpool), so the window accepts no keystrokes for as long as anything holds that lock.
  `compute_backlinks_offloaded` takes it for `for_page_split` and the fetch of each direct hit's `Arc` subtree (`BacklinkIndex::subtree`, a refcount bump, part of the lookup); the deep copy of those subtrees (`Backlink::with_children`), the sorts, the `O(hits)` shallow projection of the namespace hits and the `O(pages)` namespace walk run in `shape_reply`, after both locks drop (`tests/backlinks_reply_children.rs` pins it, with a call-site census).
  Folding them back into the locked region is invisible in a test and shows up as a window that stops taking input while a section loads ([`UX.md` → Never block a keystroke](../../UX.md#2-never-block-a-keystroke)).
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
