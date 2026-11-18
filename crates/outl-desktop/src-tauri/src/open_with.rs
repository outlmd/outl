//! OS-level "Open With → outl" plumbing.
//!
//! Three platforms deliver the same gesture three ways, and all three
//! land here:
//!
//! - **macOS** — an Apple Event, surfaced by Tauri as
//!   [`tauri::RunEvent::Opened`] carrying `file://` URLs. It fires for
//!   both a cold launch and a running app, and it is the *same* event
//!   `outl://` deep links arrive on, so [`file_path_from`] keeps only
//!   `file://` — a deep link is the `deep-link` plugin's job and
//!   handling it twice would navigate twice.
//! - **Linux / Windows, cold start** — the path is a command-line
//!   argument (`outl /path/to/notes.md`), read once at setup.
//! - **Linux / Windows, warm** — the second process's `argv`, forwarded
//!   by `tauri-plugin-single-instance`.
//!
//! # Why a pending buffer
//!
//! Same reason the deep-link path has one (issue #98): on a cold start
//! the frontend has not mounted its listener yet, so an emit would be
//! lost and the app would silently open today's journal instead of the
//! file the user double-clicked. The cold path buffers; the warm path
//! emits directly, because the listener is already up.
//!
//! # What this module does not decide
//!
//! Where the page lands, whether re-opening imports again, which
//! extensions count — all of that is
//! [`outl_actions::open_with`]'s. This module only turns an OS gesture
//! into one `source_path` string.

use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use tauri::{Emitter, Manager, Runtime};

/// Event the frontend listens on for a file opened while the app runs.
pub const OPEN_FILE_EVENT: &str = "open-file://import";

/// A file the OS asked us to open before the frontend could listen,
/// plus the one fact that says whether it can listen yet.
///
/// **`listening` is set by the frontend, not inferred.** It used to be
/// approximated as "does the main window exist", which is `true` from
/// the moment `Builder::build` runs `setup` — before the event loop
/// starts, so before any `RunEvent::Opened` can arrive. That made the
/// buffer unreachable on macOS and dropped every cold-start file on
/// the floor: double-clicking a `.md` with outl closed emitted into
/// nothing and the app opened today's journal instead. The window
/// existing never meant the webview had mounted, which is the only
/// thing that matters here.
pub struct PendingOpenFile {
    path: Mutex<Option<String>>,
    /// Whether the frontend has drained the buffer at least once, which
    /// it does immediately after registering its listener.
    ///
    /// Cleared again when the webview starts a new page load: a reload
    /// (`Cmd+R` in dev, a crash recovery) tears the listener down while
    /// the process keeps running, and a latch that only ever goes up
    /// would keep claiming somebody is listening. That is the same lie
    /// "does a window exist" told, one layer in.
    listening: AtomicBool,
}

impl PendingOpenFile {
    /// Empty buffer, for `app.manage`.
    pub fn empty() -> Self {
        Self {
            path: Mutex::new(None),
            listening: AtomicBool::new(false),
        }
    }

    /// Whether an emit would reach a listener.
    pub fn is_listening(&self) -> bool {
        self.listening.load(Ordering::Acquire)
    }

    /// Forget the listener, because the page carrying it is going away.
    ///
    /// The next file buffers instead of being emitted into a webview
    /// that is still loading, and the remount drains it.
    pub fn listener_gone(&self) {
        self.listening.store(false, Ordering::Release);
    }

    /// Take the buffered file and record that the frontend is up.
    ///
    /// One call does both because they are one event: the frontend
    /// drains the buffer directly after subscribing, so the drain *is*
    /// the announcement. Two commands would let a client do one and
    /// forget the other, which is the state that loses files.
    pub fn take(&self) -> Option<String> {
        self.listening.store(true, Ordering::Release);
        self.path.lock().take()
    }

    /// Record `path` unless a cold-start target is already buffered.
    ///
    /// First one wins, matching the deep-link buffer: opening two files
    /// at once can only show one page, and picking the last would mean
    /// the order the OS happened to enumerate them decides.
    pub fn offer(&self, path: String) {
        let mut slot = self.path.lock();
        if slot.is_none() {
            *slot = Some(path);
        }
    }
}

/// Register the buffer on the `Builder`, so it is managed for the whole
/// life of the app rather than from some point inside `setup`.
///
/// **The ordering is the contract.** macOS delivers an "open documents"
/// Apple Event while `NSApplication` is still launching (window and
/// document restoration on relaunch, or a double-click on a cold start),
/// and Tauri hands it to `RunEvent::Opened` without waiting for `setup`
/// to finish. `setup` is also fallible, with several `?` ahead of any
/// `app.manage` placed inside it, so a bail leaves the event loop alive
/// with no buffer. Managing here means [`route`], `on_page_load` and the
/// `argv` read always find it.
pub fn install<R: Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder.manage(PendingOpenFile::empty())
}

/// The buffer, or `None` when nothing registered it.
///
/// An OS event must never take the process down, so every lookup that
/// is reachable from one goes through here instead of `Manager::state`,
/// which panics on a type that was never managed.
pub fn pending<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<tauri::State<'_, PendingOpenFile>> {
    let found = app.try_state::<PendingOpenFile>();
    if found.is_none() {
        tracing::warn!("open with: PendingOpenFile is not managed, ignoring the request");
    }
    found
}

/// Frontend command: take (and clear) the file buffered during cold
/// start. Returns `null` when the app launched normally.
#[tauri::command]
pub(crate) fn take_pending_open_file(pending: tauri::State<'_, PendingOpenFile>) -> Option<String> {
    pending.take()
}

/// Turn one OS-supplied string into a path we are willing to import.
///
/// Accepts a `file://` URL or a plain path, and returns `None` for
/// anything else: another URL scheme (an `outl://` deep link sharing
/// the macOS `Opened` event), a CLI flag, an extension
/// [`outl_actions::open_with`] does not read, or a path that is not a
/// file on this machine.
///
/// The extension check is deliberately the *shared* one rather than a
/// local list — a second copy is how the OS ends up offering outl for a
/// type the importer then refuses.
pub fn file_path_from(raw: &str) -> Option<String> {
    if !raw.starts_with("file://") && (raw.contains("://") || raw.starts_with('-')) {
        return None;
    }
    let decoded = outl_tauri_shared::helpers::normalize_picker_path(raw);

    let path = std::path::Path::new(&decoded);
    (outl_actions::open_with::is_supported(path) && path.is_file()).then_some(decoded)
}

/// Every importable file in a process's arguments, skipping `argv[0]`.
pub fn file_paths_from_argv<I, S>(argv: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    argv.into_iter()
        .skip(1)
        .filter_map(|a| file_path_from(a.as_ref()))
        .collect()
}

/// Warm path: a file opened while the app is running. Emit to the
/// listener (which is up) and bring the window forward.
pub fn dispatch<R: Runtime>(app: &tauri::AppHandle<R>, path: &str) {
    if let Err(err) = app.emit(OPEN_FILE_EVENT, path) {
        tracing::warn!("open with: failed to emit {OPEN_FILE_EVENT}: {err}");
    }
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_focus();
    }
}

/// Route one OS-supplied string: emit when the frontend is listening,
/// buffer when it is not.
///
/// **Every delivery path goes through here**, warm ones included. A
/// path that calls [`dispatch`] directly is one that drops the file
/// when the user opens it during a slow boot — the app shows its
/// loading screen until the workspace is ready, and the listener only
/// exists after that.
pub fn route<R: Runtime>(app: &tauri::AppHandle<R>, raw: &str) {
    let Some(path) = file_path_from(raw) else {
        return;
    };
    // A missing buffer drops the file rather than the process: see
    // [`pending`].
    let Some(pending) = pending(app) else {
        return;
    };
    if pending.is_listening() {
        dispatch(app, &path);
    } else {
        pending.offer(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deep_link_is_not_a_file_to_import() {
        // `outl://` arrives on the same macOS event as a `file://` URL.
        // Treating it as a path here would navigate twice — once from
        // the deep-link plugin, once from us.
        assert_eq!(file_path_from("outl://today"), None);
        assert_eq!(file_path_from("https://outl.app/x.md"), None);
        assert_eq!(file_path_from("--flag"), None);
    }

    #[test]
    fn an_unsupported_or_missing_file_is_not_a_path_to_import() {
        let dir = tempfile::TempDir::new().unwrap();
        let pdf = dir.path().join("a.pdf");
        std::fs::write(&pdf, "x").unwrap();
        assert_eq!(file_path_from(pdf.to_str().unwrap()), None);
        // Right extension, no such file — the OS never sends this, but
        // a stray `argv` entry would.
        assert_eq!(
            file_path_from(dir.path().join("ghost.md").to_str().unwrap()),
            None
        );
    }

    #[test]
    fn a_file_url_is_decoded_back_to_a_path() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("my notes.md");
        std::fs::write(&path, "- hi\n").unwrap();
        let url = format!("file://{}", path.to_str().unwrap().replace(' ', "%20"));
        assert_eq!(file_path_from(&url).as_deref(), path.to_str());
    }

    #[test]
    fn argv_skips_the_binary_and_keeps_only_importable_files() {
        let dir = tempfile::TempDir::new().unwrap();
        let md = dir.path().join("notes.md");
        std::fs::write(&md, "- hi\n").unwrap();
        let argv = vec!["/usr/bin/outl", "--flag", md.to_str().unwrap()];
        assert_eq!(file_paths_from_argv(argv), vec![md.to_str().unwrap()]);
        // `argv[0]` is never a file to import, even when it looks like one.
        assert!(file_paths_from_argv(vec![md.to_str().unwrap()]).is_empty());
    }

    #[test]
    fn the_first_cold_start_target_wins() {
        let pending = PendingOpenFile::empty();
        pending.offer("/a.md".into());
        pending.offer("/b.md".into());
        assert_eq!(pending.take().as_deref(), Some("/a.md"));
    }

    #[test]
    fn a_page_reload_puts_the_buffer_back_in_charge() {
        // The listener dies with the page. Leaving the latch up would
        // emit the next "Open With" into a webview that has not
        // remounted, which is the cold-start loss with extra steps.
        let pending = PendingOpenFile::empty();
        pending.take();
        assert!(pending.is_listening());

        pending.listener_gone();
        assert!(!pending.is_listening());
        pending.offer("/after-reload.md".into());
        assert_eq!(pending.take().as_deref(), Some("/after-reload.md"));
        assert!(pending.is_listening(), "the remount drains and re-arms");
    }

    #[test]
    fn nothing_is_listening_until_the_frontend_drains_the_buffer() {
        // The window exists from `setup`, before the event loop runs,
        // so "is there a window" answered `true` for every cold-start
        // file and the buffer was never reached. Only the drain proves
        // a listener exists.
        let pending = PendingOpenFile::empty();
        assert!(!pending.is_listening());
        pending.offer("/a.md".into());
        assert!(!pending.is_listening(), "offering does not make it live");
        assert_eq!(pending.take().as_deref(), Some("/a.md"));
        assert!(pending.is_listening());
    }

    fn mock_app(with_buffer: bool) -> tauri::App<tauri::test::MockRuntime> {
        let mut builder = tauri::test::mock_builder();
        if with_buffer {
            builder = install(builder);
        }
        builder
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds")
    }

    fn md_file(dir: &tempfile::TempDir) -> String {
        let path = dir.path().join("notes.md");
        std::fs::write(&path, "- hi\n").unwrap();
        path.to_str().unwrap().to_owned()
    }

    #[test]
    fn a_file_routed_before_setup_runs_is_buffered_not_a_panic() {
        // The crash report: `RunEvent::Opened` reached `route` while
        // `setup` had not run, and `state::<PendingOpenFile>()` panicked.
        // `mock_app` never runs `setup`, so the buffer has to come from
        // `install` alone.
        let dir = tempfile::TempDir::new().unwrap();
        let file = md_file(&dir);
        let app = mock_app(true);

        route(app.handle(), &format!("file://{file}"));

        let pending = app.state::<PendingOpenFile>();
        assert!(!pending.is_listening(), "nothing drained the buffer yet");
        assert_eq!(pending.take().as_deref(), Some(file.as_str()));
    }

    #[test]
    fn routing_with_no_buffer_registered_does_not_panic() {
        // Defence in depth: if the registration is ever lost, an OS
        // event still must not abort the app.
        let dir = tempfile::TempDir::new().unwrap();
        let file = md_file(&dir);
        let app = mock_app(false);

        route(app.handle(), &file);

        assert!(app.try_state::<PendingOpenFile>().is_none());
        assert!(pending(app.handle()).is_none());
    }
}
