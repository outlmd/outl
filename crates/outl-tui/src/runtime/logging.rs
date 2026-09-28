//! Keep `tracing` off the TUI canvas.
//!
//! Every dependency that logs (Steel, wasmtime, notify) would
//! otherwise print straight onto the drawn frame, which reads as a
//! corrupted terminal. The records still go somewhere debuggable.

use std::path::Path;

/// Wire up `tracing` so nothing prints onto the TUI canvas, but logs
/// stay debuggable.
///
/// Order matters here:
/// 1. **`tracing-log` bridge** — Steel and a few other deps emit via
///    the older `log` crate. Without the bridge `tracing-subscriber`
///    never sees those records and the default `log` impl falls back
///    to printing on stderr — straight onto our TUI canvas. Install
///    the bridge *before* the subscriber so the subscriber catches
///    every record.
/// 2. **`tracing-subscriber`** with a file appender at
///    `<workspace>/.outl/tui.log`. `tail -f` it when something looks
///    off.
/// 3. Best-effort installation: if a global was already set (rare,
///    but possible when a dep front-runs us), `try_init` swallows
///    the error — the TUI keeps running with whatever subscriber
///    was registered first.
pub(super) fn install_silent_log_subscriber(workspace_root: &Path) {
    use std::fs::OpenOptions;
    use tracing_log::LogTracer;
    use tracing_subscriber::{fmt, EnvFilter};

    // Bridge `log` → `tracing`. Idempotent: re-init returns Err which
    // we ignore.
    let _ = LogTracer::init();

    let log_path = workspace_root.join(".outl").join("tui.log");
    let _ = std::fs::create_dir_all(log_path.parent().unwrap_or(workspace_root));
    let file = OpenOptions::new().create(true).append(true).open(&log_path);

    // `RUST_LOG` still wins if the user wants verbose output — useful
    // when reporting a bug. Default = warn; everything noisier gets
    // dropped on the floor.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));

    let builder = fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(true);

    let result = match file {
        Ok(f) => builder.with_writer(f).try_init(),
        // No log file? Still install a sink so the canvas stays clean.
        Err(_) => builder.with_writer(std::io::sink).try_init(),
    };
    // Errors here mean a global subscriber was set by an earlier
    // caller — fine, just move on.
    let _ = result;
}
