//! Per-client support for every [`Capability`] — the sibling catalog
//! to [`crate::support`], covering features that have no chord.
//!
//! Same reason this exists as `support.rs`: a feature with no
//! `Action` and no chord had no owner, so "does mobile have a
//! calendar?" or "can the TUI pair a device?" lived only in whoever
//! happened to remember reading the component tree. See RFC 0253 and
//! root `CLAUDE.md` invariant 12.
//!
//! Reuses [`crate::support::Support`] / [`crate::support::ClientSupport`]
//! / [`crate::support::Client`] unchanged — none of that vocabulary is
//! chord-specific, and a parallel `CapabilitySupport` type would be
//! exactly the drift this module exists to prevent.

use crate::support::{ClientSupport, Support};
use crate::Capability;

use Support::{Full, Missing, Partial};

/// Reasons repeated across rows, named once so a re-wording lands on
/// every row at once instead of some.
mod why {
    /// The TUI has no marketplace UI at all — installing an
    /// unlisted or registry plugin from inside the terminal client
    /// goes through the CLI, not a browse-and-tap surface.
    pub const NO_TUI_MARKETPLACE: &str =
        "Browsing and installing plugins from a marketplace isn't in the TUI — install by id \
         from a terminal with `outl plugin install <id>`, or use the desktop or mobile app.";

    /// Neither the TUI nor the desktop has a month-grid date picker;
    /// both step one day at a time (`PrevDay` / `NextDay` /
    /// `OpenToday`). The quick switcher is the honest workaround:
    /// journal pages are named `YYYY-MM-DD`, so typing the date finds
    /// the page the calendar grid would have opened.
    pub const NO_CALENDAR_GRID: &str =
        "There's no calendar grid here — open the quick switcher (Cmd/Ctrl+P) and type the \
         date (YYYY-MM-DD) to jump straight to that journal page.";

    /// The TUI has no pairing flow (no ticket display, no scanner) —
    /// pairing a device against a TUI-run workspace goes through the
    /// CLI's `outl peer pair` / `outl peer qr`, run from a separate
    /// terminal.
    pub const NO_TUI_PAIRING: &str = "Pairing a new device isn't in the TUI — run `outl peer \
         pair` or `outl peer qr` from a terminal, or pair from the desktop or mobile app.";

    /// `SyncPanel.tsx` only ever calls `peerPairHost()` — its own doc
    /// comment says "There is no camera path here." A desktop user
    /// can invite a device in, but has no way to *join* an existing
    /// workspace from the desktop UI itself.
    /// Neither the TUI nor the desktop can put buttons on the banner
    /// it already shows, and the reason is the same in both: the
    /// delivery channel carries no callback. The TUI writes an OSC 9
    /// escape, which is one string to the terminal. The desktop goes
    /// through `tauri-plugin-notification`, whose `ActionType` /
    /// `register_action_types` / `onAction` surface is `#[cfg(mobile)]`
    /// — its desktop `show()` spawns `notify-rust` and drops the
    /// handle, so there is nothing to attach an action to and nothing
    /// to hear back from.
    ///
    /// Both clients already have a surface that does the same job in
    /// one more keystroke, so the nudge names it rather than
    /// apologising.
    pub const NO_BANNER_ACTIONS: &str =
        "Reminder banners can't carry buttons here — open the reminders list (Ctrl+R in the \
         TUI, Cmd/Ctrl+Shift+R on desktop) to snooze or tick off what came due.";

    pub const DESKTOP_HOSTS_ONLY: &str =
        "The desktop can host a pairing (show the QR / ticket) but has no camera to scan one — \
         to join an existing workspace from a desktop, run `outl peer pair` in a terminal.";
}

/// Per-client support for every capability in the catalog.
///
/// **This `match` is exhaustive on purpose**, mirroring
/// [`crate::support::support`]: adding a [`Capability`] variant
/// breaks the build here until all three clients have declared what
/// they do with it.
pub fn capability_support(cap: Capability) -> ClientSupport {
    match cap {
        // Desktop: `TimelinePanel.tsx`, opened by the page header's
        // `⏱` button — read-only view of what the op log recorded
        // for the open page. Neither the TUI nor mobile has anything
        // that reads the op log's history back to the user.
        Capability::PageHistory => ClientSupport {
            tui: Missing(
                "Page history isn't in the TUI — open the same page in the desktop app and \
                 use the ⏱ button to see what the op log recorded.",
            ),
            desktop: Full,
            mobile: Missing(
                "Page history isn't on mobile yet — open the same page in the desktop app and \
                 use the ⏱ button.",
            ),
        },

        // Desktop `PluginMarketplace.tsx` and mobile `PluginSheet.tsx`
        // ("Browse" tab) both fetch the official registry and install
        // from it. The TUI's plugin settings overlay only configures
        // an already-installed plugin — there's no browse-and-install
        // surface in the terminal client.
        Capability::PluginMarketplace => ClientSupport {
            tui: Missing(why::NO_TUI_MARKETPLACE),
            desktop: Full,
            mobile: Full,
        },

        // Mobile's `Calendar.tsx` is a month grid for jumping to an
        // arbitrary journal date. Desktop and the TUI only step
        // day-by-day (`PrevDay` / `NextDay` / `OpenToday`) — neither
        // has a random-access date picker.
        Capability::Calendar => ClientSupport {
            tui: Missing(why::NO_CALENDAR_GRID),
            desktop: Missing(why::NO_CALENDAR_GRID),
            mobile: Full,
        },

        // All three clients can instantiate a template and browse
        // which ones exist: the TUI's `/template` (bare form opens
        // `open_template_picker`), the desktop's `/template <name>`
        // slash-menu entries (one synthesized per template, so typing
        // `/` lists them), and mobile's `TemplateSheet`.
        Capability::Templates => ClientSupport {
            tui: Full,
            desktop: Full,
            mobile: Full,
        },

        // Attaching a file works everywhere: the TUI's `/upload`
        // slash command, the desktop's drag-drop + paste handling in
        // `BlockRow`/`OutlineView`, and mobile's `attachAsset` /
        // `importAssetFile` in `Journal.tsx`.
        Capability::Assets => ClientSupport {
            tui: Full,
            desktop: Full,
            mobile: Full,
        },

        // Desktop only hosts — `SyncPanel.tsx` calls `peerPairHost()`
        // and nothing in `outl-desktop/src` ever calls
        // `peerPairJoin()`; the panel's own doc comment says "There
        // is no camera path here." So a desktop user can invite a
        // device in but cannot join an existing workspace from the
        // desktop UI, which is less than the full semantics
        // "PeerPairing" names — `Partial`, not `Full`.
        // Mobile does both (`DevicesSheet.tsx` — scan is primary,
        // "show my QR" is the secondary host path). The TUI has no
        // ticket display and no scanner at all; pairing a TUI-run
        // workspace's device goes through the CLI either way.
        Capability::PeerPairing => ClientSupport {
            tui: Missing(why::NO_TUI_PAIRING),
            desktop: Partial(why::DESKTOP_HOSTS_ONLY),
            mobile: Full,
        },

        // Every client *delivers* a reminder. Only mobile can make
        // the banner actionable, and the split is a platform fact
        // rather than an unbuilt feature: `register_action_types` /
        // `onAction` exist solely under `#[cfg(mobile)]`, and the
        // desktop plugin's `show()` discards the `notify-rust` handle,
        // so there is no callback to hang "Snooze" on. The TUI's OSC 9
        // escape has no callback either.
        //
        // Recorded as `Missing` rather than left out, because the
        // difference the user feels is real: on mobile a reminder is
        // two taps from resolved, everywhere else it is a prompt to go
        // find the block.
        Capability::ReminderNotificationActions => ClientSupport {
            tui: Missing(why::NO_BANNER_ACTIONS),
            desktop: Missing(why::NO_BANNER_ACTIONS),
            mobile: Full,
        },
    }
}

/// Render the capability parity table that `docs/client-parity.md`
/// carries below the [`Action`](crate::Action) one, so the doc is a
/// projection of the code rather than a second copy of it.
///
/// Test-only, for the same reason as
/// `support::parity_table_markdown`: this crate links into the
/// mobile binary, so the doc generator does not belong in its public
/// surface.
#[cfg(test)]
pub(crate) fn capability_parity_table_markdown() -> String {
    let mut out = String::from("| Capability | TUI | Desktop | Mobile |\n|---|---|---|---|\n");
    for cap in Capability::ALL {
        let s = capability_support(*cap);
        out.push_str(&format!(
            "| `{:?}` | {} | {} | {} |\n",
            cap,
            crate::support::cell(s.tui),
            crate::support::cell(s.desktop),
            crate::support::cell(s.mobile),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::support::Client;

    #[test]
    fn every_client_declares_support_for_every_capability() {
        // The `match` in `capability_support` is exhaustive, so this
        // cannot fail for a missing arm — it fails for a missing
        // *variant* in `Capability::ALL`, which would silently shrink
        // every other test in this module and the parity table with
        // it. Mirrors `support`'s `every_client_declares_support_for_every_action`.
        for cap in Capability::ALL {
            let s = capability_support(*cap);
            for client in Client::ALL {
                let _ = s.get(client);
            }
        }
        assert_eq!(Capability::ALL.len(), 7, "Capability::ALL changed size");
    }

    #[test]
    fn every_degraded_capability_explains_itself() {
        for cap in Capability::ALL {
            let s = capability_support(*cap);
            for client in Client::ALL {
                let why = match s.get(client) {
                    Support::Full => continue,
                    Support::Native(w)
                    | Support::Partial(w)
                    | Support::Missing(w)
                    | Support::NotApplicable(w) => w,
                };
                assert!(
                    !why.trim().is_empty(),
                    "{cap:?} on {client:?} has an empty reason",
                );
            }
        }
    }

    #[test]
    fn capability_nudges_are_written_for_the_user_not_the_developer() {
        // Reuses `support`'s DEV_WORDS list rather than copying it —
        // a second, drifting list of banned words would be its own
        // instance of the problem this whole RFC is about.
        for cap in Capability::ALL {
            let s = capability_support(*cap);
            for client in Client::ALL {
                let Some(why) = s.get(client).nudge() else {
                    continue;
                };
                let lower = why.to_lowercase();
                for bad in crate::support::DEV_WORDS {
                    assert!(
                        !lower.contains(bad),
                        "nudge for {cap:?} on {client:?} says {bad:?} — \
                         write what the user can do instead: {why:?}",
                    );
                }
                assert!(
                    why.len() > 20,
                    "nudge for {cap:?} on {client:?} is too terse to help: {why:?}",
                );
            }
        }
    }

    #[test]
    fn no_capability_is_out_of_reach_on_every_client() {
        for cap in Capability::ALL {
            let s = capability_support(*cap);
            assert!(
                Client::ALL.iter().any(|c| s.get(*c).is_reachable()),
                "{cap:?} is unreachable on every client",
            );
        }
    }
}
