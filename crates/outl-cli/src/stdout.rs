//! `print!` / `println!` that survive a closed stdout.
//!
//! `outl <cmd> | head -1` closes the read end of stdout while the command is
//! still printing. Rust ignores `SIGPIPE`, so the write fails with a broken
//! pipe (`EPIPE` on Unix, `ERROR_NO_DATA` / `ERROR_BROKEN_PIPE` on Windows),
//! and the standard macros turn that into `panic: failed printing to stdout`.
//! The crash reporter then records a fatal report for a reader that simply
//! stopped reading.
//!
//! These two macros shadow the prelude ones for every module declared after
//! `#[macro_use] mod stdout;` in `main.rs`, so the ~350 call sites need no
//! change and a new `println!` is covered without anyone remembering to be.
//! `outl_telemetry::print_stdout` owns the decision (a broken pipe drops the
//! output, any other error is still the standard panic); see its docs.
//!
//! Not compiled for unit tests: libtest captures output through the standard
//! macros, and bypassing them would spray every command's output into the
//! test log.

#[cfg(not(test))]
macro_rules! print {
    ($($arg:tt)*) => {
        ::outl_telemetry::print_stdout(format_args!($($arg)*), false)
    };
}

#[cfg(not(test))]
macro_rules! println {
    () => {
        ::outl_telemetry::print_stdout(format_args!(""), true)
    };
    ($($arg:tt)*) => {
        ::outl_telemetry::print_stdout(format_args!($($arg)*), true)
    };
}
