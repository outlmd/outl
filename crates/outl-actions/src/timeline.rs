//! Reading a page's past out of the op log.
//!
//! [`outl_core::workspace::Workspace::block_revisions`] answers "what did
//! *this block* say before". This module answers the question a user
//! actually asks — "what happened to *this page*" — by collecting the
//! blocks the page owns and merging their op histories into one
//! chronological list.
//!
//! ## What a "version" is here
//!
//! An **event**, not a rendered page. `page_timeline` reports what
//! changed, when, by whom, and what the text said on either side of the
//! change. It does not reconstruct the whole `.md` as of an arbitrary
//! `Hlc`.
//!
//! That is a deliberate v1 boundary. Rendering the page at a past instant
//! means replaying the tree to that instant, which is a second
//! materialization path alongside boot — a place where the two could
//! disagree about what the log means. The changelog needs no such replay
//! and answers the question that sends someone looking ("did this page
//! have more in it yesterday, and what was it").
//!
//! ## Which blocks count as the page's
//!
//! **A block's history follows the block.** The set is:
//!
//! - every block in the page's subtree right now, plus
//! - every block that was deleted *out of* that subtree — deletion is
//!   `Move(node, TRASH_ROOT)` (root `CLAUDE.md` invariant 6), so a trashed
//!   subtree root whose parent *at deletion time* is in this page still
//!   belongs to it, along with everything under it. That parent is folded
//!   from `Create.parent` / `Move.new_parent`, never read off
//!   `Move.old_parent` — see `block_events` for why. The scan runs to a
//!   fixpoint, because deleting a child before its parent only becomes
//!   attributable once the parent's own subtree has been admitted.
//!
//! A block moved to a *different* page is therefore absent, and shows up
//! on that page's timeline instead. That is a rule, not an oversight: the
//! alternative is attributing each op to whichever page held the node at
//! that op's instant, which needs the same point-in-time replay the
//! paragraph above declines to build. `block_timeline` reads one block's
//! full history regardless of where it has lived.
//!
//! Including the trash is the part that is not optional. A history that
//! omits deletions answers "what changed" with everything except the
//! change people go looking for.
//!
//! ## What is deliberately not an event
//!
//! `Op::SetCollapsed` and `Op::SnoozeRemind` are excluded. Folding is view
//! state and a snooze is reminder bookkeeping; neither changes what the
//! page says, and both are frequent enough to bury the events that do.
//! `page-slug` / `page-kind` property writes are excluded for the same
//! reason via [`crate::tree::is_page_model_key`] — they are book-keeping
//! the user never typed.
//!
//! ## Read-only
//!
//! Nothing here writes. Restoring a past revision is
//! [`crate::recover`]'s job for the one case with a proven-safe rule
//! (the current text is a prefix of the recovered one, so the write is
//! additive). A general "restore this version" needs its own safety
//! argument and is not in this module.
//!
//! ## Cost
//!
//! Two index-driven read sets per block in the page — one for the
//! structural ops, one for the text revisions — so O(ops-of-page), not
//! O(log). Plus one read per trashed subtree root in the workspace to
//! decide whether it came from this page.

use std::collections::{HashMap, HashSet};

use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::tree::is_page_model_key;

/// One thing that happened to one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEvent {
    /// HLC of the op. Total order across devices, so two events from two
    /// devices sort deterministically.
    pub ts: Hlc,
    /// The device that produced the op.
    pub actor: ActorId,
    /// The block the op named.
    pub node: NodeId,
    /// Whether that block is in the trash **today**. Lets a reader tell
    /// "this block was edited and still exists" from "this block was
    /// edited and is now gone".
    pub node_deleted: bool,
    /// What happened.
    pub change: Change,
}

/// The kinds of change a timeline reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The block came into existence.
    Created,
    /// The block's text changed. `from` is `None` for the first edit,
    /// which turns an empty new block into its initial text.
    Edited {
        /// What the block said before this edit.
        from: Option<String>,
        /// What it said after.
        to: String,
    },
    /// The block was moved to the trash — what a user calls deleting it.
    /// `text` is what it said at that moment, which is the whole reason
    /// to look at a timeline after losing something.
    Deleted {
        /// The block's text when it was trashed.
        text: String,
    },
    /// The block came back out of the trash.
    Restored,
    /// The block was reparented or reordered inside the page — an
    /// indent, an outdent, or a drag to a new position.
    Moved,
    /// A property was set, changed, or cleared. `to` is `None` when the
    /// property was removed.
    PropertySet {
        /// The property key.
        key: String,
        /// The previous value, if any.
        from: Option<String>,
        /// The new value, or `None` when the property was cleared.
        to: Option<String>,
    },
}

/// A page's history, newest event first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTimeline {
    /// The page's slug.
    pub slug: String,
    /// The page's root block.
    pub page_root: NodeId,
    /// How many events the page has in total, before `limit` was
    /// applied. Reported separately so a truncated listing can say so
    /// rather than looking complete.
    pub total: usize,
    /// The events, newest first, at most `limit` of them.
    pub events: Vec<TimelineEvent>,
}

impl PageTimeline {
    /// Whether events were left out of [`Self::events`] by the limit.
    pub fn truncated(&self) -> bool {
        self.events.len() < self.total
    }
}

/// Every event on `page_root`'s blocks, newest first, capped at `limit`.
///
/// `limit` caps the returned vector, never the scan — [`PageTimeline::total`]
/// always counts everything found, so a caller can say "showing 50 of
/// 812" instead of implying the page has 50 events.
pub fn page_timeline(
    workspace: &Workspace,
    page_root: NodeId,
    slug: &str,
    limit: usize,
) -> Result<PageTimeline, ActionError> {
    let nodes = page_nodes(workspace, page_root);
    let mut events = Vec::new();
    for node in &nodes {
        events.extend(block_events(workspace, *node)?);
    }
    sort_newest_first(&mut events);
    let total = events.len();
    events.truncate(limit);
    Ok(PageTimeline {
        slug: slug.to_string(),
        page_root,
        total,
        events,
    })
}

/// Every event on one block, newest first.
///
/// Unlike [`page_timeline`] this does not care where the block lives, so
/// it is the way to follow a block that moved between pages.
///
/// No `limit`: one block's history is bounded by how often that block
/// was touched, and a caller that wants fewer rows truncates — which
/// keeps the honest total in the only place that can report it.
pub fn block_timeline(
    workspace: &Workspace,
    node: NodeId,
) -> Result<Vec<TimelineEvent>, ActionError> {
    let mut events = block_events(workspace, node)?;
    sort_newest_first(&mut events);
    Ok(events)
}

/// Newest first, with the `Hlc`'s actor tiebreak doing the work for two
/// ops that share a millisecond (root `CLAUDE.md`: never compare HLCs
/// without it — `Ord for Hlc` already includes it).
fn sort_newest_first(events: &mut [TimelineEvent]) {
    events.sort_by_key(|e| std::cmp::Reverse(e.ts));
}

/// The blocks whose history belongs to this page: the live subtree plus
/// whatever was deleted out of it. See the module doc for why the trash
/// half is not optional.
fn page_nodes(workspace: &Workspace, page_root: NodeId) -> Vec<NodeId> {
    // One pass over the tree, reused for every descent below.
    // `children_of` rescans `iter_nodes()` in full per call and
    // `push_subtree` calls it once per node, so the naive version is
    // O(page x workspace) under the client's workspace lock — measured at
    // 240ms for the largest page of a 64k-node graph.
    let children = children_index(workspace);
    let mut nodes = Vec::new();
    push_subtree(&children, page_root, &mut nodes);
    let mut known: HashSet<NodeId> = nodes.iter().copied().collect();

    // Deletion is `Move(node, TRASH_ROOT)`, so every deleted subtree root
    // is a direct child of the trash — no need to walk the whole trash to
    // find them.
    let mut candidates: Vec<NodeId> = children.get(&NodeId::trash()).cloned().unwrap_or_default();

    // To a fixpoint, because a deletion admits more of the page than it
    // started with. Delete a child and *then* its parent and the child is
    // its own direct child of the trash, whose parent-at-deletion is a
    // block that is no longer in the live subtree — it only becomes
    // recognisable once the parent's own subtree has been admitted. One
    // pass drops it, and dropping it loses the `Deleted` event and the
    // text it took, which is the one thing this module promises to keep.
    loop {
        let mut admitted = false;
        candidates.retain(|&trashed| {
            if !came_from(workspace, trashed, &known) {
                return true;
            }
            let mut subtree = Vec::new();
            push_subtree(&children, trashed, &mut subtree);
            known.extend(subtree.iter().copied());
            nodes.extend(subtree);
            admitted = true;
            false
        });
        if !admitted {
            return nodes;
        }
    }
}

/// `parent -> children` for the whole tree, in one scan.
///
/// Order within a parent is arbitrary: this feeds a set of nodes to read
/// history for, and the events are sorted by `Hlc` afterwards. Nothing
/// here depends on sibling order, which is why it can skip the
/// fractional-position sort `children_of` pays for.
fn children_index(workspace: &Workspace) -> HashMap<NodeId, Vec<NodeId>> {
    let mut map: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for (id, parent, _) in workspace.tree().iter_nodes() {
        map.entry(parent).or_default().push(id);
    }
    map
}

/// Append `root` and everything under it to `out`.
fn push_subtree(children: &HashMap<NodeId, Vec<NodeId>>, root: NodeId, out: &mut Vec<NodeId>) {
    out.push(root);
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        for &child in children.get(&node).into_iter().flatten() {
            out.push(child);
            stack.push(child);
        }
    }
}

/// Whether `trashed` was moved to the trash out of `known`.
///
/// Best-effort by design: a node whose ops are unreadable, or whose
/// parent at deletion time is a block this page no longer holds, is left
/// out rather than guessed into the page. Over-including would put
/// another page's deletions in this page's history, which is worse than
/// a gap — a gap is visibly a gap.
fn came_from(workspace: &Workspace, trashed: NodeId, known: &HashSet<NodeId>) -> bool {
    let Ok(ops) = workspace.ops_for_node(trashed) else {
        return false;
    };
    let mut parent: Option<NodeId> = None;
    let mut created = false;
    for logged in &ops {
        match &logged.op {
            // First `Create` only — see `block_events` for why a
            // re-emitted one must not move the parent.
            Op::Create { parent: p, .. } if !created => {
                created = true;
                parent = Some(*p);
            }
            Op::Create { .. } => {}
            Op::Move { new_parent, .. } => {
                if *new_parent == NodeId::trash() && parent.is_some_and(|p| known.contains(&p)) {
                    return true;
                }
                parent = Some(*new_parent);
            }
            _ => {}
        }
    }
    false
}

/// Turn one block's ops into events.
///
/// Two reads: the ops for structure and properties, the revisions for
/// text. They are joined on `ts` rather than recomputed, so the text an
/// `Edited` event reports is the same string
/// [`outl_core::workspace::Workspace::block_revisions`] would give — one
/// owner of "what did this block say", no second opinion.
fn block_events(workspace: &Workspace, node: NodeId) -> Result<Vec<TimelineEvent>, ActionError> {
    let ops = workspace.ops_for_node(node)?;
    let revisions = workspace.block_revisions(node)?;
    // An ancestor walk, not `parent(node) == trash`: deleting a subtree
    // is one `Move` on its root, so every block inside it still points at
    // that root and the direct test answers `false` for all of them.
    let deleted = crate::tree::is_trashed(workspace, node);

    let mut events = Vec::new();
    let mut previous_text: Option<String> = None;
    let mut parent: Option<NodeId> = None;
    let mut created = false;
    let mut previous_prop: HashMap<String, String> = HashMap::new();
    let revision_at = |ts: Hlc| -> Option<String> {
        revisions
            .iter()
            .rfind(|rev| rev.ts <= ts)
            .map(|rev| rev.text.clone())
    };

    for logged in &ops {
        let change = match &logged.op {
            Op::Create { parent: p, .. } => {
                // `Op::Create` is idempotent, and a reconcile re-emits
                // one for a block that already exists — 5 times over for
                // some blocks in the workspace this was built against.
                // A block is created once; the rest are bookkeeping.
                //
                // The parent only moves on the first one, mirroring
                // `do_op` (`if !self.nodes.contains_key(node)`): a later
                // `Create` naming a stale parent is discarded by the
                // tree, and trusting it here would let the page that
                // block *used* to live on claim a deletion that happened
                // somewhere else.
                if std::mem::replace(&mut created, true) {
                    continue;
                }
                parent = Some(*p);
                Change::Created
            }
            Op::Edit { .. } => {
                let to = revision_at(logged.ts).unwrap_or_default();
                let from = previous_text.replace(to.clone());
                // A reconcile re-emitting a block's existing text is a
                // real op and not a change. Reporting it as one puts a
                // row saying `- x` / `+ x` at the top of the history,
                // which is where the change the user came for should be.
                if from.as_deref() == Some(to.as_str()) {
                    continue;
                }
                Change::Edited { from, to }
            }
            Op::Move { new_parent, .. } => {
                let was = parent.replace(*new_parent);
                // Deliberately not `old_parent`, and **keep it that way
                // even though `Workspace::apply` was fixed to persist the
                // field `do_op` derived.** That fix only reaches ops
                // written after it: the reference workspace still holds
                // 65,141 `Move` ops naming `root` as the old parent
                // regardless of where the block was, and an append-only
                // log never rewrites them. Reading the field would work
                // on this week's history and lie about every year before
                // it. `new_parent` is the op's own effect, correct in
                // both eras, so the parent trail is folded from that.
                if was == Some(*new_parent) {
                    // Re-emitting a block's current parent moves nothing.
                    continue;
                }
                if *new_parent == NodeId::trash() {
                    Change::Deleted {
                        text: revision_at(logged.ts).unwrap_or_default(),
                    }
                } else if was == Some(NodeId::trash()) {
                    Change::Restored
                } else {
                    Change::Moved
                }
            }
            Op::SetProp { key, value, .. } => {
                if is_page_model_key(key) {
                    continue;
                }
                // Deliberately not `old_value`, for the reason the `Move`
                // arm gives: `Workspace::apply` records it correctly now,
                // but all 14,191 `SetProp` ops already on disk in the
                // reference graph carry a null one, and the log is
                // append-only. Reading it would report every historical
                // change as a first write. The previous value is folded
                // from this node's own op stream instead, which is right
                // for both eras.
                let to = value.as_ref().map(render_prop);
                let from = match &to {
                    Some(v) => previous_prop.insert(key.clone(), v.clone()),
                    None => previous_prop.remove(key),
                };
                // Re-writing a property's current value changes nothing.
                if from == to {
                    continue;
                }
                Change::PropertySet {
                    key: key.clone(),
                    from,
                    to,
                }
            }
            // View state and reminder bookkeeping — see the module doc.
            Op::SetCollapsed { .. } | Op::SnoozeRemind { .. } => continue,
        };
        events.push(TimelineEvent {
            ts: logged.ts,
            actor: logged.actor,
            node,
            node_deleted: deleted,
            change,
        });
    }
    Ok(events)
}

/// A property value as a single line of display text.
///
/// Not [`crate::tree::renderable_prop_value`]: that one owns what
/// round-trips through the `.md` dialect and drops `List` because the
/// dialect has no syntax for it. A timeline is not a renderer — dropping
/// a value here would report "property set to nothing" for a change that
/// set it to something.
fn render_prop(value: &PropValue) -> String {
    match value {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => s.clone(),
        PropValue::List(items) => items.iter().map(render_prop).collect::<Vec<_>>().join(" "),
    }
}

#[cfg(test)]
mod tests;
