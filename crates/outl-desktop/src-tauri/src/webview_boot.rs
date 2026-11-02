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
//!
//! ## A present runtime whose webview still fails to create (OUTL-4)
//!
//! A present WebView2 runtime can still fail to hand Tauri a controller:
//! `WebView2Error(Windows): Any webview2_com::Error (WebView2 COM call
//! failure)` (wry's own error catalog) covers this, and the observed crash
//! carried `WindowsError(... ERROR_TIMEOUT ...)` — the environment/controller
//! creation timed out, not a missing install. `tauri.conf.json`'s `app.windows`
//! array is normally created automatically inside `App::run`, which is the
//! same place the missing-runtime panic above fires from, and that path has
//! no retry.
//!
//! `tauri.windows.conf.json` carries the same "main" entry as `tauri.conf.json`,
//! plus `"create": false` (`WindowConfig::create`, default `true` — see
//! `tauri-utils` 2.10.x `config.rs`), so `App::run` parses the window but does
//! not build it. Platform configs merge as a JSON merge patch
//! (`json_patch::merge`, RFC 7396): a patch array value replaces the base's
//! wholesale rather than merging element-by-element, so the Windows file
//! fully restates the entry instead of overriding one field of it — this
//! does not touch macOS/Linux, which never read `tauri.windows.conf.json`.
//! [`open_main_window`] builds "main" itself from inside `setup` (the
//! officially documented place to do this — `tauri`'s own docs build a
//! window there) via `WebviewWindowBuilder::from_config`, reading the same
//! config entry Windows skipped, so the two files describe one window
//! instead of needing to be hand-kept in sync. A `tauri::Result` is ours to
//! inspect there instead of `App::run`'s. [`webview_create_decision`] is the
//! retry/give-up call: a bounded number of attempts for a timing failure
//! that boot-time contention (antivirus scan, a cold profile, a slow disk)
//! can clear on its own, then the same message-box-and-exit `tell_and_exit`
//! the missing-runtime case uses, never a panic.
//!
//! ## Does a failed attempt leave "main" registered for the retry?
//!
//! No, not in `tauri`/`tauri-runtime-wry` 2.12.1 (the pinned version):
//! `WindowManager::attach_window` — the call that inserts "main" into the
//! map `get_webview_window`/`WindowLabelAlreadyExists` both read — only runs
//! inside the `.map()` on `RuntimeOrDispatch::create_window`'s `Ok` arm
//! (`tauri::window::mod.rs`). A controller-creation timeout (OUTL-4) makes
//! `tauri_runtime_wry::create_webview` return `Err` from inside
//! `create_window`'s free function, before that `Ok` arm, so "main" is never
//! attached and a retry's `WebviewWindowBuilder::from_config` does not
//! collide with it. The native platform window `create_window` built first
//! is a local that goes out of scope on that early return, and dropping it is what
//! closes the OS window; it never reaches the `Arc` that would keep it
//! alive. [`clear_leftover_main_window`] exists anyway: relying on that
//! being true forever is a second opinion about an internal detail one
//! `tauri`/`wry` bump can change, and checking costs nothing when, as
//! today, there is nothing to clear.

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

/// Process status for a webview that never finished creating.
#[cfg(windows)]
pub(crate) const WEBVIEW_CREATE_FAILED_EXIT_CODE: i32 = 1;

/// Bounded attempts at creating the main webview before giving up. A COM
/// timeout during `CoreWebView2Environment`/`Controller` creation is the
/// observed failure (OUTL-4) and boot-time contention — antivirus scan, a
/// cold user profile, a slow disk — is the kind of thing a short wait clears.
/// Retrying forever would hang the launch instead of crashing it, which is
/// not an improvement.
pub(crate) const WEBVIEW_CREATE_MAX_ATTEMPTS: u32 = 3;

/// Delay between attempts. Long enough to let a timed-out COM call settle,
/// short enough that three attempts don't read as a hang.
#[cfg(windows)]
pub(crate) const WEBVIEW_CREATE_RETRY_DELAY_MS: u64 = 500;

/// What to do with `attempt`'s failed webview creation.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub(crate) enum WebviewCreateStep {
    /// Below the cap: wait and try again.
    Retry,
    /// At the cap: show `message` and exit. Do not try again, do not panic.
    GiveUp { message: String },
}

/// `attempt` is the attempt number that just failed with `error` (1-based).
/// Below [`WEBVIEW_CREATE_MAX_ATTEMPTS`] this always retries — the error text
/// is not consulted to tell a timeout apart from anything else, the same
/// stance [`webview_boot`] takes on the missing-runtime error: a library
/// upgrade can reword a message, and a second reading of this result is how
/// that reading and `App::run`'s drift.
pub(crate) fn webview_create_decision(attempt: u32, error: &str) -> WebviewCreateStep {
    if attempt < WEBVIEW_CREATE_MAX_ATTEMPTS {
        WebviewCreateStep::Retry
    } else {
        WebviewCreateStep::GiveUp {
            message: webview_create_failed_message(error),
        }
    }
}

pub(crate) fn webview_create_failed_message(error: &str) -> String {
    format!(
        "outl could not open its window after {WEBVIEW_CREATE_MAX_ATTEMPTS} attempts.\n\
         \n\
         Windows reported: {error}\n\
         \n\
         Close other apps and try opening outl again. If this keeps happening,\n\
         reinstalling the Microsoft Edge WebView2 Runtime can help:\n\
         \n\
         {EVERGREEN_WEBVIEW2_URL}"
    )
}

/// Label both `tauri.conf.json` and `tauri.windows.conf.json` give the one
/// window this app creates. [`build_main_window`] looks the config entry up
/// by this label instead of a positional index.
pub(crate) const MAIN_WINDOW_LABEL: &str = "main";

/// Whatever can report and clear a leftover "main" window/webview from a
/// previous attempt. The module docs on "does a failed attempt leave 'main'
/// registered" say no for the failure this module retries on, in the
/// `tauri`/`wry` version pinned today — this exists to not depend on that
/// staying true. A trait instead of a bare function so a test can supply a
/// fake and run without a `tauri::AppHandle`.
pub(crate) trait LeftoverMainWindow {
    /// Whether "main" is currently registered.
    fn is_registered(&self) -> bool;
    /// Tears "main" down. Only called when [`is_registered`](Self::is_registered) answered `true`.
    fn destroy(&self);
}

/// Clears whatever `handle` reports as a leftover "main" window, if any.
/// Called before every attempt in [`open_main_window`], including the
/// first, where it is always a no-op — the cost is one map lookup.
pub(crate) fn clear_leftover_main_window(handle: &impl LeftoverMainWindow) {
    if handle.is_registered() {
        handle.destroy();
    }
}

/// [`LeftoverMainWindow`] over a real app handle.
#[cfg(windows)]
impl LeftoverMainWindow for tauri::AppHandle {
    fn is_registered(&self) -> bool {
        tauri::Manager::get_webview_window(self, MAIN_WINDOW_LABEL).is_some()
    }

    fn destroy(&self) {
        if let Some(window) = tauri::Manager::get_webview_window(self, MAIN_WINDOW_LABEL) {
            let _ = window.destroy();
        }
    }
}

/// Windows entry from `setup`: create the main window ourselves, retrying a
/// present-but-not-yet-ready WebView2 runtime instead of letting `App::run`
/// hit it with no retry (OUTL-4). `tauri.windows.conf.json` sets "main"'s
/// `create: false` so `App::run` does not also try to create it.
#[cfg(windows)]
pub(crate) fn open_main_window(app: &tauri::AppHandle) {
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        clear_leftover_main_window(app);
        match build_main_window(app) {
            Ok(_) => return,
            Err(err) => match webview_create_decision(attempt, &err.to_string()) {
                WebviewCreateStep::Retry => {
                    std::thread::sleep(std::time::Duration::from_millis(
                        WEBVIEW_CREATE_RETRY_DELAY_MS,
                    ));
                }
                WebviewCreateStep::GiveUp { message } => {
                    tell_and_exit(&message, WEBVIEW_CREATE_FAILED_EXIT_CODE);
                }
            },
        }
    }
}

/// Builds "main" from the config entry Windows parses but does not
/// auto-create (`tauri.windows.conf.json`'s `create: false`), so this is
/// the only place "main" is actually built on Windows, and its properties
/// come from the one config both platforms' files describe — nothing here
/// hardcodes title/size that could drift from `tauri.conf.json`.
#[cfg(windows)]
fn build_main_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    let config = main_window_config(app)?;
    tauri::WebviewWindowBuilder::from_config(app, &config)?.build()?;
    Ok(())
}

/// Looks "main" up in `app.config().app.windows` by label. `tauri.conf.json`
/// and `tauri.windows.conf.json` are pinned to both carry it by
/// `the_windows_config_matches_the_base_config_entry` below, so this should
/// never miss outside a config edit that dropped the entry — `WindowNotFound`
/// is the same error `tauri` itself uses for an unresolvable window label.
#[cfg(windows)]
fn main_window_config(app: &tauri::AppHandle) -> tauri::Result<tauri::utils::config::WindowConfig> {
    tauri::Manager::config(app)
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW_LABEL)
        .cloned()
        .ok_or(tauri::Error::WindowNotFound)
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

    #[test]
    fn a_timed_out_attempt_below_the_cap_retries() {
        for attempt in 1..WEBVIEW_CREATE_MAX_ATTEMPTS {
            let decision = webview_create_decision(attempt, "ERROR_TIMEOUT");
            assert_eq!(
                decision,
                WebviewCreateStep::Retry,
                "attempt {attempt} should still retry"
            );
        }
    }

    #[test]
    fn hitting_the_cap_gives_up_instead_of_retrying_forever() {
        let decision = webview_create_decision(
            WEBVIEW_CREATE_MAX_ATTEMPTS,
            "WindowsError(Error { code: HRESULT(0x800705B4), message: \
             \"This operation returned because the timeout period expired.\" })",
        );
        match decision {
            WebviewCreateStep::GiveUp { message } => {
                assert!(message.contains("outl could not open its window"));
                assert!(message.contains("HRESULT(0x800705B4)"));
                assert!(message.contains(EVERGREEN_WEBVIEW2_URL));
                assert!(!message.contains("panic"));
            }
            WebviewCreateStep::Retry => panic!("the attempt cap must stop retrying"),
        }
    }

    /// The decision never consults the error text — a non-timeout failure
    /// gets the exact same bounded retry, matching `webview_boot`'s stance
    /// that reading the error itself is a second opinion that can drift.
    #[test]
    fn a_non_timeout_error_follows_the_same_bounded_retry() {
        assert_eq!(
            webview_create_decision(1, "some other webview2 failure"),
            WebviewCreateStep::Retry
        );
        assert!(matches!(
            webview_create_decision(WEBVIEW_CREATE_MAX_ATTEMPTS, "some other webview2 failure"),
            WebviewCreateStep::GiveUp { .. }
        ));
    }

    struct FakeMainWindow {
        registered: std::cell::Cell<bool>,
        destroy_calls: std::cell::Cell<u32>,
    }

    impl LeftoverMainWindow for FakeMainWindow {
        fn is_registered(&self) -> bool {
            self.registered.get()
        }

        fn destroy(&self) {
            self.destroy_calls.set(self.destroy_calls.get() + 1);
            self.registered.set(false);
        }
    }

    #[test]
    fn a_leftover_main_window_gets_destroyed() {
        let fake = FakeMainWindow {
            registered: std::cell::Cell::new(true),
            destroy_calls: std::cell::Cell::new(0),
        };
        clear_leftover_main_window(&fake);
        assert_eq!(fake.destroy_calls.get(), 1);
        assert!(!fake.is_registered());
    }

    /// The common case, every first attempt: nothing to clear, so this must
    /// not call `destroy` and must not need a live `tauri::AppHandle` to say so.
    #[test]
    fn no_leftover_main_window_is_left_alone() {
        let fake = FakeMainWindow {
            registered: std::cell::Cell::new(false),
            destroy_calls: std::cell::Cell::new(0),
        };
        clear_leftover_main_window(&fake);
        assert_eq!(fake.destroy_calls.get(), 0);
    }

    const BASE_CONFIG: &str = include_str!("../tauri.conf.json");
    const WINDOWS_CONFIG: &str = include_str!("../tauri.windows.conf.json");

    /// Platform configs merge as a JSON merge patch (RFC 7396): a patch's
    /// array value replaces the base's wholesale, so
    /// `tauri.windows.conf.json`'s "main" entry is not a diff against
    /// `tauri.conf.json`'s, it is a full restatement of it. This pins the
    /// two to actually agree — the thing `build_main_window` depends on,
    /// since it builds from whichever config entry Windows parsed, not
    /// from hardcoded properties kept in sync by hand.
    #[test]
    fn the_windows_config_matches_the_base_config_entry_except_create() {
        let base: serde_json::Value =
            serde_json::from_str(BASE_CONFIG).expect("tauri.conf.json must parse");
        let windows: serde_json::Value =
            serde_json::from_str(WINDOWS_CONFIG).expect("tauri.windows.conf.json must parse");

        let base_main = base["app"]["windows"]
            .as_array()
            .and_then(|windows| windows.iter().find(|w| w["label"] == MAIN_WINDOW_LABEL))
            .expect("tauri.conf.json must declare a \"main\" window");

        let windows_main = windows["app"]["windows"]
            .as_array()
            .and_then(|windows| windows.iter().find(|w| w["label"] == MAIN_WINDOW_LABEL))
            .expect("tauri.windows.conf.json must declare a \"main\" window");

        assert_eq!(
            windows_main.get("create"),
            Some(&serde_json::Value::Bool(false)),
            "tauri.windows.conf.json's \"main\" must set create: false, \
             so App::run parses it without also building it"
        );

        let mut windows_main_without_create = windows_main.clone();
        windows_main_without_create
            .as_object_mut()
            .expect("a window config entry must be a JSON object")
            .remove("create");

        assert_eq!(
            &windows_main_without_create, base_main,
            "tauri.windows.conf.json's \"main\" must match tauri.conf.json's, aside from create"
        );
    }
}
