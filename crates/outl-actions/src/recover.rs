//! Recover block text that an `Op::Edit` truncated.
//!
//! ## The direction this covers
//!
//! [`crate::journal::content_lines_missing_from`] and `outl reconcile
//! --ahead-of-log` answer "the `.md` holds content the log never saw" by
//! reading the `.md`. That only works while the `.md` still holds it.
//!
//! A page whose `.md` was overwritten before that guard existed is out of
//! their reach — but not out of the op log's. The producer of issue #210
//! was a broken `render → parse` roundtrip: the reconcile that followed
//! it emitted an `Op::Edit` carrying the *truncated* text. That edit went
//! into the log, and the log is append-only, so the edit **before** it —
//! the one carrying the full text — is still there.
//!
//! Verified on the reporting workspace (2,561 pages, 214k ops): block
//! `01KX0SSS4F4G8DPA6NP3F2RDE7` has two `Edit`s, the first materializing
//! 34 lines and the second one line. Replaying the first alone returns
//! all 34.
//!
//! ## Why the signature is "current is a prefix of an earlier revision"
//!
//! Shrinking is not by itself a defect — deleting text is a thing users
//! do. What the parser bug produced is narrower: it dropped everything
//! after a point, so the surviving text is a **prefix** of what it
//! replaced, character for character.
//!
//! That buys two properties worth more than a broader net:
//!
//! - **It is quiet.** On the reporting workspace the whole-graph scan
//!   returns 8 blocks out of 67,213, of which 4 are the multi-line
//!   briefings the report was about.
//! - **Restoring cannot lose anything.** The recovered text *contains*
//!   the current text as a prefix, so writing it back is strictly
//!   additive from the block's point of view. [`restore_truncated_block`]
//!   re-checks that rather than trusting it, and refuses outright when
//!   the block no longer says what the scan saw — a prefix test alone
//!   would wave through a block the user cleared in between.
//!
//! A false positive costs the user a line of output. A false negative
//! costs them the content, so the threshold ([`scan_truncated_blocks`]'s
//! `min_lost_lines`) defaults to the loosest useful value at the call
//! site and is raised, never lowered, by the caller.
//!
//! ## What this is not
//!
//! Not an undo stack ([`crate::history`]) and not a `.md` restore. It
//! reads history that already exists in the op log, and writing goes
//! through [`crate::block::edit_text`] like any other edit — a **new**
//! op, never a rewrite of the log (root `CLAUDE.md` invariant 1).

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::ActionError;

/// A block whose current text is a proper prefix of an earlier revision
/// in the op log — i.e. an `Op::Edit` truncated it and the dropped tail
/// is still reconstructible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruncatedBlock {
    /// The block.
    pub node: NodeId,
    /// Slug of the page hosting the block, when it resolves. `None` for
    /// a block whose ancestor chain does not reach a page root.
    pub page_slug: Option<String>,
    /// What the block says now.
    pub current: String,
    /// The longest earlier revision that still starts with [`Self::current`].
    /// Restoring this is additive: every character the block shows today
    /// is at the head of it.
    pub recovered: String,
    /// The non-blank lines present in [`Self::recovered`] and absent from
    /// [`Self::current`], in order.
    pub lost_lines: Vec<String>,
}

/// Scan every block in the materialized tree for the truncation
/// signature, keeping the ones that lost at least `min_lost_lines`
/// non-blank lines.
///
/// Read-only. Ordered by descending loss (then by node id) so the
/// worst damage is at the top of a truncated report and two runs agree.
///
/// Cost is one index-driven op read per node — measured at ~4.7s over
/// 67,213 nodes / 214k ops, which is why there is no cheaper prefilter:
/// any prefilter would have to be a *second* opinion about which blocks
/// deserve a look, and the point of this pass is not to have one.
pub fn scan_truncated_blocks(
    workspace: &Workspace,
    min_lost_lines: usize,
) -> Result<Vec<TruncatedBlock>, ActionError> {
    let mut found = Vec::new();
    for (node, _, _) in workspace.tree().iter_nodes() {
        // A trashed block is a block the user deleted. Its history is
        // just as recoverable, and offering it back is noise at best and
        // a resurrection of something deliberately removed at worst.
        if crate::tree::is_trashed(workspace, node) {
            continue;
        }
        let history = workspace.block_text_history(node)?;
        let Some(hit) = truncation_in(&history) else {
            continue;
        };
        if hit.lost_lines.len() < min_lost_lines {
            continue;
        }
        found.push(TruncatedBlock {
            node,
            page_slug: crate::tree::page_slug_of(workspace, node),
            current: hit.current,
            recovered: hit.recovered,
            lost_lines: hit.lost_lines,
        });
    }
    found.sort_by_key(|f| (std::cmp::Reverse(f.lost_lines.len()), f.node));
    Ok(found)
}

/// Write a recovered revision back as a **new** `Op::Edit`.
///
/// Two checks, and the first is the one that matters:
///
/// - **The block must still say exactly what the scan saw**
///   ([`TruncatedBlock::current`]). Asking only "is the live text still a
///   prefix of the recovery" is weaker than it looks: every shorter
///   prefix passes it, and the empty string is a prefix of everything, so
///   a block the user cleared or trimmed between the two passes would be
///   overwritten with a revision from before that edit — silently
///   reverting a change the user made, which is the class of bug this
///   whole module exists to undo.
/// - The recovery must still contain that text as a prefix, so the write
///   stays additive. Free for a scanned entry, and the only guard left
///   for one built by hand.
pub fn restore_truncated_block(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    found: &TruncatedBlock,
) -> Result<(), ActionError> {
    let live = workspace.block_text(found.node).unwrap_or_default();
    if live.trim_end() != found.current.trim_end() {
        return Err(ActionError::NotInTree(format!(
            "{} changed since the scan; re-run the scan before restoring",
            found.node
        )));
    }
    if !found
        .recovered
        .trim_end()
        .starts_with(found.current.trim_end())
    {
        return Err(ActionError::NotInTree(format!(
            "{} would not be restored additively; the recovered text does not start with what the block says",
            found.node
        )));
    }
    crate::block::edit_text(workspace, hlc, found.node, &found.recovered)
}

/// The truncation verdict for one block's history.
struct Truncation {
    current: String,
    recovered: String,
    lost_lines: Vec<String>,
}

/// Find the longest revision in `history` that still starts with the
/// block's current text and is strictly longer than it.
///
/// Trailing whitespace is trimmed on both sides before comparing: the
/// renderer's trailing-newline behaviour changed between releases, and a
/// guard that fires on that noise is a guard the user learns to ignore
/// (the same reasoning as [`crate::journal::content_lines_missing_from`]).
///
/// An empty current text yields `None` on purpose. A cleared block is an
/// ordinary deletion — the user emptied it — and every cleared block in
/// the workspace would otherwise qualify, since the empty string is a
/// prefix of everything.
fn truncation_in(history: &[String]) -> Option<Truncation> {
    let current = history.last()?.trim_end();
    if current.is_empty() || history.len() < 2 {
        return None;
    }
    let recovered = history[..history.len() - 1]
        .iter()
        .map(|revision| revision.trim_end())
        .filter(|revision| revision.len() > current.len() && revision.starts_with(current))
        .max_by_key(|revision| revision.len())?;

    let lost_lines: Vec<String> = recovered[current.len()..]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    if lost_lines.is_empty() {
        return None;
    }
    Some(Truncation {
        current: current.to_string(),
        recovered: recovered.to_string(),
        lost_lines,
    })
}

#[cfg(test)]
mod tests;
