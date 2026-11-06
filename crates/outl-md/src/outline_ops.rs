//! Pure AST manipulation helpers used by the TUI's `App`.
//!
//! These operate on the in-memory `Vec<OutlineNode>` (the user's
//! page in flight). They have no I/O, no workspace state, and no
//! ratatui dependency — they're the kind of thing a future Tauri or
//! mobile client would call too, the same way they call
//! `outl_md::tokenize`.
//!
//! All paths are vectors of child indices, DFS-preorder. `path[0]` is
//! the index of the top-level block, `path[1]` the index inside its
//! children, and so on. `flat_index` is a single counter that walks
//! the same DFS preorder, useful for "the user's selection cursor".
//!
//! Every public function in this module is tested in `tests.rs`,
//! split out to keep this file under the file-size guard (same seam
//! as `parse/tests.rs`).

use crate::parse::OutlineNode;

/// Count of all nodes in the (possibly nested) outline.
pub fn flat_count(blocks: &[OutlineNode]) -> usize {
    blocks.iter().map(|b| 1 + flat_count(&b.children)).sum()
}

/// Task counters: `(done, total)`. A block counts when its trimmed
/// text starts with a task marker in either spelling — the canonical
/// word (`TODO `, `DOING `, `DONE `) or the CommonMark checkbox
/// (`[ ] `, `[/] `, `[x] `). Only the done states count toward `done`;
/// all of them count toward `total`.
///
/// `DOING` is deliberately not partial credit — the indicator answers
/// "how much is finished", and a started task is not a finished one.
///
/// Both spellings are counted for the same reason `outl_actions`
/// reads both: a user who typed `- [ ] ship it` sees a checkbox, and a
/// progress chip that skipped those blocks would disagree with what is
/// on screen (issue #230).
///
/// The header chip in the TUI uses this for the `●● 3/7` indicator,
/// so the count walks the whole tree (nested children included).
pub fn count_todos(blocks: &[OutlineNode]) -> (usize, usize) {
    let mut done = 0usize;
    let mut total = 0usize;
    walk_todos(blocks, &mut done, &mut total);
    (done, total)
}

/// Prefixes that mark a task, and whether that task is finished.
/// Mirrors `outl_actions::READ_PREFIXES`, which is the owner — the
/// dependency arrow points the other way, so this crate keeps a copy.
const TASK_PREFIXES: [(&str, bool); 7] = [
    ("TODO ", false),
    ("DOING ", false),
    ("DONE ", true),
    ("[ ] ", false),
    ("[/] ", false),
    ("[x] ", true),
    ("[X] ", true),
];

fn walk_todos(blocks: &[OutlineNode], done: &mut usize, total: &mut usize) {
    for b in blocks {
        let t = b.text.trim_start();
        // The marker may also sit after a single `"> "` quote prefix —
        // the legacy authoring shape (`"> TODO foo"`) that the TUI's
        // `split_block_prefixes` renders as a checkbox. The canonical
        // order (`"TODO > foo"`) already matches marker-first, and only
        // one quote marker is unwrapped ("no nested quotes" policy).
        let t = t.strip_prefix("> ").unwrap_or(t);
        if let Some((_, finished)) = TASK_PREFIXES.iter().find(|(p, _)| t.starts_with(p)) {
            *total += 1;
            if *finished {
                *done += 1;
            }
        }
        walk_todos(&b.children, done, total);
    }
}

/// Return the path of indices to reach the block at `target_index`
/// in DFS preorder. `None` if the index is out of range.
pub fn path_for_index(blocks: &[OutlineNode], target: usize) -> Option<Vec<usize>> {
    let mut cursor = 0;
    walk_path(blocks, target, &mut cursor, &mut Vec::new())
}

fn walk_path(
    blocks: &[OutlineNode],
    target: usize,
    cursor: &mut usize,
    stack: &mut Vec<usize>,
) -> Option<Vec<usize>> {
    for (i, b) in blocks.iter().enumerate() {
        stack.push(i);
        if *cursor == target {
            return Some(stack.clone());
        }
        *cursor += 1;
        if let Some(path) = walk_path(&b.children, target, cursor, stack) {
            return Some(path);
        }
        stack.pop();
    }
    None
}

/// Reverse of [`path_for_index`]: given a path, return the flat index.
pub fn index_for_path(blocks: &[OutlineNode], path: &[usize]) -> Option<usize> {
    let mut cursor = 0;
    walk_index_for_path(blocks, path, 0, &mut cursor)
}

fn walk_index_for_path(
    blocks: &[OutlineNode],
    path: &[usize],
    depth: usize,
    cursor: &mut usize,
) -> Option<usize> {
    if depth >= path.len() {
        return None;
    }
    let target = path[depth];
    for (i, b) in blocks.iter().enumerate() {
        if i == target {
            if depth + 1 == path.len() {
                return Some(*cursor);
            }
            *cursor += 1;
            return walk_index_for_path(&b.children, path, depth + 1, cursor);
        } else {
            *cursor += 1 + flat_count(&b.children);
        }
    }
    None
}

/// Borrow the node at a path. `None` if any segment is out of range.
pub fn node_at_path<'a>(blocks: &'a [OutlineNode], path: &[usize]) -> Option<&'a OutlineNode> {
    let mut current = blocks;
    let mut node: Option<&OutlineNode> = None;
    for &idx in path {
        let n = current.get(idx)?;
        node = Some(n);
        current = &n.children;
    }
    node
}

/// Mutable variant of [`node_at_path`].
pub fn node_at_path_mut<'a>(
    blocks: &'a mut [OutlineNode],
    path: &[usize],
) -> Option<&'a mut OutlineNode> {
    let mut current = blocks;
    for (depth, &idx) in path.iter().enumerate() {
        if depth + 1 == path.len() {
            return current.get_mut(idx);
        }
        current = &mut current.get_mut(idx)?.children;
    }
    None
}

/// Number of descendants directly nested under the node at `path`.
pub fn descendants_count_at_path(blocks: &[OutlineNode], path: &[usize]) -> usize {
    node_at_path(blocks, path)
        .map(|n| flat_count(&n.children))
        .unwrap_or(0)
}

/// Insert a fresh empty block as a sibling immediately *after* `path`.
///
/// If `path` points past the actual sibling list (e.g. caller passed
/// `[0]` against an empty outline because the page had no parseable
/// blocks), the new node is appended at the end instead of panicking.
/// Same fallback when a *parent* segment of `path` is out of range
/// (a stale nested path against a shorter or empty tree) — `siblings_mut`
/// returns `None` and the new node lands at the top level instead.
pub fn insert_sibling_after(blocks: &mut Vec<OutlineNode>, path: &[usize]) {
    if path.is_empty() {
        blocks.push(OutlineNode::default());
        return;
    }
    let (last, parent_path) = path.split_last().unwrap();
    let Some(siblings) = siblings_mut(blocks, parent_path) else {
        blocks.push(OutlineNode::default());
        return;
    };
    let pos = (last + 1).min(siblings.len());
    siblings.insert(pos, OutlineNode::default());
}

/// Insert a new block carrying `text` as a sibling immediately *after*
/// `path`. Same clamp behaviour as [`insert_sibling_after`] (a path past
/// the live sibling list, or a stale parent segment, appends at the end).
///
/// Used by the TUI's block-split (Enter mid-text): the tail of the
/// current block becomes the new sibling's initial text while the head
/// stays behind. The empty-text call is exactly [`insert_sibling_after`],
/// which stays as the common case.
pub fn insert_sibling_after_with_text(blocks: &mut Vec<OutlineNode>, path: &[usize], text: String) {
    let node = OutlineNode {
        text,
        ..OutlineNode::default()
    };
    if path.is_empty() {
        blocks.push(node);
        return;
    }
    let (last, parent_path) = path.split_last().unwrap();
    let Some(siblings) = siblings_mut(blocks, parent_path) else {
        blocks.push(node);
        return;
    };
    let pos = (last + 1).min(siblings.len());
    siblings.insert(pos, node);
}

/// Insert a fresh empty block as a sibling immediately *before* `path`.
///
/// Clamp behavior mirrors [`insert_sibling_after`]: a path that
/// overshoots the live sibling list, or whose parent segment is out of
/// range, falls back to appending so a stale selection cursor never panics.
pub fn insert_sibling_before(blocks: &mut Vec<OutlineNode>, path: &[usize]) {
    if path.is_empty() {
        blocks.insert(0, OutlineNode::default());
        return;
    }
    let (last, parent_path) = path.split_last().unwrap();
    let Some(siblings) = siblings_mut(blocks, parent_path) else {
        blocks.push(OutlineNode::default());
        return;
    };
    let pos = (*last).min(siblings.len());
    siblings.insert(pos, OutlineNode::default());
}

/// Borrow the sibling list of a path (i.e. the parent's children).
///
/// `None` if any segment of `parent_path` is out of range against the
/// live tree — a caller holding a stale path (a parent that moved,
/// got deleted, or was never in range to begin with) must get a
/// chance to fall back instead of indexing past the end and
/// panicking.
pub fn siblings_mut<'a>(
    blocks: &'a mut Vec<OutlineNode>,
    parent_path: &[usize],
) -> Option<&'a mut Vec<OutlineNode>> {
    let mut current = blocks;
    for &idx in parent_path {
        current = &mut current.get_mut(idx)?.children;
    }
    Some(current)
}

/// Indent: become the last child of the previous sibling. Returns the
/// new path of the moved block, or `None` if there is no previous
/// sibling (already at the top of its parent) or a parent segment of
/// `path` is out of range against the live tree.
pub fn indent_at_path(blocks: &mut Vec<OutlineNode>, path: &[usize]) -> Option<Vec<usize>> {
    let (last_idx, parent_path) = path.split_last()?;
    if *last_idx == 0 {
        return None;
    }
    let siblings = siblings_mut(blocks, parent_path)?;
    let node = siblings.remove(*last_idx);
    let prev = &mut siblings[*last_idx - 1];
    let new_idx = prev.children.len();
    prev.children.push(node);
    let mut new_path = parent_path.to_vec();
    new_path.push(*last_idx - 1);
    new_path.push(new_idx);
    Some(new_path)
}

/// Outdent: become the next sibling of the parent. Returns the new
/// path, or `None` if already at the top level or a parent/grandparent
/// segment of `path` is out of range against the live tree.
pub fn outdent_at_path(blocks: &mut Vec<OutlineNode>, path: &[usize]) -> Option<Vec<usize>> {
    if path.len() < 2 {
        return None;
    }
    let (last_idx, parent_path) = path.split_last()?;
    let (parent_idx, grandparent_path) = parent_path.split_last()?;
    let parent_idx = *parent_idx;
    let last_idx = *last_idx;
    let node = {
        let siblings = siblings_mut(blocks, parent_path)?;
        siblings.remove(last_idx)
    };
    let grandparent_siblings = siblings_mut(blocks, grandparent_path)?;
    grandparent_siblings.insert(parent_idx + 1, node);
    let mut new_path = grandparent_path.to_vec();
    new_path.push(parent_idx + 1);
    Some(new_path)
}

/// Delete the node at `path`. Silently no-ops on out-of-range leaf or
/// parent segment, or root.
pub fn delete_at_path(blocks: &mut Vec<OutlineNode>, path: &[usize]) {
    if path.is_empty() {
        return;
    }
    let (last_idx, parent_path) = path.split_last().unwrap();
    let Some(siblings) = siblings_mut(blocks, parent_path) else {
        return;
    };
    if *last_idx < siblings.len() {
        siblings.remove(*last_idx);
    }
}

/// Swap a node with its previous sibling. Returns the new path of the
/// moved node, or `None` if there is no previous sibling or a parent
/// segment of `path` is out of range against the live tree.
pub fn move_up_at_path(blocks: &mut Vec<OutlineNode>, path: &[usize]) -> Option<Vec<usize>> {
    let (last_idx, parent_path) = path.split_last()?;
    if *last_idx == 0 {
        return None;
    }
    let siblings = siblings_mut(blocks, parent_path)?;
    siblings.swap(*last_idx, *last_idx - 1);
    let mut new_path = parent_path.to_vec();
    new_path.push(*last_idx - 1);
    Some(new_path)
}

/// Swap a node with its next sibling. Returns the new path of the
/// moved node, or `None` if there is no next sibling or a parent
/// segment of `path` is out of range against the live tree.
pub fn move_down_at_path(blocks: &mut Vec<OutlineNode>, path: &[usize]) -> Option<Vec<usize>> {
    let (last_idx, parent_path) = path.split_last()?;
    let siblings = siblings_mut(blocks, parent_path)?;
    if *last_idx + 1 >= siblings.len() {
        return None;
    }
    siblings.swap(*last_idx, *last_idx + 1);
    let mut new_path = parent_path.to_vec();
    new_path.push(*last_idx + 1);
    Some(new_path)
}

#[cfg(test)]
mod tests;
