//! `--repair`'s index-sidecar half: which `ops/` caches go, and why they
//! are the one thing in this plan that is **not** backed up first.
//!
//! `outl_core::storage::sidecar::gc` owns the verdict — see
//! [RFC 0265](../../../../../../docs/rfcs/0265-index-sidecar-lifecycle.md).
//! Nothing here re-derives it; the plan is a listing and the GC is re-asked
//! at delete time.
//!
//! # No backup, unlike every other deletion here
//!
//! `snapshots.rs` copies a snapshot into the backup generation before
//! dropping it, and this deliberately does not. Three reasons, in order of
//! weight:
//!
//! 1. **Nothing can read what we would be saving.** Every file here is
//!    either a name no code path composes any more, or a temp that never
//!    became an index. Copying it produces a file that is still
//!    unreadable, one directory further away.
//! 2. **It is reconstructible from a file sitting next to it.** A snapshot
//!    encodes a *materialized tree* that a restore would otherwise have to
//!    replay for; an offset index is a byte map of a `.jsonl` that is
//!    right there and unchanged.
//! 3. **The volume inverts the point of a backup.** The run that motivated
//!    this reclaims 134 MB on a real workspace. Copying that into
//!    `.outl/repair-backup/` would double the disk the user is complaining
//!    about, and hand the backup pruner a job it did not need.
//!
//! The cost of being wrong is one slower boot, and that is the whole cost.

use std::path::{Path, PathBuf};

use outl_core::storage::sidecar::gc::{self, SidecarVerdict};

use super::super::Builder;
use super::RepairAction;

/// One `ops/` cache file the repair pass may drop, and the GC's reason.
///
/// The reason is [`SidecarVerdict`] itself, not a string this module
/// invents: `sidecar::gc` owns why a file may go, and a second vocabulary
/// here is how a report starts describing a different operation than the
/// one that runs.
#[derive(Debug, Clone)]
pub(in crate::cmd::doctor) struct IndexSidecarDrop {
    /// The file.
    pub path: PathBuf,
    /// Why it may go.
    pub verdict: SidecarVerdict,
    /// Size when it was surveyed, so the report can say what is reclaimed.
    pub bytes: u64,
}

impl IndexSidecarDrop {
    /// The `repairable[]` line for this drop.
    pub(in crate::cmd::doctor) fn describe(&self) -> String {
        format!(
            "delete index sidecar {} ({} bytes reclaimed): {}. Pure cache — it is rebuilt from \
             the `.jsonl` next to it, so the only cost of being wrong is one slower boot, and \
             it is not backed up because nothing could read the copy",
            self.path.display(),
            self.bytes,
            self.verdict.reason(),
        )
    }
}

/// Survey `ops/` for dead index caches, report them, and return the plan.
///
/// Read-only, and called **before** storage is opened — a boot rebuilds
/// sidecars, so surveying afterwards would judge files this command
/// created.
pub(in crate::cmd::doctor) fn collect_dead(
    b: &mut Builder,
    ops_dir: &Path,
) -> Vec<IndexSidecarDrop> {
    let survey = match gc::survey(ops_dir) {
        Ok(s) => s,
        Err(e) => {
            // A directory we could not read says nothing about its
            // contents. Never a finding against the workspace.
            b.info(format!("could not survey index sidecars: {e}"));
            return Vec::new();
        }
    };
    let drops: Vec<IndexSidecarDrop> = survey
        .prunable()
        .map(|e| IndexSidecarDrop {
            path: e.path.clone(),
            verdict: e.verdict,
            bytes: e.bytes,
        })
        .collect();
    if drops.is_empty() {
        return drops;
    }
    let bytes: u64 = drops.iter().map(|d| d.bytes).sum();
    // A warning, not an error: nothing is broken. But it is not an `info`
    // either — the undotted generation sits in `ops/`, which is
    // deliberately not a dotfile, so a file-sync transport has been
    // carrying every byte of it to every other device.
    b.warn(format!(
        "{} dead index sidecar file(s) in `ops/` holding {bytes} bytes — caches no code path \
         reads, left behind by a rename and by writes that were killed before they published. \
         `outl doctor --repair` removes them",
        drops.len(),
    ));
    drops
}

/// The paths a run has announced it will collect.
///
/// Handed to `OpsDirGuard::capture` so the guard neither restores them
/// nor holds their bytes in memory. Derived here rather than at the call
/// site: which files the exception covers is this module's fact.
pub(in crate::cmd::doctor) fn announced_paths(drops: &[IndexSidecarDrop]) -> Vec<PathBuf> {
    drops.iter().map(|d| d.path.clone()).collect()
}

/// Say how many per-actor write locks `ops/` is carrying.
///
/// **Info, and never a deletion.** A `.lock-<actor>` is arbitration
/// state, not cache: `ActorWriteLock` flocks that path, and removing one
/// a live process holds lets the next process create a fresh inode, flock
/// it, and write to the same `ops-<actor>.jsonl`. Every one of them is
/// also 0 bytes, so there is no reclaim to weigh against that (root
/// `CLAUDE.md` invariant 11).
///
/// It is still worth printing, because the *count* is a real signal: each
/// lock is an actor this device has written under, so far more locks than
/// op logs means the workspace has been minting ephemeral actors — the
/// symptom a permanently-running process produces.
pub(in crate::cmd::doctor) fn report_actor_locks(b: &mut Builder, ops_dir: &Path) {
    let Ok(read) = std::fs::read_dir(ops_dir) else {
        return;
    };
    let (mut locks, mut logs) = (0usize, 0usize);
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(".lock-") {
            locks += 1;
        } else if name.starts_with("ops-") && name.ends_with(".jsonl") {
            logs += 1;
        }
    }
    if locks == 0 {
        return;
    }
    b.info(format!(
        "{locks} per-actor write lock file(s) next to {logs} op log(s). They are 0-byte \
         arbitration state, not cache, so nothing removes them — but a count well above the \
         number of op logs means this device has been minting ephemeral actors"
    ));
}

/// Drop the planned sidecars, re-asking the GC first.
///
/// **The plan is a listing, not an authorisation.** The collection pass
/// ran before the workspace was opened; since then a peer sync could have
/// landed a file at that path, and the boot this command performed
/// rebuilt the live sidecars. So this re-surveys and acts on what the
/// owner says *now* — the same shape `snapshots::drop_snapshots` uses.
pub(super) fn drop_index_sidecars(
    ops_dir: &Path,
    planned: &[IndexSidecarDrop],
) -> Vec<RepairAction> {
    let action = |path: &Path, ok: bool, detail: String| RepairAction {
        kind: "prune_index_sidecar".to_string(),
        path: path.display().to_string(),
        ok,
        detail,
    };
    if planned.is_empty() {
        return Vec::new();
    }
    let survey = match gc::survey(ops_dir) {
        Ok(s) => s,
        Err(e) => {
            return planned
                .iter()
                .map(|p| action(&p.path, false, format!("could not re-read `ops/`: {e}")))
                .collect()
        }
    };
    let prunable: Vec<&gc::SidecarEntry> = survey.prunable().collect();

    planned
        .iter()
        .map(|planned| {
            let Some(entry) = prunable.iter().find(|e| e.path == planned.path) else {
                // Not a failure: the evidence expired. A file that came
                // back, or one another process already collected.
                return action(
                    &planned.path,
                    true,
                    "left in place — re-judged and no longer prunable".to_string(),
                );
            };
            match gc::prune(entry) {
                Ok(true) => action(
                    &planned.path,
                    true,
                    format!(
                        "{} bytes reclaimed (not backed up: pure cache)",
                        planned.bytes
                    ),
                ),
                // `gc::prune` re-stats and refuses when the bytes moved
                // since the survey.
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
