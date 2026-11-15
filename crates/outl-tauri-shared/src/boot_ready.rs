//! "Is the workspace open yet" — asked once and waited on, not polled.
//!
//! Both clients used to answer this by asking repeatedly. The desktop
//! polled `workspace_stats` every 500 ms behind the loading screen; the
//! phone retried `open_today_journal` up to 50 times with a 200 ms
//! backoff. Neither can do better than its own interval, so the cost is
//! structural: a workspace that becomes ready 10 ms after a tick waits
//! out the rest of it, every launch, and the interval is the thing that
//! decides how long.
//!
//! The event already exists (`workspace-ready`), and both clients already
//! listen for it. The polling is there because an event can be *missed* —
//! emitted before the frontend subscribed, or lost to a webview reload
//! mid-boot — and a missed readiness signal is an app stuck on a spinner
//! forever. So the poll was never redundant; it was a fallback wearing a
//! hot path's clothes.
//!
//! [`WorkspaceReady`] replaces the fallback with a *question that
//! cannot be missed*: the state is latched, so asking after the fact
//! returns immediately, and asking before it blocks until it is set. A
//! client needs one call per boot instead of one per interval, and the
//! answer arrives when it is true rather than on the next tick.
//!
//! # Why this is not the `FirstPaintGate`
//!
//! They look alike and run in opposite directions.
//! [`crate::first_paint::FirstPaintGate`] carries work *backwards* past
//! the first frame (the backend waiting on the client); this carries one
//! fact *forwards* to the client (the client waiting on the backend).
//! Folding them into one "boot state" object would mean one lock on the
//! path of both, and the frontend's painted signal would then be ordered
//! against the workspace open for no reason.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Latched "the workspace is published" flag with a blocking wait.
///
/// Cloneable only behind an `Arc` (it holds a `Condvar`), which both
/// clients already do for every other slot on their `AppState`.
pub struct WorkspaceReady {
    /// `true` once a workspace has been published. Never goes back to
    /// `false`: a workspace *switch* republishes, and a waiter that woke
    /// on the previous one asked a question that is still true.
    ready: Mutex<bool>,
    changed: Condvar,
}

impl WorkspaceReady {
    /// Not ready yet.
    pub fn new() -> Self {
        WorkspaceReady {
            ready: Mutex::new(false),
            changed: Condvar::new(),
        }
    }

    /// The workspace is published. Wakes every waiter.
    ///
    /// Called by the opener **after** the slot is filled, never before:
    /// a waiter that wakes and finds `ERR_LOADING` is the bug this
    /// removes, reintroduced one line earlier.
    pub fn publish(&self) {
        let mut ready = match self.ready.lock() {
            Ok(r) => r,
            Err(e) => e.into_inner(),
        };
        *ready = true;
        self.changed.notify_all();
    }

    /// Block until the workspace is published, or `timeout` elapses.
    /// Returns whether it is ready.
    ///
    /// **Blocking**, so a Tauri command calling this belongs on the
    /// blocking pool ([`crate::offload::run`]) and never on the IPC
    /// thread. On iOS that thread is the main thread, so a wait taken
    /// there is the scene-update watchdog kill (0x8BADF00D) with a
    /// perfectly healthy workspace opening behind it.
    ///
    /// A timeout is not a failure to report upward: it means "not yet",
    /// and the caller asks again. That is what keeps a slow open (a cold
    /// 700k-op replay on a phone) from being reported as a broken one.
    pub fn wait(&self, timeout: Duration) -> bool {
        let guard = match self.ready.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        let (guard, _) = match self.changed.wait_timeout_while(guard, timeout, |r| !*r) {
            Ok(pair) => pair,
            // A publisher panicked between setting the flag and
            // notifying. The flag is still the truth; read it.
            Err(e) => e.into_inner(),
        };
        *guard
    }
}

impl Default for WorkspaceReady {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn a_published_workspace_answers_without_waiting() {
        let ready = WorkspaceReady::new();
        ready.publish();
        let at = std::time::Instant::now();
        assert!(ready.wait(Duration::from_secs(30)));
        assert!(
            at.elapsed() < Duration::from_secs(1),
            "an already-published workspace blocked the waiter"
        );
    }

    /// The property the module exists for: the answer arrives when it
    /// becomes true, not on the next tick of someone's interval.
    #[test]
    fn a_waiter_wakes_on_publish() {
        let ready = Arc::new(WorkspaceReady::new());
        let publisher = Arc::clone(&ready);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            publisher.publish();
        });
        assert!(ready.wait(Duration::from_secs(30)), "the waiter never woke");
    }

    #[test]
    fn a_timeout_reports_not_ready_rather_than_failing() {
        let ready = WorkspaceReady::new();
        assert!(!ready.wait(Duration::from_millis(10)));
    }
}
