//! Boot commands. Its own macro because `catalog.rs` sits at its size
//! ceiling — the same reason `wrappers/trash.rs` and `wrappers/capture.rs`
//! exist, and `tests/command_parity.rs` scans every `src/wrappers/*.rs`,
//! so a client dropping this module fails the build.
//!
//! **Both** GUI clients register it, and here that is not a convenience.
//! A client without `boot_mark` never reports a painted frame, so its
//! deferred boot work (the iroh endpoint, the watcher, the plugin host,
//! the snapshot refresh) waits out the gate's deadline on every launch
//! instead of starting the moment the journal is on screen. It still
//! works — that is what the deadline is for — it is just slower, with
//! nothing on screen to say so.

/// `boot_mark` / `wait_workspace_ready`: report a launch milestone, and
/// wait for the workspace instead of polling for it.
#[macro_export]
macro_rules! boot_commands {
    ($state:ty) => {
        $crate::tauri_commands! {
            state = $state;

            /// Record a client boot milestone, and release the deferred
            /// boot work on `first_content`.
            fn boot_mark(name: String) -> ()
                => $crate::commands::boot::boot_mark;

            /// Block until the workspace is published. `Ok(false)`
            /// means "not yet, ask again", never "it failed".
            async fn wait_workspace_ready(timeout_ms: Option<u64>)
                -> ::std::result::Result<bool, String>
                => $crate::commands::boot::wait_workspace_ready;
        }
    };
}
