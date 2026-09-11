//! Build the `.outl` sidecar that pairs with a freshly rendered `.md`.
//!
//! Split out of `journal::apply`, which owns the *writing* of a
//! projection (locks, atomic replace, the invariant-8 guards). This
//! module owns only the question "what does the sidecar for these bytes
//! look like", which is a pure function of the tree and the rendered
//! string.

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::{file_hash, Sidecar, SidecarBlock};

use crate::outline::ChildrenIndex;

/// Construct a sidecar that lines up with the `.md` we just rendered
/// from the workspace. Walks the page subtree in DFS preorder — the
/// same order [`super::render::render_page_md`] emits — so every
/// block's index in the walk maps 1:1 to its line in the `.md`.
///
/// **That 1:1 is why the walk order here is not a detail.** It resolves
/// children through the same [`ChildrenIndex`] the renderer uses, built
/// by `crate::tree::children_index`, so the two orders cannot drift: a
/// sidecar row that names a different block than the line above it
/// mis-assigns every id below the divergence on the next reconcile.
pub(super) fn build_sidecar(workspace: &Workspace, page_root: NodeId, md: &str) -> Sidecar {
    let mut blocks: Vec<SidecarBlock> = Vec::new();
    let mut line = 1usize;
    // One map for the page, not one `children_of` rescan of the whole
    // workspace per block written. This runs on every page write,
    // including the one behind a keystroke commit, under the client's
    // workspace lock.
    let children = crate::tree::children_index(workspace, page_root);
    walk_sidecar(workspace, page_root, 0, &mut line, &mut blocks, &children);
    Sidecar {
        // Never a literal: this builder writes whatever fields the
        // current `SidecarBlock` carries, so a hardcoded number labels a
        // v3 payload as v2 the moment the schema moves — and the reader
        // trusts the label.
        version: outl_md::sidecar::SIDECAR_VERSION,
        page_id: page_root,
        last_synced_hash: file_hash(md),
        last_synced_at: chrono::Local::now().fixed_offset(),
        blocks,
        // This builder runs after a workspace-driven render — the
        // workspace tree already holds the page properties, so by
        // construction they're in the op log. Stamp the current
        // pipeline version to keep the orphan scanner from looping
        // on this page.
        pipeline_version: outl_md::sidecar::CURRENT_PIPELINE_VERSION,
    }
}

fn walk_sidecar(
    workspace: &Workspace,
    parent: NodeId,
    indent: u32,
    line: &mut usize,
    out: &mut Vec<SidecarBlock>,
    children: &ChildrenIndex,
) {
    for &id in children.get(&parent).into_iter().flatten() {
        let text = workspace.block_text(id).unwrap_or_default();
        // `from_text` keeps hash, handle and stored text derived from
        // one revision — level-2 matching diffs against that text, so a
        // hand-built literal that drifts would mis-assign ids.
        out.push(SidecarBlock::from_text(id, *line, indent, &text));
        *line += 1;
        walk_sidecar(workspace, id, indent + 1, line, out, children);
    }
}
