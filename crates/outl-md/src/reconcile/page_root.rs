//! The page node itself, as the filesystem describes it.
//!
//! Everything a pass has to say about the page *before* it says
//! anything about the blocks inside it: which id the page has (derived
//! from its filename, never minted), whether it is rooted under
//! [`NodeId::root`], and the three page-level properties the file
//! dictates — `page-slug`, `page-kind` and the frontmatter fence under
//! `page-frontmatter`.
//!
//! One concept, because all three answers come from the same source
//! (the path and the file's own head) and all three ride the op log for
//! the same reason: page-level state that must converge between devices
//! goes through an `Op` (invariant 7).

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::LogOp;
use outl_core::workspace::{Workspace, WorkspaceError};
use std::path::Path;

/// The page slug for `md_path` — the filename without extension.
///
/// `to_string_lossy` keeps the slug non-empty even when the filename is
/// not valid UTF-8 (replaces invalid sequences with U+FFFD). This is the
/// same slug `ensure_page_root_in_tree` writes into the `page-slug`
/// property, so seeding the page-root id from it here keeps the id and
/// the slug property in agreement.
fn slug_from_md_path(md_path: &Path) -> String {
    md_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Deterministic page-root [`NodeId`] for `md_path`, derived from its
/// slug via [`NodeId::from_slug`] — the single owner of the derivation
/// shared with `outl_actions::page::page_id_from_slug`.
pub(super) fn page_id_from_stem(md_path: &Path) -> NodeId {
    NodeId::from_slug(&slug_from_md_path(md_path))
}

/// Guarantee the page node `page_id` is rooted in the workspace tree
/// as a child of `NodeId::root` with the `page-slug` / `page-kind`
/// properties set, deriving the slug from the filename and the kind
/// from the parent directory (`pages/` vs `journals/`).
///
/// Returns the number of ops applied (0–3). Idempotent: each op is
/// emitted only when the workspace state disagrees with what the
/// filesystem says the page should look like.
///
/// Why this lives in `outl-md` and inlines the key constants:
/// `page-slug` / `page-kind` are owned by `outl-actions::page` but
/// `outl-md` cannot depend on it (layering). The pair of strings
/// stays inlined — if either side ever renames, both call-sites need
/// to update together (the diff.rs's `PAGE_SLUG_KEY` skip-list and
/// here). Keep these in sync with `outl_actions::page::{SLUG_KEY,
/// KIND_KEY}`.
///
/// Public because `outl-actions::desync` (the projection-ahead-of-log
/// recovery) needs the exact same "materialise the page root" step
/// without going through the full `reconcile_md` pipeline — a second
/// implementation is how the two paths would drift.
pub fn ensure_page_root_in_tree(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    page_id: NodeId,
    md_path: &Path,
) -> Result<usize, WorkspaceError> {
    const PAGE_SLUG_KEY: &str = "page-slug";
    const PAGE_KIND_KEY: &str = "page-kind";

    // Shared slug derivation (see `slug_from_md_path`): the same value
    // that seeds the page-root id in `reconcile_md`'s no-sidecar arm, so
    // the id and this `page-slug` property never disagree.
    let slug = slug_from_md_path(md_path);
    let kind_value = if md_path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        == Some("journals")
    {
        "journal"
    } else {
        "page"
    };

    let mut applied = 0usize;

    // Pick a fractional position that lands **after** the last
    // existing child of `NodeId::root`. `Fractional::between(None, None)`
    // always returns the midpoint (`"m"`), so every externally-authored
    // page handled here would collide on the same key; iterators over
    // `children_of(root)` would then see nondeterministic ordering
    // (ties come from `HashMap` iteration, since fractional positions
    // are equal).
    let position_after_last_root_child = || {
        let max = ws
            .tree()
            .iter_nodes()
            .filter(|(_, parent, _)| *parent == NodeId::root())
            .map(|(_, _, pos)| pos.clone())
            .max();
        outl_core::fractional::Fractional::between(max.as_ref(), None)
    };

    // **Materialise the page node in the tree.**
    //
    // `Op::Move` on a node that has never been `Op::Create`d is a
    // **no-op** inside `tree::do_op` (see `outl-core/src/tree/op.rs`,
    // the `None` arm of the `match self.nodes.get(node)`). Pages
    // authored externally (`vim` writing `pages/samara.md` directly)
    // never receive a Create through any pipeline — `reconcile_md`
    // only emits Create for the blocks inside the page, never for the
    // page node itself. So emitting only `Op::Move` here would
    // silently fail, the page would never appear under
    // `children_of(root)`, and `search_persons` / `list_all_pages`
    // would skip it forever. That was the bug behind "samara has
    // `type:: person` in the .md, the op log has the SetProp, the
    // sidecar carries the current `pipeline_version`, but the
    // desktop autocomplete still doesn't see it".
    //
    // Three cases:
    //   - node absent from `self.nodes` (parent == None) → Create at root.
    //   - node present but parented somewhere other than root → Move to root.
    //   - node already at root → no-op.
    let current_parent = ws.tree().parent(page_id);
    if current_parent.is_none() {
        // Fresh page: emit `Op::Create` so the node lands in
        // `self.nodes` with the correct parent. Subsequent block
        // `Op::Create` ops (whose parent is `page_id`) and
        // `Op::SetProp` ops were already idempotent against
        // non-existent nodes for `SetProp`, but `Move` was the
        // failure mode that masked this bug for months.
        let position = position_after_last_root_child();
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: outl_core::op::Op::Create {
                node: page_id,
                parent: NodeId::root(),
                position,
            },
        })?;
        applied += 1;
    } else if let Some(old_parent) = current_parent.filter(|p| *p != NodeId::root()) {
        // Node exists somewhere else in the tree (rare: shouldn't
        // happen for orphan reconcile, but covers the case where a
        // page node migrates from being a block descendant — defensive).
        let old_position = ws
            .tree()
            .position(page_id)
            .cloned()
            .unwrap_or_else(outl_core::fractional::Fractional::first);
        let position = position_after_last_root_child();
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: outl_core::op::Op::Move {
                node: page_id,
                new_parent: NodeId::root(),
                position,
                old_parent,
                old_position,
            },
        })?;
        applied += 1;
    }
    // `page-slug` property: must equal the filename stem.
    let want_slug = outl_core::property::PropValue::Text(slug.clone());
    if ws.tree().property(page_id, PAGE_SLUG_KEY) != Some(&want_slug) {
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: outl_core::op::Op::SetProp {
                node: page_id,
                key: PAGE_SLUG_KEY.to_string(),
                value: Some(want_slug),
                old_value: None,
            },
        })?;
        applied += 1;
    }
    // `page-kind` property: `page` or `journal` based on the directory.
    let want_kind = outl_core::property::PropValue::Text(kind_value.to_string());
    if ws.tree().property(page_id, PAGE_KIND_KEY) != Some(&want_kind) {
        let ts = hlc.next();
        ws.apply(LogOp {
            ts,
            actor: ts.actor,
            op: outl_core::op::Op::SetProp {
                node: page_id,
                key: PAGE_KIND_KEY.to_string(),
                value: Some(want_kind),
                old_value: None,
            },
        })?;
        applied += 1;
    }
    Ok(applied)
}

/// **The YAML frontmatter fence, into the op log.**
///
/// Outl does not model YAML, but the file it lives in is the user's and
/// a workspace folder that is also an Obsidian vault is the interop
/// story `transport = "file"` exists for. `parse` keeps the fence out of
/// the outline (it used to become bullets, one of them `- ---`, issue
/// #281); this is the other half. Page-level state that must converge
/// between devices goes through an `Op` — invariant 7 — and leaving the
/// fence on disk only would hand invariant 8 a choice between freezing
/// the page and deleting the fence on the first projection.
///
/// One comparison covers set, update **and** clear: `want` is `None`
/// when the user deleted the fence, and the renderer materialises this
/// property into file syntax, so a stale one would grow the fence back
/// on the next projection.
pub(super) fn sync_page_frontmatter(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    page_id: NodeId,
    frontmatter: Option<&str>,
) -> Result<usize, WorkspaceError> {
    let want_fm = frontmatter.map(|yaml| outl_core::property::PropValue::Text(yaml.to_string()));
    if ws
        .tree()
        .property(page_id, crate::frontmatter::PAGE_FRONTMATTER_KEY)
        == want_fm.as_ref()
    {
        return Ok(0);
    }
    let ts = hlc.next();
    ws.apply(LogOp {
        ts,
        actor: ts.actor,
        op: outl_core::op::Op::SetProp {
            node: page_id,
            key: crate::frontmatter::PAGE_FRONTMATTER_KEY.to_string(),
            value: want_fm,
            old_value: None,
        },
    })?;
    Ok(1)
}
