//! The `log` crate's stdout target.
//!
//! `tauri-plugin-log`'s `TargetKind::Stdout` is fern's stdout logger.
//! fern writes with `write!`, and on failure it retries on stderr and then
//! panics if that write fails too. A desktop with no terminal has both
//! closed. `Output::call` does not retry, so a closed pipe drops the line.

use std::io::Write;

pub(crate) fn stdout_target() -> tauri_plugin_log::Target {
    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Dispatch(
        tauri_plugin_log::fern::Dispatch::new().chain(tauri_plugin_log::fern::Output::call(
            |record| {
                let _ = writeln!(std::io::stdout(), "{}", record.args());
            },
        )),
    ))
}
