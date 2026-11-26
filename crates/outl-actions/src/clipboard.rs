//! Serialize a selection of blocks to clean outl markdown for the clipboard.
//!
//! This is the **inverse** of [`crate::paste::paste_markdown`] (which is
//! itself a thin wrapper over [`outl_md::parse::parse`]): copy a subtree
//! out, paste it back into outl, and the same tree is reconstructed. The
//! two ship as a pair so the round-trip is testable (`markdown-roundtrip-tester`).
//!
//! Every client wraps [`copy_markdown`]; nobody re-implements outline
//! serialization in client code. The TUI writes the result to the OS
//! clipboard on `yy` / `Y` / visual `y` (in addition to its in-memory
//! yank register); the desktop and mobile clients call it from their
//! copy handlers via a shared Tauri command.
//!
//! The core only ever produces the **canonical** outl markdown. Other
//! output formats (plain text, org-mode, HTML, …) are the domain of
//! optional format plugins, not this module — see
//! `docs/design/clipboard.md`.

use std::collections::HashSet;

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::parse::OutlineNode;

use crate::journal::render::{project_blocks, render_fragment};

/// Serialize `roots` — and their full subtrees — to clean outl markdown
/// suitable for the OS clipboard.
///
/// Each root becomes a top-level `- ` bullet; descendants follow at two
/// spaces of indent per level. Block properties (`key:: value`) ride
/// inline under their block, alphabetically sorted so the output is
/// stable across runs. `TODO ` / `DONE ` and `> ` quote markers are part
/// of the block's text and round-trip verbatim.
///
/// `roots` is taken in the caller's order — clients pass blocks in
/// document order (a single yank, a visual range top-to-bottom). An empty
/// slice yields an empty string.
///
/// The blocks are projected exactly as [`crate::render_block_md`]
/// projects one — same walk, same property rule — so a block copied
/// here and the same block copied through the single-block command are
/// byte-identical. That rule keeps `Text`, `PageRef` and `Tag` values
/// and drops `List`, which has no `.md` syntax. A selected **page** root
/// leaves the page model's book-keeping keys behind (`page-slug`,
/// `page-kind`, …), as a page's `.md` header does.
///
/// The whole selection is one property pass, however many roots it has:
/// see `journal::render::project_blocks`.
pub fn copy_markdown(workspace: &Workspace, roots: &[NodeId]) -> String {
    // Drop any id whose ancestor is also in the selection: a parent
    // already carries that descendant inside its subtree, so keeping the
    // descendant as its own root would emit it twice. A Visual range that
    // spans a parent and its child is the common trigger. Order of the
    // surviving roots is preserved.
    let selected: HashSet<NodeId> = roots.iter().copied().collect();
    let kept: Vec<NodeId> = roots
        .iter()
        .copied()
        // Skip ids no longer in the tree (a stale/re-minted selection a
        // client still holds): the projection would otherwise emit an
        // empty `- ` bullet from `block_text(..).unwrap_or_default()`,
        // corrupting the clipboard with a blank line.
        .filter(|&id| workspace.tree().contains(id))
        .filter(|&id| !has_selected_ancestor(workspace, id, &selected))
        .collect();
    render_fragment(project_blocks(workspace, &kept))
}

/// Walk `node`'s ancestors; return `true` if any is in `selected`.
fn has_selected_ancestor(workspace: &Workspace, node: NodeId, selected: &HashSet<NodeId>) -> bool {
    let mut cursor = workspace.tree().parent(node);
    while let Some(parent) = cursor {
        if selected.contains(&parent) {
            return true;
        }
        cursor = workspace.tree().parent(parent);
    }
    false
}

/// Serialize already-projected outline nodes (each with its subtree) to
/// clean outl markdown.
///
/// This is the AST-first entry point: the TUI edits an in-memory
/// `Vec<OutlineNode>` and its yank register already holds the exact nodes
/// to copy, so it serializes them directly without round-tripping through
/// the workspace. [`copy_markdown`] is the workspace-first wrapper the GUI
/// backends call (they hold `NodeId`s, not nodes).
///
/// The caller owns ordering and is responsible for passing nodes whose
/// `text` is the raw block body (TODO/quote prefixes included) — i.e. what
/// [`outl_md::parse::parse`] produced — so the output round-trips.
pub fn copy_markdown_nodes(nodes: &[OutlineNode]) -> String {
    render_fragment(nodes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::paste::{paste_markdown, PasteAnchor};
    use crate::tree::children_of;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    fn roots_under(workspace: &Workspace, parent: NodeId) -> Vec<NodeId> {
        children_of(workspace, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    }

    #[test]
    fn empty_selection_is_empty_string() {
        let (workspace, _hlc) = ws();
        assert_eq!(copy_markdown(&workspace, &[]), "");
    }

    #[test]
    fn single_block() {
        let (mut workspace, hlc) = ws();
        let b = append_block(&mut workspace, &hlc, None, Some("hello")).unwrap();
        assert_eq!(copy_markdown(&workspace, &[b]), "- hello\n");
    }

    #[test]
    fn block_with_subtree() {
        let (mut workspace, hlc) = ws();
        let parent = append_block(&mut workspace, &hlc, None, Some("parent")).unwrap();
        let _child = append_block(&mut workspace, &hlc, Some(parent), Some("child")).unwrap();
        assert_eq!(
            copy_markdown(&workspace, &[parent]),
            "- parent\n  - child\n"
        );
    }

    #[test]
    fn multiple_roots_in_caller_order() {
        let (mut workspace, hlc) = ws();
        let a = append_block(&mut workspace, &hlc, None, Some("a")).unwrap();
        let b = append_block(&mut workspace, &hlc, None, Some("b")).unwrap();
        assert_eq!(copy_markdown(&workspace, &[a, b]), "- a\n- b\n");
    }

    #[test]
    fn todo_prefix_rides_along_verbatim() {
        let (mut workspace, hlc) = ws();
        let t = append_block(&mut workspace, &hlc, None, Some("TODO buy milk")).unwrap();
        assert_eq!(copy_markdown(&workspace, &[t]), "- TODO buy milk\n");
    }

    #[test]
    fn quote_prefix_rides_along_verbatim() {
        let (mut workspace, hlc) = ws();
        let q = append_block(&mut workspace, &hlc, None, Some("> a wise thing")).unwrap();
        assert_eq!(copy_markdown(&workspace, &[q]), "- > a wise thing\n");
    }

    /// The whole point: copy-out then paste-in reconstructs the same
    /// tree, properties and all. Build a tree via `paste_markdown`,
    /// `copy_markdown` it back, and the markdown must re-parse to an
    /// identical AST.
    #[test]
    fn roundtrips_through_paste_with_properties() {
        let (mut workspace, hlc) = ws();
        let host = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();
        let src = "- objective\n  priority:: high\n  - sub one\n  - sub two\n";
        paste_markdown(&mut workspace, &hlc, PasteAnchor::AsLastChildOf(host), src).unwrap();

        let copied = copy_markdown(&workspace, &roots_under(&workspace, host));

        // copy-out is the inverse of the parse paste went through:
        // re-parsing the copy yields the same AST as parsing the source.
        assert_eq!(outl_md::parse::parse(&copied), outl_md::parse::parse(src));
    }

    /// The copy↔paste round-trip must hold for the text-prefix markers
    /// (`TODO`/`DONE`/`> ` and their composition), not just plain blocks +
    /// properties — these are exactly the prefixes recent work touched.
    #[test]
    fn roundtrips_todo_done_and_quote_markers() {
        let (mut workspace, hlc) = ws();
        let host = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();
        let src = "- TODO open task\n- DONE closed task\n- > a quote\n- TODO > quoted task\n";
        paste_markdown(&mut workspace, &hlc, PasteAnchor::AsLastChildOf(host), src).unwrap();

        let copied = copy_markdown(&workspace, &roots_under(&workspace, host));
        assert_eq!(outl_md::parse::parse(&copied), outl_md::parse::parse(src));
    }

    #[test]
    fn range_spanning_parent_and_child_does_not_duplicate() {
        // A Visual range that grabs both a parent and one of its
        // children must not emit the child twice (once inside the
        // parent's subtree, once as its own root).
        let (mut workspace, hlc) = ws();
        let parent = append_block(&mut workspace, &hlc, None, Some("parent")).unwrap();
        let child = append_block(&mut workspace, &hlc, Some(parent), Some("child")).unwrap();
        // Selection passes both ids, parent first (document order).
        let out = copy_markdown(&workspace, &[parent, child]);
        assert_eq!(out, "- parent\n  - child\n");
    }

    #[test]
    fn properties_are_alphabetically_sorted_and_stable() {
        let (mut workspace, hlc) = ws();
        let host = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();
        // Properties supplied out of alpha order in the source.
        let src = "- task\n  zeta:: 1\n  alpha:: 2\n";
        paste_markdown(&mut workspace, &hlc, PasteAnchor::AsLastChildOf(host), src).unwrap();

        let copied = copy_markdown(&workspace, &roots_under(&workspace, host));
        assert_eq!(copied, "- task\n  alpha:: 2\n  zeta:: 1\n");
    }

    #[test]
    fn stale_ids_not_in_tree_are_skipped_not_blank_bullets() {
        // A client can hold a selection id that no longer resolves (peer
        // reload / re-mint). It must be dropped, not serialized as an
        // empty `- ` bullet.
        let (mut workspace, hlc) = ws();
        let live = append_block(&mut workspace, &hlc, None, Some("live")).unwrap();
        let ghost = NodeId::new(); // never created → not in the tree
        assert_eq!(copy_markdown(&workspace, &[ghost]), "");
        assert_eq!(copy_markdown(&workspace, &[live, ghost]), "- live\n");
    }
}
