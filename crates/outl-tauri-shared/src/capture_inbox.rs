//! `drain_capture_inbox`: the only path from a capture inbox
//! (RFC 0363) into the workspace. Each capture lands through
//! `outl_actions::capture` and is projected like any typed block, then
//! its file is deleted.
//!
//! Two inboxes, drained in that order: the one a mobile shell exports in
//! `OUTL_CAPTURE_INBOX` (the iOS App Group container, Android's
//! `filesDir`), then the open workspace's own `.outl/captures`, which is
//! where `outl capture` writes. A client registers this command once and
//! gets whichever of the two its platform actually has — the desktop has
//! only the second, iOS only the first, Android both.
//!
//! That last sentence is enforced by `mobile_inbox`, not just stated.
//! The env arm used to run everywhere, so on a desktop anything able to
//! set the app's launch environment could aim the drain at a folder the
//! user never chose.

use std::path::Path;

use outl_actions::capture::{
    capture_day, capture_page, drain_captures, inbox_in_workspace, land_capture,
};
use serde::Serialize;

use crate::commit::finish_in_page_with;
use crate::helpers::with_ws_mut;
use crate::host::AppHost;

/// What a drain did, for the frontend to refresh and say so.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureDrainDto {
    /// Captures that became blocks.
    pub landed: usize,
    /// Files left in the inbox because they could not be read.
    pub skipped: usize,
    /// Journal pages that gained blocks, in landing order, deduplicated.
    pub page_ids: Vec<String>,
}

impl CaptureDrainDto {
    /// Fold a second inbox's drain into this one, keeping the page list
    /// deduplicated across both.
    fn absorb(&mut self, other: Self) {
        self.landed += other.landed;
        self.skipped += other.skipped;
        for id in other.page_ids {
            if !self.page_ids.contains(&id) {
                self.page_ids.push(id);
            }
        }
    }
}

/// Drain every inbox this device has: the one a mobile shell exported in
/// `OUTL_CAPTURE_INBOX`, then the open workspace's `.outl/captures`.
///
/// Zeros when neither is there — no App Group entitlement, and nothing
/// has written a capture into this workspace. A workspace that is not
/// open yet is not an error either: the captures keep waiting, which is
/// the whole point of an inbox.
pub fn drain_capture_inbox<S: AppHost>(state: &S) -> Result<CaptureDrainDto, String> {
    let mut out = CaptureDrainDto::default();
    for dir in mobile_inbox()
        .into_iter()
        .chain(state.storage_root().ok().map(|r| inbox_in_workspace(&r)))
    {
        out.absorb(drain_capture_dir(state, &dir)?);
    }
    Ok(out)
}

/// The inbox a mobile shell exported in `OUTL_CAPTURE_INBOX`, on the
/// platforms that have one.
///
/// Mobile-only deliberately. The variable exists because an iOS
/// extension and an Android component cannot resolve which workspace is
/// open, so the shell hands them a folder. A desktop has no such writer:
/// `outl capture` resolves the workspace itself and writes inside it. So
/// reading the variable there adds no capability and does add a way to
/// aim the drain somewhere the user did not choose — anything that can
/// set the app's launch environment (`launchctl setenv`, same user, no
/// root) would get every file in that folder landed in the journal, and
/// `inbox_from_env` checks only that the path is a directory.
///
/// This is what the module doc means by "the desktop has only the
/// second". Before this split the code honoured the variable everywhere
/// and the sentence was aspirational.
#[cfg(any(target_os = "ios", target_os = "android"))]
fn mobile_inbox() -> Option<std::path::PathBuf> {
    outl_actions::capture::inbox_from_env()
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn mobile_inbox() -> Option<std::path::PathBuf> {
    None
}

/// [`drain_capture_inbox`] for an explicit folder.
///
/// The loop is `outl_actions::capture::drain_captures`, which is also
/// what the TUI's launch drain runs. Only the per-capture step differs
/// and it is the reason this function exists: the workspace lock is
/// taken per capture and the page is projected, where a client holding a
/// plain `&mut Workspace` needs neither. Keeping a second copy of the
/// loop here is how "which captures get skipped" starts differing
/// between the phone and the terminal.
pub fn drain_capture_dir<S: AppHost>(state: &S, dir: &Path) -> Result<CaptureDrainDto, String> {
    let report = drain_captures(dir, capture_day, |capture, day| {
        let page = with_ws_mut(state, |ws| {
            capture_page(ws, state.hlc(), day).map_err(|e| e.to_string())
        })?;
        finish_in_page_with(state, page, |ws| {
            land_capture(ws, state.hlc(), page, capture)
        })?;
        Ok::<_, String>(page)
    })?;

    let mut out = CaptureDrainDto {
        landed: report.landed.len(),
        skipped: report.skipped,
        page_ids: Vec::new(),
    };
    for (page, _) in &report.landed {
        let id = page.to_string();
        if !out.page_ids.contains(&id) {
            out.page_ids.push(id);
        }
    }
    Ok(out)
}
