//! Stderr writes that survive a closed pipe.
//!
//! Rust's runtime sets `SIGPIPE` to `SIG_IGN`, so a write to a closed stderr
//! returns `EPIPE` (os error 32) instead of killing the process. The `print!`
//! family then turns that error into `panic: failed printing to stderr`.
//! A panic hook that prints the same way panics again, and the process aborts.
//!
//! That is the desktop failure: `tracing-subscriber`'s fmt layer, with
//! `log_internal_errors` left on, reports a failed write through `eprintln!`.
//! The first log line to a closed stderr is a fatal panic, and the stack that
//! reaches Sentry is the printer, not the bug.

use std::io::{self, Write};
use std::sync::Once;

/// Stderr that treats a broken pipe as a successful write of the whole buffer.
///
/// Pass [`lossy_stderr`] to `tracing_subscriber::fmt().with_writer`, and also
/// call `.log_internal_errors(false)`. The writer keeps the fmt layer from
/// seeing `Err`; the flag keeps the layer from reporting that `Err` with
/// `eprintln!` if a future writer change lets one through.
#[derive(Debug, Default)]
pub struct LossyStderr;

/// Writer for `tracing_subscriber::fmt().with_writer`.
pub fn lossy_stderr() -> LossyStderr {
    LossyStderr
}

impl Write for LossyStderr {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        absorb_broken_pipe(io::stderr().write(buf), buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match io::stderr().flush() {
            Err(err) if is_broken_pipe(&err) => Ok(()),
            other => other,
        }
    }
}

/// Installs a panic hook that records the panic and ignores a closed stderr.
///
/// Must run **before** `sentry::init`. The panic integration takes the current
/// hook and calls it after capturing the event. The default hook is
/// `eprintln!`, so a closed stderr panics again inside the hook and aborts
/// before the report is the one worth reading — and the abort is itself the
/// panic Sentry records.
///
/// The hook does not call the previous hook. Chaining into the default hook
/// puts `eprintln!` back on the path this exists to remove. A later hook (the
/// TUI's terminal restore) may chain to this one; this one is the end of the
/// chain.
pub fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let _ = writeln!(io::stderr(), "{info}");
        }));
    });
}

fn absorb_broken_pipe(result: io::Result<usize>, len: usize) -> io::Result<usize> {
    match result {
        Err(err) if is_broken_pipe(&err) => Ok(len),
        other => other,
    }
}

fn is_broken_pipe(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::BrokenPipe || err.raw_os_error() == Some(32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_pipe_counts_as_a_finished_write() {
        let err = io::Error::new(io::ErrorKind::BrokenPipe, "closed");
        assert!(is_broken_pipe(&err));
        assert_eq!(absorb_broken_pipe(Err(err), 4).expect("absorbed"), 4);
    }

    #[test]
    fn the_os_error_for_epipe_counts_too() {
        let err = io::Error::from_raw_os_error(32);
        assert!(is_broken_pipe(&err));
    }

    #[test]
    fn any_other_write_error_is_returned() {
        let err = io::Error::new(io::ErrorKind::PermissionDenied, "no");
        assert!(!is_broken_pipe(&err));
        let again = absorb_broken_pipe(Err(err), 4);
        assert_eq!(
            again.expect_err("kept").kind(),
            io::ErrorKind::PermissionDenied
        );
    }
}
