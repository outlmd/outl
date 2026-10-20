//! The `log` crate's stdout target.
//!
//! `tauri-plugin-log`'s `TargetKind::Stdout` is fern's stdout logger.
//! fern writes with `write!`, and on failure it retries on stderr and then
//! panics if that write fails too. A desktop with no terminal has both
//! closed. `Output::call` does not retry, so a closed pipe drops the line.
//!
//! This dispatch has no formatter of its own. The plugin's parent dispatch
//! formats first, then hands the child `record.args()` already shaped as
//! `[date][time][target][LEVEL] message` (desktop `Builder` default). Printing
//! `record.args()` is what keeps that line. Rebuilding it from the level and
//! the original message would drop the time.

use std::io::Write;

pub(crate) fn stdout_target() -> tauri_plugin_log::Target {
    tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Dispatch(
        tauri_plugin_log::fern::Dispatch::new().chain(tauri_plugin_log::fern::Output::call(
            |record| emit_stdout_line(&mut std::io::stdout(), record),
        )),
    ))
}

fn emit_stdout_line(out: &mut dyn Write, record: &log::Record) {
    let _ = writeln!(out, "{}", record.args());
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;

    fn captured() -> &'static Mutex<Vec<u8>> {
        static LINE: OnceLock<Mutex<Vec<u8>>> = OnceLock::new();
        LINE.get_or_init(|| Mutex::new(Vec::new()))
    }

    #[test]
    fn the_stdout_line_is_the_plugin_format_not_the_raw_message() {
        // Same parent/child split as `tauri_plugin_log::Builder`: the parent
        // applies the desktop format, the child is the dispatch
        // `stdout_target` builds (no formatter, `Output::call`).
        captured().lock().expect("line").clear();
        let dispatch = tauri_plugin_log::fern::Dispatch::new()
            .format(|out, message, record| {
                out.finish(format_args!(
                    "[2026-10-20][16:42:00][{}][{}] {}",
                    record.target(),
                    record.level(),
                    message
                ))
            })
            .chain(tauri_plugin_log::fern::Dispatch::new().chain(
                tauri_plugin_log::fern::Output::call(|record| {
                    let mut buf = captured().lock().expect("line");
                    emit_stdout_line(&mut *buf, record);
                }),
            ));
        let (_max, logger) = dispatch.into_log();
        logger.log(
            &log::Record::builder()
                .args(format_args!("hello"))
                .level(log::Level::Info)
                .target("outl_desktop")
                .build(),
        );
        assert_eq!(
            String::from_utf8(captured().lock().expect("line").clone()).expect("utf8"),
            "[2026-10-20][16:42:00][outl_desktop][INFO] hello\n"
        );
    }
}
