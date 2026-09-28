//! What takes a page write back out of the plan.
//!
//! `tree::check_projections` answers which pages disagree with the op
//! log. That is not the same question as which of them `--repair` may
//! rewrite, and the two conditions that withdraw the permission live
//! here:
//!
//! - the op log is itself damaged, so the tree replayed from it has no
//!   authority over the `.md` it would overwrite;
//! - the write would remove more content than a pass nobody is watching
//!   should remove without being asked. [`RepairScope`] is the user's
//!   answer to that one, and it is only ever `Forced` from an explicit
//!   flag.
//!
//! Both clear the same two lists out of the [`Plan`], and both say so
//! in the report. A gate that suppressed quietly would leave the
//! read-only listing offering a repair the writing pass then refuses,
//! which is the divergence that listing exists to prevent.

use super::oplog::OpLogHealth;
use super::{Builder, Plan, RepairVolume};

/// How much authority a `--repair` run carries.
///
/// Re-projection removes content by design — a peer deleted a block, the
/// log is right, the `.md` is behind. What it must not do is remove
/// *thousands* of lines because something systemic is wrong, print a
/// page count, and leave nothing to compare against afterwards. So the
/// volume is measured first and a large one needs a second, deliberate
/// act.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepairScope {
    /// Default. Page writes stand down when the measured volume is past
    /// [`RepairVolume::needs_confirmation`]; everything else still runs.
    #[default]
    Guarded,
    /// `--force`: the user has read the count and authorised it.
    ///
    /// Reachable only from an explicit flag. Never a retry, never a
    /// fallback — "attempted twice" is not consent.
    Forced,
}

/// Every projection repair writes a `.md` rendered from
/// the materialized tree, and the tree is only ever as complete as
/// the op log it replayed. `JsonlStorage` skips unreadable records
/// by design, so a damaged log boots a **truncated** tree that
/// looks perfectly healthy from the inside — and a `.md` that is
/// still a faithful projection of the whole page then reads as
/// "stale", because the truncated tree renders less than it.
///
/// Repairing that overwrites the user's content with the render of
/// a log we just told them is broken. The op log is the source of
/// truth (root `CLAUDE.md` invariant 1); when it is damaged it has
/// no authority over the projection, and the correct move is to
/// stop and recover the log first.
///
/// Snapshot deletion stays allowed: it is a pure boot cache, and
/// dropping it only forces the full replay a damaged log wants
/// anyway.
pub(super) fn check_damaged_log(b: &mut Builder, plan: &mut Plan, health: &OpLogHealth) {
    let held_back = plan.reproject.len() + plan.rebuild_sidecar.len();
    if health.is_compromised() && held_back > 0 {
        plan.reproject.clear();
        plan.rebuild_sidecar.clear();
        b.warn(format!(
            "{held_back} page repair(s) suppressed — the op log is damaged, so the tree replayed \
             from it may be missing blocks, and re-projecting a page would overwrite a good \
             `.md` with an incomplete render. Recover the op log first (restore `ops/` from a \
             backup, or let a healthy peer sync it back), then re-run. Reasons: {}",
            health.compromised_by.join("; ")
        ));
    }
}

/// The volume guard. Everything above decided *whether* a page
/// may be rewritten; this decides whether the total is small
/// enough to happen without being asked.
///
/// Re-projection removes content legitimately — a peer deleted a
/// block and this device is behind — so the gate cannot be "any
/// deletion". It is the scale: on a healthy workspace the total
/// is single digits, and the run that motivated this removed
/// 1,426 lines from 233 pages while printing `708 fixed`
/// (RFC 0210). A destructive operation that scales silently turns
/// a small bug into an unrecoverable one.
///
/// Announced in **both** modes and suppressed only when actually
/// repairing, so the read-only listing keeps naming what it found
/// and states the condition attached to it — rather than offering
/// a repair `--repair` then silently refuses.
pub(super) fn check_volume(b: &mut Builder, plan: &mut Plan, do_repair: bool, scope: RepairScope) {
    let volume = plan.volume();
    if volume.is_destructive() {
        let (max_pages, max_lines) = RepairVolume::ceilings();
        if volume.needs_confirmation() {
            let held_back = plan.reproject.len() + plan.rebuild_sidecar.len();
            if do_repair && scope == RepairScope::Guarded {
                plan.reproject.clear();
                plan.rebuild_sidecar.clear();
            }
            let tail = match (do_repair, scope) {
                (true, RepairScope::Guarded) => format!(
                    "{held_back} page repair(s) suppressed — re-run with \
                     `outl doctor --repair --force` once the list above reads right"
                ),
                (true, RepairScope::Forced) => "proceeding: `--force` was given".to_string(),
                (false, _) => {
                    "`outl doctor --repair` will refuse this without `--force`".to_string()
                }
            };
            b.err(format!(
                "`--repair` would remove {} content line(s) from {} page(s), past the \
                 point this runs unattended (ceilings: {max_lines} line(s), {max_pages} \
                 page(s)). Every write is backed up under `.outl/repair-backup/`, but a \
                 deletion this size is a decision, not a repair. {tail}",
                volume.lines_removed, volume.pages_losing_content,
            ));
        } else {
            b.warn(format!(
                "`--repair` would remove {} content line(s) from {} page(s) — under the \
                 ceilings ({max_lines} line(s), {max_pages} page(s)), so it runs without \
                 `--force`",
                volume.lines_removed, volume.pages_losing_content,
            ));
        }
    }
}
