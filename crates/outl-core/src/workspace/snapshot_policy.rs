//! When the workspace writes its boot cache, and what is still in flight.
//!
//! Four fields used to sit on [`Workspace`](super::Workspace) itself:
//! the snapshots directory, the in-flight worker handles, the count of
//! ops since the last write, and the trigger threshold. None of them
//! describe the document. They describe a *policy* about a local cache
//! — a cache that is deliberately not part of the op log, because a
//! snapshot is a projection of state the log already holds.
//!
//! Keeping them here means `Workspace`'s remaining fields are the
//! document (tree, log, content) plus its storage routing, and it means
//! the "should we snapshot now" decision is testable without building a
//! workspace at all.
//!
//! # What it is not
//!
//! It does not decide *what* goes into a snapshot — that is
//! `Workspace::build_snapshot_body`, which needs the tree and the
//! content store. This type answers only *whether* and *where*, and owns
//! the threads once they exist.

use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

use tracing::{debug, warn};

use crate::id::ActorId;
use crate::snapshot::{self, SnapshotBody};

/// Snapshot cadence plus the workers currently writing.
pub(crate) struct SnapshotPolicy {
    /// `<root>/.outl/snapshots`, or `None` for an in-memory workspace
    /// (which has nowhere to write, so every path here no-ops).
    dir: Option<PathBuf>,
    /// Background writers still running. Drained opportunistically on
    /// every trigger so the list stays bounded across a long session,
    /// and joined by [`Self::wait`] on shutdown.
    workers: Vec<JoinHandle<()>>,
    /// Ops applied since the last write.
    ops_since: u32,
    /// Trigger threshold. `0` disables the in-band write entirely — the
    /// CLI sets this, being ephemeral and having no business churning
    /// the snapshots dir.
    threshold: u32,
}

impl SnapshotPolicy {
    /// Default `apply`-count between in-band snapshot writes. Clients
    /// override from `[snapshot]` in `outl.toml` via
    /// [`Self::set_policy`].
    pub(crate) const DEFAULT_THRESHOLD: u32 = 10_000;

    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        SnapshotPolicy {
            dir,
            workers: Vec::new(),
            ops_since: 0,
            threshold: Self::DEFAULT_THRESHOLD,
        }
    }

    /// Where snapshots go, when anywhere.
    pub(crate) fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Whether an in-band write can fire at all: a threshold was asked
    /// for **and** there is somewhere to write.
    fn is_armed(&self) -> bool {
        self.threshold > 0 && self.dir.is_some()
    }

    /// Configure cadence. `enabled = false` opts out; a threshold below
    /// 1 is clamped so we never snapshot on literally every op.
    pub(crate) fn set_policy(&mut self, enabled: bool, threshold: u32) {
        self.threshold = if enabled { threshold.max(1) } else { 0 };
        self.ops_since = 0;
    }

    /// Seed the counter after a boot, so a long-lived workspace opened
    /// without a snapshot on disk does not have to wait a full threshold
    /// of *new* ops before producing one.
    pub(crate) fn seed_from_log_len(&mut self, log_len: usize) {
        self.ops_since = (log_len as u32).min(self.threshold);
    }

    /// Count `applied` ops and answer whether a write should fire now.
    ///
    /// Returns `false` — without counting — when the policy is not
    /// armed, so a disabled policy cannot drift its counter upward and
    /// fire the instant someone enables it.
    #[must_use]
    pub(crate) fn record(&mut self, applied: u32) -> bool {
        if !self.is_armed() {
            return false;
        }
        self.ops_since = self.ops_since.saturating_add(applied);
        if self.ops_since < self.threshold {
            return false;
        }
        self.ops_since = 0;
        // Non-blocking drain, so the handle list does not grow unbounded
        // over a session that snapshots many times.
        self.workers.retain(|h| !h.is_finished());
        true
    }

    /// Hand `body` to a worker thread that writes it, then collects the
    /// siblings that write made unreachable.
    ///
    /// Failure inside the worker is logged and discarded: a snapshot is
    /// a boot cache, never source of truth, so a failed write costs the
    /// next boot some replay time and nothing else.
    ///
    /// The GC runs **here** rather than on boot because this is the
    /// moment the directory gains a file, and because this thread has
    /// already serialized and fsynced a multi-MB body — reading the
    /// remaining candidates to judge them is the same order of magnitude
    /// as the work it just did, off the hot path, once per threshold.
    /// A boot-time sweep would pay that on every launch to reclaim disk
    /// that is not costing anything yet (root `CLAUDE.md` invariant 11:
    /// attribute the cost before letting it decide). The sweep runs even
    /// when the write failed — the directory's existing garbage does not
    /// stop being garbage.
    ///
    /// Workers are **chained**, not parallel: the new one first joins
    /// every worker still in flight, so publications for one actor land
    /// in the order they were requested. Two writers for the same actor
    /// share one `snap-<actor>.bin.tmp`, so running them side by side
    /// let a slower older body rename over a newer one — or truncate the
    /// scratch file the other was still writing — and let the sweep judge
    /// a snapshot that had not finished landing. The join happens on the
    /// worker, so the caller still returns immediately.
    pub(crate) fn spawn_write(&mut self, actor: ActorId, body: SnapshotBody) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        let predecessors = std::mem::take(&mut self.workers);
        let handle = std::thread::Builder::new()
            .name(format!("outl-snapshot-{actor}"))
            .spawn(move || {
                for h in predecessors {
                    if let Err(e) = h.join() {
                        warn!("snapshot worker panicked: {e:?}");
                    }
                }
                if let Err(e) = snapshot::write_to_disk(&dir, &body) {
                    warn!("background snapshot write failed (non-fatal): {e}");
                }
                match snapshot::gc::sweep(&dir, actor) {
                    Ok(removed) if !removed.is_empty() => {
                        debug!(
                            "snapshot gc: collected {} unreachable file(s)",
                            removed.len()
                        );
                    }
                    Ok(_) => {}
                    Err(e) => warn!("snapshot gc skipped (non-fatal): {e}"),
                }
            })
            .expect("spawn snapshot worker");
        self.workers.push(handle);
    }

    /// Block until every in-flight worker finishes.
    ///
    /// Called on graceful shutdown so a long-lived client does not exit
    /// with a write in flight, which would race the process exit and
    /// could leave a stale `.tmp` behind.
    pub(crate) fn wait(&mut self) {
        for h in std::mem::take(&mut self.workers) {
            if let Err(e) = h.join() {
                warn!("snapshot worker panicked: {e:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SnapshotPolicy;
    use crate::hlc::Hlc;
    use crate::id::ActorId;
    use crate::snapshot::{read_from_disk, write_to_disk, SnapshotBody};
    use std::collections::{BTreeMap, BTreeSet};
    use tempfile::TempDir;

    fn body_at(actor: ActorId, high: u64) -> SnapshotBody {
        let mut cutoff = BTreeMap::new();
        cutoff.insert(actor, Hlc::new(high, 0, actor));
        SnapshotBody::from_parts(
            actor,
            cutoff,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeSet::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .expect("test body encodes")
    }

    /// Publishing a snapshot is the moment the directory *gains* a file,
    /// and the worker thread that just serialized and fsynced a multi-MB
    /// body is the cheapest place to notice that an older sibling has
    /// stopped being reachable. A boot-time sweep would read the whole
    /// directory to reclaim disk that is not costing anything yet.
    #[test]
    fn publishing_a_snapshot_collects_the_siblings_it_made_unreachable() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".outl").join("snapshots");
        std::fs::create_dir_all(&dir).unwrap();

        let me = ActorId::new();
        let behind = ActorId::new();
        let ahead = ActorId::new();
        write_to_disk(&dir, &body_at(behind, 100)).unwrap();
        write_to_disk(&dir, &body_at(ahead, 900)).unwrap();

        let mut p = SnapshotPolicy::new(Some(dir.clone()));
        p.spawn_write(me, body_at(me, 1_000));
        p.wait();

        assert!(dir.join(format!("snap-{me}.bin")).exists(), "own written");
        assert!(
            !dir.join(format!("snap-{behind}.bin")).exists(),
            "the candidate the boot selector can never choose again is collected"
        );
        assert!(
            dir.join(format!("snap-{ahead}.bin")).exists(),
            "the one a boot after an actor rotation would adopt survives"
        );
    }

    /// Two writers for one actor share one `snap-<actor>.bin.tmp`. Run
    /// side by side, the slower older body could rename over the newer
    /// one; chained, the last request is the one on disk when the queue
    /// drains.
    #[test]
    fn later_snapshots_publish_after_earlier_ones() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".outl").join("snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        let me = ActorId::new();

        let mut p = SnapshotPolicy::new(Some(dir.clone()));
        for high in 1..=20u64 {
            p.spawn_write(me, body_at(me, high));
        }
        p.wait();

        let on_disk = read_from_disk(&dir, me).unwrap().expect("own snapshot");
        assert_eq!(
            on_disk.cutoff.get(&me).map(|h| h.physical_ms),
            Some(20),
            "the last requested body is the one published"
        );
        assert!(
            !dir.join(format!("snap-{me}.bin.tmp")).exists(),
            "no scratch file left behind"
        );
    }

    fn armed(threshold: u32) -> SnapshotPolicy {
        let mut p = SnapshotPolicy::new(Some("/tmp/nowhere".into()));
        p.set_policy(true, threshold);
        p
    }

    #[test]
    fn fires_only_once_the_threshold_is_crossed() {
        let mut p = armed(3);
        assert!(!p.record(1));
        assert!(!p.record(1));
        assert!(p.record(1), "the third op crosses 3");
        assert!(!p.record(1), "and the counter restarts");
    }

    #[test]
    fn a_batch_that_overshoots_fires_once() {
        // The batch path calls `record` once with the whole batch size;
        // it must not owe several snapshots for one commit.
        let mut p = armed(3);
        assert!(p.record(50));
        assert!(!p.record(1));
    }

    #[test]
    fn an_in_memory_workspace_never_fires() {
        let mut p = SnapshotPolicy::new(None);
        p.set_policy(true, 1);
        assert!(!p.is_armed(), "nowhere to write is not armed");
        assert!(!p.record(1_000));
    }

    #[test]
    fn disabling_stops_it_firing() {
        let mut p = armed(2);
        p.set_policy(false, 2);
        assert!(!p.is_armed());
        assert!(!p.record(1_000));
    }

    /// A disabled policy must not accumulate, or enabling it later fires
    /// immediately on a counter nobody was watching.
    #[test]
    fn a_disabled_policy_does_not_bank_ops() {
        let mut p = armed(5);
        p.set_policy(false, 5);
        let _ = p.record(1_000);
        p.set_policy(true, 5);
        assert!(!p.record(1), "the disabled run must not have counted");
    }

    #[test]
    fn seeding_from_the_log_is_capped_at_the_threshold() {
        let mut p = armed(10);
        p.seed_from_log_len(1_000_000);
        assert!(
            p.record(1),
            "a log already past the threshold snapshots on the next op"
        );
    }

    #[test]
    fn seeding_below_the_threshold_still_waits() {
        let mut p = armed(10);
        p.seed_from_log_len(4);
        assert!(!p.record(5), "4 + 5 is still short of 10");
        assert!(p.record(1));
    }
}
