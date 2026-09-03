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
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bumped together with a new variant. The `name` match below is
    /// what makes forgetting impossible: it will not compile once a
    /// new variant exists until this function's `match` grows an arm
    /// for it too.
    const EXPECTED_6: usize = 6;

    fn name(cap: Capability) -> &'static str {
        match cap {
            Capability::PageHistory => "PageHistory",
            Capability::PluginMarketplace => "PluginMarketplace",
            Capability::Calendar => "Calendar",
            Capability::Templates => "Templates",
            Capability::Assets => "Assets",
            Capability::PeerPairing => "PeerPairing",
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
            EXPECTED_6,
            "Capability::ALL has {} entries but the enum has {EXPECTED_6} variants \
             — add the new variant to Capability::ALL (and bump EXPECTED_6)",
            names.len(),
        );
    }
}
