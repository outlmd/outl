//! Somewhere a developer can actually read the backend's `tracing` output.
//!
//! The subscriber in `lib.rs` writes to stderr, with a comment saying that
//! on iOS stderr "surfaces in idevicesyslog / Xcode". It does not. A
//! release build on the simulator emits **nothing** to
//! `simctl launch --console`, `--console-pty` or `log stream`: the only
//! lines that arrive are from dylibs using `NSLog`, which goes through
//! `os_log`. Rust's `io::stderr()` is not wired to anything on either
//! simulator or device.
//!
//! So every `info!` / `warn!` the backend emits — the sync transport, the
//! workspace opener, every boot repair, every projection refusal — has
//! been invisible on the one platform where the app is hardest to debug
//! and where there is no `outl doctor` to fall back on. That is why a
//! blank-screen report could only be narrowed by elimination.
//!
//! `tauri_plugin_log` already owns the `log` crate's slot and writes the
//! **frontend's** lines to a rotating file. Our `tracing::` events
//! deliberately bypass it (see `lib.rs` on why the two logger slots are
//! kept apart), so they need their own sink.
//!
//! This is that sink: `$HOME/Library/Logs/outl-backend.log` inside the app
//! sandbox, **truncated on every launch**. Truncating rather than rotating
//! is the right trade here — what you want from a backend log on a phone
//! is "what did *this* launch do", and a file that only ever holds one
//! launch cannot grow without bound on a device nobody prunes.
//!
//! Read it with:
//!
//! ```text
//! cat "$(xcrun simctl get_app_container <udid> <bundle> data)/Library/Logs/outl-backend.log"
//! ```
//!
//! Writes still go to stderr too, so a desktop-style run is unchanged.

use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;

/// Create (and truncate) the backend log, returning a handle to clone per
/// event.
///
/// `$HOME` is the app container on iOS and Android, so the path never
/// escapes the sandbox. `None` when there is nowhere to put it — the
/// writer then degrades to the stderr-only behaviour this replaced,
/// never to a panic.
pub fn open() -> Option<File> {
    let dir = PathBuf::from(std::env::var_os("HOME")?)
        .join("Library")
        .join("Logs");
    std::fs::create_dir_all(&dir).ok()?;
    File::create(dir.join("outl-backend.log")).ok()
}

/// A `tracing` writer that fans every line out to stderr **and** the
/// backend log.
///
/// Both halves are best-effort: a log that cannot be written must never
/// take down the app that was only trying to describe itself.
pub struct Tee(Option<File>);

impl Tee {
    /// Clone the shared handle for one event. A failed clone degrades to
    /// stderr-only rather than dropping the line.
    pub fn for_event(file: Option<&File>) -> Self {
        Tee(file.and_then(|f| f.try_clone().ok()))
    }
}

impl Write for Tee {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(file) = self.0.as_mut() {
            let _ = file.write_all(buf);
        }
        outl_telemetry::lossy_stderr().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = self.0.as_mut() {
            let _ = file.flush();
        }
        outl_telemetry::lossy_stderr().flush()
    }
}
