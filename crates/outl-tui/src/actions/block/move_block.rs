//! Move the selected block among its siblings.
//!
//! Two branches, like the other structural ops: with `Focus::Outline`
//! the in-flight AST is rotated and the save is coalesced; with
//! `Focus::Backlink` the citing block's `NodeId` goes to
//! `outl_actions::move_up` / `move_down` through
//! [`App::commit_backlink_mutation`].
//!
//! [`App::rotate_run`] is the one place the outline half touches the
//! AST, so a range move can reuse it. It owns three things a bare
//! `rotate_siblings` does not: the zoom guard, the undo snapshot (taken
//! only once the move is known to be legal, because
//! [`App::snapshot_for_undo`] clears redo and a boundary press must not),
//! and keeping `id_by_flat` aligned. `id_by_flat` is indexed by the same
//! DFS position as the AST and is only rebuilt by `load` and `persist`,
//! so a move that left it alone made `c` fold the block that used to sit
//! under the cursor.

use super::backlink_gesture::BacklinkGesture;
use super::structural::{
    has_next_sibling, has_previous_sibling, on_a_backlink_or_mid_source_edit, BacklinkMove,
};
use crate::actions::block::InsertCursor;
use crate::outline_ops::{
    can_rotate, flat_count, index_for_path, node_at_path, path_for_index, rotate_siblings,
    Direction,
};
use crate::state::{App, Focus, Mode};

impl App {
    /// Swap the current block with its previous sibling (drags its
    /// subtree). No-op if already at the top of its parent.
    pub(crate) fn move_block_up(&mut self) {
        self.move_block(Direction::Up);
    }

    /// Swap with the next sibling.
    pub(crate) fn move_block_down(&mut self) {
        self.move_block(Direction::Down);
    }

    fn move_block(&mut self, dir: Direction) {
        if matches!(self.focus, Focus::Backlink { .. }) {
            self.move_backlink_block(dir);
            return;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        self.move_one(&path, dir);
    }

    /// Rotate the block at `path` one sibling step, follow it with the
    /// selection and queue the save. No-op at an edge or under a zoom guard.
    fn move_one(&mut self, path: &[usize], dir: Direction) {
        let Some((&idx, parent)) = path.split_last() else {
            return;
        };
        if !self.rotate_run(parent, idx, idx, dir) {
            return;
        }
        let mut moved = parent.to_vec();
        moved.push(match dir {
            Direction::Up => idx - 1,
            Direction::Down => idx + 1,
        });
        self.selected = index_for_path(&self.page.blocks, &moved).unwrap_or(self.selected);
        self.save();
    }

    fn move_backlink_block(&mut self, dir: Direction) {
        let Some(target) = self.focused_backlink_target_or_report() else {
            return;
        };
        // `move_up` reports "already the first sibling" as `Ok(())`, and
        // `commit_backlink_mutation` snapshots unconditionally on success,
        // so committing a boundary no-op would clear redo and bury the
        // displayed page's history under a foreign entry
        // (`a_boundary_no_op_leaves_both_history_stacks_unchanged`).
        match dir {
            Direction::Up if has_previous_sibling(&self.workspace, target.block) => {
                self.commit_backlink_mutation(
                    target,
                    BacklinkGesture::Structural,
                    outl_actions::move_up,
                );
            }
            Direction::Down if has_next_sibling(&self.workspace, target.block) => {
                self.commit_backlink_mutation(
                    target,
                    BacklinkGesture::Structural,
                    outl_actions::move_down,
                );
            }
            _ => {}
        }
    }

    /// Move the block being edited without leaving Insert.
    ///
    /// The order below is the design. The draft is committed first, as an
    /// edit of its own, because the TUI re-matches ids by content and a
    /// swap plus a text change in one persist can lose the block's
    /// identity: an edited block gets a flush on each side of the commit
    /// (the first persists an earlier swap, the second the text alone).
    /// With no edit neither flush runs, so a held chord never writes
    /// inline. A deferred peer reload is held back for the whole gesture
    /// and handed back on every way out, because `commit_insert` would
    /// drain it and reload the page under the caret.
    pub(crate) fn move_block_in_insert(&mut self, dir: Direction) {
        let held = std::mem::take(&mut self.pending_reload);
        self.move_edited_block(dir);
        self.pending_reload |= held;
    }

    fn move_edited_block(&mut self, dir: Direction) {
        if on_a_backlink_or_mid_source_edit(self) {
            self.reparent_a_backlink(match dir {
                Direction::Up => BacklinkMove::MoveUp,
                Direction::Down => BacklinkMove::MoveDown,
            });
            return;
        }
        let Mode::Insert {
            block_path,
            buffer,
            original_text,
            ..
        } = &self.mode
        else {
            return;
        };
        let (block_path, caret) = (block_path.clone(), buffer.cursor);
        let text = buffer.as_string();
        let edited = text != *original_text;
        if let Some(row) = index_for_path(&self.page.blocks, &block_path) {
            self.selected = row;
        }
        if edited {
            self.flush_pending_save();
        }
        self.commit_insert();
        if edited {
            self.flush_pending_save();
        }
        // A flush can reparse the page (auto-run blocks), so the path may
        // no longer name the edited block. Moving then would aim at a
        // neighbour: stay in Normal instead.
        if node_at_path(&self.page.blocks, &block_path).map(|n| n.text.as_str())
            != Some(text.as_str())
        {
            return;
        }
        self.move_one(&block_path, dir);
        self.cursor_col = caret;
        self.enter_insert(InsertCursor::AtCursor);
    }

    /// Rotate the sibling run `first..=last` under `parent` one step.
    ///
    /// Returns `false`, having touched nothing, at an edge, for a stale
    /// path, and when the zoom hides the move: a zoom root is drawn as
    /// the page's only block, so moving it would reorder something the
    /// user cannot see land. A stale zoom path counts as not zoomed.
    ///
    /// The caller owns `selected` and `save()`.
    pub(crate) fn rotate_run(
        &mut self,
        parent: &[usize],
        first: usize,
        last: usize,
        dir: Direction,
    ) -> bool {
        if let (Some(root), Some(_)) = (self.zoom_stack.last(), self.zoom_root_node()) {
            if !parent.starts_with(root) {
                return false;
            }
        }
        if !can_rotate(&self.page.blocks, parent, first, last, dir) {
            return false;
        }
        let ids_aligned = self.id_by_flat.len() == flat_count(&self.page.blocks);
        self.snapshot_for_undo();
        let Some(rotated) = rotate_siblings(&mut self.page.blocks, parent, first, last, dir) else {
            // `can_rotate` said yes a line ago and nothing ran between.
            return false;
        };
        if ids_aligned {
            // The window both orders occupy starts at the lower of the two
            // sibling indices, and rotating it left by the size of whatever
            // was in front turns "before" into "after".
            let mut at = parent.to_vec();
            at.push(first.min(rotated.first));
            let lead = match dir {
                Direction::Up => rotated.displaced,
                Direction::Down => rotated.run,
            };
            let len = rotated.run + rotated.displaced;
            if let Some(window) = index_for_path(&self.page.blocks, &at)
                .and_then(|start| self.id_by_flat.get_mut(start..start + len))
            {
                window.rotate_left(lead);
            }
        }
        // `hidden_by_collapse` is derived from `id_by_flat` + the AST, so
        // a folded neighbour only stays folded if this runs after both
        // have moved.
        self.recompute_hidden_by_collapse();
        true
    }
}

#[cfg(test)]
mod tests;
