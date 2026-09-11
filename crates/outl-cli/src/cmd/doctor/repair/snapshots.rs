//! `--repair`'s snapshot half: which boot snapshots go, and how.
//!
//! `outl_core::snapshot::gc` owns the verdict — see
//! [RFC 0258](../../../../../../docs/rfcs/0258-snapshot-cache-lifecycle.md).
//! Nothing here re-derives it; the plan is a listing and the GC is
//! re-asked at write time.

use std::path::{Path, PathBuf};

use outl_core::id::ActorId;
use outl_core::snapshot::gc::{self, SnapshotVerdict};

use super::{backup, RepairAction};

/// One `snap-*.bin` the repair pass may drop, and the GC's reason.
///
/// The reason is [`SnapshotVerdict`] itself, not a string this module
/// invents: `outl_core::snapshot::gc` owns why a snapshot may go, and a
/// second vocabulary here is how the report starts describing a
/// different operation than the one that runs.
#[derive(Debug, Clone)]
pub(in crate::cmd::doctor) struct SnapshotDrop {
    /// The file.
    pub path: PathBuf,
    /// Why it may go.
    pub verdict: SnapshotVerdict,
    /// Size when it was surveyed, so the report can say what is
    /// reclaimed.
    pub bytes: u64,
}

impl SnapshotDrop {
    /// The `repairable[]` line for this drop.
    ///
    /// The verdict is spelled out rather than summarised as "corrupt":
    /// `--repair` now also reclaims snapshots that are perfectly
    /// readable and simply unreachable, and a user reading a deletion
    /// list about their notes directory is owed the difference.
    pub(in crate::cmd::doctor) fn describe(&self) -> String {
        let why = match self.verdict {
            SnapshotVerdict::Superseded => {
                "superseded — it decodes, but another snapshot outranks it, so the boot \
                 selector can never choose it again"
            }
            SnapshotVerdict::Unusable => {
                "unusable — read end to end, and this build cannot decode it"
            }
            // Unreachable: nothing else is ever planned. Phrased as a
            // verdict rather than an `unreachable!` because a panic
            // inside a report is a worse bug than a vague line.
            SnapshotVerdict::Own | SnapshotVerdict::Selected | SnapshotVerdict::Inconclusive => {
                "prunable"
            }
        };
        format!(
            "delete boot snapshot {} ({} bytes reclaimed): {why}. Pure cache — the op log is \
             the source of truth, so no notes are in it and the only cost is one slower boot \
             (backup first)",
            self.path.display(),
            self.bytes
        )
    }
}

/// Drop the planned snapshots, re-asking the GC first.
///
/// **The plan is a listing, not an authorisation.** `outl_core::snapshot::gc`
/// owns "may this file go", and the collection pass ran before the
/// workspace was even opened — since then a peer pull could have
/// published a fresh body at that path, a co-resident process could have
/// written its own, and the boot cache GC could have collected the file
/// during the open. So this re-surveys and acts on what the owner says
/// *now*, the same shape `prune_binding` uses against
/// `DeviceStore::prune_binding`.
///
/// Every file is copied into the backup generation before it goes, so
/// "delete" stays reversible even for a file we believe is dead cache.
pub(super) fn drop_snapshots(
    root: &Path,
    backup_dir: &Path,
    actor: ActorId,
    planned: &[SnapshotDrop],
) -> Vec<RepairAction> {
    let action = |path: &Path, ok: bool, detail: String| RepairAction {
        kind: "delete_snapshot".to_string(),
        path: path.display().to_string(),
        ok,
        detail,
    };
    if planned.is_empty() {
        return Vec::new();
    }
    let dir = root.join(".outl").join("snapshots");
    let survey = match gc::survey(&dir, actor) {
        Ok(survey) => survey,
        Err(e) => {
            return planned
                .iter()
                .map(|p| {
                    action(
                        &p.path,
                        false,
                        format!("could not re-read the directory: {e}"),
                    )
                })
                .collect()
        }
    };
    let prunable: Vec<&gc::SnapshotEntry> = survey.prunable().collect();

    planned
        .iter()
        .map(|planned| {
            let Some(entry) = prunable.iter().find(|e| e.path == planned.path) else {
                // Not a failure: the evidence simply expired. The
                // commonest cause is benign and worth naming — the boot
                // cache GC collects this device's own unusable snapshot
                // while the workspace opens, which is *after* the
                // collection pass read this directory.
                return action(
                    &planned.path,
                    true,
                    "left in place — re-judged and no longer prunable, or already collected \
                     by the boot cache GC"
                        .to_string(),
                );
            };
            let backed_up = match backup(root, backup_dir, &entry.path) {
                Ok(Some(dest)) => dest.display().to_string(),
                Ok(None) => "nothing to back up".to_string(),
                Err(e) => {
                    return action(
                        &planned.path,
                        false,
                        format!("backup failed, file left alone: {e}"),
                    )
                }
            };
            match gc::prune(entry) {
                Ok(true) => action(&planned.path, true, format!("backup: {backed_up}")),
                // `gc::prune` re-stats and refuses when the bytes moved
                // since the survey. Nothing is wrong; a fresh body landed.
                Ok(false) => action(
                    &planned.path,
                    true,
                    "left in place — the file changed since the scan".to_string(),
                ),
                Err(e) => action(&planned.path, false, format!("could not remove: {e}")),
            }
        })
        .collect()
}

/// Delete one abandoned `snap-*.bin.tmp`.
///
/// Not backed up, for the same reason `prune_scratch` is not: a scratch
/// file is a write that never published. Its content became a snapshot
/// or it did not, and if it did, the snapshot is already there.
/// `gc::prune_tmp` re-checks both the age and that the path really sits
/// in a snapshots directory before unlinking.
pub(super) fn prune_snapshot_tmp(path: &Path) -> RepairAction {
    let (kind, shown) = ("prune_snapshot_tmp".to_string(), path.display().to_string());
    match gc::prune_tmp(path, gc::STALE_TMP_TTL) {
        Ok(true) => RepairAction {
            kind,
            path: shown,
            ok: true,
            detail: "half-written snapshot, never published".to_string(),
        },
        Ok(false) => RepairAction {
            kind,
            path: shown,
            ok: true,
            detail: "left in place — it was touched since the scan".to_string(),
        },
        Err(e) => RepairAction {
            kind,
            path: shown,
            ok: false,
            detail: format!("could not remove: {e}"),
        },
    }
}
