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

use tracing::warn;

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

    /// Hand `body` to a worker thread that writes it.
    ///
    /// Failure inside the worker is logged and discarded: a snapshot is
    /// a boot cache, never source of truth, so a failed write costs the
    /// next boot some replay time and nothing else.
    pub(crate) fn spawn_write(&mut self, actor: ActorId, body: SnapshotBody) {
        let Some(dir) = self.dir.clone() else {
            return;
        };
        let handle = std::thread::Builder::new()
            .name(format!("outl-snapshot-{actor}"))
            .spawn(move || {
                if let Err(e) = snapshot::write_to_disk(&dir, &body) {
                    warn!("background snapshot write failed (non-fatal): {e}");
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
