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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::{info, warn};

/// One deferred job: a label for the log, and the work.
type Job = (&'static str, Box<dyn FnOnce() + Send + 'static>);

/// Runs queued boot work once the client reports a painted frame, or
/// once the deadline elapses — whichever comes first, and exactly once.
pub struct FirstPaintGate {
    /// `None` once the gate has opened. Jobs queued after that run
    /// immediately, so a late registration is never silently dropped.
    queued: Mutex<Option<Vec<Job>>>,
    /// Whether the client's signal (rather than the deadline) opened it.
    /// Reported in the log, because "we waited" and "we gave up waiting"
    /// are different launches and the difference is a bug report.
    by_client: AtomicBool,
}

impl FirstPaintGate {
    /// A closed gate with nothing queued.
    pub fn new() -> Arc<Self> {
        Arc::new(FirstPaintGate {
            queued: Mutex::new(Some(Vec::new())),
            by_client: AtomicBool::new(false),
        })
    }

    /// Queue `job` to run when the gate opens, or run it now if it
    /// already has.
    ///
    /// `label` names the work in the log. Jobs run in registration
    /// order, on one thread, so two of them cannot contend for the
    /// workspace lock against each other — they are each a cheap spawn
    /// today, and the order is what makes a slow one visible in the log
    /// rather than interleaved with the next.
    pub fn on_open<F>(self: &Arc<Self>, label: &'static str, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let mut slot = match self.queued.lock() {
            Ok(s) => s,
            // A poisoned lock means a queued job panicked. The gate's
            // state is still readable and the right answer is to run,
            // not to strand sync behind a mutex.
            Err(e) => e.into_inner(),
        };
        match slot.as_mut() {
            Some(jobs) => jobs.push((label, Box::new(job))),
            None => {
                drop(slot);
                run_jobs(vec![(label, Box::new(job))]);
            }
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
        let jobs = {
            let mut slot = match self.queued.lock() {
                Ok(s) => s,
                Err(e) => e.into_inner(),
            };
            match slot.take() {
                Some(jobs) => jobs,
                // Already open. Not a warning: the deadline racing a
                // painted frame is the ordinary case.
                None => return,
            }
        };
        if jobs.is_empty() {
            info!("boot: interactive phase opened by {why} (nothing deferred)");
            return;
        }
        info!(
            "boot: interactive phase opened by {why}, starting {} deferred job(s)",
            jobs.len()
        );
        run_jobs(jobs);
    }
}

/// Run `jobs` in order, off the calling thread.
fn run_jobs(jobs: Vec<Job>) {
    let spawned = std::thread::Builder::new()
        .name("outl-interactive-phase".to_string())
        .spawn(move || {
            for (label, job) in jobs {
                let at = std::time::Instant::now();
                job();
                info!(
                    "boot: deferred job {label} started in {}ms",
                    at.elapsed().as_millis()
                );
            }
        });
    if let Err(e) = spawned {
        warn!("interactive-phase thread failed to start: {e}");
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
