//! Structural ops over outline blocks: create / indent / outdent /
//! delete / reorder.
//!
//! Each entry point has two branches: when `Focus::Outline`, mutates
//! `app.page` directly and calls `save()` or trusts the next
//! commit; when `Focus::Backlink { .. }`, delegates to
//! `apply_to_backlink_source` so the change lands in the source
//! page's `.md` instead.

use crate::actions::block::insert::InsertCursor;
use crate::edit_buffer::EditBuffer;
use crate::outline_ops::{
    delete_at_path, descendants_count_at_path, flat_count, indent_at_path, index_for_path,
    insert_sibling_after, insert_sibling_after_with_text, insert_sibling_before, node_at_path,
    outdent_at_path, path_for_index, siblings_mut,
};
use crate::state::{App, EditTarget, Focus, Mode};

impl App {
    /// Create a new block after the current selection at the same indent,
    /// then enter Insert. Used by `o` (Normal) and Enter (Insert).
    pub(crate) fn create_block_below(&mut self) {
        // Cross-page route (focus on a backlink, or in-place Insert on
        // a source page). The op mutates the source AST, refreshes the
        // index optimistically, and re-enters Insert on the new block.
        if matches!(self.focus, Focus::Backlink { .. })
            || matches!(
                &self.mode,
                Mode::Insert {
                    target: EditTarget::SourcePage { .. },
                    ..
                }
            )
        {
            // Don't drop `Mode::Insert` before calling the helper —
            // it inspects the live mode to commit the user's in-flight
            // buffer into the source AST. Resetting first would
            // discard whatever was typed into the prior block.
            let result = self.apply_to_backlink_source(|page, abs_path| {
                insert_sibling_after(&mut page.blocks, abs_path);
                // Sibling-after lives at parent ++ [last + 1].
                if let Some(last) = abs_path.last_mut() {
                    *last += 1;
                }
            });
            if let Some((source_path, source_page)) = result {
                // Enter Insert on the freshly inserted block.
                let Focus::Backlink { idx, sub_path } = &self.focus else {
                    return;
                };
                let backlinks = self.backlinks_for_current();
                let Some(bl) = backlinks.get(*idx) else {
                    return;
                };
                let mut abs = bl.source_block_path.clone();
                abs.extend_from_slice(sub_path);
                if let Some(node) = node_at_path(&source_page.blocks, &abs) {
                    let buf = EditBuffer::from_text(&node.text);
                    let original_text = node.text.clone();
                    self.cursor_col = 0;
                    self.mode = Mode::Insert {
                        target: EditTarget::SourcePage {
                            path: source_path,
                            page: source_page,
                        },
                        block_path: abs,
                        buffer: buf,
                        original_text,
                    };
                }
            }
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
            // Keep `Mode::Insert` alive: `apply_to_backlink_source`
            // needs it to commit the in-flight buffer before the
            // sibling-before is inserted.
            let result = self.apply_to_backlink_source(|page, abs_path| {
                insert_sibling_before(&mut page.blocks, abs_path);
                // The new block now occupies the slot the focused
                // one used to — focus stays at `abs_path` unchanged
                // (the closure does nothing to it), which means we
                // end up inserting *and* selecting the new block.
            });
            if let Some((source_path, source_page)) = result {
                let Focus::Backlink { idx, sub_path } = &self.focus else {
                    return;
                };
                let backlinks = self.backlinks_for_current();
                let Some(bl) = backlinks.get(*idx) else {
                    return;
                };
                let mut abs = bl.source_block_path.clone();
                abs.extend_from_slice(sub_path);
                if let Some(node) = node_at_path(&source_page.blocks, &abs) {
                    let buf = EditBuffer::from_text(&node.text);
                    let original_text = node.text.clone();
                    self.cursor_col = 0;
                    self.mode = Mode::Insert {
                        target: EditTarget::SourcePage {
                            path: source_path,
                            page: source_page,
                        },
                        block_path: abs,
                        buffer: buf,
                        original_text,
                    };
                }
            }
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
        if matches!(self.focus, Focus::Backlink { .. })
            || matches!(
                &self.mode,
                Mode::Insert {
                    target: EditTarget::SourcePage { .. },
                    ..
                }
            )
        {
            let in_insert = matches!(self.mode, Mode::Insert { .. });
            let buf_state = if let Mode::Insert { buffer, .. } = &self.mode {
                Some(buffer.clone())
            } else {
                None
            };
            // Don't reset Mode::Insert before the helper — it needs
            // to see the live buffer to commit it into the source AST
            // before the indent. (Resetting first would save a
            // structural change against stale text.)
            let mut moved = false;
            let result = self.apply_to_backlink_source(|page, abs_path| {
                if let Some(new_path) = indent_at_path(&mut page.blocks, abs_path) {
                    *abs_path = new_path;
                    moved = true;
                }
            });
            // Re-enter Insert if we were editing, on the (possibly
            // re-pathed) block.
            if let (Some(buf), Some((source_path, source_page))) = (buf_state, result) {
                if in_insert {
                    let Focus::Backlink { idx, sub_path } = &self.focus else {
                        return moved;
                    };
                    let backlinks = self.backlinks_for_current();
                    let Some(bl) = backlinks.get(*idx) else {
                        return moved;
                    };
                    let mut abs = bl.source_block_path.clone();
                    abs.extend_from_slice(sub_path);
                    if let Some(node) = node_at_path(&source_page.blocks, &abs) {
                        let original_text = node.text.clone();
                        self.mode = Mode::Insert {
                            target: EditTarget::SourcePage {
                                path: source_path,
                                page: source_page,
                            },
                            block_path: abs,
                            buffer: buf,
                            original_text,
                        };
                    }
                }
            }
            return moved;
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
        if matches!(self.focus, Focus::Backlink { .. })
            || matches!(
                &self.mode,
                Mode::Insert {
                    target: EditTarget::SourcePage { .. },
                    ..
                }
            )
        {
            let in_insert = matches!(self.mode, Mode::Insert { .. });
            let buf_state = if let Mode::Insert { buffer, .. } = &self.mode {
                Some(buffer.clone())
            } else {
                None
            };
            // Same rationale as `indent_current`: keep Mode::Insert
            // so the helper commits the buffer before the outdent.
            let mut moved = false;
            let result = self.apply_to_backlink_source(|page, abs_path| {
                if let Some(new_path) = outdent_at_path(&mut page.blocks, abs_path) {
                    *abs_path = new_path;
                    moved = true;
                }
            });
            if let (Some(buf), Some((source_path, source_page))) = (buf_state, result) {
                if in_insert {
                    let Focus::Backlink { idx, sub_path } = &self.focus else {
                        return moved;
                    };
                    let backlinks = self.backlinks_for_current();
                    let Some(bl) = backlinks.get(*idx) else {
                        return moved;
                    };
                    let mut abs = bl.source_block_path.clone();
                    abs.extend_from_slice(sub_path);
                    if let Some(node) = node_at_path(&source_page.blocks, &abs) {
                        let original_text = node.text.clone();
                        self.mode = Mode::Insert {
                            target: EditTarget::SourcePage {
                                path: source_path,
                                page: source_page,
                            },
                            block_path: abs,
                            buffer: buf,
                            original_text,
                        };
                    }
                }
            }
            return moved;
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
            self.mode = Mode::Normal;
            self.apply_to_backlink_source(|page, abs_path| {
                delete_at_path(&mut page.blocks, abs_path);
                // After delete, the focused path no longer resolves;
                // collapse to source_block_path (handled outside via
                // the clamp to `[]`).
                abs_path.clear();
            });
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
            self.apply_to_backlink_source(|page, abs_path| {
                let Some(&last) = abs_path.last() else {
                    return;
                };
                if last == 0 {
                    return;
                }
                let parent = &abs_path[..abs_path.len() - 1];
                let Some(siblings) = siblings_mut(&mut page.blocks, parent) else {
                    return;
                };
                siblings.swap(last - 1, last);
                *abs_path.last_mut().unwrap() = last - 1;
            });
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
        self.snapshot_for_undo();
        let parent_path = &path[..path.len() - 1];
        let Some(siblings) = siblings_mut(&mut self.page.blocks, parent_path) else {
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
            self.apply_to_backlink_source(|page, abs_path| {
                let Some(&last) = abs_path.last() else {
                    return;
                };
                let parent = abs_path[..abs_path.len() - 1].to_vec();
                let Some(sibling_count) =
                    siblings_mut(&mut page.blocks, &parent).map(|siblings| siblings.len())
                else {
                    return;
                };
                if last + 1 >= sibling_count {
                    return;
                }
                let Some(siblings) = siblings_mut(&mut page.blocks, &parent) else {
                    return;
                };
                siblings.swap(last, last + 1);
                *abs_path.last_mut().unwrap() = last + 1;
            });
            return;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return;
        };
        let Some(&last_idx) = path.last() else {
            return;
        };
        let parent_path = path[..path.len() - 1].to_vec();
        let Some(sibling_count) =
            siblings_mut(&mut self.page.blocks, &parent_path).map(|siblings| siblings.len())
        else {
            return;
        };
        if last_idx + 1 >= sibling_count {
            return;
        }
        self.snapshot_for_undo();
        let Some(siblings) = siblings_mut(&mut self.page.blocks, &parent_path) else {
            return;
        };
        siblings.swap(last_idx, last_idx + 1);
        let mut new_path = parent_path;
        new_path.push(last_idx + 1);
        self.selected = index_for_path(&self.page.blocks, &new_path).unwrap_or(self.selected);
        self.save();
    }
}
