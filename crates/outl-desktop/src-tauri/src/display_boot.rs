//! Whether Linux can open a display, decided before Tauri builds the app.
//!
//! `tao` 0.37.2 (the version in this lockfile), `EventLoop::new`, builds its
//! GTK backend with `new_gtk(..).expect("Failed to initialize gtk backend!")`,
//! and `new_gtk` starts with `gtk::init()?`. `gtk` 0.18.2's `gtk::init` calls
//! `gtk_init_check` and returns `Err("Failed to initialize GTK")` when the
//! windowing system cannot be opened. That is an environment with no usable
//! display: no `DISPLAY` and no `WAYLAND_DISPLAY` (SSH, a headless session, a
//! systemd unit), or a `DISPLAY` that points at an X server that is gone.
//! The `expect` turns it into a panic, and the panic hook of the crash
//! reporter files it (OUTL-6).
//!
//! [`display_boot`] is that `gtk::init()` result, with a message instead of a
//! panic. `run` calls it on Linux only, and before crash reporting starts, so
//! the exit is not a report. The result is the authority, not the environment:
//! `GDK_BACKEND=broadway` and similar setups open a display with neither
//! variable set, and reading the variables ourselves would refuse them. The
//! variables only shape the sentence the user gets.
//!
//! Calling `gtk::init()` here, on the main thread, is safe for what comes
//! next: `gtk::init` returns `Ok(())` at once when
//! `is_initialized_main_thread()` is true, so the call `tao` makes later is a
//! no-op instead of a second initialization. `run` is on the main thread, which
//! is also the thread `tao` builds its event loop on.
//!
//! The decision is unused on other targets outside tests. It stays in this
//! crate unconditionally so the tests do not need a Linux machine.

#![cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]

/// Process status for a machine with no usable display. Same as the missing
/// WebView2 runtime: the app did not start.
pub(crate) const NO_DISPLAY_EXIT_CODE: i32 = 1;

/// What `run` should do with the result of `gtk::init`.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum DisplayBoot {
    /// GTK initialized. Build the app, whatever the environment says.
    Continue,
    /// No display. Print `message` and exit `code`. Do not build the app.
    Exit { code: i32, message: String },
}

/// `gtk_init` is the result of `gtk::init`, with the error mapped to `String`
/// so this decision does not name `glib::BoolError`.
/// `display` and `wayland_display` are `DISPLAY` and `WAYLAND_DISPLAY`.
///
/// `Ok` continues with no environment at all: that is what `tao` will see too.
/// `Err` is the case `tao` panics on. The error's text is not consulted
/// (`gtk` words it the same for every cause), and the variables only pick the
/// message. An empty string counts as unset, because GTK does not treat it as
/// a display either.
pub(crate) fn display_boot(
    gtk_init: Result<(), String>,
    display: Option<&str>,
    wayland_display: Option<&str>,
) -> DisplayBoot {
    match gtk_init {
        Ok(()) => DisplayBoot::Continue,
        Err(_) => DisplayBoot::Exit {
            code: NO_DISPLAY_EXIT_CODE,
            message: no_display_message(non_empty(display), non_empty(wayland_display)),
        },
    }
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.is_empty())
}

const NO_VARIABLES_MESSAGE: &str = "outl needs a graphical session, and neither DISPLAY nor \
     WAYLAND_DISPLAY is set.\n\
     \n\
     This usually means outl was started over SSH, from a headless session, \
     or from a service. Open it from your desktop session instead.\n\
     \n\
     The terminal client `outl` works without a display.";

pub(crate) fn no_display_message(display: Option<&str>, wayland_display: Option<&str>) -> String {
    match (display, wayland_display) {
        (None, None) => NO_VARIABLES_MESSAGE.to_string(),
        (display, wayland_display) => {
            let set = [("DISPLAY", display), ("WAYLAND_DISPLAY", wayland_display)]
                .into_iter()
                .filter_map(|(name, value)| value.map(|value| format!("{name}={value}")))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "outl could not open the display ({set}).\n\
                 \n\
                 The X or Wayland server may have stopped, or this session is not allowed \
                 to use it. Check that your graphical session is running and open outl \
                 from it.\n\
                 \n\
                 The terminal client `outl` works without a display."
            )
        }
    }
}

/// Linux entry from `run`. No display is a message and a non-zero exit, before
/// the crash reporter exists, so the failure is not a report.
#[cfg(target_os = "linux")]
pub(crate) fn exit_if_display_unavailable() {
    use std::io::Write;

    let gtk_init = gtk::init().map_err(|err| err.to_string());
    let display = std::env::var("DISPLAY").ok();
    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    if let DisplayBoot::Exit { code, message } =
        display_boot(gtk_init, display.as_deref(), wayland_display.as_deref())
    {
        // Same writer as the tracing subscriber: a closed stderr must not turn
        // this exit into `failed printing to stderr`.
        let _ = writeln!(outl_telemetry::lossy_stderr(), "{message}");
        std::process::exit(code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gtk_failed() -> Result<(), String> {
        Err("Failed to initialize GTK".to_string())
    }

    fn exit_message(decision: DisplayBoot) -> String {
        match decision {
            DisplayBoot::Exit { code, message } => {
                assert_eq!(code, NO_DISPLAY_EXIT_CODE);
                assert_ne!(code, 0);
                message
            }
            DisplayBoot::Continue => panic!("a failed gtk::init must not reach tao's expect"),
        }
    }

    #[test]
    fn no_display_variables_exits_naming_the_missing_display() {
        let message = exit_message(display_boot(gtk_failed(), None, None));
        assert!(message.contains("graphical session"));
        assert!(message.contains("neither DISPLAY nor WAYLAND_DISPLAY is set"));
        assert!(message.contains("SSH"));
        assert!(message.contains("`outl` works without a display"));
        assert!(!message.contains("panic"));
    }

    #[test]
    fn empty_variables_count_as_unset() {
        let message = exit_message(display_boot(gtk_failed(), Some(""), Some("")));
        assert_eq!(message, no_display_message(None, None));
        assert!(message.contains("neither DISPLAY nor WAYLAND_DISPLAY is set"));
    }

    #[test]
    fn a_display_that_cannot_be_opened_names_its_value() {
        let message = exit_message(display_boot(gtk_failed(), Some(":99"), None));
        assert!(message.contains("DISPLAY=:99"));
        assert!(!message.contains("WAYLAND_DISPLAY="));
        assert!(!message.contains("neither"));
        assert!(!message.contains("panic"));
    }

    #[test]
    fn a_wayland_display_that_cannot_be_opened_names_its_value() {
        let message = exit_message(display_boot(gtk_failed(), None, Some("wayland-7")));
        assert!(message.contains("WAYLAND_DISPLAY=wayland-7"));
        assert!(!message.contains("DISPLAY=:"));
        assert!(!message.contains("neither"));
        assert!(!message.contains("panic"));
    }

    #[test]
    fn both_variables_are_named_and_an_empty_one_is_left_out() {
        let both = exit_message(display_boot(gtk_failed(), Some(":0"), Some("wayland-0")));
        assert!(both.contains("DISPLAY=:0, WAYLAND_DISPLAY=wayland-0"));

        let one_empty = exit_message(display_boot(gtk_failed(), Some(":0"), Some("")));
        assert!(one_empty.contains("DISPLAY=:0"));
        assert!(!one_empty.contains("WAYLAND_DISPLAY="));
    }

    #[test]
    fn the_error_text_is_not_consulted() {
        assert_eq!(
            display_boot(Err(String::new()), None, None),
            display_boot(gtk_failed(), None, None)
        );
    }

    /// `GDK_BACKEND=broadway` opens a display with neither variable set.
    /// The `gtk::init` result decides, not the environment.
    #[test]
    fn a_working_gtk_continues_whatever_the_environment_says() {
        assert_eq!(display_boot(Ok(()), None, None), DisplayBoot::Continue);
        assert_eq!(
            display_boot(Ok(()), Some(""), Some("")),
            DisplayBoot::Continue
        );
        assert_eq!(
            display_boot(Ok(()), Some(":0"), Some("wayland-0")),
            DisplayBoot::Continue
        );
    }
}
