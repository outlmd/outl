//! The two commands a launch itself uses: report a milestone, and wait
//! for the workspace.
//!
//! Neither touches the workspace shape, and that is deliberate — a boot
//! command that takes the workspace lock is a boot command that queues
//! behind the open it is reporting on.

use std::time::Duration;

use crate::host::AppHost;

/// The milestones a GUI client reports, in the order a cold launch
/// reaches them.
///
/// They are declared here, once, because the names are a **wire
/// contract**: the frontend sends the string and the log, the CI budget
/// checker and the simulator job all match on it. A client spelling
/// `firstContent` is not a compile error anywhere, it is a mark that
/// never appears and a budget that silently never fails.
/// `@outl/shared/boot`'s `BOOT_MARKS` is the declared mirror, pinned by
/// `tests/wire_mirrors.rs`.
///
/// - `html_parsed` — the document exists; nothing of the app is running.
/// - `theme_applied` — the palette is on `<html>`, so the frame is
///   branded. Reported even when it came from the cached palette rather
///   than the backend (that is the point of Q5).
/// - `first_content` — real workspace content is on screen. **This is
///   the one that opens the interactive phase**; see [`boot_mark`].
/// - `interactive` — the app accepts input and every boot step has run.
///   Reaching it emits the summary line.
pub const BOOT_MARKS: &[&str] = &[
    "html_parsed",
    "theme_applied",
    "first_content",
    "interactive",
];

/// The mark that releases [`crate::first_paint::FirstPaintGate`].
///
/// `first_content`, not `interactive`: the gate exists to wait for
/// pixels, and `interactive` is reported *after* the boot sequence, so
/// gating on it would make the deferred work wait for the deep-link
/// drain and the first sync — one of which this gate is holding back.
const OPENS_GATE: &str = "first_content";

/// The mark a client reports last.
const LAST_MARK: &str = "interactive";

/// Record a client boot milestone, and release the deferred boot work
/// once the client says real content is on screen.
///
/// Two jobs in one command on purpose. They are the same fact — *the
/// user can see the app now* — and splitting them gives the backend two
/// opinions about when a launch finished: a measurement that says 180 ms
/// and a sync stack that started at 900 ms, with nothing that makes the
/// two agree.
///
/// Unknown names are recorded, not rejected. The log is a diagnostic
/// surface and a client experimenting with a fifth mark should get it in
/// the log; what it does **not** get is the gate, which only
/// `OPENS_GATE` and `LAST_MARK` open.
///
/// Infallible by construction, and `()`-returning: a frontend that has
/// to handle an error from a measurement will eventually stop reporting
/// one.
pub fn boot_mark<S: AppHost>(state: &S, name: String) {
    outl_core::boot_timing::mark_client_phase(&name);
    if name != OPENS_GATE && name != LAST_MARK {
        return;
    }
    if let Some(gate) = state.first_paint_gate() {
        gate.painted();
    }
}

/// Block until the workspace is published, then report whether it is.
///
/// Replaces a poll on both clients; the reasoning and the "a timeout is
/// not a failure" rule live on [`crate::boot_ready::WorkspaceReady`].
///
/// `timeout_ms` is the caller's patience for **one** call, not a budget
/// for the open. `Ok(false)` means "not yet, ask again", which is what a
/// client renders as a still-running loading state. A client that has no
/// readiness signal wired (the trait default) gets its answer from the
/// slot instead, so this command degrades to the truth rather than to a
/// permanent `false`.
///
/// `Err` is reserved for a wait that could not be taken at all (the
/// blocking pool refused the job). It is deliberately **not** what a
/// timeout returns: a frontend that treats "still opening" as a failure
/// is the retry loop with a failure state, which is what a cold 700k-op
/// replay on a phone would then hit on every launch.
///
/// Must be registered `async`: it blocks, and the IPC thread is the main
/// thread on iOS.
pub async fn wait_workspace_ready<S: AppHost>(
    state: &S,
    timeout_ms: Option<u64>,
) -> Result<bool, String> {
    // Clamped, not trusted. The frontend picks this, and an unbounded
    // value would park a blocking-pool thread for as long as the webview
    // felt like it.
    let timeout = Duration::from_millis(timeout_ms.unwrap_or(10_000).clamp(1, 60_000));
    match state.workspace_ready() {
        Some(ready) => {
            let ready = std::sync::Arc::clone(ready);
            crate::offload::run("wait_workspace_ready", move || {
                Ok::<bool, String>(ready.wait(timeout))
            })
            .await
        }
        // No signal wired: answer from the slot. One lock, no wait — a
        // client on this path still polls, exactly as it did before.
        None => Ok(state.workspace().lock().is_some()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names are a wire contract with the frontend and with CI, so
    /// the two that carry behaviour are pinned against the list rather
    /// than spelled twice.
    #[test]
    fn the_marks_that_carry_behaviour_are_in_the_catalog() {
        assert!(BOOT_MARKS.contains(&OPENS_GATE));
        assert!(BOOT_MARKS.contains(&LAST_MARK));
    }

    #[test]
    fn the_catalog_is_in_launch_order_and_has_no_duplicates() {
        let mut sorted = BOOT_MARKS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), BOOT_MARKS.len(), "duplicate mark name");
        assert_eq!(BOOT_MARKS.first(), Some(&"html_parsed"));
        assert_eq!(BOOT_MARKS.last(), Some(&LAST_MARK));
    }
}
