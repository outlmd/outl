# CLAUDE.md — outl-actions

The **UI-agnostic** workspace operations layer.
Every outl client (`outl-tui`, `outl-mobile`, future Tauri desktop) consumes this crate so we never duplicate edit / indent / toggle / journal-render logic.

If you add a workspace operation that two or more clients need, **it belongs here**, not in the binary that asked for it first.

## Layering

```text
outl-core           (CRDT, op log, storage trait)
   ↑
outl-md             (.md parse/render, sidecar, matching)
   ↑
outl-actions        ← you are here
   ↑
outl-cli / outl-tui / outl-mobile / future clients
```

## Public surface

The full catalogue — every function, its signature and when to reach for it — lives in
[`docs/outl-actions-surface.md`](../../docs/outl-actions-surface.md).

It is reference material: you look up an entry, you do not read it top to bottom.
Keeping it here cost every task in this crate the context to load it, and pushed this file past the size ceiling (issue #216).

**Before adding a helper here, search that catalogue first** — see [Reuse-first](#reuse-first) below.

## Contract

Every mutating function:

1. Takes `&mut Workspace` (caller-owned) and `&HlcGenerator` (caller-owned).
2. Reads tree state, computes op parameters, generates a `LogOp` with a fresh HLC.
3. Routes the op through `Workspace::apply` so the op log stays the single source of truth (invariant #1 of `outl-core`).
4. Returns `Result<T, ActionError>` — never panics on user error.

Functions **never**:

- Touch storage directly.
  Storage is `Workspace::apply`'s responsibility.
- Put bytes that are not workspace state into the op log.
  An uploaded file, a capture inbox entry, a backup, a fired-reminder log: replaying a multi-MB PDF through the CRDT would bloat every device's op log irreversibly.
  Those bytes go to disk directly and only a **link** or a derived fact becomes an `Op`, the ordinary way.
  This crate does touch the filesystem, each time for a reason of its own:
  - `.md` projection: `journal::write_md_atomic`, `ProjectionLock`, `history::restore_page_md`.
  - non-op bytes: `asset::import_asset` (`<root>/assets/<hash>.<ext>`, tmp file + rename), `backup::*` (a git repo), `export_md`.
  - local state files: `capture::write_capture` / `drain_captures` (inbox), `merges` (`.outl/state/*.json`), `reminders::fired` (`reminders-fired.json`).
  - read-only: `open_with::read_source`.

  Before adding a new filesystem user, answer "is this workspace state?".
  If yes it is an `Op`; if no, say why it stays out of the log.
- Hold per-client state (selections, modes, toasts, keymaps).
- Round-trip through `.md` to reconstruct workspace state.
  The op log is the source of truth; `.md` is a projection.

A composite action calls `apply` more than once for a single user-visible mutation (`append_forest`/`append_tree`, `page::open_or_create`, `paste::paste_markdown`, `template::instantiate_template`, `block::split_block`).
It wraps its whole body in one `Workspace::begin_batch()` so the N ops flush as one `Storage::append_ops` per destination instead of fsyncing per op.
Each op still goes through `apply` individually (dedup, Yrs merge, and the CRDT stay untouched); only the persist is deferred to the guard's `commit()`.
See `outl-core/CLAUDE.md` → "Batch append" and [`docs/storage.md`](../../docs/storage.md) for the mechanics and durability contract — this crate only needs to know: open a batch around a multi-`apply` action, commit it before returning.

## Inserting a block next to another one

`a..=z` is dense enough for a midpoint and not dense enough for every pair, so "the slot between these two siblings" sometimes does not exist.
`outl_core::Fractional::between` says so by **dropping the bound it cannot satisfy** and returning a key above `left` — which is above `right` too.

`tree::interior_between` is the single owner of noticing that — one comparison, three callers: `position_before`, `position_after`, and `block::slot`, which needs the same question asked of two loose positions.
`block::slot` is the single owner of what to do next: `slot_between` decides, `vacate` frees a neighbour's key by shifting it up, and both **refuse to move a neighbour that cannot be cleared**.
A new block is allowed to land one slot off its intended place; an existing sibling is never allowed to move to a key that would send it past a block the user did not touch.
Relocating content nobody touched is the worse of the two failures, and the one that looks like data loss.

**Five places put a node next to an existing one, and every one of them goes through `block::slot`**: `create_before`, `create_after`, `outdent`, `move_after`, and `desync::recover_level`.
Four of them took `between`'s key unchecked and placed a block behind its neighbour ([issue #476](https://github.com/outlmd/outl/issues/476)); the recovery path was the fifth and the worst, because a user only reaches it already in trouble.
`create_before` had the repair written out inline, which is exactly why none of the others got it.
`tests/sibling_order.rs` drives the actions rather than `between`, because `between` was never the thing misbehaving; `tests/desync_recovery.rs` covers the fifth.

**None of this is a migration**, and reading it as one is how it gets deleted.
`between` still mints `(k, k + "a")` pairs on an all-new workspace wherever the gap forces a floor tail (18,278 of 12.8M sampled gaps), and a peer on an older binary keeps minting them tomorrow rather than only yesterday.
The measurement lives on `tree::position_after`.

## The commit pipeline

A mutation is never just the mutation. Five things happen around it, and
the order matters:

1. snapshot the page's `.md` **before** the change, for undo — kept only
   when the change actually altered the render, so a no-op command does
   not turn `Cmd+Z` into a visible nothing;
2. run the mutation;
3. drop the cached backlinks index;
4. announce the new ops to peers, so a device pulls now rather than on
   the next catch-up sweep;
5. project the page back to `.md` + sidecar.

`commit::commit_page(ws, hooks, page, mutate)` is that sequence.
It takes `&mut Workspace`, so a plain workspace from anywhere can run it
— no lock, no `Arc`, no DTO in the signature.

`CommitHooks` has exactly **one** required method, `project`. Undo,
backlink invalidation and the peer announce all default to no-ops, so a
CLI command with none of that state implements one method and still gets
the ordering guarantee.

Only the mutation can fail the commit. A projection failure arrives after
the op log already holds the truth, so `project` returns nothing and an
implementation that needs to surface the failure keeps it on `self` — the
Tauri clients do exactly that (`TauriCommitHooks` in
`outl-tauri-shared/src/helpers.rs`). Aborting there would report an error
for a change that did happen.

**Why it lives here.** The sequence had one implementation, generic over
a trait that wanted `&Mutex<Option<Workspace>>` — the Tauri state shape.
The TUI holds a plain `Workspace` and the CLI holds one in a command
context, so neither could reach it and both re-derived the parts they
thought applied. Repo-wide that left 34 direct calls to
`apply_page_md_with_sidecar_guarded` across 18 files: step 5 alone, each
one a caller that decided the other four did not apply to it. Some of
those calls are right. None of them were written down.
[#264](https://github.com/outlmd/outl/issues/264).

**Reach for it whenever you mutate a page and then want the world to
agree with the op log.** A bare `apply_page_md_with_sidecar_guarded` is
only correct when you can say which of the other four steps you are
skipping and why — put that reason in a comment next to the call.

## The `tree → .md` executor

`journal::survey_page_projections` is the **single owner** of "where does this page's `.md` stand relative to the op log", and `journal::reproject_stale_pages` is the executor built on it.
Both live in `journal/survey/`: `mod.rs` classifies, `sweep.rs` executes.

The classification used to be inline and private inside `outl doctor`, which is why this direction had no executor at all: the only code that knew which pages were safe to re-project was a read-only report a human had to run by hand.
The `.md → tree` direction has had a permanent executor since `outl serve` existed; this one had none, so ops arriving by sync left a page's `.md` wrong until somebody opened it.
Measured on a real 2,574-page workspace: **704 stale pages**, 702 of them re-projecting with zero content lines removed.
[RFC 0260](../../docs/rfcs/0260-tree-to-md-executor.md).

Three rules that are not negotiable here:

- **The survey selects; `apply_page_md_with_sidecar_if_stale` decides.**
  The executor hands every candidate back to that function, which re-asks the verdict under the page lock.
  A second opinion about which pages are safe to overwrite is how a read-only listing promises a repair the writing pass then refuses (root `CLAUDE.md` invariant 8).
- **The executor writes only what removes nothing.**
  A stale page whose re-projection would drop a content line lands in `ReprojectionSweep::withheld` and stays for `outl doctor --repair`, which backs every file up into `.outl/repair-backup/` and applies volume ceilings.
  That gate is also what makes a torn op log safe here without this module knowing anything about op-log health: a truncated replay renders *less* than disk holds, so every page it touches reports `lines_removed > 0`.
- **Every refusal is returned, never counted.**
  `ReprojectionSweep::refused` carries the `ActionError` per page, because a page that stopped syncing has to be named to whoever ran the pass.
  `ReprojectionSweep::unreadable` is the same rule for a page the sweep could not *look* at — an I/O error, or bytes that are not on this device yet.
  The states it stays quiet about are the ones somebody else will act on: an unreconciled external edit and a withheld hash are the `.md → tree` watcher's next job, and a sidecar that cannot vouch for its page is named by `outl doctor`.

`NotFound` is not "this page has no `.md`".
An undownloaded iCloud file answers `NotFound` too — the real name does not exist, only `.foo.md.icloud` does — and so does a `.md` that vanished from beside a live sidecar.
`journal::guard::guard_absent_markdown` is the single owner of that distinction, and **two** writers ask it: `mutate_page_md` and `apply_page_md_with_sidecar_if_stale`.
`apply_page_md_with_sidecar_guarded` does not, so an absent `.md` is written after a mutation.
`mutate_page_md` would otherwise recreate the page as a single block;
`apply_page_md_with_sidecar_if_stale` would otherwise write a file the arriving bytes collide with, unattended, on every `outl serve` sweep.
The survey routes the same call into `PageProjectionState::MarkdownNotHereYet`, so the listing and the pass cannot reach different verdicts.

`lines_removed` is measured by asking `content_lines_missing_from` about the **new render** ("will this line survive the write"), which is a different question from the guard's ("does the op log know this line", asked of the *sidecar*).
Both are correct and they are not interchangeable — see the `lines_removed_by` doc comment.

**A YAML frontmatter fence is a third case, on a second channel.**
It rides `Op::SetProp` on the page root rather than a block's text ([`docs/markdown-frontmatter.md`](../../docs/markdown-frontmatter.md)), so a block list cannot answer for it and `content_lines_missing_from` skips the region.
`guard::frontmatter_loss_error` asks `outl_md::unlogged::frontmatter_lines_missing_from` of the **render** instead, and both write gates run it (unlike `guard_absent_markdown`, which only `_if_stale` and `mutate_page_md` ask).
`lines_removed_by` adds the same count, so the listing and the pass cannot disagree.
Narrow on purpose: only a render carrying *no* fence counts as a loss, because a fence that merely differs is a peer edit, and refusing that would freeze the page.

**Which writers ask, and which do not, is a test rather than a sentence.**
`journal/apply.rs` holds six public writers and two of them are gates, and the module doc used to say "both write gates run all three", which was false (`_guarded` never asks `guard_absent_markdown`) and still read as a census.
`apply_page_md_with_sidecar_rendered` sat next to that sentence: public, exported, straight to `write_page_projection`, no guard, no caller anywhere in the repo.
During the #281 upgrade window (op log carrying a page's frontmatter fence as bullets, render that does not) that call is the write that deletes the fence.
It is gone, and `tests/projection_writer_gates.rs` now pins both halves: every `pub fn` in `apply.rs` has a recorded verdict, and `guard.rs`'s doc names all six.
Adding a writer without saying which side of the gate it is on fails there.

## Refusals: who tells the user, and in what words

`refusal.rs` is the catalog (one exhaustive `match` over `(Refusal, Surface)`, RFC 0255) **and**, since [#292](https://github.com/outlmd/outl/issues/292), the owner of the Rust-side wording: `ahead_of_log_notice(surface, lines, sample)`.

`ProjectionRefusal` (in `journal/refresh.rs`) is the other half — the split between "this page has stopped converging and only a command fixes it" and "an I/O condition the next open retries out of". `reproject_stale_md` is the page-open call every client makes: it refreshes a `.md` the tree outran (issue #166) and classifies the refusal that can come back instead.

Both exist because the policy had two homes and one of them was empty. `outl-tauri-shared::helpers::reproject_stale_md` held the classification, which the TUI cannot reach, so the TUI had no page-open check at all — opening a frozen page told the user nothing until they tried to write. That is the failure invariant 8's last paragraph names, moved into the client the maintainer daily-drives.

Two rules:

- **A client decides where a notice goes, never what it says.** A surface that writes its own sentence is a second copy of the fact, and the GUI clients already share one (`@outl/shared/warnings::aheadOfLogNotice`, a declared mirror — `the_shared_frontend_copy_says_the_same_thing` pins the headline and the command across the language boundary).
- **The notice's counts come off the `ActionError`, never recounted.** The number the user reads has to be the number the guard refused on.

### A refusal's *way out* is per surface

`refusal/trash.rs` is the second table, and it is **not** shaped like the first.
`refusal_support` answers *does this surface tell the user*, and for the trash that was already yes on all five.
What they lacked was a true second half to it.
`ActionError::TrashParentMissing` told a desktop user to run `outl doctor` and `TrashPageRestoreUnsupported` told them `outl block tree <ULID>`, in a panel that renders `e.to_string()` verbatim and shows no block ids (invariant 2).

`Remedy` is the closed set of ways out and `remedy_action` is one exhaustive `match` over (`Remedy`, `Surface`).
**Keyed on the remedy, not on the refusal**: four refusals share three ways out, so a per-refusal table would declare one answer twice.
Its module doc owns the rest; three rules from it:

- **`ActionError`'s `Display` keeps the diagnosis only**, no command and no instruction, because that is the half true everywhere (`no_diagnosis_prescribes_a_command`).
- **`trash::list` takes the `Surface`.** The forcing function, not a convenience: a caller cannot get a worded refusal without saying who it is for.
- **A surface that re-lists after a failed restore leaves the mutation's error bare; a one-shot surface words it.** Both GUI clients refetch on purpose, so the way out reaches the row the sentence is about; `outl trash restore` has no listing to come back to.

`TrashParentTrashed` is deliberately absent: "restore the ancestor first" works on every surface, since that ancestor is its own row in the same listing.

## The capture inbox

`capture.rs` is both halves of RFC 0363: the writer a capture gesture calls, and the **only** path from an inbox into the op log.

The rule that matters: **no writer touches the workspace.** A widget, a share target, a Shortcut and `outl capture` each leave one `<ULID>.json` file, and `drain_capture_dir` lands it. That is what keeps root invariant 7 — a second process appending to `ops-<actor>.jsonl` is the thing this design exists to avoid.

Three decisions that are not obvious from the signatures:

- **The block id *is* the capture's ULID.** That is what makes a drain idempotent (a crash between the op and the `remove_file` replays onto the same block) and what makes two devices draining the same `file`-transport-synced capture converge on one block rather than duplicating it, since `do_op(Create)` is idempotent.
- **`capture_day` resolves the day the capture was *made*, in the configured timezone.** A capture at 23:58 drained at 00:10 belongs to the 23:58 day, so a drain can create a *past* day's journal — which opening the app never does on its own. Any caller must run `clock::init` first or a late-evening capture files under the wrong day.
- **Two inbox locations, answering different questions.** `inbox_from_env` is the mobile one (an iOS extension only reaches the App Group container, an Android component only knows `filesDir`, so neither can resolve which workspace is open). `inbox_in_workspace` is `<root>/.outl/captures` — what `outl capture` writes, aimed at the workspace it resolved.

**The format has exactly one second implementation**, `OutlKit/Capture/CaptureInbox.swift`, because an iOS widget and share extension are separate processes under a memory ceiling this library would not fit. Android goes through JNI to `write_capture`, so Kotlin holds no copy — and neither does it hold `shared_line`, the "a share becomes one line" rule, whose only mirror is `SharedLine.swift`. **Adding a third copy of either is the thing to refuse in review.**

`write_capture` is create-temp, fsync, rename: only the rename publishes the `<ULID>.json` name `pending_captures` looks for, so a drain racing a write sees a whole capture or none.

**The loop has one owner, the per-capture step does not.** `drain_captures` holds everything that must not differ between clients — which captures are skipped, that a file is deleted only after its op is applied, the order — and takes the landing as a closure, because the Tauri clients take the workspace lock per capture and project the page while a caller holding a plain `&mut Workspace` needs neither. `drain_capture_dir` is that second shape. A second copy of the loop is how "does a blank capture get deleted" starts differing between the phone and the terminal, and the loop's tests only ever covered one of them.

The report does not take the block id from the landing either: it **is** the capture's ULID, so the loop derives it rather than trusting the closure to agree.

**The reader normalizes; it does not assume a writer did.**
`one_line` is the single owner of "a capture is one block", and all four entry points go through it: both halves of `shared_line`, `write_capture`, and `PendingCapture::line` on the way in.
Read `capture.line()`, never `record.text`.

That is not tidiness.
The inbox is written by processes this crate did not run — an iOS extension, a Shortcut handing the App Intent a multi-line string, a peer's file arriving over a `file` transport, a script using the documented format.
Landing a newline raw makes the renderer emit indented continuation lines, which the parser reads back as **tree structure and `key:: value` properties**.
One multi-line capture becomes several blocks plus a forged `title::`, and the capture's own block is trashed as an orphan on the next reconcile.
Root `CLAUDE.md` invariant 8 does not catch it: those lines came out of outl's own renderer, so they are inside `last_synced_hash`.
This is the producer half that invariant names.

One writer-side gap closed with it.
`shared_line`'s `url` half used to only `trim()`, which held while callers passed something URL-shaped and broke when Android's share target started putting an arbitrary `EXTRA_TEXT` in that slot.
The Swift twin takes a typed `URL?` and cannot express a newline, which is why `shared-line-cases.json` has no case for it and both parity suites passed.
`a_multi_line_url_is_still_one_line` is the Rust-only pin.

**"Present" is not "landed".**
Landing is two independently fsynced ops (`Op::Create`, then `Op::Edit` with the text) and nothing batches them, so a process killed between the two replays with the block in the tree and empty.
`land_capture` therefore re-asserts the text when the block is **empty**, not when it merely differs.
A diverging block may have been edited by the user after it landed, and the same capture file reaches a second device over a `file` transport, so re-asserting on divergence would let that second drain overwrite the edit.
Empty is the one state only the torn write produces (`a_torn_landing_is_completed_by_the_next_drain`, `a_landed_capture_that_was_edited_is_left_alone`).

`MAX_CAPTURE_BYTES` (1 MiB) bounds the read.
A capture is one line, the drain runs on every foreground return, and a file it cannot land stays in the inbox — so an unbounded read was paid again forever.
Oversized files are counted in `skipped`, never deleted.

Who drains: `outl_tauri_shared::capture_inbox::drain_capture_inbox` for both GUI clients (over both inboxes, projecting each page), and `App::drain_capture_inbox` once at launch for the TUI.

## Trash

`trash.rs` is the reading half of invariant 6. Delete is `Move(node, TRASH_ROOT)`, so the *preserving* half has always worked; until issue #287 nothing could list a deletion or put one back, which made the invariant's safety property "the bytes are still on disk" rather than "you can get it back".

Two single-owner rules, and both exist because a second copy already went wrong once:

- **`deletion_of`** folds the parent trail from `Create.parent` / `Move.new_parent`, once — `parent_at_deletion` and `TrashEntry::deleted_at_ms` are two readings of that one walk. `timeline::came_from` calls `parent_at_deletion` rather than its own. The old copy returned `true` on the *first* `Move` to the trash whose parent matched, which answers "the first page this block was ever deleted from" — wrong the moment a block can be restored and deleted somewhere else, which is exactly what `restore` made reachable. Pinned by `a_deletion_is_attributed_to_the_page_the_block_left_last`.
- **`refusal_for`** is the only thing that decides whether a restore would work. `restore` asks it before touching anything and `list` asks it per entry, so a listing cannot promise an action the mutation then refuses — the same shape as invariant 8's "one owner of the verdict". Pinned by `a_listing_carries_the_same_refusal_restore_would_return`.

**A restored block lands as the last child**, not in the slot it held. `Move.old_position` carries the same "originating replica's local derivation, undo-only" caveat as `old_parent`, so the original position is not recoverable from the log as data.

Two things are deliberately absent. Restoring a **page** needs a re-projected `.md` on top of the `Move`, and on the reference workspace 16 of 18 deleted pages have their slug taken by a live page today — picking a free slug would make this module a second owner of the slug rule, so it refuses and names where the content still is. **`empty`** is the only operation here that actually destroys, so it belongs with op-log compaction ([#110](https://github.com/outlmd/outl/issues/110)), not here.

Surfaces: `outl trash list` / `outl trash restore`, the `outl_trash_*` MCP tools, `outl doctor`, the desktop's `TrashPanel` and the TUI's `/trash` — all over `list` / `restore`. Per-client reach: `Capability::Trash`.

### Reopening a deleted page is not `restore` — `page/revive.rs` is

`trash` refuses a page root because *its* caller cannot pick a slug.
`page::open_or_create` is the slug rule's owner and has already established the slug is free, so it revives the root itself ([issue #473](https://github.com/outlmd/outl/issues/473)).
`page/revive.rs`'s module doc owns the reasoning; three rules from it:

- **The parallel implementation is `outl_md::reconcile::page_root::ensure_page_root_in_tree`, not `restore`.**
  Its `Move` fires for any non-`root()` parent, so it has revived trashed roots all along, and it leaves the children live where this trashes them.
  Routing either through the other is the thing to refuse in review.
- **Each direct child is trashed in its own op, before the root leaves the trash**, or "I deleted this page" means "hidden until tomorrow morning".
  They become ordinary `trash::list` entries: one entry becomes N, deliberately, which keeps this *hiding* rather than *destroying*.
  Except a child that is itself a **page root**, which `trash::restore` refuses — reopening it by its own slug is its recovery.
- **It is the first caller to move a node to `NodeId::root()`**, which caught two guards asking `contains` about a parent that never appears in `Tree::nodes`: `move_under`'s destination and `trash::landing_for`'s (`tests/root_is_a_legal_parent.rs`).
  `NodeId::trash()` stays refused there — landing in the trash is `delete`.

## Page namespaces

A page named `os/linux/debian` is nested under `os/linux`, which is nested under `os` (issue #275).
`namespace.rs` owns that, and the rule it owns is one sentence: **the hierarchy comes from the page `title`, never from the slug.**

`page::is_valid_slug` rejects `/` because a slug is joined into `pages/<slug>.md` as a single path component, so `outl_md::slug::slugify` folds `/` to `-` and `os/linux` lands on disk as `pages/os-linux.md`.
The `/` the user typed survives in exactly one place — the `title::` property — which is why every namespace question is asked of titles.

Two consequences worth knowing before touching it:

- **Comparison is per slugified segment, not per string prefix.**
  `OS/Linux` and `os/linux` are the same namespace (they resolve to the same page); `oscar/wilde` is not under `os`, which a `starts_with("os")` would have said it was.
- **`ancestors` is a *proper* prefix list.**
  `backlinks_keys::mentions_of` indexes a namespaced mention under `TargetKey::Namespace` for each ancestor, so `#os/linux` and `[[os/linux/debian]]` both reach the `os` page.
  Including the name itself would make every namespaced page its own backlink.

**The listing half depends on `title::` existing, and on an imported graph it usually does not.**
`page_repair_namespaces::repair_namespaced_titles` is the recovery: the namespaced names are still spelled out in the mentions, and `slugify` maps them onto the slug the ingested page already carries, so it joins them back rather than guessing.
Both GUI clients run it on their background reconcile pass, beside `repair_doubled_journal_titles`.
On a real 2,575-page workspace it took the nested-pages count from 1 to 100.
It refuses a page that already has a title, and reports a slug two spellings claim instead of flipping a coin into the op log.

`descendants` returns rows with `depth` and `label` already computed, so the TUI's `view/namespace.rs` and the desktop / mobile `<NestedPages />` render the same list without three copies of the split rule.
Per-client coverage is recorded as `Capability::NestedPages` (the TUI lists them but has no cursor in the list).

## Opening an external file (`open_with`)

`open_with.rs` owns the "Open With → outl" import: an OS gesture hands a client a `.md` / `.txt` that lives outside the workspace, and what lands is an ordinary page.
User-facing contract: [`docs/clients.md` → Opening a file from the OS](../../docs/clients.md#opening-a-file-from-the-os-open-with--outl).

Four decisions worth not re-litigating:

- **The namespace is a title, never a slug.** `slugify` folds the `/`, so the page projects to `pages/open-in-<stem>.md` while `open-in` becomes a real parent page (same mechanism as [Page namespaces](#page-namespaces)).
  **`import_into` calls `page::open_or_create` with the slug `slug_for` already resolved, never `resolve::open_or_create_by_name`.** The by-name path re-slugifies the title, and that is precisely how a stem that slugifies to nothing (`📝`, `...`) reaches the namespace page: `open-in/<stem>` collapses to bare `open-in`, and the import lands on the parent. `slug_for` refuses that collision; going back through the by-name helper undoes it. Pinned by `a_stem_that_slugifies_to_nothing_still_links_back`.
  This used to be *every* file for a user who names them in their own script, because `slugify` dropped each character it could not fold to ASCII ([issue #405](https://github.com/outlmd/outl/issues/405)). It keeps letters and digits now, so `会議メモ` projects to itself and only punctuation or emoji still reach the fallback — which is why the test that covered this path needed a replacement rather than an update: `a_non_latin_file_name_never_lands_on_the_namespace_page` still passes and no longer exercises it.
- **Re-opening a file does not import again.** The page carries `page-source::` (the canonicalised absolute path), and `resolve_target` returns `OpenWithTarget::Existing` when it matches. That key is in `tree::is_page_model_key`, so it **never reaches the `.md`** — the value is the user's directory structure, and `outl export hugo` copies unknown properties straight into published front matter. Hiding it costs nothing, because the check reads the op log and `outl_md::diff` only emits `SetProp` for properties the `.md` actually has. A second import would duplicate every block; overwriting would delete whatever the user wrote after the first import. Neither is acceptable, so the client navigates instead.
  The same property is what stops two *different* files named `notes.md` from being merged into one page — the second becomes `open-in/notes 2`.
  **The value is carried on `OpenWithTarget::New`, never recomputed at import time.** It used to be derived twice — once to match against, once to write — and a file that moved between the two made `canonicalize` fail on the second, so the page recorded the uncanonicalised path while the match had used the canonical one. The next open then failed to recognise its own page and minted `open-in/notes 2`: the duplicate the property exists to prevent, produced by the property itself. Pinned by `the_recorded_source_survives_the_file_moving_mid_import`.
  **A `New` target is re-checked by `import_into`, on the workspace it is mutating.** A client resolves under a read lock and imports under the mutation lock, and two deliveries can both see the slug free in between. `open_or_create` would silently return the page the first one made, and the second would paste every block again and link the journal twice. So `import_into` asks `find_by_slug` again: the same `page-source::` means "already landed" and behaves as `Existing`; a different one is `ActionError::ExternalFileTargetTaken`, because picking the next free slug there would create a page under an id the caller's `commit_page` is not snapshotting or projecting. The client re-resolves and gets `open-in/notes 2`. Pinned by `a_stale_target_for_the_same_file_lands_on_the_page_that_won` and `a_stale_target_for_a_different_file_is_refused_not_merged`.
- **`OpenWithTarget::page_id()` answers for a page that does not exist yet.** Page ids derive from the slug, which is what lets a client run the whole create + import inside one `commit_page`. `the_page_id_is_known_before_the_page_exists` pins it; if id derivation ever stops being deterministic, that commit would snapshot and project the wrong page.
- **A new import is linked from today's journal.** outl is journal-first, and a page reachable only by search is a page the user forgets they have, so `import_into` appends `[[<title>]]` to today's journal. It is a ref rather than plain text, so the imported page answers "where did this come from" through its own backlinks.
  That dirties **two** pages, and `commit_page` is scoped to one ([#264](https://github.com/outlmd/outl/issues/264)), which is why `import_into` returns `ImportOutcome { page, journal }` instead of a bare `NodeId` — the caller projects the journal itself. `journal` is `None` for an `Existing` target, which is what stops re-opening the same file five times from leaving five entries.

`read_source` owns the three refusals (extension, size, non-UTF-8) so the wording exists once rather than per client.
Content goes in through `paste::paste_markdown`, deliberately — a bulleted file lands as an outline and a bullet-free one as one block per line, which is the behaviour a paste of the same text already has.

## Page model

Pages are **regular nodes** directly under [`NodeId::root`] tagged with a `page-slug` property.
A `page-kind` property says whether the page is a regular `page` or a date-keyed `journal`.
The node's text is the page's title; its children are the page's blocks.
Keeping pages as ordinary nodes lets the tree CRDT handle move / delete / re-parent for free.

Disk layout when projected to `.md`:

```text
<root>/
├── journals/YYYY-MM-DD.md     ← page-kind = "journal"
├── pages/<slug>.md            ← page-kind = "page"
├── pages/<slug>.outl          ← sidecar (block IDs + hashes)
└── ops/ops-<actor>.jsonl      ← op log, one file per actor
```

`migrate_legacy_into_today` reshuffles any pre-page-model blocks (direct children of root that lack `page-slug`) under today's journal.
Clients call it once on startup; it's idempotent.

`page::toggle_pin` is the single owner of flipping the `pinned::` property (RFC 0254 phase 4).
It already writes an ordinary `Op::SetProp`, so pinned state converges through the op log like any other page property — root `CLAUDE.md` invariant 7 is satisfied, not something this function had to add.
Refuses journal pages (`ActionError::CannotPinJournal`), same rule the TUI's `g P` enforces.

**Reopening a deleted page by its own slug revives it instead of leaving it stuck in the trash.**
Page ids are deterministic (`page::page_id_from_slug`), so deleting a page and later reopening that same slug resolves to the same root rather than minting a fresh one.
`find_by_slug` only scans children of `NodeId::root`, so it never sees that root sitting under `TRASH_ROOT`, and `Op::Create` is a no-op for a node already in the tree: routing a reopen through the plain create path left the root trashed forever, with every edit on it rejected as `NotInTree` (issue #473, most visible on the journal, reopened every day).
`page::open_or_create` now checks `tree::is_trashed` on that deterministic id and, when it is, routes through `page_revive::revive_trashed_page` instead: the root moves back under `NodeId::root` with a fresh `Op::Move`, its pre-delete children are trashed individually (still ordinary, restorable trash entries, just detached from the page), and the slug/kind/title properties are re-applied.
A reopened journal is treated as a fresh daily: `open_journal`'s template re-stamp runs because `find_by_slug` did not find it before the revive.
This is deliberately **not** the same operation as `trash::restore`, which still refuses a page (see "Restoring a deleted block" in `docs/clients.md`); that refusal is about putting an arbitrary trashed node back wherever it fits today, not about reopening the one page a slug has always deterministically named.

## Task state convention

Task state lives **in the block's text** as a prefix:

```
"foo"             ← plain block
"TODO foo"        ← open task
"DOING foo"       ← task somebody has started
"DONE foo"        ← completed task
```

This matches the TUI's existing wire format.
`cycle_todo` walks `None → TODO → DOING → DONE → None`.

**`split_todo` reads a second spelling it never writes**: the CommonMark checkbox (`"[ ] foo"`, `"[/] foo"`, `"[x] foo"`, `"[X] foo"`), issue #230.
A block typed that way is a task everywhere a `TODO ` block is one, and the first `cycle_todo` / `set_todo` rewrites it to the word form.
The bytes on disk are untouched until then — recognising a spelling must never turn opening a page into a write.
The trailing space is load-bearing: `[x](url)` is a markdown link and stays one.
Three consumers keep their own copy of this alphabet: `outl-exec`'s query engine and `@outl/shared`'s `cycleTodo` (the dependency arrow forbids importing it) plus `outl_md::outline_ops::count_todos`.
All three learn a new spelling in the same change, or a block reads as a task on one surface and as prose on another.

**One deliberate asymmetry:** those copies also unwrap a leading `"> "`, so the legacy order `"> TODO foo"` reads as a task there.
`split_todo` cannot: it returns a `&str` slice of the body, so dropping the quote would strip the marker from `OutlineNode.text` and the GUI would stop drawing the `│` bar.
That shape is therefore a task to the TUI render, the TUI chip and `status:` queries, and prose to the DTO, the CLI and plugins.
`cycle_todo` rewrites it to canonical order on the first toggle.

**`TodoState::prefix` is the single owner of the marker spelling**, and the prefixes are **not** the same width — `"DOING "` is six characters, the other two are five.
Any caller doing cursor math (the TUI's inline cycle, a GUI draft splice) must measure the prefix it is adding or removing instead of assuming five.
A local `match` re-spelling the markers is the second owner that goes stale the next time a state lands: `quote.rs` had one, and it stopped compiling the moment `DOING` existed — which is the good outcome, but only because the enum is exhaustive.
`edit_text` writes the caller's text **verbatim** — including the prefix — so the user can drop a TODO just by erasing `TODO `/`DONE ` in the editor.
UIs that surface state separately (mobile checkbox) must reattach the prefix before calling `edit_text`; helper `rawTextWithTodo` on the mobile side does this.
The historical "auto-preserve prefix" behaviour was removed because it made `TODO`/`DONE` impossible to delete from the editor.

## Open tasks (`tasks`)

`collect_open_tasks(blocks, pages, limit)` is the single owner of **"what do I still have to do"**, with `OpenTasks` / `TaskHit` / `TASK_LIMIT`.

**The ordering is the product decision, and no client re-derives it**: started tasks first, then journals newest-first, then named pages alphabetically, each page in outline order. Journals and named pages need separate comparison arms — one descending slug compare for both sorts every letter-initial slug above every ISO date (`i` is 0x69, `2` is 0x32), which put journals last and named pages Z-to-A. Journal-ness is `PageEntry::is_journal`, asked of the same index the blocks came from, so it cannot disagree with the rest of the app about what a journal is.

It lived in `outl-tauri-shared` until [#454](https://github.com/outlmd/outl/issues/454). That crate is the Tauri backend, so the TUI cannot depend on it, and `Capability::TaskList` was `Missing` there for a reason about crate layout rather than about the terminal. Third time this move was the answer: `take_due` came down for the TUI's OSC 9 delivery, and `ProjectionRefusal` came down in [#292](https://github.com/outlmd/outl/issues/292) because the client with the hardest failure mode had no verdict at all.

What stayed on the Tauri side is the half that needs Tauri: the `AppHost` lookup and the offload, since the index build reads and parses every `.md`.

`crates/outl-tui/tests/task_list_reachable.rs` fails to **compile** if it goes back behind that edge.

## What this crate does NOT own

- **UI state.**
  Selections, modes, keymaps, and the undo stack for **in-flight text editing** (per-keystroke history inside an uncommitted draft) live in the clients.
  Committed-mutation undo is different: the bounded snapshot stacks + `.md` restore live here in `history` so every GUI shares one engine.
- **In-flight outline AST.**
  When the user is typing into a buffer that hasn't been parsed yet, the manipulation happens on `Vec<OutlineNode>` via `outl_md::outline_ops` (re-exported through the `outl-tui/src/outline_ops.rs` shim).
  We don't pull that up because it's not workspace-grounded — it's a stage *before* ops exist.
  It lives in `outl-md` because the mobile client also needs it, but no `Workspace` is touched, so it stays out of `outl-actions`.
- **Storage backends.** `JsonlStorage`, future `ChronDbStorage` implement `outl_core::Storage` and live in the binary that needs them.

## Reuse-first

This is the **shared layer**.
Every client (TUI, mobile, future desktop) consumes it — and they all consume the same struct, the same constants, the same policy.
Two parallel implementations of the same concept across clients is the bug we paid to delete,
see the `outl_md::index::Backlink` → `outl_actions::Backlink` consolidation,
where policy drifted on self-references and the user was the one who caught it.

When adding a new operation here:

1. **Search first.** `rg` for the symbol across `outl-core`, `outl-md`, and this crate before writing it.
2. **Promote, don't fork.**
   If a client crate already has a helper for the same concept, lift it here (and delete the client copy) — even if it's a small refactor.
   The `flatten_backlink_subtree` → `flatten_subtree_paths` move from `outl-md` is the canonical pattern: one owner, every client wraps.
3. **Generalize the parameter set** when migrating.
   The Backlink rewrite added `source_block: OutlineNode` + `source_path` so *both* the mobile linear renderer and the TUI subtree renderer could share the same struct.
   Capping features at "what mobile needs today" would force the TUI to keep its own copy.

The root [`CLAUDE.md`](../../CLAUDE.md#reuse-first) "Reuse-first" section documents the policy at the workspace level, in full at [`docs/contributing.md`](../../docs/contributing.md#reuse-first-no-parallel-implementations).

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-actions -- -D warnings`
3. `cargo test -p outl-actions`
4. If you changed the public API surface, update the table in "Public surface" above and the matching entry in the root `CLAUDE.md`.

## `activity` and `export_md`

- `device_activity(ws, local)`: ops and newest HLC wall time per actor in the resident log, plus `complete` (false after a snapshot boot, when the counts only cover ops since the snapshot). The local actor always gets a row.
- `export_markdown(root, dest_parent, stamp)`: copies `journals/**/*.md`, `pages/**/*.md` and `assets/**` into a fresh `outl-export-<stamp>` folder (suffixing `-2`, `-3` on collision). Sidecars, `ops/` and dotfiles stay behind. It is a copy, not a render: the `.md` on disk is already clean (invariant 2).
