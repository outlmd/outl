//! Stderr writes that survive a closed pipe.
//!
//! Rust's runtime sets `SIGPIPE` to `SIG_IGN`, so a write to a closed stderr
//! returns `EPIPE` instead of killing the process. The `print!` family then
//! turns that error into `panic: failed printing to stderr`. A panic hook
//! that prints the same way panics again, and the process aborts.
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
            // Capture before locking stderr. A backtrace that itself writes
            // to stderr must not deadlock on this lock.
            let backtrace = std::backtrace::Backtrace::capture();
            let mut err = io::stderr().lock();
            let _ = write_panic(&mut err, info, &backtrace);
        }));
    });
}

/// Writes the panic the way the default hook does, and ignores a closed stderr.
///
/// The first line keeps the thread name. `Backtrace::capture` is a no-op
/// unless `RUST_BACKTRACE` is set to something other than `0`, so the variable
/// still decides whether a stack is printed. A disabled backtrace is not
/// written: its `Display` is the words "disabled backtrace", which the default
/// hook also omits.
fn write_panic(
    mut out: impl Write,
    info: &std::panic::PanicHookInfo<'_>,
    backtrace: &std::backtrace::Backtrace,
) -> io::Result<()> {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    writeln!(out, "thread '{name}' {info}")?;
    if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
        writeln!(out, "stack backtrace:\n{backtrace}")?;
    }
    Ok(())
}

fn absorb_broken_pipe(result: io::Result<usize>, len: usize) -> io::Result<usize> {
    match result {
        Err(err) if is_broken_pipe(&err) => Ok(len),
        other => other,
    }
}

fn is_broken_pipe(err: &io::Error) -> bool {
    // The kind is the whole check. Unix `EPIPE` (errno 32) and Windows
    // `ERROR_BROKEN_PIPE` (109) / `ERROR_NO_DATA` (232) all arrive as
    // `ErrorKind::BrokenPipe`. Windows error 32 is `ERROR_SHARING_VIOLATION`,
    // and matching the number would hide a real sharing failure.
    err.kind() == io::ErrorKind::BrokenPipe
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
    fn the_os_error_for_epipe_counts_because_its_kind_is_broken_pipe() {
        // Unix EPIPE is 32 and std maps it to BrokenPipe. The same number on
        // Windows is ERROR_SHARING_VIOLATION, which must pass through.
        // ERROR_BROKEN_PIPE is 109 and ERROR_NO_DATA is 232; std maps both
        // to BrokenPipe, so the kind check covers them without a raw code.
        #[cfg(unix)]
        {
            let err = io::Error::from_raw_os_error(32);
            assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
            assert!(is_broken_pipe(&err));
        }
        #[cfg(windows)]
        {
            let sharing = io::Error::from_raw_os_error(32);
            assert_ne!(sharing.kind(), io::ErrorKind::BrokenPipe);
            assert!(!is_broken_pipe(&sharing));
            for code in [109, 232] {
                let err = io::Error::from_raw_os_error(code);
                assert_eq!(err.kind(), io::ErrorKind::BrokenPipe, "{code}");
                assert!(is_broken_pipe(&err));
            }
        }
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
