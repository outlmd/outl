//! Whether Windows has a WebView2 runtime, decided before Tauri's setup runs.
//!
//! `tauri::webview_version` (tauri 2.12.1, the version in this lockfile) is
//! `Result<String, tauri::Error>`, documented as "Get WebView/Webkit version
//! on current platform". The caller maps the error to `String` so this
//! decision does not name `tauri::Error` and the tests run on Linux.
//! `tauri-runtime-wry` records `wry::webview_version().is_ok()`
//! when the runtime starts. On Windows a `false` becomes
//! `Error::WebviewRuntimeNotInstalled`, whose `Display` is
//! `Could not find the webview runtime, make sure it is installed`.
//! Wrapped as `runtime error: …`, that is what `App::run` panics with:
//! `Failed to setup app: {e}`. The release build shows Tauri's dialog and
//! then panics, and that panic is the crash report.
//!
//! [`webview_boot`] is that `is_ok()` check, with a message instead of a
//! panic. `run` calls it on Windows only, and before crash reporting starts.
//! macOS and Linux do not: a failed WebKit probe on macOS is not a missing
//! runtime, and Tauri still creates the webview there.
//!
//! The decision is therefore unused on those targets outside tests.
//! It stays in this crate so the tests do not need a Windows machine.

#![cfg_attr(not(any(windows, test)), allow(dead_code))]

/// Microsoft's WebView2 page.
/// It is the download for the Evergreen runtime (bootstrapper and standalone
/// installer). The same URL is what Tauri's own Windows dialog names.
pub(crate) const EVERGREEN_WEBVIEW2_URL: &str =
    "https://developer.microsoft.com/en-us/microsoft-edge/webview2/";

/// Process status for a machine with no usable WebView2 runtime.
pub(crate) const MISSING_RUNTIME_EXIT_CODE: i32 = 1;

/// What `run` should do with the result of `tauri::webview_version`.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum WebviewBoot {
    /// A version came back. Tauri's gate is `is_ok()`, so start the app.
    Continue,
    /// No runtime. Show `message` and exit `code`. Do not build the app.
    Exit { code: i32, message: String },
}

/// `version` is the result of `tauri::webview_version`.
///
/// `Ok` continues, including an empty string: that is what `is_ok()` means,
/// and it is the condition under which Tauri will create a window. Treating
/// an empty `Ok` as missing would refuse machines Tauri is willing to start.
/// `Err` is the missing-runtime case. The error's text is not consulted —
/// Tauri does not consult it either, and a second reading of the same
/// `Result` is how the two answers drift.
pub(crate) fn webview_boot(version: Result<String, String>) -> WebviewBoot {
    match version {
        Ok(_) => WebviewBoot::Continue,
        Err(_) => WebviewBoot::Exit {
            code: MISSING_RUNTIME_EXIT_CODE,
            message: missing_runtime_message(),
        },
    }
}

pub(crate) fn missing_runtime_message() -> String {
    format!(
        "outl needs the Microsoft Edge WebView2 Runtime, and this PC does not have it.\n\
         \n\
         Download the Evergreen runtime, install it, and open outl again:\n\
         \n\
         {EVERGREEN_WEBVIEW2_URL}"
    )
}

/// Windows entry from `run`. A missing runtime is a message and a non-zero
/// exit, before the crash reporter exists, so the failure is not a report.
#[cfg(windows)]
pub(crate) fn exit_if_runtime_missing(version: Result<String, String>) {
    if let WebviewBoot::Exit { code, message } = webview_boot(version) {
        tell_and_exit(&message, code);
    }
}

/// Release builds set `windows_subsystem = "windows"`, so there is no console
/// for a launched app. The message box is what the user sees. The same text
/// also goes to stderr, for a terminal launch, through the writer that treats
/// a closed pipe as a finished write.
#[cfg(windows)]
fn tell_and_exit(message: &str, code: i32) -> ! {
    use std::io::Write;
    // Same writer as the tracing subscriber: a closed stderr must not turn
    // this exit into `failed printing to stderr`.
    let _ = writeln!(outl_telemetry::lossy_stderr(), "{message}");
    show_message_box(message);
    std::process::exit(code);
}

#[cfg(windows)]
fn show_message_box(message: &str) {
    // Win32 MessageBoxW flags. The process has no window yet.
    const MB_OK: u32 = 0x0000_0000;
    const MB_ICONERROR: u32 = 0x0000_0010;
    const MB_SETFOREGROUND: u32 = 0x0001_0000;
    const MB_TOPMOST: u32 = 0x0004_0000;

    let text = wide(message);
    let caption = wide("outl");
    // SAFETY: both pointers are NUL-terminated UTF-16 produced by `wide`
    // and live for this call. MessageBoxW does not retain them. A null
    // owner is correct: no webview window exists yet.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBoxW(
        hwnd: *mut std::ffi::c_void,
        text: *const u16,
        caption: *const u16,
        typ: u32,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_runtime_exits_with_the_evergreen_download() {
        let decision = webview_boot(Err(
            "Could not find the webview runtime, make sure it is installed".to_string(),
        ));
        match decision {
            WebviewBoot::Exit { code, message } => {
                assert_eq!(code, MISSING_RUNTIME_EXIT_CODE);
                assert_ne!(code, 0);
                assert_eq!(message, missing_runtime_message());
                assert!(message.contains("WebView2"));
                assert!(message.contains("Evergreen"));
                assert!(message.contains(EVERGREEN_WEBVIEW2_URL));
                assert!(!message.contains("panic"));
            }
            WebviewBoot::Continue => panic!("a missing runtime must not start the app"),
        }
    }

    #[test]
    fn any_probe_error_is_a_missing_runtime() {
        let decision = webview_boot(Err(String::new()));
        assert!(matches!(
            decision,
            WebviewBoot::Exit {
                code: MISSING_RUNTIME_EXIT_CODE,
                ..
            }
        ));
    }

    #[test]
    fn a_reported_version_starts_the_app() {
        let decision = webview_boot(Ok("131.0.2903.112".to_string()));
        assert_eq!(decision, WebviewBoot::Continue);
    }

    /// Tauri's gate is `is_ok()`. An empty `Ok` is not the panic this
    /// module exists to stop, and refusing it would be a second opinion.
    #[test]
    fn an_empty_version_string_still_starts_the_app() {
        let decision = webview_boot(Ok(String::new()));
        assert_eq!(decision, WebviewBoot::Continue);
    }
}
