//! Structural ops over outline blocks: create / indent / outdent /
//! delete / reorder.
//!
//! Each entry point has two branches: when `Focus::Outline`, mutates
//! `app.page` directly and calls `save()` or trusts the next commit;
//! when `Focus::Backlink { .. }`, the citing block's `NodeId` goes to
//! the matching `outl_actions` function through
//! [`App::commit_backlink_mutation`], so the op log carries the change
//! and the source page's `.md` is projected from it.
//!
//! One pair per op: `create_after` / `create_before`, `indent` /
//! `outdent`, `delete`, `move_up` / `move_down`. Nothing here renders
//! another page's `.md`.
//!
//! **Two things the branches own that the commit helper does not.**
//! Landing the in-flight Insert buffer, because `commit_backlink_mutation`
//! drains the AST save queue and knows nothing about a buffer; and
//! asking whether the op has anywhere to go, because the helper
//! snapshots unconditionally on success and a boundary no-op must not
//! bury the open page's history under a foreign entry.

use super::backlink_gesture::BacklinkGesture;
use crate::actions::block::insert::InsertCursor;
use crate::edit_buffer::EditBuffer;
use crate::outline_ops::{
    delete_at_path, descendants_count_at_path, flat_count, indent_at_path, index_for_path,
    insert_sibling_after_with_text, insert_sibling_before, outdent_at_path, path_for_index,
    siblings_mut,
};
use crate::state::{App, EditTarget, Focus, Mode};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

/// The in-flight buffer's caret column, or `None` outside Insert.
///
/// The cross-page structural ops re-enter Insert after the mutation and
/// the text comes back out of the op log, so the column is the only
/// thing they have to carry over themselves.
fn caret_column(app: &App) -> Option<usize> {
    match &app.mode {
        Mode::Insert { buffer, .. } => Some(buffer.cursor),
        _ => None,
    }
}

/// A cross-page text edit is still in flight.
fn source_edit_in_flight(app: &App) -> bool {
    matches!(
        &app.mode,
        Mode::Insert {
            target: EditTarget::SourcePage { .. },
            ..
        }
    )
}

/// The cross-page route applies: the cursor is on a backlink row, or a
/// cross-page edit is in flight after focus went back to the outline.
///
/// Three branches ask it in this compound form and four ask about
/// `Focus` alone. That difference predates the move onto the op log and
/// is left exactly as it was: `focused_backlink_target` requires
/// `Focus::Backlink`, so the in-flight-only case declines on its own —
/// which is where the `.md`-first helper declined it too, at its own
/// focus match, before anything ran.
fn on_a_backlink_or_mid_source_edit(app: &App) -> bool {
    matches!(app.focus, Focus::Backlink { .. }) || source_edit_in_flight(app)
}

/// Where `node` sits among its siblings: `(index, sibling count)`, or
/// `None` when the tree does not hold it.
///
/// One read answers every boundary question these branches ask.
/// `outl_actions::next_sibling` is public and its `previous_sibling`
/// twin is `pub(crate)`, so taking the sibling list directly keeps both
/// halves coming from the same place.
fn sibling_slot(workspace: &Workspace, node: NodeId) -> Option<(usize, usize)> {
    let parent = workspace.tree().parent(node)?;
    let siblings = outl_actions::children_of(workspace, parent);
    let at = siblings.iter().position(|(id, _)| *id == node)?;
    Some((at, siblings.len()))
}

/// `node` has a previous sibling — so `indent` and `move_up` have
/// somewhere to go.
fn has_previous_sibling(workspace: &Workspace, node: NodeId) -> bool {
    sibling_slot(workspace, node).is_some_and(|(at, _)| at > 0)
}

/// `node` has a next sibling — so `move_down` has somewhere to go.
fn has_next_sibling(workspace: &Workspace, node: NodeId) -> bool {
    sibling_slot(workspace, node).is_some_and(|(at, count)| at + 1 < count)
}

/// `node` sits under a block rather than directly under `page`, so
/// `outdent` has a grandparent to promote it to.
///
/// `outl_actions::outdent` answers `AlreadyAtRoot` otherwise, which is
/// correct and unusable here: it is an `Err`, so nothing is committed
/// and no history is touched, but the sentence it carries names a
/// `NodeId` — and no outl surface shows a user a block id. Asking first
/// keeps a top-level citing block the same silent no-op the outline
/// path is.
fn can_outdent(workspace: &Workspace, node: NodeId, page: NodeId) -> bool {
    matches!(
        workspace.tree().parent(node),
        Some(parent) if parent != page && parent != NodeId::root()
    )
}

/// Which side of the citing block `o` / `O` put the new sibling on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Side {
    Above,
    Below,
}

/// Which way `Tab` / `Shift+Tab` re-parent the citing block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reparent {
    Indent,
    Outdent,
}

/// Re-enter Insert on the row that names `block`, caret at `col`.
///
/// **Guarded on `Focus`, like the hand-written re-entry this replaced.**
/// `enter_insert` dispatches on it, so entering with the cursor back in
/// the outline edits `App.page` through `EditTarget::CurrentPage` — a
/// keystroke aimed at one page landing on another, and then rendered
/// over the file by the next `persist`. The `||` in each caller's guard
/// also admits `Focus::Outline` with a `SourcePage` edit in flight, and
/// the old code declined that case the same way.
///
/// **And guarded on the block, by id.** `Focus` alone was not enough:
/// the in-flight `commit_insert` this caller runs first re-indexes the
/// source page and `clamp_backlink_focus` pulls `idx` back inside the
/// new list, so by the time the caret comes back `idx` can name a
/// different citing block — on a different page. `enter_insert`
/// re-resolves the row through `backlinks_for_current()[idx]`, so a
/// re-entry that trusted the index put every following keystroke, and
/// the `Op::Edit` that `Esc` commits, onto text the user never aimed
/// at. That is the failure `create_beside_a_backlink` documents,
/// reappearing one function over on the other side of the clamp.
///
/// A row the panel no longer holds — the edit dropped the mention that
/// put it there — declines, which is the same silence `Focus::Outline`
/// already got: the buffer is committed either way, and there is no row
/// for `Focus::Backlink { idx }` to name.
fn reenter_backlink_insert(app: &mut App, col: usize, block: NodeId) {
    if !matches!(app.focus, Focus::Backlink { .. }) {
        return;
    }
    let Some(focus) = app.backlink_focus_of(block) else {
        return;
    };
    app.focus = focus;
    app.cursor_col = col;
    app.enter_insert(InsertCursor::AtCursor);
}

impl App {
    /// Land an in-flight cross-page buffer before a structural op runs.
    ///
    /// The `.md`-first helper this replaced did it inline, and the
    /// reason survives the move: the op goes through
    /// [`Self::commit_backlink_mutation`], which drains the AST save
    /// queue but knows nothing about an Insert buffer. Without it,
    /// whatever was typed into the row before `Tab` is dropped.
    fn commit_in_flight_source_edit(&mut self) {
        if source_edit_in_flight(self) {
            self.commit_insert();
        }
    }

    /// Create a sibling of the citing block on its own page.
    ///
    /// **Insert is deliberately not re-entered.** `o` / `O` land the
    /// caret in the new block everywhere else, and here they cannot:
    /// the cursor in this panel is `Focus::Backlink { idx }`, which
    /// names a *backlink row*, and a brand-new sibling cites nothing —
    /// so it is not a row and the focus model has nothing to point at.
    /// The re-entry this replaced used that focus anyway and put the
    /// caret back on the **citing** block, where the next keystroke
    /// edited text the user had not aimed at.
    ///
    /// A declared limit beats a wrong target, so the block is created
    /// and the status line names the page it landed on. Nor is the new
    /// block invisible-and-unmentioned: it is not in the panel (it
    /// cites nothing), and that sentence is the only thing that says
    /// where to find it.
    ///
    /// **A sibling made inside a shown subtree also finishes**: after the
    /// refetch the new block is a stop of its row (`backlink_focus_of`),
    /// so the cursor moves onto it and Insert opens there. Only a
    /// sibling of the citing block itself has no row.
    ///
    /// **The other case that *can* finish is a self-citing page**, where
    /// the commit's reload has already put the new block in `App.page`.
    /// There `o` does what it does anywhere else: the cursor leaves the
    /// panel for the outline and Insert opens on the new block, which is
    /// the same thing the desktop does. Declining it would have told a
    /// user looking straight at the block that it is "not editable from
    /// the panel".
    fn create_beside_a_backlink(&mut self, side: Side) {
        let Some(target) = self.focused_backlink_target_or_report() else {
            return;
        };
        let page = target.page;
        self.commit_in_flight_source_edit();
        let mut created = None;
        let landed =
            self.commit_backlink_mutation(target, BacklinkGesture::Structural, |ws, hlc, node| {
                let block = match side {
                    Side::Below => outl_actions::create_after(ws, hlc, node, None),
                    Side::Above => outl_actions::create_before(ws, hlc, node, None),
                }?;
                created = Some(block);
                Ok(())
            });
        if !landed {
            return;
        }
        // `id_by_flat` is rebuilt by the reload `commit_backlink_mutation`
        // runs for an open source page, so a hit here means the block is
        // on screen and has a path `enter_insert` can use.
        if let Some(row) = created.and_then(|block| self.outline_row_of(block)) {
            self.focus = Focus::Outline;
            self.selected = row;
            self.cursor_col = 0;
            self.enter_insert(InsertCursor::Start);
            return;
        }
        // A sibling made inside a shown subtree has a row in the panel
        // after the refetch, so Insert opens on it there. A sibling of
        // the citing block itself cites nothing and has none.
        if let Some(focus) = created.and_then(|block| self.backlink_focus_of(block)) {
            self.focus = focus;
            self.cursor_col = 0;
            self.enter_insert(InsertCursor::Start);
            return;
        }
        let title = outl_actions::page_meta(&self.workspace, page)
            .map(|meta| meta.title)
            .unwrap_or_else(|| "the source page".to_string());
        self.status = format!("new block on `{title}` · not editable from the panel");
    }

    /// `block`'s flat row in the page on screen, or `None` when it is
    /// not on that page at all.
    fn outline_row_of(&self, block: NodeId) -> Option<usize> {
        self.id_by_flat.iter().position(|id| *id == block)
    }

    /// Indent or outdent the citing block on its own page.
    ///
    /// `true` when the block moved. The two share every step: the caret
    /// is read before anything commits (landing the buffer leaves
    /// Normal mode behind, and the column is all the re-entry has to
    /// carry — the text comes back out of the op log), the boundary is
    /// asked of the tree before the commit, and Insert is re-entered
    /// afterwards **on the block this chord resolved**, not on whatever
    /// row the cursor's index names by then.
    fn reparent_a_backlink(&mut self, how: Reparent) -> bool {
        let caret = caret_column(self);
        let Some(target) = self.focused_backlink_target_or_report() else {
            return false;
        };
        // Kept across the commit, which consumes `target`: the re-entry
        // needs the id, and taking it from the cursor afterwards is the
        // defect the re-entry's doc describes.
        let aimed_at = target.block;
        self.commit_in_flight_source_edit();
        let somewhere_to_go = match how {
            Reparent::Indent => has_previous_sibling(&self.workspace, target.block),
            Reparent::Outdent => can_outdent(&self.workspace, target.block, target.page),
        };
        let moved = somewhere_to_go
            && self.commit_backlink_mutation(
                target,
                BacklinkGesture::Structural,
                match how {
                    Reparent::Indent => outl_actions::indent,
                    Reparent::Outdent => outl_actions::outdent,
                },
            );
        // Re-entered whether or not the block moved, which is what the
        // `.md`-first route did: the buffer has already been committed,
        // so leaving Normal mode here would strand a user mid-edit on a
        // boundary `Tab`. On the block the chord was aimed at, never on
        // whichever row `idx` names by then — see the re-entry's doc.
        if let Some(caret) = caret {
            reenter_backlink_insert(self, caret, aimed_at);
        }
        moved
    }

    /// Create a new block after the current selection at the same indent,
    /// then enter Insert. Used by `o` (Normal) and Enter (Insert).
    pub(crate) fn create_block_below(&mut self) {
        // Cross-page route: focus on a backlink row, or an in-flight
        // Insert on a source page. The sibling is created on that page
        // through the op log; see `create_beside_a_backlink` for why
        // Insert is not re-entered there.
        if on_a_backlink_or_mid_source_edit(self) {
            self.create_beside_a_backlink(Side::Below);
            return;
        }

        let path = if let Mode::Insert { block_path, .. } = &self.mode {
            block_path.clone()
        } else {
            path_for_index(&self.page.blocks, self.selected).unwrap_or_else(|| vec![0])
        };
        self.snapshot_for_undo();

        // Split the in-flight buffer at the cursor when we're editing: the
        // head stays in the current block, the tail seeds the new sibling
        // (issue #184). Cursor at end → empty tail (the plain "new block
        // below"); cursor at start → empty head, the text rides down onto
        // the sibling (open an empty block above). When invoked from Normal
        // (`o`, no live buffer) there's nothing to split — the sibling is
        // empty, exactly as before.
        let in_insert = matches!(self.mode, Mode::Insert { .. });
        let tail = if let Mode::Insert { buffer, .. } = &mut self.mode {
            // `take`/`skip` saturate, so a cursor at end yields an empty tail.
            let full = buffer.as_string();
            let head: String = full.chars().take(buffer.cursor).collect();
            let tail: String = full.chars().skip(buffer.cursor).collect();
            // Truncate the buffer to the head so `commit_insert` writes only
            // the head back to the current block.
            *buffer = EditBuffer::from_text(&head);
            tail
        } else {
            String::new()
        };

        insert_sibling_after_with_text(&mut self.page.blocks, &path, tail);
        // New selection = the newly inserted block (right after the previous one in DFS).
        self.flat_len = flat_count(&self.page.blocks);
        let new_idx = self.selected + 1 + descendants_count_at_path(&self.page.blocks, &path);
        self.selected = new_idx.min(self.flat_len.saturating_sub(1));
        // If we were in Insert, commit the old buffer (now the head) first,
        // then re-enter on the new block with the caret at its start — the
        // tail begins there.
        if in_insert {
            self.commit_insert();
        }
        self.enter_insert(InsertCursor::Start);
    }

    /// Create a new block above the current selection, then enter Insert.
    pub(crate) fn create_block_above(&mut self) {
        if matches!(self.focus, Focus::Backlink { .. }) {
            self.create_beside_a_backlink(Side::Above);
            return;
        }
        let path = path_for_index(&self.page.blocks, self.selected).unwrap_or_else(|| vec![0]);
        self.snapshot_for_undo();
        insert_sibling_before(&mut self.page.blocks, &path);
        self.flat_len = flat_count(&self.page.blocks);
        // Selection stays at `self.selected` — the new block now occupies it.
        self.enter_insert(InsertCursor::AtCursor);
    }

    pub(crate) fn indent_current(&mut self) -> bool {
        if on_a_backlink_or_mid_source_edit(self) {
            return self.reparent_a_backlink(Reparent::Indent);
        }
        let path = if let Mode::Insert { block_path, .. } = &self.mode {
            block_path.clone()
        } else if let Some(p) = path_for_index(&self.page.blocks, self.selected) {
            p
        } else {
            return false;
        };
        // Snapshot before we know if the op succeeds — cheap to take,
        // and we only ever discard it on a literal no-op.
        let snapshot_idx = self.undo.len();
        self.snapshot_for_undo();
        if let Some(new_path) = indent_at_path(&mut self.page.blocks, &path) {
            self.flat_len = flat_count(&self.page.blocks);
            if let Mode::Insert { block_path, .. } = &mut self.mode {
                *block_path = new_path.clone();
            }
            self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
            true
        } else {
            // Roll back the optimistic snapshot.
            if self.undo.len() > snapshot_idx {
                self.undo.truncate(snapshot_idx);
            }
            false
        }
    }

    pub(crate) fn outdent_current(&mut self) -> bool {
        if on_a_backlink_or_mid_source_edit(self) {
            return self.reparent_a_backlink(Reparent::Outdent);
        }
        let path = if let Mode::Insert { block_path, .. } = &self.mode {
            block_path.clone()
        } else if let Some(p) = path_for_index(&self.page.blocks, self.selected) {
            p
        } else {
            return false;
        };
        let snapshot_idx = self.undo.len();
        self.snapshot_for_undo();
        if let Some(new_path) = outdent_at_path(&mut self.page.blocks, &path) {
            self.flat_len = flat_count(&self.page.blocks);
            if let Mode::Insert { block_path, .. } = &mut self.mode {
                *block_path = new_path.clone();
            }
            self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
            true
        } else {
            if self.undo.len() > snapshot_idx {
                self.undo.truncate(snapshot_idx);
            }
            false
        }
    }

    pub(crate) fn delete_current(&mut self) {
        if matches!(self.focus, Focus::Backlink { .. }) {
            // Normal first, deliberately: the block is on its way to
            // the trash, so an in-flight buffer aimed at it is dropped
            // rather than committed onto it. That is what this branch
            // has always done.
            self.mode = Mode::Normal;
            if let Some(target) = self.focused_backlink_target_or_report() {
                // `Move(node, TRASH_ROOT)`, per root invariant 6 — the
                // op stays in the log and `/trash` can put it back.
                self.commit_backlink_mutation(
                    target,
                    BacklinkGesture::Structural,
                    outl_actions::delete,
                );
            }
            return;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        self.snapshot_for_undo();
        delete_at_path(&mut self.page.blocks, &path);
        self.save(); // also handles refilling an empty page with a single bullet
    }

    /// Swap the current block with its previous sibling (drags its
    /// subtree). No-op if already at the top of its parent.
    pub(crate) fn move_block_up(&mut self) {
        if matches!(self.focus, Focus::Backlink { .. }) {
            let Some(target) = self.focused_backlink_target_or_report() else {
                return;
            };
            // `move_up` reports "already the first sibling" as `Ok(())`,
            // and `commit_backlink_mutation` snapshots unconditionally
            // on success — so committing a boundary no-op would clear
            // redo and bury the displayed page's history under a
            // foreign entry
            // (`a_boundary_no_op_leaves_both_history_stacks_unchanged`).
            if !has_previous_sibling(&self.workspace, target.block) {
                return;
            }
            self.commit_backlink_mutation(
                target,
                BacklinkGesture::Structural,
                outl_actions::move_up,
            );
            return;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        let Some(&last_idx) = path.last() else {
            return;
        };
        if last_idx == 0 {
            return;
        }
        let snapshot_idx = self.undo.len();
        self.snapshot_for_undo();
        let parent_path = &path[..path.len() - 1];
        let Some(siblings) = siblings_mut(&mut self.page.blocks, parent_path) else {
            self.undo.truncate(snapshot_idx);
            return;
        };
        siblings.swap(last_idx - 1, last_idx);
        let mut new_path = parent_path.to_vec();
        new_path.push(last_idx - 1);
        self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
        self.save();
    }

    /// Swap with the next sibling.
    pub(crate) fn move_block_down(&mut self) {
        if matches!(self.focus, Focus::Backlink { .. }) {
            let Some(target) = self.focused_backlink_target_or_report() else {
                return;
            };
            // Same boundary reasoning as `move_block_up`.
            if !has_next_sibling(&self.workspace, target.block) {
                return;
            }
            self.commit_backlink_mutation(
                target,
                BacklinkGesture::Structural,
                outl_actions::move_down,
            );
            return;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        let Some(&last_idx) = path.last() else {
            return;
        };
        let parent_path = path[..path.len() - 1].to_vec();
        let snapshot_idx = self.undo.len();
        self.snapshot_for_undo();
        let Some(siblings) = siblings_mut(&mut self.page.blocks, &parent_path) else {
            self.undo.truncate(snapshot_idx);
            return;
        };
        let sibling_count = siblings.len();
        if last_idx + 1 >= sibling_count {
            self.undo.truncate(snapshot_idx);
            return;
        }
        siblings.swap(last_idx, last_idx + 1);
        let mut new_path = parent_path;
        new_path.push(last_idx + 1);
        self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
        self.save();
    }
}

#[cfg(test)]
mod tests;
