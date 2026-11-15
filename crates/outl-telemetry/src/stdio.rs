//! Stdio writes that survive a closed pipe.
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

use std::fmt;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
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

/// Set once a write to stdout hit a closed pipe, so later prints skip the
/// syscall instead of failing the same way for every remaining line.
static STDOUT_CLOSED: AtomicBool = AtomicBool::new(false);

/// What `print!` / `println!` do, except that a closed stdout is not a panic.
///
/// The standard macros turn any write error into
/// `panic: failed printing to stdout: ...`. For `outl <cmd> | head -1` that is
/// `EPIPE` (Rust sets `SIGPIPE` to `SIG_IGN`), and on Windows it is
/// `ERROR_NO_DATA` (232) or `ERROR_BROKEN_PIPE` (109). Either way the reader
/// left on purpose, so the panic is a fatal crash report about nothing wrong.
///
/// A broken pipe drops the output and the command **keeps running**. Exiting
/// at that point would abandon a mutating command half way (`outl doctor
/// --repair | head`), and the reader choosing not to read says nothing about
/// whether the work should stop. The exit status is the command's own.
///
/// Any other error is still the panic the standard macros raise, with the
/// same message, so a real stdout failure keeps reaching the crash reporter.
///
/// `newline` is `println!` versus `print!`. The CLI's `print!` / `println!`
/// shadows call this; see `outl-cli/src/stdout.rs`.
pub fn print_stdout(args: fmt::Arguments<'_>, newline: bool) {
    if STDOUT_CLOSED.load(Ordering::Relaxed) {
        return;
    }
    let mut out = io::stdout().lock();
    if let Err(err) = write_print(&mut out, args, newline) {
        settle_stdout_error(&err, &STDOUT_CLOSED);
    }
}

/// Writes one `print!` / `println!` worth of output.
fn write_print(mut out: impl Write, args: fmt::Arguments<'_>, newline: bool) -> io::Result<()> {
    out.write_fmt(args)?;
    if newline {
        out.write_all(b"\n")?;
    }
    Ok(())
}

/// Decides what a failed stdout write means: a closed pipe is recorded in
/// `closed` and forgiven, anything else is the standard macros' panic.
fn settle_stdout_error(err: &io::Error, closed: &AtomicBool) {
    if is_broken_pipe(err) {
        closed.store(true, Ordering::Relaxed);
        return;
    }
    panic!("failed printing to stdout: {err}");
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

    /// A writer whose every write fails with the given error.
    struct Failing(io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(self.0, "closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_print_renders_the_arguments_and_the_newline() {
        let mut buf = Vec::new();
        write_print(&mut buf, format_args!("a {}", 1), true).expect("write");
        write_print(&mut buf, format_args!("b"), false).expect("write");
        assert_eq!(buf, b"a 1\nb");
    }

    #[test]
    fn a_closed_stdout_is_remembered_and_not_a_panic() {
        let closed = AtomicBool::new(false);
        let err = write_print(Failing(io::ErrorKind::BrokenPipe), format_args!("x"), true)
            .expect_err("the writer fails");
        settle_stdout_error(&err, &closed);
        assert!(closed.load(Ordering::Relaxed));
    }

    #[test]
    fn the_os_errors_a_closed_stdout_arrives_as_are_forgiven() {
        // Unix EPIPE is 32. Windows reports the same condition as 109 or 232.
        // std maps each to BrokenPipe, and the kind is what is checked.
        #[cfg(unix)]
        let codes = [32];
        #[cfg(windows)]
        let codes = [109, 232];
        for code in codes {
            let closed = AtomicBool::new(false);
            settle_stdout_error(&io::Error::from_raw_os_error(code), &closed);
            assert!(closed.load(Ordering::Relaxed), "{code}");
        }
    }

    #[test]
    #[should_panic(expected = "failed printing to stdout: no")]
    fn any_other_stdout_error_is_still_the_standard_panic() {
        let closed = AtomicBool::new(false);
        settle_stdout_error(
            &io::Error::new(io::ErrorKind::PermissionDenied, "no"),
            &closed,
        );
    }

    #[test]
    fn another_error_does_not_mark_stdout_closed() {
        let closed = AtomicBool::new(false);
        let err = io::Error::new(io::ErrorKind::PermissionDenied, "no");
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            settle_stdout_error(&err, &closed)
        }));
        assert!(caught.is_err());
        assert!(!closed.load(Ordering::Relaxed));
    }

    #[cfg(windows)]
    #[test]
    fn windows_sharing_violation_is_not_a_closed_stdout() {
        // Error 32 is ERROR_SHARING_VIOLATION on Windows, the number EPIPE has
        // on Unix. It must stay a panic, same rule as `is_broken_pipe`.
        let closed = AtomicBool::new(false);
        let err = io::Error::from_raw_os_error(32);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            settle_stdout_error(&err, &closed)
        }));
        assert!(caught.is_err());
        assert!(!closed.load(Ordering::Relaxed));
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
