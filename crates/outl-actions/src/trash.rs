//! Reading the trash back, and putting something back where it came
//! from.
//!
//! Invariant 6 makes delete a `Move(node, TRASH_ROOT)` — "simplifies the
//! algorithm and preserves history". The preserving half has worked since
//! day one. This module is the reading half: without it, the safety
//! property the invariant buys is "the bytes are still on disk", which is
//! a much weaker promise than "you can get it back" (issue #287).

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::ChildrenIndex;

/// One top-level deletion.
///
/// "Top-level" is the node the `Move` named. Deleting a subtree is one
/// op on its root, so every block underneath rides along implicitly and
/// is **not** a separate entry — it comes back with its root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashEntry {
    /// The node the `Move` named — the root of what was deleted.
    pub node: NodeId,
    /// Where it sat immediately before the move to the trash, folded
    /// from the op log. See [`parent_at_deletion`].
    /// Only the landing parent of a restorable entry: `None` for every
    /// entry with a `refusal`, even when the origin is known.
    pub parent_at_deletion: Option<NodeId>,
    /// `Some(slug)` when the deleted node is a page root rather than a
    /// block. [`restore`] refuses these, so a listing that could not
    /// tell them apart would offer an action that always fails.
    pub page: Option<String>,
    /// How many blocks come back together — the root plus everything
    /// under it. This is what the user would lose if the trash were
    /// ever emptied.
    pub subtree_len: usize,
    /// One line of the block's text, for recognising it in a list.
    pub preview: String,
    /// Wall-clock half of the HLC on the `Move` that put this node in
    /// the trash, in milliseconds since the epoch. `None` when the log
    /// holds no such move — the same condition that makes
    /// [`parent_at_deletion`] answer `None`.
    ///
    /// Folded from the **same** trail walk that decides the landing
    /// parent (`deletion_of`), never a second pass: a listing that
    /// asked twice would pay the whole cost of listing again for one
    /// number.
    ///
    /// It is a *clock*, not an ordering. [`list`] sorts by id, and a
    /// device with a skewed clock can legitimately report a time that
    /// reads out of order — the same caveat `timeline`'s `at_ms`
    /// carries. Render it; never re-sort on it.
    pub deleted_at_ms: Option<u64>,
    /// Why [`restore`] would refuse this entry, or `None` when it would
    /// succeed. **Produced by the same function `restore` consults**
    /// ([`refusal_for`]) — a listing that decided this for itself would
    /// be a second owner of the rule, and the direction it drifts is
    /// offering the user an action that then fails.
    ///
    /// Worded for the [`crate::refusal::Surface`] [`list`] was asked for, through
    /// [`crate::refusal::trash_refusal_text`]: the diagnosis is the same
    /// everywhere, the way out is not. A phone cannot run `outl doctor`
    /// and no outline can reach a block inside the deletion being
    /// restored, so a shared sentence was a to-do two surfaces could not
    /// perform.
    pub refusal: Option<String>,
}

/// Where [`restore`] would put `node`, or why it refuses.
///
/// **The single owner of that verdict.** [`restore`] asks it before
/// touching anything and [`list`] asks it per entry, so the listing and
/// the mutation cannot disagree — a listing that decided "restorable"
/// for itself would drift towards offering an action that then fails.
///
/// Returning the landing parent rather than a bare verdict lets `list`
/// ask once per entry, and lets `restore` use the verdict as its target
/// without a separate [`parent_at_deletion`] call.
/// `restore` still asks twice overall, since `entry_for` asks again.
fn landing_for(workspace: &Workspace, node: NodeId) -> Result<NodeId, ActionError> {
    landing_from(workspace, node, &deletion_of(workspace, node))
}

/// [`landing_for`] against a trail fold the caller already has.
///
/// Split out so [`entry_for`] can answer "where would it land", "why
/// would it not" and "when was it deleted" from **one** walk. The fold
/// is the expensive half of listing, and a second call for the
/// timestamp alone would double it.
fn landing_from(
    workspace: &Workspace,
    node: NodeId,
    deletion: &Result<Option<Deletion>, ActionError>,
) -> Result<NodeId, ActionError> {
    if !crate::tree::is_trashed(workspace, node) {
        return Err(ActionError::NotTrashed(node.to_string()));
    }

    // A page root is a `Move` *plus* a re-projected `.md`, and the slug
    // is usually taken by now. See `ActionError::TrashPageRestoreUnsupported`.
    if let Some(meta) = crate::page::page_meta(workspace, node) {
        return Err(ActionError::TrashPageRestoreUnsupported {
            node: node.to_string(),
            slug: meta.slug,
        });
    }

    // Past this point `node` is provably in the trash, so no arm may
    // answer `NotTrashed` — that is the contradiction
    // `a_block_the_log_cannot_place_is_not_reported_as_untrashed` pins.
    let unknown = |why: &str| ActionError::TrashOriginUnknown {
        node: node.to_string(),
        why: why.to_string(),
    };

    let parent = match deletion {
        Ok(Some(Deletion {
            parent: Some(parent),
            ..
        })) => *parent,
        // Both `None`s are the same answer to the user: the log holds no
        // move to the trash, or holds one that cannot say where from.
        Ok(_) => return Err(unknown("no move in the log put it there")),
        // No binary named here. `outl doctor` is the right thing to run
        // and it is not the right thing to *say* on a phone, which has
        // no binary, so the per-surface half lives in
        // `refusal::Remedy::CheckWorkspaceHealth`.
        Err(_) => return Err(unknown("its ops could not be read")),
    };

    // The fold only keeps moves the tree applied, so `parent` was outside
    // `node` when it was deleted. It can have moved inside since (a later
    // `Move` under the trashed subtree), and landing there would be a
    // cycle the tree refuses, reported as a restore that worked.
    // Reachable *concurrently*, not just by a local sequence: one device
    // deletes the block while another moves its old parent inside it.
    // Both ops are kept (invariant 4), the tree converges, and the loser
    // of that race is a trash row whose refusal never clears on its own
    // — there is no further op that un-nests the parent.
    //
    // It is **not** `unknown`, and it carries no instruction. Two things
    // were wrong with the version that did both.
    //
    // `TrashOriginUnknown`'s prefix said the op log could not say where
    // the block was, which is false here: the log says exactly that, and
    // the saying is what produced this refusal. It also nested one
    // parenthetical inside another. So this is its own variant now, with
    // its own error code.
    //
    // And the way out is not this string's to carry. "Move that block
    // back out from under this one" is performable on the CLI and the
    // MCP — `move_under` guards with `ensure_in_tree` rather than
    // `ensure_live` precisely so a node in the trash is a legal source
    // (`block/moves.rs`) — and performable on **no** outline: the guard
    // is `is_under(ws, parent, node)`, so `parent` sits inside the
    // trashed subtree and nothing live draws it. By invariant 2 these
    // surfaces show no block ids either, so a GUI user has nothing to
    // look up and nothing to drag. That difference is real and
    // per-surface, which is what `refusal::Remedy::MoveOutOfTrash`
    // records: two surfaces get the command, three are told there is
    // nothing to do here instead of a to-do they cannot perform.
    //
    // Nothing here *does* the un-nesting either way: that is an ordinary
    // move, the user's call about where their content belongs, and
    // performing it from a listing would be this module deciding the
    // shape of someone else's page.
    if crate::tree::is_under(workspace, parent, node) {
        return Err(ActionError::TrashOriginInsideSubtree {
            node: node.to_string(),
            parent: parent.to_string(),
        });
    }

    // `is_trashed` is an ancestor walk, not `parent(target) == trash`:
    // deleting a subtree is one `Move` on its root, so a node buried
    // inside a trashed subtree still points at a live-looking parent.
    if crate::tree::is_trashed(workspace, parent) {
        return Err(ActionError::TrashParentTrashed {
            node: node.to_string(),
            parent: parent.to_string(),
        });
    }

    // `NodeId::root()` is skipped, not special-cased for convenience: it is
    // an implicit parent that never appears in `Tree::nodes` (see
    // `Tree::new`), so `contains` answers no for a parent that always
    // exists. A block deleted from directly under root — every
    // pre-page-model block, and anything `migrate_legacy_into_today` has
    // not swept yet — was therefore unrestorable, and the refusal named
    // the all-zero root id as a "missing node". Same wrong question
    // `block::move_under` was asking one function away.
    if parent != NodeId::root() && !workspace.tree().contains(parent) {
        return Err(ActionError::TrashParentMissing {
            node: node.to_string(),
            parent: parent.to_string(),
        });
    }

    // Present and not trashed is still not the same as **reachable**.
    // A `Create` this replica never received (a torn `ops-*.jsonl`, a
    // partial sync, a compaction that dropped a line) leaves a parent
    // whose own ancestor chain dead-ends at a node that does not exist
    // here, satisfying both guards above while belonging to neither
    // `root()` nor `trash()` — invariant 4's third clause.
    //
    // Restoring into it is the sharp case, not merely an odd one: the
    // node was a **top-level trash row** before, so the user had a
    // handle on it, and landing it under an unreachable parent takes
    // that handle away while the listing had promised `refusal: None`.
    // A refusal keeps the row listed and the content reachable.
    //
    // `enclosing_page_id` already answers exactly this and already
    // returns `None` for a dead-ending chain, so this reuses the walk
    // rather than adding a second notion of reachable.
    if parent != NodeId::root() && crate::tree::enclosing_page_id(workspace, parent).is_none() {
        return Err(unknown(
            "the block it was deleted from is no longer reachable",
        ));
    }

    Ok(parent)
}

/// Why [`restore`] would refuse `node`, or `None` when it would work.
///
/// The read-only half of `landing_for`, for a caller that wants the
/// verdict without the destination.
pub fn refusal_for(workspace: &Workspace, node: NodeId) -> Option<ActionError> {
    landing_for(workspace, node).err()
}

/// Every top-level deletion currently in the trash, with each refusal
/// worded for `surface`.
///
/// Reads the **materialized tree**, not the op log's deletion history:
/// a block that was deleted and then restored is not in the trash and
/// does not belong in a listing of what can be restored.
///
/// **`surface` is the forcing function, not a convenience.**
/// [`TrashEntry::refusal`] used to be one shared string, so a caller got
/// the CLI's wording — naming `outl doctor` and `outl block tree <ULID>`
/// — whether or not it had a binary or an id to offer. Taking the
/// surface here means a caller cannot obtain a sentence without saying
/// who it is for, which is the same shape
/// [`crate::refusal::ahead_of_log_notice`] already has.
pub fn list(workspace: &Workspace, surface: crate::refusal::Surface) -> Vec<TrashEntry> {
    // The whole-workspace index: this walks every top-level deletion, so
    // the one scan is the cheap side of `children_index`'s trade-off.
    let children = crate::tree::children_index_unordered(workspace);

    let mut entries: Vec<TrashEntry> = children
        .get(&NodeId::trash())
        .into_iter()
        .flatten()
        .map(|&node| entry_for(workspace, &children, node, surface))
        .collect();

    // By id, for a stable order. Block ids are ULIDs, so they sort by
    // creation time, but page roots carry slug-hash ids with no time
    // order, so that holds only among blocks.
    //
    // Deliberately **not** by `deleted_at_ms`, even though the fold now
    // hands it over for free: that number is a wall clock from whichever
    // device deleted the block, so a peer with a skewed clock would
    // reshuffle this list between reads. A surface that wants recency
    // sorts on it knowing that; the canonical order stays the one that
    // cannot drift.
    entries.sort_by_key(|entry| entry.node);
    entries
}

/// Describe one trashed node, wording any refusal for `surface`.
fn entry_for(
    workspace: &Workspace,
    children: &ChildrenIndex,
    node: NodeId,
    surface: crate::refusal::Surface,
) -> TrashEntry {
    // One ask, every half. Where it would land, why it would not, and
    // when it was deleted are the same question of the same trail, and
    // asking twice was the whole cost of listing.
    let deletion = deletion_of(workspace, node);
    let deleted_at_ms = deletion
        .as_ref()
        .ok()
        .and_then(|found| found.as_ref())
        .map(|found| found.at_ms);
    let landing = landing_from(workspace, node, &deletion);
    TrashEntry {
        node,
        parent_at_deletion: landing.as_ref().ok().copied(),
        page: crate::page::page_meta(workspace, node).map(|meta| meta.slug),
        subtree_len: crate::tree::subtree_ids(children, node).len(),
        preview: preview(workspace, node),
        deleted_at_ms,
        refusal: landing
            .as_ref()
            .err()
            .map(|err| crate::refusal::trash_refusal_text(surface, err)),
    }
}

/// One-line preview of a block's text, safe to print.
fn preview(workspace: &Workspace, node: NodeId) -> String {
    let text = workspace.block_text(node).unwrap_or_default();
    let single = text.replace(['\n', '\r'], " ");
    let trimmed = single.trim();
    if trimmed.is_empty() {
        return "(empty block)".to_string();
    }
    let mut out: String = trimmed.chars().take(PREVIEW_CHARS).collect();
    if trimmed.chars().count() > PREVIEW_CHARS {
        out.push('…');
    }
    out
}

/// How much of a block's text a listing shows.
const PREVIEW_CHARS: usize = 80;

/// The parent a node sat under immediately before it was moved to the
/// trash, or `None` if the log never moved it there.
///
/// **Folded from `Create.parent` / `Move.new_parent`, never read off
/// `Move.old_parent`.** That field is the originating replica's local
/// derivation for `undo_op`, and the root `CLAUDE.md` says a reader of
/// the log as data must not trust it: on the workspace this was built
/// against, 65,141 of 65,703 stored `Move`s name `root` as the old
/// parent regardless of where the block actually was. An append-only
/// log never rewrites them, so reading the field would work on this
/// week's history and lie about every year before it.
///
/// Only placements the tree **applied** count. A `Move` refused as a
/// cycle stays in the log (invariant 4), and whether it was refused
/// depends on the tree at the time: `Move(A, B)` with `B` under `A`,
/// then `B` moved away, leaves nothing in today's tree to say so. The
/// replay decides each one the way `do_op` did.
///
/// This is the same fold [`crate::timeline`] needs to attribute a
/// deletion to a page, and it is deliberately the only one — two
/// answers to "where did this block live" is the drift the reuse-first
/// rule exists to prevent.
pub fn parent_at_deletion(
    workspace: &Workspace,
    node: NodeId,
) -> Result<Option<NodeId>, ActionError> {
    Ok(deletion_of(workspace, node)?.and_then(|found| found.parent))
}

/// The `Move` that put a node in the trash.
///
/// One struct because the parent and the instant come out of the same
/// fold, and two functions over the same trail is two walks of the op
/// log for one question.
struct Deletion {
    /// The parent it sat under immediately before the move, or `None`
    /// when the log holds the move and cannot say where from.
    parent: Option<NodeId>,
    /// Wall-clock half of that move's HLC.
    at_ms: u64,
}

/// Fold the parent trail once. **The single owner of both halves** —
/// [`parent_at_deletion`] and [`TrashEntry::deleted_at_ms`] are two
/// readings of this one walk, never two walks.
fn deletion_of(workspace: &Workspace, node: NodeId) -> Result<Option<Deletion>, ActionError> {
    // Only the placements the tree applied. Invariant 4 keeps a `Move`
    // refused as a cycle in the log, and whether one was refused depends
    // on the tree when it ran, not now; see `replay`.
    let mut replay = replay::Replay::new(workspace);
    let mut previous: Option<NodeId> = None;
    let mut deletion: Option<Deletion> = None;

    for (ts, parent) in replay.trail(node)? {
        // Re-emitting the trash as the parent of a block already there
        // moves nothing, and must not overwrite the answer with the
        // trash itself.
        if parent == Some(NodeId::trash()) && previous != parent {
            deletion = Some(Deletion {
                parent: previous,
                at_ms: ts.physical_ms,
            });
        }
        previous = parent;
    }

    Ok(deletion)
}

/// The page a [`restore`] of `node` would land on, for a caller that
/// has to name one **before** mutating.
///
/// [`crate::commit_page`] is scoped to a single page and a trashed
/// node's own ancestor walk reaches the trash root, so every client
/// resolves this first and commits second. Two of them were doing it by
/// hand with `parent_at_deletion(...).ok().flatten()` plus
/// `enclosing_page_id`, which is this function minus its one real rule.
///
/// **It asks `page_meta`, not just `enclosing_page_id`.** That walk
/// returns the first node whose parent is `root()`, which on a workspace
/// `migrate_legacy_into_today` has not swept is a pre-page-model block
/// rather than a page. Handing that to `commit_page` projects
/// `NotInTree`, which a client then reports as "this page has stopped
/// syncing" about a page that does not exist. `None` instead means "no
/// page to project", which is the honest answer.
///
/// Deliberately **looser than [`refusal_for`]**: it applies none of the
/// landing rules, because the mutation re-asks them under its own lock
/// and a second verdict here would be the drift this module exists to
/// prevent. A `Some` is not a promise the restore will succeed.
pub fn landing_page(workspace: &Workspace, node: NodeId) -> Option<NodeId> {
    let parent = parent_at_deletion(workspace, node).ok().flatten()?;
    let page = crate::tree::enclosing_page_id(workspace, parent)?;
    crate::page::page_meta(workspace, page).map(|_| page)
}

/// Put `node` back under the parent it was deleted from.
pub fn restore(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
) -> Result<TrashEntry, ActionError> {
    let target = landing_for(workspace, node)?;

    // Captured before the move: afterwards the node is no longer in the
    // trash, and an entry describing where it *now* lives would be a
    // receipt for something the caller did not ask for.
    //
    // `landing_for` succeeded a line up, so this receipt's `refusal` is
    // `None` and the surface words nothing — any value would do. That is
    // why `restore` does not take one: a mutation has no wording to
    // choose, and a `Surface` parameter here would be a fourth place
    // that looks like it words a refusal. `a_restore_receipt_carries_no_refusal`
    // is what keeps that true rather than this comment.
    let entry = entry_for(
        workspace,
        &crate::tree::children_index(workspace, node),
        node,
        crate::refusal::Surface::Cli,
    );
    crate::block::move_under(workspace, hlc, node, target)?;
    Ok(entry)
}

mod replay;

#[cfg(test)]
mod tests;
