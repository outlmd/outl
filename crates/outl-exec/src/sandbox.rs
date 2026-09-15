//! Cross-platform sandbox helpers shared by every runtime.
//!
//! # Stopping a block, and the two grades of it
//!
//! The **strong** form is a cancellation point inside the interpreter:
//! the VM notices the deadline and unwinds itself, so the work stops,
//! the thread ends and nothing leaks. Two runtimes have one: `lua`,
//! through mlua's instruction hook, and `rust`, through wasmtime epoch
//! interruption (`crate::wasm::module`). Neither goes through this
//! module — in both the check rides in-band with the interpreter rather
//! than through a flag this module could own.
//!
//! [`with_timeout`] is the **weak** form, for a backend with no such
//! point: the caller is released, the work is not. The worker thread is
//! deliberately not joined, so an infinite loop keeps burning a core
//! until the process exits.
//!
//! That leak is a real cost and still the right trade, because the
//! alternative is not "no leak" — it is the TUI event loop frozen, or
//! the desktop wedged while holding the workspace mutex, with Force
//! Quit as the only way out (issue #279).
//!
//! It is a **bounded** leak. The process keeps a count of workers that
//! were abandoned past their deadline and are still running, and once
//! it reaches [`MAX_RUNAWAY_WORKERS`] a new call refuses with
//! `ExecError::Sandbox` instead of spawning. Without the cap, one
//! `while (true) {}` fence under `auto-run::` would spawn a fresh core-
//! burning thread on every page load, and `outl mcp serve` under an
//! agent would do the same without a page load in sight. A refused run
//! is a visible error in the result block; a machine slowly running out
//! of cores is not. A worker that finishes late hands its slot back, so
//! the cap is on work that is *still* running, not on work that ever
//! overran.
//!
//! `lua` also goes through [`with_timeout`], for a different reason: its
//! hook is a real cancellation point for Lua code, but not for a single
//! long C call. The wrapper releases the caller through that call, and
//! the hook ends the worker as soon as the call returns, so a `lua`
//! worker only ever holds a slot for the length of one C call.
//!
//! Reach for the weak form only after checking the interpreter for a
//! cancellation point, and record in the runtime what you found. As of
//! the versions pinned in `Cargo.toml`:
//!
//! - **boa 0.22** (`js`) — `RuntimeLimits` caps loop iterations,
//!   recursion depth and stack size; nothing per-instruction or
//!   wall-clock, and `HostHooks` has no interrupt point.
//! - **rustpython 0.5** (`python`) — `eval_breaker_tripped` is
//!   `pub(crate)`, and the opcode trace hook only fires when a frame's
//!   `f_trace_opcodes` is set from Python.
//! - **steel 0.8.3** (`lisp`, opt-in, not in the default feature set).
//!   `Engine::with_interrupted` takes an `Arc<AtomicBool>` and the VM
//!   **never reads it**: every occurrence in the crate is a write. An
//!   API that looks like the strong form and is not. Steel *does* have
//!   one behind a different door,
//!   `get_thread_state_controller().interrupt()`, checked by
//!   `VmCore::safepoint_or_interrupt`, so `lisp` can be upgraded; it
//!   has not been yet.
//!
//! Each is worth re-checking on a dependency bump: any one of them
//! gaining a real cancellation point moves that runtime up a grade.

use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use crate::runtime::ExecError;

/// How many abandoned workers may still be running before
/// [`with_timeout`] refuses to start another.
///
/// Four is a budget, not a target. One runaway block is a mistake, two
/// is a pattern, and a fifth would mean the user has been shown four
/// timeouts and kept going. The count only includes work that is still
/// running, so a worker that finishes late frees its slot.
pub const MAX_RUNAWAY_WORKERS: usize = 4;

/// Worker still running and its caller still waiting.
const RUNNING: u8 = 0;
/// The caller gave up on this worker; it is counted against the budget.
const ABANDONED: u8 = 1;
/// The worker finished before its caller gave up; nothing to count.
const FINISHED: u8 = 2;

/// The process-wide count of workers that overran their deadline and
/// have not finished.
///
/// A struct rather than a bare atomic so a test can have its own copy
/// (`Box::leak`) instead of poisoning the counter every other test in
/// the binary shares.
pub(crate) struct RunawayBudget {
    runaway: AtomicUsize,
    max: usize,
}

impl RunawayBudget {
    /// A budget that refuses once `max` abandoned workers are running.
    pub(crate) const fn new(max: usize) -> Self {
        Self {
            runaway: AtomicUsize::new(0),
            max,
        }
    }

    /// How many abandoned workers are running right now.
    pub(crate) fn runaway(&self) -> usize {
        self.runaway.load(Ordering::SeqCst)
    }
}

static BUDGET: RunawayBudget = RunawayBudget::new(MAX_RUNAWAY_WORKERS);

/// Hands a slot back when the worker's closure returns, however it
/// returns: a panic inside the interpreter must not pin the slot.
struct WorkerSlot {
    state: Arc<AtomicU8>,
    budget: &'static RunawayBudget,
}

impl Drop for WorkerSlot {
    fn drop(&mut self) {
        // Either the caller is still waiting (mark finished so it does
        // not count us) or it already abandoned us (our count is live,
        // take it back).
        if self
            .state
            .compare_exchange(RUNNING, FINISHED, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            self.budget.runaway.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Run `work` on a worker thread; return its result or
/// `Err(ExecError::Timeout)` if it doesn't finish within `timeout`.
///
/// `work` must be `'static + Send` so it can move to the worker. Pass
/// owned data in (clone if needed) — borrowing across the boundary
/// would force `'static` lifetimes everywhere upstream.
///
/// Refuses with `ExecError::Sandbox` when [`MAX_RUNAWAY_WORKERS`]
/// abandoned workers are still running; see the module doc.
pub fn with_timeout<F, T>(timeout: Duration, work: F) -> Result<T, ExecError>
where
    F: FnOnce() -> Result<T, ExecError> + Send + 'static,
    T: Send + 'static,
{
    with_timeout_on(&BUDGET, timeout, work)
}

/// [`with_timeout`] against an explicit budget. Tests use their own so
/// an abandoned worker in one does not refuse a run in another.
pub(crate) fn with_timeout_on<F, T>(
    budget: &'static RunawayBudget,
    timeout: Duration,
    work: F,
) -> Result<T, ExecError>
where
    F: FnOnce() -> Result<T, ExecError> + Send + 'static,
    T: Send + 'static,
{
    let running = budget.runaway();
    if running >= budget.max {
        return Err(ExecError::Sandbox(format!(
            "{running} code blocks are still running past their timeout; \
             refusing to start another until one finishes or outl restarts"
        )));
    }

    let state = Arc::new(AtomicU8::new(RUNNING));
    let slot = WorkerSlot {
        state: state.clone(),
        budget,
    };
    let (tx, rx) = mpsc::sync_channel::<Result<T, ExecError>>(1);
    thread::Builder::new()
        .name("outl-exec".into())
        .spawn(move || {
            // Dropped when the closure returns, including by unwinding.
            let _slot = slot;
            // If the receiver has been dropped (timeout fired), this
            // send fails silently — that's fine, we're just throwing
            // the result away.
            let _ = tx.send(work());
        })
        .map_err(|e| ExecError::Sandbox(format!("spawn worker: {e}")))?;

    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // Count first, then claim. If the worker finishes before
            // the claim lands, the claim fails and we take the count
            // back ourselves; if it finishes after, it sees ABANDONED
            // and takes it back. Either order nets to zero, and because
            // the increment always precedes the decrement it can pair
            // with, the count never goes below it.
            budget.runaway.fetch_add(1, Ordering::SeqCst);
            if state
                .compare_exchange(RUNNING, ABANDONED, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                budget.runaway.fetch_sub(1, Ordering::SeqCst);
            }
            Err(ExecError::Timeout(timeout))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(ExecError::Sandbox(
            "worker thread vanished without a result".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_work_returns_value() {
        let v: Result<i32, ExecError> = with_timeout(Duration::from_secs(2), || Ok(42));
        assert!(matches!(v, Ok(42)));
    }

    #[test]
    fn slow_work_times_out() {
        let v: Result<(), ExecError> = with_timeout(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(500));
            Ok(())
        });
        assert!(matches!(v, Err(ExecError::Timeout(_))));
    }

    #[test]
    fn inner_error_propagates() {
        let v: Result<(), ExecError> = with_timeout(Duration::from_secs(2), || {
            Err(ExecError::Language("oops".into()))
        });
        match v {
            Err(ExecError::Language(m)) => assert_eq!(m, "oops"),
            other => panic!("expected Language error, got {other:?}"),
        }
    }

    /// Its own budget, leaked: the process-wide one is shared with
    /// every other test in this binary, and abandoning two workers in
    /// it would refuse runs elsewhere.
    fn fresh_budget(max: usize) -> &'static RunawayBudget {
        Box::leak(Box::new(RunawayBudget::new(max)))
    }

    #[test]
    fn runaway_workers_are_bounded() {
        let budget = fresh_budget(2);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Arc::new(std::sync::Mutex::new(release_rx));

        // Two workers that overrun and stay alive until told to stop.
        for _ in 0..2 {
            let rx = release_rx.clone();
            let v: Result<(), ExecError> =
                with_timeout_on(budget, Duration::from_millis(20), move || {
                    let _ = rx.lock().unwrap().recv();
                    Ok(())
                });
            assert!(matches!(v, Err(ExecError::Timeout(_))));
        }
        assert_eq!(budget.runaway(), 2);

        // The third is refused before it spawns, even though it would
        // have finished instantly.
        let v: Result<i32, ExecError> = with_timeout_on(budget, Duration::from_secs(2), || Ok(1));
        match v {
            Err(ExecError::Sandbox(m)) => assert!(m.contains("still running"), "{m}"),
            other => panic!("expected the budget to refuse, got {other:?}"),
        }

        // Let the abandoned workers finish; each hands its slot back.
        release_tx.send(()).unwrap();
        release_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while budget.runaway() != 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(budget.runaway(), 0, "a finished worker must free its slot");

        let v: Result<i32, ExecError> = with_timeout_on(budget, Duration::from_secs(2), || Ok(1));
        assert!(matches!(v, Ok(1)));
    }

    #[test]
    fn a_worker_that_finishes_in_time_is_never_counted() {
        let budget = fresh_budget(1);
        for _ in 0..3 {
            let v: Result<(), ExecError> =
                with_timeout_on(budget, Duration::from_secs(2), || Ok(()));
            assert!(v.is_ok());
        }
        assert_eq!(budget.runaway(), 0);
    }

    #[test]
    fn a_panicking_worker_hands_its_slot_back() {
        let budget = fresh_budget(1);
        // Abandon one worker that will later panic.
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let rx = Arc::new(std::sync::Mutex::new(go_rx));
        let v: Result<(), ExecError> =
            with_timeout_on(budget, Duration::from_millis(20), move || {
                let _ = rx.lock().unwrap().recv();
                panic!("interpreter blew up after the caller left");
            });
        assert!(matches!(v, Err(ExecError::Timeout(_))));
        assert_eq!(budget.runaway(), 1);

        go_tx.send(()).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while budget.runaway() != 0 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(budget.runaway(), 0, "a panic must settle the slot too");
    }
}
