//! Commit a mutation to a block on another page, through the op log.
//!
//! The one place a cross-page mutation is committed: a text edit, a
//! task-state flip, and every structural op reachable from the
//! backlinks panel. The two embed-source helpers in `metadata.rs` are
//! the one cross-page writer left on `save_page_with`, and they are
//! out of scope rather than out of reach: they are about **embeds**,
//! and `BlockEntry` already carries the `id` a commit through here
//! would be keyed on.
//!
//! It is the shared pipeline (`outl_actions::commit_page`) rather than this crate's
//! hand-written `save_page_with` sequence, per `outl-tui/CLAUDE.md` →
//! "The commit pipeline this crate does not use yet": the op comes
//! first and the source page's `.md` is downstream of it, instead of
//! the `.md` being rendered and `reconcile_md` inferring which ops that
//! implied.
//!
//! [`RestoreCommit`](crate::actions::trash) is the closest sibling —
//! both land on a page the user is not looking at — and the two things
//! this one does differently are reasoned out on
//! [`App::commit_backlink_mutation`].
//!
//! **What the ten gestures do not share is whether they may run
//! against a `.md` the op log does not hold.** That is one question per
//! gesture, not one per client, and
//! [`BacklinkGesture`](super::backlink_gesture::BacklinkGesture) is
//! where each caller answers it.

use std::path::PathBuf;
use std::sync::Arc;

use outl_actions::{commit_page, ActionError, CommitHooks, ProjectionRefusal, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::parse::ParsedPage;

use super::backlink_gesture::BacklinkGesture;
use super::backlink_target::BacklinkTarget;
use crate::state::{App, Focus, ToastKind};

impl App {
    /// Run `mutate` on the block a backlink row names, through the
    /// shared commit pipeline. `true` when the op landed.
    ///
    /// Five things happen around `commit_page`, and each was the
    /// alternative:
    ///
    /// - **The row's block is asked whether it is still live, before
    ///   anything else.** The panel's structural ops reach
    ///   `outl_actions::block::moves`, which guards with
    ///   `ensure_in_tree` rather than `ensure_live` *by design*, so a
    ///   trashed block was a legal target for all four of them — see
    ///   [`App::backlink_block_can_take_the_write`], which owns the
    ///   argument and the one gesture the gate must not sit in front of.
    /// - **A page ahead of its log is refused before the mutation —
    ///   for the gestures whose op is computed from the logged text,
    ///   and only those.** [`BacklinkGesture`] is where each caller
    ///   says which it is, and
    ///   [`App::source_page_can_take_the_write`] owns why that is the
    ///   criterion and the three states it must *not* refuse.
    ///
    /// - **The pending save is flushed first.** This crate is AST-first
    ///   with coalesced saves, so a queued `persist` renders the
    ///   pre-edit AST over whatever the op log gained in between. That
    ///   is `outl-tui/CLAUDE.md` → Persistence model's rule, and the
    ///   whole reason it has to be the first statement here.
    /// - **The undo snapshot is taken here, not through the hook.**
    ///   `CommitHooks::record_undo` hands over a rendered `String` keyed
    ///   by `NodeId`; this crate's stack holds a `ParsedPage` keyed by
    ///   `view_path`, which `top_applies_here`
    ///   ([#430](https://github.com/outlmd/outl/issues/430)) resolves by
    ///   path. Routing through the hook would mean a `NodeId → path`
    ///   lookup plus a re-parse to arrive where
    ///   [`App::snapshot_page_for_undo`] already is, so `records_undo`
    ///   stays at its `false` default.
    /// - **The open page is reloaded when it *is* the source page.** A
    ///   page can cite itself and `backlinks_for_page` includes that
    ///   deliberately, so the in-memory AST would otherwise sit behind
    ///   the op log and the next `persist` would write the stale text
    ///   back over the op.
    ///
    /// Two things the **caller** owns:
    ///
    /// - **Not calling this for a mutation that changes nothing.** The
    ///   snapshot is unconditional, and pushing one clears `redo`, so a
    ///   boundary no-op (moving the first sibling up) would bury the
    ///   displayed page's history under a foreign entry. `commit_insert`
    ///   gates on `new_text != original_text`; the structural branches
    ///   in `structural.rs` ask the tree whether the op has anywhere to
    ///   go before they call this.
    /// - **Reading `self.status` on `false`.** The failure is reported
    ///   there and nowhere else, and `NotInTree` is reworded on the way
    ///   out — see the `match` below.
    pub(crate) fn commit_backlink_mutation<F>(
        &mut self,
        target: BacklinkTarget,
        gesture: BacklinkGesture,
        mutate: F,
    ) -> bool
    where
        F: FnOnce(&mut Workspace, &HlcGenerator, NodeId) -> Result<(), ActionError>,
    {
        self.flush_pending_save();

        if !self.backlink_block_can_take_the_write(target.page, target.block, gesture) {
            return false;
        }
        let Some(meta) = outl_actions::page_meta(&self.workspace, target.page) else {
            // The diagnosis is a `tracing::warn!`, not the status line:
            // the row resolved its page out of the backlink index a
            // moment ago, so this is an internal condition the user can
            // do nothing with. What they do need is the **cost**, which
            // is the catalog's to word — the sentence that used to sit
            // here named a page and said nothing about whether
            // anything had been lost, which is C1's shape one arm over.
            tracing::warn!("backlink commit: the source page a row named is not in the workspace");
            self.status = gesture
                .stale_block_refusal(&self.workspace, target.block)
                .to_string();
            return false;
        };
        let source_path = outl_actions::page_md_path(&self.workspace_root, &meta);
        if gesture.shape().computed_from_the_logged_text()
            && !self.source_page_can_take_the_write(&meta.slug, target.page)
        {
            return false;
        }
        // The pre-mutation page as it stands on disk, because that is
        // what `restore_snapshot` writes back. Read before the commit
        // and kept only once the commit succeeded. `None` when the page
        // has no `.md` yet (in the log, never projected): there is no
        // prior file state for `u` to restore, so it records nothing
        // rather than offering an empty page as "before".
        let before = std::fs::read_to_string(&source_path)
            .map(|md| outl_md::parse(&md))
            .ok();

        let hlc = self.hlc.clone();
        let mut hooks = BacklinkCommit {
            root: self.workspace_root.clone(),
            hlc: hlc.clone(),
            transport: self.sync_transport.clone(),
            backlinks_stale: false,
            projected: None,
            projection_failure: None,
        };
        let block = target.block;
        if let Err(e) = commit_page(&mut self.workspace, &mut hooks, target.page, |ws| {
            mutate(ws, &hlc, block)
        }) {
            // `NotInTree` is the one refusal every chord here can get
            // from an ordinary state: the backlink index is built from
            // disk, so a row outlives a peer's delete of the block it
            // names (`ensure_live` returns it for gone *and* trashed).
            // It is also the one whose `Display` names a `NodeId`, and
            // no outl surface shows a user a block id.
            //
            // The words are **not this crate's**:
            // `outl_actions::refusal` owns what a stale backlink write
            // says, so the desktop and this client cannot describe the
            // same event differently. Which of its two halves is true
            // is [`BacklinkGesture`]'s answer, not a constant: a
            // `Ctrl+T` or a `move_up` typed nothing, so "your text
            // could not be saved" would name a loss that did not
            // happen, while `commit_insert` arrives here with the
            // buffer already consumed and owes the user that sentence.
            //
            // A **text** edit should not reach this arm at all:
            // `enter_insert_backlink` refuses to start on a block that
            // is gone or trashed, and this client holds peer reloads
            // back for the whole of Insert (`pending_reload`), so the
            // block cannot die between `i` and `Esc` either. The
            // wording is routed through the gesture anyway, because an
            // argued-unreachable path that silently drops keystrokes is
            // how C1 happened: the cost is declared at the call site
            // rather than re-derived here.
            self.status = match e {
                ActionError::NotInTree(_) => gesture
                    .stale_block_refusal(&self.workspace, block)
                    .to_string(),
                e => format!("backlink edit failed: {e}"),
            };
            return false;
        }

        if let Some(before) = before {
            self.snapshot_page_for_undo(source_path, before);
        }
        if hooks.backlinks_stale {
            // The panel reads the backlink index, which is built from
            // `.md` on disk — so the row keeps its old text until the
            // source page is re-read. Incremental, like `save`'s: every
            // ref this commit could have added or removed originates on
            // this one page.
            self.reindex_backlinks_for_slug(&meta.slug);
        }
        if let Some((path, projected)) = hooks.projected.take() {
            // `save_page_with` patched the `WorkspaceIndex` here and
            // dropping that was a defect, not a saving. It has **three**
            // readers and all of them go stale together: `/` workspace
            // search and the `((` autocomplete both fuzzy-match
            // `index.iter_blocks()`, and `((blk-XXXXXX))` chips resolve
            // through it. Searching the new text would find nothing while
            // the old text still matched.
            //
            // And the window is the rest of the session, not a frame:
            // navigation deliberately does **not** rebuild this index
            // (`loading.rs` says so in place), so only `Ctrl+L`, a peer
            // reload, a paste, a template, a merge, a sidebar delete or a
            // relaunch would have closed it. Every chord in the panel
            // comes through here now, so `Ctrl+T` and `i`+`Esc` on the
            // same row cannot disagree about what is searchable.
            self.patch_index_page(&path, &projected);
        }
        if let Some(e) = hooks.projection_failure.take() {
            self.report_backlink_projection_failure(&meta.slug, target.page, e);
        }
        // The panel can be shorter than it was a moment ago — the
        // commit may have taken the row the cursor is on out of it — so
        // the clamp runs here, on **every** path, and not inside the
        // reload below. The reload only happens for a self-citing page,
        // which is the rarer half; a cross-page `d d` on a page's only
        // backlink empties the panel without it, and `Focus::Backlink`
        // over zero rows has no keyboard way out (`j` and `k` both
        // decline a stale `idx` by design, and every chord declines it
        // too), so the cursor was stranded until a navigation or a
        // mouse click — and mouse capture is opt-in and off by default.
        self.focus = self.clamp_backlink_focus(self.focus.clone());
        // A page can cite itself, so the page on screen is sometimes the
        // one just written to.
        if self.page_is_open(target.page) {
            self.reload_open_page_keeping_the_cursor();
        }
        true
    }

    /// Whether `block` is live **and still under `page`**, the page the
    /// row was built against.
    ///
    /// A peer can move the block to another page while the async index
    /// rebuild is in flight; the row still reads `(page, block)`, and
    /// `commit_page` would scope the undo snapshot and the projection
    /// to the wrong page. Same semantics as
    /// `outl_tauri_shared`'s `row_is_live_on`, over the same two
    /// `outl_actions` primitives.
    pub(crate) fn backlink_row_is_live_on(&self, page: NodeId, block: NodeId) -> bool {
        outl_actions::is_live(&self.workspace, block)
            && outl_actions::enclosing_page_id(&self.workspace, block) == Some(page)
    }

    /// Whether the block a backlink row names is still **live** —
    /// asked once, before anything is written, for every operation the
    /// panel has.
    ///
    /// The shape is [`App::source_page_can_take_the_write`]'s and for
    /// the same reason: ten chords reach this helper, so one question
    /// asked here covers all of them where ten guards in the branches
    /// would cover whichever ones somebody remembered.
    ///
    /// **Why the backend does not already answer it.**
    /// `outl_actions::block::edit` asks `ensure_live` at all three of
    /// its entries, so the text edit, `Ctrl+T` and `g D` were guarded.
    /// `outl_actions::block::moves` asks `ensure_in_tree` instead, and
    /// says why in place: `trash::restore` is a move out of the trash,
    /// so a trashed node has to be a legal *source* there. That
    /// tolerance is load-bearing and is not a bug to fix one layer
    /// down — the missing question was the caller's.
    ///
    /// **The predicate is still the backend's.**
    /// [`outl_actions::is_live`] is `ensure_live`'s own `bool` half, so
    /// this gate cannot come to a different conclusion about "live" than
    /// the guard behind the three actions that do ask. This crate used
    /// to spell the pair out itself, which is the duplication that
    /// `ensure_live`'s own doc already warned about one layer down.
    ///
    /// **Why this crate's boundary predicates could not fill the gap.**
    /// `has_previous_sibling` / `has_next_sibling` / `can_outdent` ask
    /// the tree about **position**, and a trashed node's parent is
    /// `NodeId::trash()` — a node the tree keeps, whose children are
    /// every deletion in the workspace. So `Tab` found a previous
    /// sibling inside the trash and `outl_actions::indent` re-parented
    /// the block under an unrelated deletion, where `trash::list` (the
    /// trash root's **direct** children) can no longer see it; `d d`
    /// re-deleted a deleted block and reported success; `Alt+↑` swapped
    /// two deletions the user never touched; and `Shift+Tab` reached
    /// `NoGrandparent`, whose `Display` names a `NodeId`.
    ///
    /// Three things about the placement:
    ///
    /// - **It is the *write* gate, so it is the only door that covers
    ///   every path.** Eight call sites reach this helper and one of
    ///   them — `commit_insert`'s `EditTarget::SourcePage` arm — builds
    ///   its [`BacklinkTarget`] directly and never goes through
    ///   `focused_backlink_target_or_report`. A gate in the resolver
    ///   would have missed the text edit.
    /// - **It must not sit anywhere `trash::restore` passes through.**
    ///   This crate reaches `restore` from `actions::trash`, through its
    ///   own `RestoreCommit`, so the gate cannot reach it — and the
    ///   outline path never calls the `outl_actions` moves at all (it
    ///   edits the in-flight AST). Both are asserted in
    ///   `liveness_tests`, not reasoned about.
    /// - **It runs before [`App::source_page_can_take_the_write`]**,
    ///   which *writes*: that call refreshes a merely-stale `.md`. A
    ///   chord aimed at a block that is gone should not touch the disk
    ///   at all.
    ///
    /// The sentence is [`BacklinkGesture`]'s choice between the
    /// catalog's two halves, like the `NotInTree` arm below. A text
    /// edit should not reach here having typed anything —
    /// `enter_insert_backlink` refuses to *start* on a block that is
    /// gone or trashed and `pending_reload` holds peer reloads back for
    /// the whole of Insert — and the cost is declared anyway, because
    /// the one refusal on this path that *was* reachable from a
    /// consumed buffer reported a condition that said nothing about
    /// the keystrokes it had just dropped.
    fn backlink_block_can_take_the_write(
        &mut self,
        page: NodeId,
        block: NodeId,
        gesture: BacklinkGesture,
    ) -> bool {
        if self.backlink_row_is_live_on(page, block) {
            return true;
        }
        self.status = gesture
            .stale_block_refusal(&self.workspace, block)
            .to_string();
        false
    }

    /// Whether the source page's `.md` can take a write at all — asked
    /// **before the mutation**, and only for a gesture whose op is
    /// computed from the block's **logged** text
    /// ([`BacklinkGesture::MarkerFromLoggedText`], which is `Ctrl+T`
    /// and `g D`).
    ///
    /// A page whose `.md` holds lines the op log never recorded is
    /// frozen in both directions (root `CLAUDE.md` invariant 8). The
    /// cost of that is **per gesture**, not per client, and this is the
    /// criterion:
    ///
    /// - `toggle_todo` / `mark_done` read `block_text`, move a marker
    ///   and write the result back. The row the chord was aimed at was
    ///   rendered from the `.md`, so the op would be computed from text
    ///   the user never saw — and [`BacklinkCommit::project`] then
    ///   refuses to write it out, so the row comes back unchanged while
    ///   the hidden state walks on in the log. That is the one shape a
    ///   notice cannot repair, so these two are refused up front.
    /// - The **text edit** is not asked, and that is the difference
    ///   between this client and the GUI ones.
    ///   `enter_insert_backlink` seeds the buffer from
    ///   `workspace.block_text` — the op log — so the text the user
    ///   edited *is* the logged text and the `.md` cannot surprise it.
    ///   The desktop and mobile seed theirs from `link.source_block`,
    ///   which `build_backlink_index_from_disk` built from the `.md`,
    ///   which is why `outl_tauri_shared`'s `edit_backlink_block` does
    ///   ask. Gating it here cost the only unrecoverable thing in the
    ///   feature: `commit_insert` consumes the Insert buffer before it
    ///   calls, so a refusal dropped the keystrokes and reported a
    ///   *file* condition that named none of it.
    /// - A **structural** op names a node and converges through the op
    ///   log whatever the local `.md` says. The stale `.md` is the
    ///   pre-existing condition, not something the gesture caused
    ///   (root `CLAUDE.md` invariant 11), and the missing notice for it
    ///   is its own open item.
    ///
    /// There is no entry-side twin because the two gestures that ask
    /// have no entry: a chord is one atomic act with nothing typed, so
    /// here the write side *is* the first opportunity. Where a gesture
    /// does have an entry, the gate belongs there —
    /// `enter_insert_backlink`'s `outl_actions::is_live` call is the
    /// worked example, and its own comment says why.
    ///
    /// Four things about the shape:
    ///
    /// - **It is `outl_actions::reproject_stale_md`, the same call
    ///   `load_current_no_autorun` and every GUI open path makes.** The
    ///   trail runs to `outl_md::unlogged::sidecar_can_vouch_for`, the
    ///   single owner of "can this sidecar vouch for the `.md` on
    ///   disk"; asking that predicate here directly would be a second
    ///   assembly of a verdict that also has to pair it with "is
    ///   anything actually on disk" and with the frontmatter channel —
    ///   the pairing [issue #332] froze 233 pages for getting wrong.
    /// - **The refusal narrows to pages with something to lose, and
    ///   that is load-bearing.** An empty page, a page with no sidecar
    ///   and a brand-new journal day all come back `None` here, so the
    ///   edit proceeds; gating on `sidecar_can_answer` instead would
    ///   refuse all three, silently, which is [issue #332]'s mirror and
    ///   the half nobody reports. Pinned by
    ///   `a_source_page_with_nothing_to_lose_still_commits`.
    /// - **A retryable condition is not a refusal.** I/O, permissions,
    ///   an unreadable sidecar: a `tracing::warn!` and the commit goes
    ///   ahead, exactly as on every other path in this crate. The op
    ///   log is still the right place for the edit and the next open or
    ///   the orphan scanner picks the page up.
    /// - **It runs before the undo snapshot is read**, because this
    ///   call also *refreshes* a merely-stale `.md`. Reading `before`
    ///   first would snapshot the pre-refresh file and `u` would
    ///   restore the tree's own past over it.
    ///
    /// The words are the catalog's
    /// ([`outl_actions::refusal::ahead_of_log_notice`] at
    /// `Surface::Tui`) — the same call the post-commit report makes, so
    /// the two cannot describe one event differently. The slug prefix
    /// is this client's, like the toast's: the notice says "this page"
    /// and the page the user is looking at is a different one.
    ///
    /// [issue #332]: https://github.com/outlmd/outl/issues/332
    fn source_page_can_take_the_write(&mut self, slug: &str, page: NodeId) -> bool {
        match outl_actions::reproject_stale_md(&self.workspace, &self.workspace_root, page) {
            None => true,
            Some(ProjectionRefusal::Retryable(message)) => {
                tracing::warn!("backlink edit: source page re-projection failed: {message}");
                true
            }
            Some(ProjectionRefusal::AheadOfLog { lines, sample, .. }) => {
                let notice = outl_actions::refusal::ahead_of_log_notice(
                    outl_actions::refusal::Surface::Tui,
                    lines,
                    &sample,
                );
                self.status = format!("`{slug}`: {}", notice.one_line());
                false
            }
        }
    }

    /// Reparse the open page without moving the user.
    ///
    /// `load_current_no_autorun` resets `focus` to `Focus::Outline` and
    /// clears `zoom_stack`, and its own comments say why: a *view
    /// change* carries a `Focus::Backlink { idx }` that indexes the
    /// previous page's list, and a zoom root is a path into the previous
    /// page's AST. Neither applies here — this is the same page reparsed
    /// after its own edit, the backlink list is the same list and the
    /// row the cursor sits on is the row that was just edited.
    ///
    /// Leaving them reset was not cosmetic. `enter_insert` dispatches on
    /// `Focus`, so the next structural op from the panel (which commits
    /// an in-flight edit through here first) re-entered Insert on
    /// `App.page` with `EditTarget::CurrentPage` — an AST loaded
    /// *before* that op wrote its own change — and the following
    /// `persist` rendered it back over the file, reverting the op in
    /// silence.
    ///
    /// `focus` is restored verbatim: the caller clamped it before
    /// calling, on purpose. Clamping *here* would cover only the
    /// self-citing half, and the cross-page half is the one that strands
    /// the cursor — see the call site.
    fn reload_open_page_keeping_the_cursor(&mut self) {
        let focus = self.focus.clone();
        let zoom = std::mem::take(&mut self.zoom_stack);
        let selected = self.selected;
        self.load_current();
        self.focus = focus;
        self.zoom_stack = zoom;
        // Clamped against the *reparsed* page, because the edit may have
        // changed how many blocks it has.
        self.selected = selected.min(self.flat_len.saturating_sub(1));
    }

    /// `focus`, with a `Backlink { idx }` pulled back inside the list it
    /// indexes — or dropped to the outline when the list is now empty.
    ///
    /// Counted rather than listed: `backlinks_count_for_current` answers
    /// this without cloning a `Backlink` per row, and every cross-page
    /// commit runs through here.
    pub(crate) fn clamp_backlink_focus(&self, focus: Focus) -> Focus {
        let Focus::Backlink { idx, node } = focus else {
            return focus;
        };
        match self.backlinks_count_for_current() {
            0 => Focus::Outline,
            rows => {
                let idx = idx.min(rows - 1);
                // A descendant the row's subtree no longer holds (it was
                // deleted, or outdented out of it) falls back to the
                // citing block rather than naming a stranger.
                let node = node.filter(|n| {
                    self.backlinks_for_current()
                        .get(idx)
                        .is_some_and(|bl| self.backlink_row_stops(bl).contains(&Some(*n)))
                });
                Focus::Backlink { idx, node }
            }
        }
    }

    /// Report a withheld projection — **classified**, the same split
    /// `actions::trash` makes, because the two cases want opposite
    /// surfaces. An invariant-8 refusal means the source page has
    /// stopped converging and the user has to hear it; an I/O condition
    /// is retryable and this crate's rule for those is a `tracing::warn!`
    /// and nothing else.
    ///
    /// The page on screen already has a surface for it —
    /// `load_current`'s persistent banner — so only a page the user is
    /// *not* looking at is toasted here.
    fn report_backlink_projection_failure(&mut self, slug: &str, page: NodeId, e: ActionError) {
        match ProjectionRefusal::from_error(&e) {
            ProjectionRefusal::Retryable(message) => {
                tracing::warn!("backlink edit: re-projection failed: {message}");
            }
            ProjectionRefusal::AheadOfLog { lines, sample, .. } => {
                if self.page_is_open(page) {
                    return;
                }
                let notice = outl_actions::refusal::ahead_of_log_notice(
                    outl_actions::refusal::Surface::Tui,
                    lines,
                    &sample,
                );
                self.toast(
                    ToastKind::Warning,
                    format!("`{slug}`: {}", notice.one_line()),
                );
            }
        }
    }
}

/// This crate's [`CommitHooks`] for a cross-page block mutation.
///
/// `CaptureCommit`'s shape (`actions/lifecycle/capture.rs`) with owned
/// fields, so nothing borrows `self` while `commit_page` holds
/// `&mut self.workspace`. `records_undo` stays `false` — see
/// [`App::commit_backlink_mutation`].
struct BacklinkCommit {
    root: PathBuf,
    hlc: HlcGenerator,
    transport: Option<Arc<dyn SyncTransport>>,
    /// Set when the commit changed what some page's backlinks are.
    backlinks_stale: bool,
    /// What the projection wrote, for the `WorkspaceIndex` patch the
    /// caller owes. `None` when the projection was refused, because then
    /// the `.md` on disk is *not* this render and patching the index from
    /// it would make the index disagree with the file.
    projected: Option<(PathBuf, ParsedPage)>,
    projection_failure: Option<ActionError>,
}

impl CommitHooks for BacklinkCommit {
    fn project(&mut self, workspace: &Workspace, page: NodeId) {
        // Guarded: the mutation must project, but projecting must not
        // delete content no op produced (invariant 8). The edit is in
        // the op log either way, which is why this only records.
        match outl_actions::apply_page_md_with_sidecar_guarded(workspace, &self.root, page) {
            // Re-rendered rather than read back: `render_page_md` walks
            // the tree the write just used, so the index patch costs no
            // disk I/O at all. The guarded writer returns the path it
            // took the lock on, which is the path to patch under.
            Ok(path) => {
                let md = outl_actions::render_page_md(workspace, page);
                self.projected = Some((path, outl_md::parse(&md)));
            }
            Err(e) => self.projection_failure = Some(e),
        }
    }

    fn invalidate_backlinks(&mut self) {
        // A flag, not the reindex: the index lives on `App`, which this
        // hook cannot borrow while `commit_page` holds the workspace.
        self.backlinks_stale = true;
    }

    fn announce(&mut self, workspace: &Workspace, page: NodeId) {
        let Some(transport) = &self.transport else {
            return;
        };
        if let Some(meta) = outl_actions::page_meta(workspace, page) {
            transport.announce_local_ops(&meta.slug, self.hlc.next());
        }
    }
}

#[cfg(test)]
mod ahead_of_log_tests;
#[cfg(test)]
mod liveness_tests;
#[cfg(test)]
mod tests;
