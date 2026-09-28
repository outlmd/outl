//! The second pass over a reconciled page: block **text**.
//!
//! `diff_to_ops` only knows tree structure, so without this the op log
//! carries a page whose blocks exist and are empty. Locally that looks
//! fine (the sidecar holds the content hash); a peer replaying the log
//! materialises empty blocks and ships the empty `.md` back. Kept in
//! its own file because it is a distinct pass with a distinct failure
//! mode, not a helper of the diff.

use crate::parse::OutlineNode;
use crate::sidecar::SidecarBlock;
use outl_core::hlc::HlcGenerator;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::{Workspace, WorkspaceError};

/// Walk the parsed AST and the freshly built sidecar block list in
/// lockstep (both in DFS preorder) and emit one `Op::Edit` per block
/// whose text doesn't already match what's in the workspace.
///
/// Returns the number of `Op::Edit` ops applied. Idempotent: skips
/// blocks whose text already matches (the Yrs delta would be empty).
pub(super) fn sync_block_text(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    ast_blocks: &[OutlineNode],
    sidecar_blocks: &[SidecarBlock],
) -> Result<usize, WorkspaceError> {
    let mut idx = 0usize;
    let mut applied = 0usize;
    walk_text_sync(ws, hlc, ast_blocks, sidecar_blocks, &mut idx, &mut applied)?;
    Ok(applied)
}

fn walk_text_sync(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    ast_blocks: &[OutlineNode],
    sidecar_blocks: &[SidecarBlock],
    idx: &mut usize,
    applied: &mut usize,
) -> Result<(), WorkspaceError> {
    for block in ast_blocks {
        if let Some(entry) = sidecar_blocks.get(*idx) {
            let node = entry.id;
            let current = ws.block_text(node).unwrap_or_default();
            if current != block.text {
                let update = ws.build_text_replace_update(node, &block.text);
                if !update.is_empty() {
                    let ts = hlc.next();
                    ws.apply(LogOp {
                        ts,
                        actor: ts.actor,
                        op: Op::Edit {
                            node,
                            text_op: update,
                        },
                    })?;
                    *applied += 1;
                }
            }
        }
        *idx += 1;
        walk_text_sync(ws, hlc, &block.children, sidecar_blocks, idx, applied)?;
    }
    Ok(())
}
