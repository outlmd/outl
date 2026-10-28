//! A closed stderr must not abort the process.
//!
//! `harness = false` in the manifest: libtest captures stdout, and the child
//! has to tell the parent it has reached the point where stderr may close.
//!
//! The desktop (and the CLI, and mobile) install a `tracing` subscriber on
//! stderr at boot. `tracing-subscriber` reports a failed write with
//! `eprintln!`, and that macro panics with `failed printing to stderr: Broken
//! pipe (os error 32)`. Sentry's panic integration then calls the previous
//! hook, which is the default `eprintln!` hook, which panics again and aborts.
//! The event that arrives is that abort, with no useful stack.
//!
//! This spawns the test binary with stderr piped and drops the read end, the
//! shape of a desktop launched with no terminal whose pipe has already closed.
//! The child runs the same two calls the binaries do — `init`, then a fmt
//! subscriber on [`outl_telemetry::lossy_stderr`] — and then panics on purpose.
//! Exit 0 is the assertion. A printer panic leaves 101; a hook that panics
//! again aborts, and `catch_unwind` does not hide either.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

fn main() {
    if std::env::var_os("OUTL_CLOSED_STDERR_CHILD").is_some() {
        child();
        return;
    }
    if std::env::var_os("OUTL_PANIC_REPORT_CHILD").is_some() {
        panic_report_child();
        return;
    }

    let exe = std::env::current_exe().expect("the test binary has a path");
    let mut child = Command::new(&exe)
        .env("OUTL_CLOSED_STDERR_CHILD", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the child with a pipe for stderr");

    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut seen = String::new();
    let mut buf = [0u8; 64];
    loop {
        let n = stdout
            .read(&mut buf)
            .expect("read the child's ready signal");
        assert!(n > 0, "child exited before it was ready to close stderr");
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        if seen.contains("READY\n") {
            break;
        }
    }

    // Drop the read end first, so the writes below see EPIPE rather than
    // blocking on a full pipe buffer.
    drop(child.stderr.take());
    let mut stdin = child.stdin.take().expect("stdin is piped");
    stdin
        .write_all(b"go\n")
        .expect("release the child after stderr is closed");
    drop(stdin);

    let rest = std::thread::spawn(move || {
        let mut tail = Vec::new();
        let _ = stdout.read_to_end(&mut tail);
        tail
    });
    let status = child.wait().expect("wait for the child");
    let tail = rest.join().expect("stdout drain");
    assert!(
        status.success(),
        "closed stderr panicked the child: {status:?}\n{seen}{}",
        String::from_utf8_lossy(&tail)
    );

    // Stderr stays open here. The hook has to name the thread and, when
    // RUST_BACKTRACE is set, print the stack `Backtrace::capture` produced.
    let mut report = Command::new(&exe)
        .env("OUTL_PANIC_REPORT_CHILD", "1")
        .env("RUST_BACKTRACE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the panic-report child");
    let mut reported = String::new();
    report
        .stderr
        .take()
        .expect("stderr is piped")
        .read_to_string(&mut reported)
        .expect("read the panic report");
    let report_status = report.wait().expect("wait for the panic report");
    assert!(
        report_status.success(),
        "panic report child failed: {report_status:?}\n{reported}"
    );
    assert!(
        reported.contains("thread 'main'"),
        "panic report dropped the thread name:\n{reported}"
    );
    assert!(
        reported.contains("probe-backtrace"),
        "panic report dropped the message:\n{reported}"
    );
    assert!(
        reported.contains("stack backtrace:"),
        "RUST_BACKTRACE=1 produced no stack:\n{reported}"
    );
}

fn panic_report_child() {
    let _guard = outl_telemetry::init("test", false);
    let caught = std::panic::catch_unwind(|| panic!("probe-backtrace"));
    assert!(caught.is_err(), "the probe panic was swallowed");
    std::process::exit(0);
}

fn child() {
    let mut out = std::io::stdout();
    let _ = out.write_all(b"READY\n");
    let _ = out.flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);

    // `enabled: false` keeps the reporter off. The hook is installed either
    // way, which is the half under test.
    let _guard = outl_telemetry::init("test", false);

    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(outl_telemetry::lossy_stderr)
        .log_internal_errors(false)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("install the stderr subscriber");

    tracing::info!("a line written after stderr was closed");

    let caught = std::panic::catch_unwind(|| panic!("probe"));
    assert!(caught.is_err(), "the probe panic was swallowed");
    std::process::exit(0);
}
