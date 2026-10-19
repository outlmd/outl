//! Every user-facing capability that has no [`crate::Action`] and is
//! therefore invisible to [`crate::support::support`]'s exhaustive
//! `match`.
//!
//! [`crate::Action`] names a thing a chord can fire; `lookup()`
//! resolves a chord to one. Page history, the plugin marketplace, a
//! calendar grid, and pairing a new device are not reached by a
//! chord on any client — they're a button, a sheet, a slash command.
//! That does not make them un-trackable: it makes them a second
//! catalog, sharing the [`crate::support::Support`] vocabulary but
//! not the chord one. See RFC 0253.
//!
//! Variants are grouped alphabetically; there's no help-overlay
//! ordering to preserve here the way there is for [`crate::Action`].

use serde::{Deserialize, Serialize};

/// A user-facing capability that is NOT reachable through a chord.
///
/// **Do not add a capability that already has an [`crate::Action`].**
/// Backlinks (`ToggleBacklinks`), block properties (`OpenProperties`
/// / `AddProperty`) and reminders (`OpenReminders` / `InsertRemind`)
/// all have chords and full parity already, tracked in the `Action`
/// table — a second entry here for any of them would be two catalogs
/// disagreeing about the same fact, which is the drift RFC 0253
/// exists to prevent.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Capability {
    /// Viewing the op log's history for the open page (desktop:
    /// `TimelinePanel`, the page header's `⏱` button). Read-only —
    /// it shows the past, it does not restore it.
    PageHistory,
    /// Browsing the official plugin registry and installing from it
    /// (as opposed to configuring an already-installed plugin, which
    /// every client can do).
    PluginMarketplace,
    /// A month-grid, random-access "jump to this date" widget for
    /// journal navigation, distinct from the `PrevDay` / `NextDay` /
    /// `OpenToday` step chords every client already has.
    Calendar,
    /// Instantiating a structural or callable template under a
    /// block, and browsing which templates exist.
    Templates,
    /// Attaching a file (paste, drag-drop, or an explicit upload
    /// picker) so it lives under `assets/` and gets a markdown link.
    Assets,
    /// Pairing a new device into the workspace's peer set — hosting
    /// a ticket / QR for another device to scan, or scanning one.
    PeerPairing,
    /// Acting on a reminder from the notification itself — snoozing
    /// or marking it done from the banner's own buttons, and being
    /// taken to the block when the banner is tapped.
    ///
    /// Distinct from *delivering* the notification, which every
    /// client does. This is whether the banner is a dead end: a
    /// client without it can still buzz you, but you have to open
    /// the app and find the block by hand.
    ReminderNotificationActions,
    /// Browsing the pages nested under the open page's namespace —
    /// `os` listing `os/linux` and `os/linux/debian` — and opening
    /// one from that list (issue #275).
    ///
    /// Distinct from the backlinks section, which every client has in
    /// full: backlinks are blocks that *point here*, this is pages
    /// that *live under here*. The two are derived from different
    /// sources (referencing blocks vs. page titles) and a client can
    /// ship one without the other.
    NestedPages,
    /// Pinning the on-screen keyboard toolbar's button order so it
    /// stops rearranging itself by usage, plus resetting that usage
    /// back to the shipped layout.
    ///
    /// Presupposes a toolbar whose order *moves on its own*, which
    /// is a soft-keyboard affordance: a client driven by a physical
    /// keyboard reaches the same actions through fixed chords, so
    /// there is no order to pin.
    ToolbarOrderLock,
    /// Being handed a file by the operating system — "Open With →
    /// outl" in a file manager, or a share sheet — and having it
    /// arrive as a page under the `open-in/` namespace.
    ///
    /// This is about the **OS integration**, not the import: the
    /// `open_external_file` command is registered on every Tauri
    /// client, so any of them can import a path it is given. What
    /// differs is whether the platform ever hands one over.
    OpenExternalFile,
    /// Rendering a markdown table a user wrote as a table, rather
    /// than as the pipe rows it is on disk.
    ///
    /// Every client **models** one — `outl_md::table` is shared, so
    /// the cells and the alignment are read identically everywhere
    /// (issue #329). What differs is what the client can do with a
    /// table too wide for the space it has.
    ///
    /// Not about *editing* a table: no client has add-row /
    /// add-column, by design, and the block's raw markdown is what
    /// gets edited on all three. A capability every client lacks
    /// equally would be a row that says nothing.
    TableRendering,
    /// Reading the trash back: listing what has been deleted and
    /// putting a block back where it was deleted from (issue #287).
    ///
    /// Invariant 6 makes delete a `Move(node, TRASH_ROOT)`, so every
    /// client already *preserves* deletions. This is the other half —
    /// whether a client can show them and undo one. Distinct from
    /// `Undo`, which has a chord and only reaches the current
    /// session's stack: this reaches a deletion from any device, on
    /// any day, because it reads the op log rather than a stack.
    Trash,
    /// Adding a block to the end of today's journal from wherever the
    /// user is, in one gesture, without navigating there first.
    ///
    /// Distinct from `OpenToday`, which every client has: that moves
    /// you, this keeps you where you are and only writes. Mobile's
    /// bottom bar `+` is the reference shape.
    QuickCapture,
    /// Adding a line to the journal from **outside** the app — without
    /// it being on screen, and without opening it first.
    ///
    /// Distinct from `QuickCapture`, which is the same destination from
    /// inside the app: that one keeps you where you are, this one does
    /// not need you to be there at all. The gestures are a Home or Lock
    /// Screen widget, a share sheet, Siri or the Action button, a quick
    /// settings tile, a macOS Service or a hotkey daemon.
    ///
    /// Every route goes through the capture inbox rather than the op log
    /// (RFC 0363): the writer is a different process that cannot hold
    /// the workspace open, so it leaves one file and the app lands it.
    /// That is also what this row is really about — a client without a
    /// drain cannot have the capability, however many gestures the OS
    /// offers.
    CaptureOutsideTheApp,
    /// One list of every open task (`TODO` / `DOING`) across the
    /// workspace, each row opening its block.
    ///
    /// A ` ```query ` block with `status: open` answers the same
    /// question on every client, inside a page; this is whether the
    /// client has a place for it of its own.
    TaskList,
    /// Drawing a mermaid fence as a diagram.
    ///
    /// Desktop and mobile do this with Pele when `[editor] diagrams`
    /// is `pele` (the default). `off` leaves the highlighted source
    /// on those two. The TUI has no diagram surface and keeps the
    /// fence as source either way.
    DiagramRendering,
    /// Turning crash reporting off.
    ///
    /// Reports are **on by default** (RFC 0377), so a client without
    /// this is a client whose user has no opt-out — and the
    /// legitimate-interest footing the whole feature stands on is the
    /// user's ability to say no. That makes a missing toggle a legal
    /// problem, not a thinner settings screen.
    ///
    /// Not about *what* gets sent: the allowlist lives in
    /// `outl_telemetry` and is identical on every client, as is the
    /// promise that no note text is part of it. This row is only
    /// whether the person on the device can switch it off from where
    /// they are.
    CrashReportingOptOut,
    /// Telling the user, unprompted, that crashes are sent.
    ///
    /// The sibling of [`Capability::CrashReportingOptOut`] and not the
    /// same question. That row asks whether the user *can* say no;
    /// this one asks whether they were ever told there was something
    /// to say no to.
    ///
    /// Both halves are load-bearing for the same reason, and RFC 0377
    /// section 6 is explicit about it: reporting is on by default, so
    /// the legitimate-interest footing rests on **disclosure coming
    /// first**. A user whose first notice is reading the source has
    /// not been disclosed to, and a client that reports without ever
    /// saying so has the opt-out and still not the consent story.
    ///
    /// "Proactive" is the whole content of the verdict. `outl
    /// telemetry status` answers truthfully and answers only when
    /// asked, which is a different thing from arriving unasked, so a
    /// client that has the command and no notice is `Missing` here
    /// rather than `Full`. `docs/privacy.md` is disclosure too, and it
    /// is not in the product.
    CrashReportingDisclosure,
}

impl Capability {
    /// Every variant, for exhaustive iteration in tests and in the
    /// parity table generator.
    ///
    /// Kept in step with the enum by `all_matches_the_enum` in this
    /// module's tests: a variant missing here is a variant every
    /// coverage test and the parity table would silently skip — the
    /// same "nothing fails, the gap just goes unrecorded" shape this
    /// list exists to close (see `Action::ALL`'s identical contract).
    pub const ALL: &'static [Capability] = &[
        Capability::PageHistory,
        Capability::PluginMarketplace,
        Capability::Calendar,
        Capability::Templates,
        Capability::Assets,
        Capability::PeerPairing,
        Capability::ReminderNotificationActions,
        Capability::NestedPages,
        Capability::ToolbarOrderLock,
        Capability::OpenExternalFile,
        Capability::TableRendering,
        Capability::Trash,
        Capability::QuickCapture,
        Capability::CaptureOutsideTheApp,
        Capability::TaskList,
        Capability::DiagramRendering,
        Capability::CrashReportingOptOut,
        Capability::CrashReportingDisclosure,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bumped together with a new variant. The `name` match below is
    /// what makes forgetting impossible: it will not compile once a
    /// new variant exists until this function's `match` grows an arm
    /// for it too.
    const EXPECTED_18: usize = 18;

    fn name(cap: Capability) -> &'static str {
        match cap {
            Capability::PageHistory => "PageHistory",
            Capability::PluginMarketplace => "PluginMarketplace",
            Capability::Calendar => "Calendar",
            Capability::Templates => "Templates",
            Capability::Assets => "Assets",
            Capability::PeerPairing => "PeerPairing",
            Capability::ReminderNotificationActions => "ReminderNotificationActions",
            Capability::NestedPages => "NestedPages",
            Capability::ToolbarOrderLock => "ToolbarOrderLock",
            Capability::OpenExternalFile => "OpenExternalFile",
            Capability::TableRendering => "TableRendering",
            Capability::Trash => "Trash",
            Capability::QuickCapture => "QuickCapture",
            Capability::CaptureOutsideTheApp => "CaptureOutsideTheApp",
            Capability::TaskList => "TaskList",
            Capability::DiagramRendering => "DiagramRendering",
            Capability::CrashReportingOptOut => "CrashReportingOptOut",
            Capability::CrashReportingDisclosure => "CrashReportingDisclosure",
        }
    }

    #[test]
    fn all_matches_the_enum() {
        let names: Vec<_> = Capability::ALL.iter().map(|c| name(*c)).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            names.len(),
            "Capability::ALL lists the same variant twice",
        );
        assert_eq!(
            names.len(),
            EXPECTED_18,
            "Capability::ALL has {} entries but the enum has {EXPECTED_18} variants \
             — add the new variant to Capability::ALL (and bump EXPECTED_18)",
            names.len(),
        );
    }
}
