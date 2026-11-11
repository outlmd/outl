//! Reopening a page whose root is sitting in the trash.
//!
//! Three facts meet here, and each one is correct on its own:
//!
//! 1. delete is `Move(root, TRASH_ROOT)` and the node keeps its id
//!    (invariant 6 — delete preserves history);
//! 2. a page root's id is derived from its slug
//!    ([`super::page_id_from_slug`]), so the slug always resolves to that
//!    same node;
//! 3. `Op::Create` is idempotent — "if the node already exists, keep its
//!    current parent/position" — which is what makes a `Create` invertible
//!    to *nothing* (`outl-core/CLAUDE.md` invariant 6).
//!
//! Together they meant a deleted page could never be reopened.
//! [`super::find_by_slug`] walks only `children_of(root())`, so it reported
//! the page absent; `open_or_create` emitted `Create` for the deterministic
//! id; `Create` found the node and left it in the trash. The page opened
//! normally, because the view is built from that node, and every keystroke
//! under it was dropped with no error — every morning, on a journal day the
//! user had deleted ([issue #473](https://github.com/outlmd/outl/issues/473)).
//!
//! The TUI recovered by accident: it saves by rendering the `.md` and
//! reconciling it, which emits `Move(page, root)` with a current HLC. That
//! accident is the fix, done on purpose and on the path every client takes.
//!
//! ## The second implementation is `outl_md`'s, not `trash::restore`'s
//!
//! **`outl_md::reconcile::page_root::ensure_page_root_in_tree` revives a
//! trashed page root too**, and has since long before this module existed:
//! it emits `Op::Move(page, root())` for any page root whose parent is not
//! `root()`, and the trash is such a parent. That is the accident the TUI
//! was recovering through, and the reason `outl serve`'s watcher never
//! reproduced #473 either.
//!
//! It moves **only the root**, leaving the children live. This module
//! trashes them. Both are right, because the inputs differ: reconcile has a
//! file in hand with content in it, and that file is evidence of the user's
//! intent the op log does not hold, so it answers *"make the tree match
//! this"*. Here there is no such file — a GUI delete removed the `.md` — so
//! the question is *"the user wants to write here today"*. They visibly
//! disagree only for a `.md` that outlived a delete, which is the
//! peer-delete case.
//!
//! **Routing one through the other is the thing to refuse in review.**
//! Reconcile calling this function would trash blocks whose text is in the
//! `.md` it is reconciling against, and the parse immediately after would
//! recreate them with fresh ids — breaking every `((blk-…))` handle aimed at
//! them and rewriting the sidecar. Two paths beats that.
//! `tests/revive_convergence.rs`'s
//! `the_md_reconcile_revives_a_page_root_too_and_keeps_its_children` is the
//! durable form of this paragraph, because a doc sentence cannot fail.
//!
//! ## `trash::restore` is a different question, and refuses
//!
//! [`crate::trash`] owns putting a deletion back and **refuses** a page
//! root: restoring one needs a re-projected `.md`, and on the reference
//! workspace 16 of 18 deleted pages have their slug taken by a live page, so
//! `trash` would have to pick a free slug and become a second owner of the
//! slug rule.
//!
//! Neither condition holds here. The caller is `open_or_create` — the owner
//! of the slug rule — and it has already established the slug is free, since
//! `find_by_slug` found nothing. So the refusal does not transfer, and
//! wrapping `restore` would mean relaxing a guard that is right for its own
//! caller. It was never the parallel implementation, though: that is the
//! `outl_md` path above.
//!
//! ## Yesterday's blocks stay in the trash
//!
//! The user deleted the page and its blocks rode along in that single
//! `Op::Move`. Reviving the root alone would bring every one of them back
//! live, so "I deleted this page" would mean "hidden until tomorrow
//! morning". Each direct child is therefore moved to the trash in its own
//! op, *before* the root leaves, so the page is never momentarily live with
//! content the user deleted — in the tree or in the op log a peer replays.
//!
//! Hiding is not destroying: invariant 6 keeps every block in the log, each
//! one becomes an ordinary `trash::list` entry, and `trash::restore` puts it
//! back under the revived page with its subtree. That is the asymmetry root
//! `CLAUDE.md` invariant 8 argues from — content *hidden* is recoverable,
//! content *deleted* is not — and it is why the children are trashed rather
//! than resurrected.
//!
//! **One child shape does not come back through `trash::restore`**: a direct
//! child that is itself a **page root**, reachable when one page was
//! re-parented under another via [`crate::block::move_under`] (the shape
//! `tests/revive_convergence.rs`'s `q2b` builds). `trash::landing_for`
//! refuses a page root with `TrashPageRestoreUnsupported`, for its own good
//! reasons — so for those the recovery is not the trash at all: **reopening
//! that nested page by its own slug revives it**, through this same
//! function, because a trashed root keeps its `page-slug`. The content is
//! recoverable either way; the route differs, and the sentence above would
//! be false without saying so.
//!
//! The cost is that one trash entry becomes N. Deliberate: the alternative
//! holders for those children are all new structure, and per-block restore
//! is the more useful shape anyway.
//!
//! ## What this does not touch
//!
//! The `.md`. Projection belongs to the caller's `commit_page` /
//! `reproject_stale_md`, which already own invariant 8's gate; a revive is
//! `Op::Move`s only and never advances `last_synced_hash`, so a `.md`
//! holding lines the log never saw still refuses to be overwritten. Pinned
//! by `tests/revived_page_md.rs`.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::children_of;

/// Bring `slug`'s trashed root back under [`NodeId::root`], leaving the
/// blocks it was deleted with in the trash.
///
/// `Ok(None)` when there is nothing to revive — the ordinary case, and the
/// reason this is cheap enough to sit on the page-open path: two map reads
/// before it can decide anything else.
pub(super) fn revive_if_trashed(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    slug: &str,
) -> Result<Option<NodeId>, ActionError> {
    let node = super::page_id_from_slug(slug);
    if !workspace.tree().contains(node) {
        return Ok(None);
    }
    // `is_trashed` is an ancestor walk, not `parent == trash()`: a page root
    // normally sits directly under the trash, but a root re-parented under
    // some other page that was later deleted is just as unreachable, and
    // just as broken for the user.
    if !crate::tree::is_trashed(workspace, node) {
        return Ok(None);
    }
    // The id alone is not evidence this node is a page for this slug. A
    // trashed page root keeps its `page-slug` property (a `Move` touches
    // nothing else), so a node that does not claim the slug is something
    // else wearing the id — and reviving it would promote a stray block to
    // a page root. Fall through to the `Create` instead: it is idempotent,
    // so the worst case is the state we already had.
    //
    // Which is also exactly #473 again, for this one page, with nothing on
    // screen to say so. The `warn!` is the only thing that separates the
    // residual case from the original bug when somebody has to diagnose it.
    if !super::claims_slug(workspace, node, slug) {
        tracing::warn!(
            "page {node}: the deterministic id for slug {slug:?} is in the trash but does \
             not carry that slug, so it is not revived — writes to this page will be \
             dropped until it is repaired (`outl doctor`)"
        );
        return Ok(None);
    }

    // Read the children before mutating: the trashing below rewrites the
    // parent map as it goes.
    let deleted_with_the_page: Vec<NodeId> = children_of(workspace, node)
        .into_iter()
        .map(|(id, _)| id)
        .collect();

    // One user-visible action, so one flush per destination rather than an
    // fsync per child.
    let mut batch = workspace.begin_batch();
    for child in deleted_with_the_page {
        crate::block::delete(&mut batch, hlc, child)?;
    }
    crate::block::move_under(&mut batch, hlc, node, NodeId::root())?;
    batch.commit()?;
    Ok(Some(node))
}
