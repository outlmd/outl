//! The executor: re-project the pages [`super::survey_page_projections`]
//! selected.
//!
//! ## What the executor will and will not write
//!
//! It writes a page whose re-projection removes **nothing** from disk,
//! and only that. A write that would remove content lines — a peer
//! genuinely deleted blocks — is [`ReprojectionSweep::withheld`] and
//! belongs to `outl doctor --repair`, which backs every file up first
//! and applies volume ceilings. A background pass that deletes is a
//! background pass that needs an undo, and the command with the undo
//! already exists.
//!
//! That gate is also what makes a **torn op log** safe here without this
//! module knowing anything about op-log health: a truncated replay
//! renders *less* than the disk holds, so every page it touches reports
//! `lines_removed > 0` and is withheld. (Root `CLAUDE.md` invariant 8's
//! precedence order — a damaged log is reported as a damaged log — stays
//! with `outl doctor`, which is the surface that can diagnose it.)
//!
//! ## Refusals are returned, never swallowed
//!
//! A page whose `.md` holds content the op log never saw is frozen in
//! both directions until `outl reconcile --ahead-of-log` runs. Every
//! refusal lands in [`ReprojectionSweep::refused`] with its
//! [`crate::ActionError`] so the caller can name the page — invariant 8,
//! and [`docs/clients.md` → Surfacing a page that stopped
//! syncing](../../../../../docs/clients.md#surfacing-a-page-that-stopped-syncing).
//!
//! ## A skip is not automatically silent
//!
//! Most states this pass declines to write belong to someone who *will*
//! act: an unreconciled external edit and a withheld hash are both the
//! `.md → tree` watcher's next job, and a sidecar that cannot vouch for
//! its page is named by `outl doctor`. Those are quiet on purpose.
//!
//! A `.md` the pass could not read is not one of them — nothing else is
//! going to notice it — so it lands in [`ReprojectionSweep::unreadable`]
//! with the reason, alongside a page whose bytes are simply not on this
//! device yet.

use std::path::{Path, PathBuf};

use outl_core::workspace::Workspace;

use super::{survey_page_projections, PageProjectionState};
use crate::journal::apply::{apply_page_md_with_sidecar_if_stale, ProjectionFailure};

/// One page the sweep could not look at.
#[derive(Debug, Clone)]
pub struct UnreadablePage {
    /// The `.md` that was skipped.
    pub path: PathBuf,
    /// Why its bytes could not be read — an I/O error, or the evidence
    /// that they are simply not on this device yet.
    pub reason: String,
}

/// One page the executor deliberately did not write.
#[derive(Debug, Clone)]
pub struct WithheldPage {
    /// The `.md` left untouched.
    pub path: PathBuf,
    /// Content lines the write would have removed from disk.
    pub lines_removed: usize,
}

/// What one [`reproject_stale_pages`] pass did.
#[derive(Debug, Default)]
pub struct ReprojectionSweep {
    /// Pages classified — the denominator for every count below.
    pub surveyed: usize,
    /// `.md` paths re-projected from the op log.
    pub written: Vec<PathBuf>,
    /// Stale pages whose write would have removed content. Left for
    /// `outl doctor --repair`, which backs up first.
    pub withheld: Vec<WithheldPage>,
    /// Pages the write guard refused. Each one has **stopped syncing**
    /// and the caller owes the user its name (invariant 8).
    pub refused: Vec<ProjectionFailure>,
    /// Pages whose `.md` the sweep could not read: an I/O error, or
    /// bytes that are not on this device yet (an iCloud placeholder, a
    /// `.md` gone from beside its sidecar). Each is a skip the user
    /// cannot otherwise see — the failure invariant 8 exists to prevent.
    pub unreadable: Vec<UnreadablePage>,
    /// Selected by the survey and declined by the guard anyway: the page
    /// changed between the two reads, or the two disagree. Never silent
    /// — a skip nobody can see is the failure invariant 8 exists to
    /// prevent.
    pub declined: Vec<PathBuf>,
}

/// Re-project every page whose `.md` is a stale projection the op log
/// has moved past — and **only** where that write removes nothing.
///
/// This is the `tree → .md` executor. The `.md → tree` direction has had
/// one for as long as `outl serve` has existed; this side had none, so
/// ops arriving by sync left the `.md` wrong until a human opened the
/// page.
///
/// The survey selects; [`apply_page_md_with_sidecar_if_stale`] decides.
/// Calling it again rather than trusting the survey is deliberate: it
/// re-reads the file under the page lock, so a page that changed between
/// the two is refused rather than raced, and there is exactly one owner
/// of "may this `.md` be overwritten".
pub fn reproject_stale_pages(workspace: &Workspace, root: &Path) -> ReprojectionSweep {
    let mut sweep = ReprojectionSweep::default();
    for page in survey_page_projections(workspace, root, false) {
        sweep.surveyed += 1;
        match page.state {
            // Someone else's job, and a job that gets done: the
            // `.md → tree` watcher next door reconciles an external edit
            // and restamps a withheld hash, and `outl doctor` names a
            // sidecar that cannot vouch for its page. Nothing here is a
            // page that silently stopped converging.
            //
            // **That last sentence was false for two releases.** The
            // quiet is only earned because an unanswerable sidecar gets
            // rewritten *with* text by the orphan scan's reconcile, which
            // arms the real check. A page holding only bare bullets never
            // gets there: being queued is not the problem — the observed
            // page carried `pipeline_version` 4 against a current 5, so it
            // was queued — but its blocks are genuinely empty, so the
            // reconcile rewrites the same `text: ""` and the page is
            // refused again, forever (issue #332). The self-healing is
            // structurally unable to reach this class, so the gate has to
            // tell it apart up front: `sidecar_can_vouch_for`.
            PageProjectionState::InSync
            | PageProjectionState::SidecarMissingButFaithful
            | PageProjectionState::PendingExternalEdit
            | PageProjectionState::SidecarMissingAndDrifted
            | PageProjectionState::SidecarCannotAnswer
            | PageProjectionState::HashWithheldButClean => {}
            // A `.md` the pass could not look at **is** such a page, and
            // it used to fold into the arm above — so `outl serve` said
            // nothing at all about it.
            PageProjectionState::Unreadable { error } => sweep.unreadable.push(UnreadablePage {
                path: page.path,
                reason: error,
            }),
            PageProjectionState::MarkdownNotHereYet { reason } => {
                sweep.unreadable.push(UnreadablePage {
                    path: page.path,
                    reason,
                })
            }
            PageProjectionState::Stale { lines_removed } if lines_removed > 0 => {
                sweep.withheld.push(WithheldPage {
                    path: page.path,
                    lines_removed,
                });
            }
            // `AheadOfLog` is handed to the guard too, precisely so the
            // refusal that reaches the caller is the guard's own error
            // and not a second opinion phrased here.
            PageProjectionState::Absent
            | PageProjectionState::Stale { .. }
            | PageProjectionState::AheadOfLog { .. } => {
                match apply_page_md_with_sidecar_if_stale(workspace, root, page.page_root) {
                    Ok(Some(path)) => sweep.written.push(path),
                    Ok(None) => sweep.declined.push(page.path),
                    Err(error) => sweep.refused.push(ProjectionFailure {
                        path: page.path,
                        error,
                    }),
                }
            }
        }
    }
    sweep
}
