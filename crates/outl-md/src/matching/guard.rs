//! Volume guard over level-3 orphans.
//!
//! [`super::match_blocks`] treats one orphan and five thousand orphans
//! identically: both come back in the same `Vec<NodeId>`, and the caller
//! turns every entry into a `Move(node, TRASH_ROOT)`. That is correct
//! for the *shape* of an edit and blind to its *scale*, so a `.md` that
//! arrived truncated — an iCloud placeholder whose bytes never
//! downloaded, a half-flushed write, a parser that stopped reading the
//! dialect halfway — empties a page as quietly as deleting one bullet.
//! A data-destroying operation that scales silently is how a small bug
//! becomes an unrecoverable one ([issue #210]).
//!
//! So the volume is a separate question from the match, asked after it:
//!
//! ```text
//! let (matches, orphans) = match_blocks_guarded(&ast.blocks, &old, &OrphanGuard::Enforced)?;
//! ```
//!
//! Three properties this guard is built to have:
//!
//! - **It cannot lose half a page.** [`super::match_blocks`] is pure —
//!   it reads two slices and allocates two vectors — so refusing
//!   *after* it ran is refusing before anything exists to apply. There
//!   is no partial state to roll back.
//! - **It is never silent.** The refusal is an `Err`, not a shorter
//!   orphan list. A guard that quietly drops the deletions leaves the
//!   blocks in the tree and out of the `.md`, which is the divergence
//!   the reconcile exists to close.
//! - **It has an explicit way out.** [`OrphanGuard::Disabled`] is what
//!   a caller wires to the user saying "yes, I meant to delete that".
//!   A guard with no escape hatch is a wall, and RFC 0211 names that as
//!   its own defect class.
//!
//! [issue #210]: https://github.com/outlmd/outl/issues/210

use std::fmt;

use outl_core::id::NodeId;

use crate::parse::OutlineNode;
use crate::sidecar::SidecarBlock;

/// Absolute ceiling on orphans produced by a single reconcile of one
/// page.
///
/// No hand edit removes five hundred blocks from one page in one save.
/// An import, a migration or a script legitimately might, and those are
/// exactly the callers that should have to say so out loud — they run
/// unattended, over the whole workspace, and are the ones that turn a
/// one-page defect into a workspace-wide one.
pub const MAX_ORPHANED_BLOCKS: usize = 500;

/// Relative ceiling: the share of a page's previously-known blocks that
/// may orphan in one pass.
///
/// Chosen high on purpose. Deleting a section is ordinary editing and
/// usually costs well under half a page, while the failure this guard
/// exists for — a `.md` that arrived truncated or empty — takes
/// essentially all of it. Setting this near the middle would fire on
/// real edits, and RFC 0210 already recorded what that costs: a guard
/// that fires constantly gets disabled, and then it guards nothing.
pub const MAX_ORPHANED_RATIO: f64 = 0.75;

/// Below this many previously-known blocks, the ratio never fires.
///
/// A ratio is meaningless on a small page: clearing a four-block scratch
/// note is 100% and completely routine. Under the floor only
/// [`MAX_ORPHANED_BLOCKS`] applies, which by construction cannot trip —
/// small pages are deliberately unguarded, because their blast radius is
/// small and their false-positive cost is not.
pub const RATIO_FLOOR_BLOCKS: usize = 20;

/// How much of a page one reconcile pass proposes to delete.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrphanVolume {
    /// Blocks that found no counterpart in the new `.md` and would be
    /// moved to `TRASH_ROOT`.
    pub orphaned: usize,
    /// Blocks the previous sidecar recorded — what the op log held when
    /// the `.md` and the log last agreed.
    pub previously_known: usize,
}

impl OrphanVolume {
    /// Share of the previously-known blocks that would be deleted.
    ///
    /// `0.0` when the page had nothing recorded, so an empty sidecar can
    /// never look like a total wipe.
    pub fn ratio(&self) -> f64 {
        if self.previously_known == 0 {
            return 0.0;
        }
        self.orphaned as f64 / self.previously_known as f64
    }
}

impl fmt::Display for OrphanVolume {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} of {} known block(s), {:.0}%",
            self.orphaned,
            self.previously_known,
            self.ratio() * 100.0
        )
    }
}

/// Refusal raised by [`OrphanGuard::check`].
#[derive(Clone, Debug, thiserror::Error)]
pub enum MatchGuardError {
    /// The proposed deletion is too large to apply without the user
    /// saying so.
    ///
    /// Which of the two ceilings was crossed is not carried separately:
    /// `volume` prints both the count and the share, so the number the
    /// user needs to compare against their own edit is already in the
    /// message, and nothing in the codebase branches on the distinction.
    #[error(
        "refusing to delete {volume} in one pass — nothing was written, \
         the page is left exactly as the op log has it. Re-run with the bulk-delete \
         escape hatch if the deletion is intentional"
    )]
    BulkDelete {
        /// What the pass proposed to delete.
        volume: OrphanVolume,
    },
}

/// Whether a reconcile pass is held to the ceilings above.
///
/// Two states, not a set of knobs: the thresholds are constants with a
/// documented rationale, and no caller has ever wanted a third policy.
/// A tunable struct would invite one to be invented at a call site,
/// which is how a guard ends up weaker in the path that needed it most.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OrphanGuard {
    /// The shipped policy: [`MAX_ORPHANED_BLOCKS`],
    /// [`MAX_ORPHANED_RATIO`], [`RATIO_FLOOR_BLOCKS`].
    #[default]
    Enforced,
    /// The explicit way out: "yes, I meant to delete that".
    ///
    /// Reach this only from a deliberate user act — a `--force`-shaped
    /// flag, a confirmed prompt — never from a default or a retry. The
    /// point of the guard is that a bulk delete is *authorised*, not
    /// merely *attempted twice*.
    Disabled,
}

impl OrphanGuard {
    /// Verdict on a proposed deletion.
    pub fn check(&self, volume: OrphanVolume) -> Result<(), MatchGuardError> {
        if matches!(self, Self::Disabled) || volume.orphaned == 0 {
            return Ok(());
        }
        let past_absolute = volume.orphaned > MAX_ORPHANED_BLOCKS;
        let past_share =
            volume.previously_known >= RATIO_FLOOR_BLOCKS && volume.ratio() > MAX_ORPHANED_RATIO;

        if past_absolute || past_share {
            return Err(MatchGuardError::BulkDelete { volume });
        }
        Ok(())
    }
}

/// [`super::match_blocks`] with the volume of the resulting deletion
/// checked before the caller can act on it.
///
/// Returns exactly what `match_blocks` returns when the volume is within
/// `guard`, and `Err` — with nothing else changed anywhere — when it is
/// not. Prefer this over the raw function wherever the orphans go on to
/// become `Move(node, TRASH_ROOT)`.
pub fn match_blocks_guarded(
    new_blocks: &[OutlineNode],
    old_blocks: &[SidecarBlock],
    guard: &OrphanGuard,
) -> Result<(Vec<super::Match>, Vec<NodeId>), MatchGuardError> {
    match_blocks_guarded_except(new_blocks, old_blocks, guard, |_| false)
}

/// [`match_blocks_guarded`], leaving out of the **volume** the orphans
/// `carried_elsewhere` says the same pass keeps some other way.
///
/// One caller, `reconcile_md`, asking on behalf of two migrations with
/// the same shape — a sidecar written before a construct was modelled
/// holds its lines as separate blocks, so the pass that consolidates
/// them orphans most of a page and would be refused forever, frozen:
///
/// - **the pre-fence sidecar** (issue #281), which recorded each YAML
///   line as a block; their content is the fence still on disk, logged
///   as a page property in the same pass;
/// - **the pre-table sidecar** (issue #329), which recorded each table
///   row as a block; their content is the table block the same pass
///   creates (`table::row_carried_by`).
///
/// Both predicates are **positive evidence** rather than a bypass: each
/// asks whether the orphan's content is on disk right now, so a
/// truncated `.md` — the thing this guard exists for — gets no
/// discount from either.
///
/// Exempt orphans are still returned, trashed and logged: this changes
/// what the guard counts, never what the pass deletes. A predicate that
/// answers `true` for real content is a hole in the guard, so it has to be
/// narrow and grounded in bytes on disk, not in the sidecar alone.
pub(crate) fn match_blocks_guarded_except(
    new_blocks: &[OutlineNode],
    old_blocks: &[SidecarBlock],
    guard: &OrphanGuard,
    carried_elsewhere: impl Fn(&SidecarBlock) -> bool,
) -> Result<(Vec<super::Match>, Vec<NodeId>), MatchGuardError> {
    let (matches, orphans) = super::match_blocks(new_blocks, old_blocks);
    let orphan_ids: std::collections::HashSet<NodeId> = orphans.iter().copied().collect();
    let exempt = old_blocks
        .iter()
        .filter(|b| orphan_ids.contains(&b.id) && carried_elsewhere(b))
        .count();
    // `exempt` comes off the **numerator only**. Taking it off both
    // sides raises the ratio for the orphans that are not exempt —
    // `(O-E)/(K-E) >= O/K` — so a page carrying a big exempt set
    // refuses deletions that are nowhere near the ceiling.
    //
    // Harmless while the only predicate was `is_legacy_fence_block`,
    // which exempts the handful of lines in a YAML fence. `row_carried_by`
    // routinely exempts dozens, which is what made the term observable:
    // a 60-row table plus 25 prose bullets, 20 of them cleared in an
    // editor, measured as "20 of 25 known, 80%" and was refused — an
    // honest 20 of 85 is 24%. Nothing is written on a refusal, so the
    // next pass sees the same inputs and refuses again: the page freezes
    // permanently, which is the outcome the exemption exists to prevent.
    //
    // Pinned by `a_big_exempt_set_does_not_refuse_an_ordinary_delete`.
    let volume = OrphanVolume {
        orphaned: orphans.len().saturating_sub(exempt),
        previously_known: old_blocks.len(),
    };
    if let Err(e) = guard.check(volume) {
        // Loud on the way out: the caller gets the `Err`, and an
        // operator reading logs gets the page-level story without
        // needing the caller to have re-logged it.
        tracing::error!(
            orphaned = volume.orphaned,
            previously_known = volume.previously_known,
            "orphan volume guard refused a bulk delete"
        );
        return Err(e);
    }
    Ok((matches, orphans))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn old_page(blocks: usize) -> Vec<SidecarBlock> {
        (0..blocks)
            .map(|i| SidecarBlock::from_text(NodeId::new(), i + 1, 0, format!("block {i}")))
            .collect()
    }

    fn md_with(blocks: usize) -> String {
        (0..blocks)
            .map(|i| format!("- block {i}\n"))
            .collect::<String>()
    }

    // ------------------------------------------------------- the policy

    #[test]
    fn an_ordinary_section_delete_passes() {
        // 30-block page, 10 bullets removed: 33%, the shape of a user
        // clearing a section. A guard that fires here gets disabled.
        let guard = OrphanGuard::Enforced;
        assert!(guard
            .check(OrphanVolume {
                orphaned: 10,
                previously_known: 30,
            })
            .is_ok());
    }

    #[test]
    fn emptying_a_page_trips_the_relative_arm() {
        let guard = OrphanGuard::Enforced;
        let err = guard
            .check(OrphanVolume {
                orphaned: 24,
                previously_known: 24,
            })
            .expect_err("a page that lost every block must not be applied silently");
        // 24 of 24 is 100% but well under the absolute ceiling, so only
        // the share arm can have fired.
        assert!(matches!(err, MatchGuardError::BulkDelete { .. }));
    }

    #[test]
    fn a_small_page_stays_unguarded() {
        // Under the floor the ratio says nothing: clearing a four-block
        // scratch note is routine, and the absolute arm cannot fire that
        // low. Deliberately permissive.
        let guard = OrphanGuard::Enforced;
        assert!(guard
            .check(OrphanVolume {
                orphaned: 4,
                previously_known: 4,
            })
            .is_ok());
    }

    #[test]
    fn a_large_partial_delete_trips_the_absolute_arm() {
        // 5,000 of 20,000 blocks is only 25% — under the ratio, and far
        // past anything a human typed in one save.
        let guard = OrphanGuard::Enforced;
        let err = guard
            .check(OrphanVolume {
                orphaned: 5_000,
                previously_known: 20_000,
            })
            .expect_err("5,000 blocks in one pass is not an edit");
        // 25% is under the share ceiling, so only the absolute arm can
        // have fired.
        assert!(matches!(err, MatchGuardError::BulkDelete { .. }));
    }

    #[test]
    fn nothing_deleted_never_trips() {
        // `previously_known == 0` makes the ratio 0/0; it must read as
        // "no deletion", never as a total wipe.
        let guard = OrphanGuard::Enforced;
        assert!(guard
            .check(OrphanVolume {
                orphaned: 0,
                previously_known: 0,
            })
            .is_ok());
    }

    #[test]
    fn the_escape_hatch_lets_a_full_wipe_through() {
        let err = OrphanGuard::Enforced.check(OrphanVolume {
            orphaned: 9_000,
            previously_known: 9_000,
        });
        assert!(err.is_err(), "the default policy must refuse this");
        assert!(
            OrphanGuard::Disabled
                .check(OrphanVolume {
                    orphaned: 9_000,
                    previously_known: 9_000,
                })
                .is_ok(),
            "an explicit opt-in must be able to apply the same deletion"
        );
    }

    #[test]
    fn the_refusal_names_the_volume_and_the_way_out() {
        let err = OrphanGuard::Enforced
            .check(OrphanVolume {
                orphaned: 30,
                previously_known: 30,
            })
            .expect_err("must refuse");
        let msg = err.to_string();
        assert!(
            msg.contains("30 of 30"),
            "counts must be in the message: {msg}"
        );
        assert!(
            msg.contains("nothing was written"),
            "the message must say the page is untouched: {msg}"
        );
        assert!(
            msg.contains("escape hatch"),
            "the message must point at the way out: {msg}"
        );
    }

    // -------------------------------------------------- the wrapper

    #[test]
    fn a_truncated_md_is_refused_and_nothing_is_returned_to_delete() {
        // The failure this exists for: the whole file arrived empty
        // (iCloud placeholder, half-flushed write). Level 3 orphans
        // every block and the caller would trash the page.
        let old = old_page(40);
        let ast = parse("");
        let err = match_blocks_guarded(&ast.blocks, &old, &OrphanGuard::Enforced)
            .expect_err("an emptied page must be refused");
        match err {
            MatchGuardError::BulkDelete { volume, .. } => {
                assert_eq!(volume.orphaned, 40);
                assert_eq!(volume.previously_known, 40);
            }
        }
    }

    #[test]
    fn an_ordinary_edit_passes_through_unchanged() {
        // The guarded call must be a drop-in: same matches, same
        // orphans, whenever the volume is within policy.
        let old = old_page(30);
        let ast = parse(&md_with(25));
        let (guarded_matches, guarded_orphans) =
            match_blocks_guarded(&ast.blocks, &old, &OrphanGuard::Enforced)
                .expect("5 of 30 blocks is an ordinary edit");
        let (matches, orphans) = super::super::match_blocks(&ast.blocks, &old);
        assert_eq!(guarded_matches.len(), matches.len());
        assert_eq!(guarded_orphans, orphans);
        assert_eq!(guarded_orphans.len(), 5);
    }

    // ------------------------------------- the table-row exemption (#329)

    /// A pre-#329 sidecar: one block per table row, which is what the
    /// parser produced before a table was a block of its own.
    fn old_table_rows(body_rows: usize) -> (Vec<SidecarBlock>, String) {
        let mut rows = vec!["| a | b |".to_string(), "| --- | --- |".to_string()];
        for i in 0..body_rows {
            rows.push(format!("| {i} | x |"));
        }
        let old = rows
            .iter()
            .enumerate()
            .map(|(i, text)| SidecarBlock::from_text(NodeId::new(), i + 1, 0, text))
            .collect();
        (old, rows.join("\n") + "\n")
    }

    #[test]
    fn consolidating_a_tables_rows_is_not_a_bulk_delete() {
        // Every known block orphans — the table is now one block whose
        // text is too dissimilar to any single row for level 2. On a
        // page that is mostly the table that is 100%, so without the
        // exemption the guard refuses the migrating pass forever and the
        // page is frozen in both directions.
        let (old, md) = old_table_rows(30);
        let ast = parse(&md);
        assert_eq!(ast.blocks.len(), 1, "the table is one block now");

        let refused = match_blocks_guarded(&ast.blocks, &old, &OrphanGuard::Enforced);
        assert!(
            refused.is_err(),
            "precondition: unexempted, this is a 100% orphan rate"
        );

        let (_, orphans) =
            match_blocks_guarded_except(&ast.blocks, &old, &OrphanGuard::Enforced, |b| {
                crate::table::row_carried_by(&ast.blocks, &b.text)
            })
            .expect("the migrating pass is allowed through");
        // The volume changed, the outcome did not: the rows are still
        // trashed and logged, and their content is in the table block.
        assert_eq!(orphans.len(), 32);
    }

    #[test]
    fn a_truncated_file_gets_no_discount_from_the_table_exemption() {
        // The direction that matters. Same sidecar, but the `.md` is
        // gone — there is no table on disk for a row to belong to, so
        // the exemption finds nothing and the guard still refuses.
        let (old, _) = old_table_rows(30);
        let ast = parse("");
        let err = match_blocks_guarded_except(&ast.blocks, &old, &OrphanGuard::Enforced, |b| {
            crate::table::row_carried_by(&ast.blocks, &b.text)
        })
        .expect_err("an empty file is exactly what the guard is for");
        assert!(matches!(err, MatchGuardError::BulkDelete { .. }), "{err:?}");
    }

    #[test]
    fn a_row_the_user_also_edited_is_not_exempt() {
        // The exemption is positive evidence, not a bypass: a row whose
        // cell changed is not a line of the table on disk, so it counts
        // towards the volume like any other orphan.
        let (old, md) = old_table_rows(30);
        let edited = md.replace("| 0 | x |", "| 0 | CHANGED |");
        let ast = parse(&edited);
        let exempt = old
            .iter()
            .filter(|b| crate::table::row_carried_by(&ast.blocks, &b.text))
            .count();
        assert_eq!(exempt, old.len() - 1, "the edited row is not carried");
    }

    #[test]
    fn a_big_exempt_set_does_not_refuse_an_ordinary_delete() {
        // A 60-row table plus 25 prose bullets, pre-#329 sidecar (one
        // block per table line). The user clears 20 of the 25 bullets in
        // an editor — ordinary section-clearing, and `MAX_ORPHANED_RATIO`
        // is set high precisely to allow it.
        //
        // The table's 60 rows orphan too (they are now one block) and are
        // exempt. Subtracting them from the denominator as well reported
        // "20 of 25, 80%" and refused; the honest figure is 20 of 85.
        let mut rows = vec!["| a | b |".to_string(), "| --- | --- |".to_string()];
        for i in 0..58 {
            rows.push(format!("| {i} | x |"));
        }
        let mut old: Vec<SidecarBlock> = rows
            .iter()
            .enumerate()
            .map(|(i, t)| SidecarBlock::from_text(NodeId::new(), i + 1, 0, t))
            .collect();
        for i in 0..25 {
            old.push(SidecarBlock::from_text(
                NodeId::new(),
                old.len() + 1,
                0,
                format!("prose {i}"),
            ));
        }
        assert_eq!(old.len(), 85);

        let mut md = rows.join("\n");
        md.push('\n');
        for i in 20..25 {
            md.push_str(&format!("- prose {i}\n"));
        }
        let ast = parse(&md);
        match_blocks_guarded_except(&ast.blocks, &old, &OrphanGuard::Enforced, |b| {
            crate::table::row_carried_by(&ast.blocks, &b.text)
        })
        .expect("20 deletions out of 85 known blocks is an ordinary edit");
    }

    #[test]
    fn the_escape_hatch_returns_the_full_orphan_list() {
        // Opting out must delete *everything* the match found — a
        // partial application would be the exact failure the guard
        // exists to prevent, arriving through the door marked "yes".
        let old = old_page(40);
        let ast = parse("");
        let (_, orphans) = match_blocks_guarded(&ast.blocks, &old, &OrphanGuard::Disabled)
            .expect("the explicit opt-in applies the deletion");
        assert_eq!(orphans.len(), 40);
    }
}
