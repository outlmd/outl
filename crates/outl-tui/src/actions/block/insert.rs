//! Enter / commit / abort Insert mode.
//!
//! Two entry paths: outline edits land on `app.page` via
//! `EditTarget::CurrentPage`, backlink edits name the citing block by
//! id via `EditTarget::SourcePage`. Commit fans back out into either
//! `save()` (current page) or `commit_backlink_mutation` (source page,
//! one op on the op log). Abort throws away the in-flight buffer.

use super::backlink_gesture::BacklinkGesture;
use super::backlink_target::BacklinkTarget;
use crate::edit_buffer::EditBuffer;
use crate::outline_ops::{node_at_path, node_at_path_mut};
use crate::state::{App, EditTarget, Focus, Mode};
use outl_md::parse::OutlineNode;

/// Where to drop the Insert-mode cursor when entering Insert from
/// Normal. Mirrors vim's three entry points: `I` (start of line),
/// `i` (at cursor), `a` (one char after cursor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InsertCursor {
    /// `I` — jump to the start of the block.
    Start,
    /// `i` / `Enter` — land at `cursor_col`.
    AtCursor,
    /// `a` — land one char past `cursor_col` (append after the char
    /// under the cursor). Clamps at end of buffer.
    AfterCursor,
}

impl App {
    /// Enter Insert mode at the currently focused block.
    ///
    /// Dispatches by `Focus`: outline blocks edit `App.page`; backlink
    /// blocks load the source page from disk and edit it in place via
    /// `EditTarget::SourcePage`. The `pos` argument controls where the
    /// Insert cursor lands (`I` / `i` / `a` semantics).
    pub(crate) fn enter_insert(&mut self, pos: InsertCursor) {
        if let Focus::Backlink { .. } = self.focus.clone() {
            self.enter_insert_backlink(pos);
            return;
        }
        self.enter_insert_outline(pos);
    }

    /// Insert path for the current page. Mutations land on `app.page`
    /// and commit through the usual `save()`.
    fn enter_insert_outline(&mut self, pos: InsertCursor) {
        if self.flat_len == 0 {
            // No blocks yet — create one and start editing.
            self.page.blocks.push(OutlineNode::default());
            self.flat_len = 1;
            self.selected = 0;
        }
        let Some(path) = crate::outline_ops::path_for_index(&self.page.blocks, self.selected)
        else {
            return;
        };
        let Some(node) = node_at_path(&self.page.blocks, &path) else {
            return;
        };
        let mut buf = EditBuffer::from_text(&node.text);
        buf.cursor = insert_cursor_for(pos, self.cursor_col, buf.chars.len());
        self.mode = Mode::Insert {
            target: EditTarget::CurrentPage,
            block_path: path,
            buffer: buf,
            original_text: node.text.clone(),
        };
    }

    /// Insert path for a backlink block. Resolves the citing block's
    /// id, seeds the buffer from the **op log** and stashes the pair in
    /// `EditTarget::SourcePage` so `commit_insert` can key the op on it.
    ///
    /// No `.md` is read here. The old version parsed the source page off
    /// disk and edited that AST, which handed the buffer any bytes
    /// sitting beside the sidecar — and then wrote them back as the
    /// user's own text. The op log is the source of truth (invariant 1),
    /// so it is what the buffer starts from.
    fn enter_insert_backlink(&mut self, pos: InsertCursor) {
        let Some(target) = self.focused_backlink_target() else {
            return;
        };
        // Whether the block is live is asked here, before the edit
        // starts — and through `outl_actions::is_live`, the one owner of
        // that question, so this gate, the write gate in
        // `commit_backlink_mutation` and the backend's own
        // `block::ensure_live` cannot ask it differently. This crate
        // used to spell the pair out inline here and nothing at all over
        // there, which is how the structural chords ended up acting on
        // trashed blocks.
        //
        // **`block_text` alone is not that question.** It answers from
        // the content cache or the Yrs doc and neither consults the
        // tree, so the text survives a delete — the record just moves
        // under the trash — and a block a peer deleted passed this gate.
        // The user typed into it and `Esc` then refused the commit with
        // the keystrokes gone.
        //
        // Refusing at the entry is what keeps the two halves of the
        // refusal catalog honest, too: nothing has been typed yet, so
        // the `unchanged` sentence is the true one. The "your text could
        // not be saved" half belongs to a surface with no entry gate.
        let live_text = self
            .workspace
            .block_text(target.block)
            .filter(|_| outl_actions::is_live(&self.workspace, target.block));
        let Some(original_text) = live_text else {
            self.status = outl_actions::refusal::stale_backlink_block_unchanged_refusal(
                &self.workspace,
                target.block,
            )
            .to_string();
            return;
        };
        let mut buf = EditBuffer::from_text(&original_text);
        buf.cursor = insert_cursor_for(pos, self.cursor_col, buf.chars.len());
        self.mode = Mode::Insert {
            target: EditTarget::SourcePage {
                page: target.page,
                block: target.block,
            },
            // Empty on purpose: the id is the cursor here, and an empty
            // path matches no block in `App.page` (`path_for_index`
            // always returns at least one index). `view::outline`'s
            // "is this row being edited" check reads `block_path`
            // without consulting the target — it is gated on
            // `Focus::Outline` instead, which is the stronger guard
            // while focus and target agree. The empty path is what
            // holds when they *disagree*, which is reachable: the
            // self-citing reload used to leave `Focus::Outline` with a
            // `SourcePage` edit live, and `[0]` would have painted the
            // outline's first block as the one under the caret.
            block_path: Vec::new(),
            buffer: buf,
            original_text,
        };
    }

    /// Commit Insert: write buffer back into the AST and persist.
    ///
    /// Routes by `EditTarget`:
    /// - `CurrentPage`: mutate `app.page`, snapshot undo, `save()`.
    /// - `SourcePage`: one `Op::Edit` on the citing block through
    ///   [`App::commit_backlink_mutation`], which owns the flush, the
    ///   undo snapshot under the *source* page's path, the projection
    ///   and the announce.
    ///
    /// **The buffer is consumed on the first line**, which is what
    /// makes the `SourcePage` arm the one place in the panel where a
    /// refusal can cost the user something that exists nowhere else. So
    /// it carries [`BacklinkGesture::TypedText`]: the commit asks the
    /// source page no ahead-of-log question for it (the buffer was
    /// seeded from the op log, so the logged text *is* what was
    /// edited), and any refusal it does reach says the text could not
    /// be saved instead of describing a file.
    pub(crate) fn commit_insert(&mut self) {
        if let Mode::Insert {
            target,
            block_path,
            buffer,
            original_text,
        } = std::mem::replace(&mut self.mode, Mode::Normal)
        {
            let new_text = buffer.as_string();
            match target {
                EditTarget::CurrentPage => {
                    if new_text != original_text {
                        // Only snapshot when the user actually changed
                        // something — pressing Esc without typing
                        // should not pollute history.
                        self.snapshot_for_undo();
                        let is_call = outl_actions::parse_call_invocation(&new_text).is_some();
                        if let Some(node) = node_at_path_mut(&mut self.page.blocks, &block_path) {
                            node.text = new_text;
                        }
                        self.save();
                        // Finishing an edit on a `call:<name>` block
                        // re-runs it so the `> **result:**` reflects the
                        // freshly-typed params. The re-run reads the op
                        // log, so persist the coalesced edit first —
                        // otherwise it would run against a stale tree.
                        if is_call {
                            self.flush_pending_save();
                            self.rerun_call_block_at(&block_path);
                        }
                    } else if let Some(node) = node_at_path_mut(&mut self.page.blocks, &block_path)
                    {
                        // Empty round-trip; restore exactly to be safe.
                        node.text = original_text;
                    }
                }
                EditTarget::SourcePage { page, block } => {
                    if new_text != original_text {
                        // One op on the citing block, through the
                        // shared commit pipeline. Nothing here renders
                        // the source page's `.md`: `commit_backlink_mutation`
                        // projects it *from* the op log afterwards, and
                        // withholds that projection when the file holds
                        // lines no op produced (invariant 8) — which is
                        // the right half to withhold. Withholding the
                        // *op* instead is what dropped `new_text` with
                        // nothing on screen to say so.
                        self.commit_backlink_mutation(
                            BacklinkTarget { page, block },
                            BacklinkGesture::TypedText,
                            |ws, hlc, node| outl_actions::edit_text(ws, hlc, node, &new_text),
                        );
                    }
                    // Nothing to restore on a no-op — this route never
                    // touched `App.page`.
                }
            }
        }
        // If the peer-ops poller fired while we were inside Insert
        // mode, we held the reload back to avoid clobbering the
        // in-flight buffer. Now that the edit landed (or was a
        // no-op), it's safe to fold peer ops in.
        if std::mem::take(&mut self.pending_reload) {
            self.reload_workspace_from_disk();
        }
    }

    /// Abort Insert: throw away the buffer, leave AST unchanged.
    pub(crate) fn abort_insert(&mut self) {
        let Mode::Insert {
            target,
            block_path,
            original_text,
            ..
        } = std::mem::replace(&mut self.mode, Mode::Normal)
        else {
            return;
        };
        // SourcePage: nothing to restore anywhere. That route never
        // mutated `app.page`, and it carries two ids rather than a
        // working copy, so dropping the buffer *is* the abort — no op
        // was emitted for it.
        if let (EditTarget::CurrentPage, Some(node)) =
            (target, node_at_path_mut(&mut self.page.blocks, &block_path))
        {
            node.text = original_text;
        }
    }
}

/// Map the Normal-mode `cursor_col` into the Insert-mode buffer
/// cursor, applying vim semantics. Clamps `AfterCursor` at `buf_len`
/// so `a` at end-of-line is a no-op (cursor stays where `i` would
/// land).
fn insert_cursor_for(pos: InsertCursor, cursor_col: usize, buf_len: usize) -> usize {
    match pos {
        InsertCursor::Start => 0,
        InsertCursor::AtCursor => cursor_col.min(buf_len),
        InsertCursor::AfterCursor => cursor_col.saturating_add(1).min(buf_len),
    }
}

#[cfg(test)]
mod tests {
    use super::{insert_cursor_for, InsertCursor};

    #[test]
    fn start_is_always_zero() {
        assert_eq!(insert_cursor_for(InsertCursor::Start, 5, 10), 0);
        assert_eq!(insert_cursor_for(InsertCursor::Start, 0, 0), 0);
    }

    #[test]
    fn at_cursor_clamps_to_buf_len() {
        assert_eq!(insert_cursor_for(InsertCursor::AtCursor, 3, 10), 3);
        assert_eq!(insert_cursor_for(InsertCursor::AtCursor, 20, 10), 10);
        assert_eq!(insert_cursor_for(InsertCursor::AtCursor, 0, 0), 0);
    }

    #[test]
    fn after_cursor_advances_one_then_clamps() {
        // mid-buffer: lands one char to the right
        assert_eq!(insert_cursor_for(InsertCursor::AfterCursor, 3, 10), 4);
        // last char (cursor at N-1 of N): lands at end
        assert_eq!(insert_cursor_for(InsertCursor::AfterCursor, 9, 10), 10);
        // already at end: stays at end
        assert_eq!(insert_cursor_for(InsertCursor::AfterCursor, 10, 10), 10);
        // empty buffer: stays at 0
        assert_eq!(insert_cursor_for(InsertCursor::AfterCursor, 0, 0), 0);
    }
}
