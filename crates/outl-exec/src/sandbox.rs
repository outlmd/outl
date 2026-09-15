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
//! - **steel 0.8.2** (`lisp`) — `Engine::with_interrupted` takes an
//!   `Arc<AtomicBool>` and the VM **never reads it**: every occurrence
//!   in the crate is a write. An API that looks like the strong form
//!   and is not. Steel *does* have one behind a different door —
//!   `get_thread_state_controller().interrupt()`, checked by
//!   `VmCore::safepoint_or_interrupt` — so `lisp` can be upgraded; it
//!   has not been yet.
//!
//! Each is worth re-checking on a dependency bump: any one of them
//! gaining a real cancellation point moves that runtime up a grade.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::runtime::ExecError;

/// Run `work` on a worker thread; return its result or
/// `Err(ExecError::Timeout)` if it doesn't finish within `timeout`.
///
/// `work` must be `'static + Send` so it can move to the worker. Pass
/// owned data in (clone if needed) — borrowing across the boundary
/// would force `'static` lifetimes everywhere upstream.
pub fn with_timeout<F, T>(timeout: Duration, work: F) -> Result<T, ExecError>
where
    F: FnOnce() -> Result<T, ExecError> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = mpsc::sync_channel::<Result<T, ExecError>>(1);
    thread::Builder::new()
        .name("outl-exec".into())
        .spawn(move || {
            // If the receiver has been dropped (timeout fired), this
            // send fails silently — that's fine, we're just throwing
            // the result away.
            let _ = tx.send(work());
        })
        .map_err(|e| ExecError::Sandbox(format!("spawn worker: {e}")))?;

    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(ExecError::Timeout(timeout)),
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
}
