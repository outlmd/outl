//! What waits for the first frame, and what is allowed to wait for it.
//!
//! A cold launch has two consumers of the same CPU and the same disk: the
//! work the user is waiting for (open the workspace, paint today's
//! journal) and the work that makes the app a *synced* app (bind the iroh
//! endpoint, start the filesystem watcher, load the plugin host, take a
//! backup, sweep orphan `.md`, re-persist the boot cache). The second set
//! is not optional and not urgent, and every item in it used to start the
//! moment the workspace was published — which on a phone is the same
//! moment the webview is laying out the first screen.
//!
//! So this gate exists, and the rule it encodes is narrow: **deferred
//! means deferred until the user has something on screen, never
//! cancelled.**
//!
//! # Why a deadline is not a nicety
//!
//! The painted signal comes from the webview, over IPC. A webview that
//! failed to load, a frontend built before this signal existed, a
//! `boot_mark` lost to a reload mid-boot — each leaves the signal
//! unsent, and without a deadline that is an app that syncs with nobody,
//! silently, forever. That is the failure mode root `CLAUDE.md`
//! invariant 10 item 2 names: "degrades gracefully" is a claim until the
//! path is written down and reachable.
//!
//! [`FirstPaintGate::arm_deadline`] is that path. It is armed by the
//! client at boot, next to the opener, so the gate cannot be constructed
//! without one in review.
//!
//! # Why the callbacks run on their own thread
//!
//! `painted()` is called from a Tauri command, i.e. the IPC thread. The
//! queued work spawns threads, takes the workspace lock and touches the
//! network stack. Running it inline would put all of that in front of
//! the next keystroke the webview sends — the stall this gate exists to
//! move, relocated one layer up rather than removed (invariant 9).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use tracing::{info, warn};

/// One deferred job: a label for the log, and the work.
type Job = (&'static str, Box<dyn FnOnce() + Send + 'static>);

/// What the gate is holding: jobs not yet released, or the channel that
/// feeds the one worker running them.
enum Gate {
    /// Closed. Jobs accumulate in registration order.
    Closed(Vec<Job>),
    /// Open. A single worker drains this, so a job registered **after**
    /// the gate opened still runs on that one thread, in order.
    ///
    /// This is not a detail. Both clients register their jobs *after*
    /// the open returns, and the deadline can fire before that on a
    /// workspace slow enough to need the gate at all — so "registered
    /// onto an already-open gate" is the ordinary path, not the edge
    /// case. Spawning per job there put four threads on the workspace
    /// mutex at once, which is the contention the gate exists to avoid.
    Open(mpsc::Sender<Job>),
}

/// Runs queued boot work once the client reports a painted frame, or
/// once the deadline elapses — whichever comes first, and exactly once.
pub struct FirstPaintGate {
    state: Mutex<Gate>,
    /// Whether the client's signal (rather than the deadline) opened it.
    /// Reported in the log, because "we waited" and "we gave up waiting"
    /// are different launches and the difference is a bug report.
    by_client: AtomicBool,
}

impl FirstPaintGate {
    /// A closed gate with nothing queued.
    pub fn new() -> Arc<Self> {
        Arc::new(FirstPaintGate {
            state: Mutex::new(Gate::Closed(Vec::new())),
            by_client: AtomicBool::new(false),
        })
    }

    /// Queue `job` to run when the gate opens, or run it now if it
    /// already has.
    ///
    /// `label` names the work in the log. **Jobs run in registration
    /// order, on one thread**, whether they were queued before the gate
    /// opened or handed to it afterwards — so two of them can never
    /// contend for the workspace mutex against each other.
    pub fn on_open<F>(self: &Arc<Self>, label: &'static str, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let mut state = self.lock_state();
        match &mut *state {
            Gate::Closed(jobs) => jobs.push((label, Box::new(job))),
            // Already open: the worker owns the ordering now. A send
            // that fails means the worker is gone (it panicked, or the
            // process is shutting down); running the job inline is the
            // honest fallback, since dropping it would strand sync.
            Gate::Open(tx) => {
                if let Err(mpsc::SendError((label, job))) = tx.send((label, Box::new(job))) {
                    drop(state);
                    warn!("interactive-phase worker is gone; running {label} inline");
                    job();
                }
            }
        }
    }

    /// Take the state lock, resuming through a poisoned one.
    ///
    /// Poisoning is close to unreachable: jobs run on the worker
    /// thread, outside this lock, so nothing foreign panics under it.
    /// If it happened anyway the state behind it is still a readable
    /// `Gate`, and stranding sync behind a mutex would be the worse
    /// answer.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, Gate> {
        match self.state.lock() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        }
    }

    /// The client reported a painted frame. Opens the gate.
    ///
    /// Idempotent: a frontend that reports twice (a reload re-runs its
    /// boot sequence) does not re-run the queue.
    pub fn painted(self: &Arc<Self>) {
        self.by_client.store(true, Ordering::Relaxed);
        self.open("first paint");
    }

    /// Open the gate `after` regardless of whether the client reported.
    ///
    /// Call once, at boot. The thread exits as soon as it has opened the
    /// gate; if the client got there first, opening is a no-op.
    pub fn arm_deadline(self: &Arc<Self>, after: Duration) {
        let gate = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("outl-first-paint-deadline".to_string())
            .spawn(move || {
                std::thread::sleep(after);
                if gate.by_client.load(Ordering::Relaxed) {
                    return;
                }
                gate.open("deadline (no painted frame reported)");
            });
        if let Err(e) = spawned {
            // Nothing deferred may be lost to a thread that would not
            // start, so open now and take the stall over the silence.
            warn!("first-paint deadline thread failed to start ({e}); opening gate now");
            self.open("deadline thread unavailable");
        }
    }

    fn open(&self, why: &str) {
        let queued = {
            let mut state = self.lock_state();
            match &mut *state {
                Gate::Closed(jobs) => {
                    let queued = std::mem::take(jobs);
                    // Install the worker's channel **under the same
                    // lock** that drained the queue, so a job arriving
                    // between the two cannot be dropped or reordered
                    // ahead of what was already waiting.
                    match start_worker(queued) {
                        Some((tx, count)) => {
                            *state = Gate::Open(tx);
                            count
                        }
                        None => {
                            // The worker would not start. Leave the gate
                            // closed rather than accepting jobs into a
                            // channel nothing drains.
                            warn!("interactive-phase worker failed to start; gate stays closed");
                            return;
                        }
                    }
                }
                // Already open. Not a warning: the deadline racing a
                // painted frame is the ordinary case.
                Gate::Open(_) => return,
            }
        };
        // Said even when the queue was empty, because that is the
        // interesting case: it means the clients have not registered
        // yet, and everything they register next runs immediately.
        info!("boot: interactive phase opened by {why}, {queued} job(s) waiting");
    }
}

/// Start the one worker that drains every deferred job, seeded with
/// whatever was already queued. Returns its sender and that seed count.
fn start_worker(queued: Vec<Job>) -> Option<(mpsc::Sender<Job>, usize)> {
    let count = queued.len();
    let (tx, rx) = mpsc::channel::<Job>();
    let spawned = std::thread::Builder::new()
        .name("outl-interactive-phase".to_string())
        .spawn(move || {
            for (label, job) in queued.into_iter().chain(rx) {
                let at = std::time::Instant::now();
                job();
                info!(
                    "boot: deferred job {label} started in {}ms",
                    at.elapsed().as_millis()
                );
            }
        });
    match spawned {
        Ok(_) => Some((tx, count)),
        Err(e) => {
            warn!("interactive-phase thread failed to start: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// How long a test waits for the gate's worker thread. Generous: it
    /// bounds a hang, it is not a measurement.
    const WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn a_queued_job_waits_for_the_painted_signal() {
        let gate = FirstPaintGate::new();
        let (tx, rx) = mpsc::channel();
        gate.on_open("test", move || {
            let _ = tx.send(());
        });
        assert!(
            rx.try_recv().is_err(),
            "the job ran before anything opened the gate"
        );
        gate.painted();
        assert!(rx.recv_timeout(WAIT).is_ok(), "the job never ran");
    }

    /// The property the whole module exists for: an unreported frame
    /// must not be able to cancel sync, only delay it.
    #[test]
    fn the_deadline_opens_a_gate_the_client_never_signals() {
        let gate = FirstPaintGate::new();
        let (tx, rx) = mpsc::channel();
        gate.on_open("test", move || {
            let _ = tx.send(());
        });
        gate.arm_deadline(Duration::from_millis(10));
        assert!(
            rx.recv_timeout(WAIT).is_ok(),
            "the deadline did not open the gate"
        );
    }

    #[test]
    fn a_job_queued_after_the_gate_opened_runs_immediately() {
        let gate = FirstPaintGate::new();
        gate.painted();
        let (tx, rx) = mpsc::channel();
        gate.on_open("late", move || {
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(WAIT).is_ok(),
            "a late job was queued onto an open gate and never ran"
        );
    }

    /// **The ordering the clients actually use.** Both register their
    /// jobs after the workspace open returns, and on a workspace slow
    /// enough to need the gate the deadline has already fired by then —
    /// so every job lands on an open gate. They still have to run one
    /// at a time, in order, or they contend for the workspace mutex,
    /// which is the stall the gate exists to remove.
    #[test]
    fn jobs_registered_after_the_gate_opened_still_run_one_at_a_time() {
        let gate = FirstPaintGate::new();
        gate.painted();

        let live = Arc::new(AtomicBool::new(false));
        let overlapped = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();

        for label in ["a", "b", "c", "d"] {
            let live = Arc::clone(&live);
            let overlapped = Arc::clone(&overlapped);
            let done = tx.clone();
            gate.on_open(label, move || {
                if live.swap(true, Ordering::SeqCst) {
                    overlapped.store(true, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(20));
                live.store(false, Ordering::SeqCst);
                let _ = done.send(label);
            });
        }
        drop(tx);

        let order: Vec<&str> = (0..4)
            .map(|_| rx.recv_timeout(WAIT).expect("every job runs"))
            .collect();
        assert_eq!(order, ["a", "b", "c", "d"], "registration order lost");
        assert!(
            !overlapped.load(Ordering::SeqCst),
            "two deferred jobs ran at once; they contend for the workspace lock"
        );
    }

    /// The mixed case: some queued before the open, some after. All of
    /// them share the one worker, seeded-first.
    #[test]
    fn jobs_queued_before_and_after_the_open_share_one_order() {
        let gate = FirstPaintGate::new();
        let (tx, rx) = mpsc::channel();
        for label in ["early-1", "early-2"] {
            let done = tx.clone();
            gate.on_open(label, move || {
                let _ = done.send(label);
            });
        }
        gate.painted();
        for label in ["late-1", "late-2"] {
            let done = tx.clone();
            gate.on_open(label, move || {
                let _ = done.send(label);
            });
        }
        drop(tx);

        let order: Vec<&str> = (0..4)
            .map(|_| rx.recv_timeout(WAIT).expect("every job runs"))
            .collect();
        assert_eq!(order, ["early-1", "early-2", "late-1", "late-2"]);
    }

    #[test]
    fn opening_twice_does_not_run_the_queue_twice() {
        let gate = FirstPaintGate::new();
        let (tx, rx) = mpsc::channel();
        gate.on_open("test", move || {
            let _ = tx.send(());
        });
        gate.painted();
        gate.painted();
        assert!(rx.recv_timeout(WAIT).is_ok());
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "the queue ran a second time"
        );
    }

    #[test]
    fn the_deadline_does_not_re_open_after_a_painted_frame() {
        let gate = FirstPaintGate::new();
        let (tx, rx) = mpsc::channel();
        gate.on_open("test", move || {
            let _ = tx.send(());
        });
        gate.arm_deadline(Duration::from_millis(10));
        gate.painted();
        assert!(rx.recv_timeout(WAIT).is_ok());
        assert!(
            rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "the deadline re-ran the queue after the client opened it"
        );
    }
}
