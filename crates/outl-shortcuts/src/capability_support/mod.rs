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

use Support::{Full, Missing, NotApplicable, Partial};

// Shared with `support.rs`'s `OpenTaskList` row; the rest of `why` stays private.
pub(crate) use why::NO_TASK_LIST;

mod why;

/// Per-client support for every capability in the catalog.
///
/// **This `match` is exhaustive on purpose**, mirroring
/// [`crate::support::support`]: adding a [`Capability`] variant
/// breaks the build here until all three clients have declared what
/// they do with it.
/// Capabilities that no client reaches, because their only surface is
/// the CLI (and the MCP tools built on it).
///
/// `no_capability_is_out_of_reach_on_every_client` would otherwise be
/// right to reject them: a capability nobody can reach is normally a
/// feature that does not exist, and the catalog exists to stop those
/// going unrecorded. A CLI-only capability is the one case where
/// "missing on all three" is a true and useful statement rather than a
/// bug — the feature ships, the surface does not.
///
/// So it is a declared list with a reason, the same shape
/// `outl-tauri-shared`'s `DECLARED_GAPS` uses, rather than a weakened
/// assertion. A row here is a decision; an empty reason is not one.
///
/// **Empty today, and that is the point.** `Capability::Trash` was the
/// only row it ever had, and it is gone because the exemption outlived
/// its reason: the desktop and the TUI reach the trash now (issue #287),
/// so `a_cli_only_capability_is_declared_with_a_reason_and_really_unreachable`
/// would fail on it. That test is what makes the list self-retiring —
/// an exemption cannot quietly start hiding a real gap.
pub const CLI_ONLY: &[(Capability, &str)] = &[];

pub fn capability_support(cap: Capability) -> ClientSupport {
    match cap {
        // Desktop: `TimelinePanel.tsx`, opened by the page header's
        // `⏱` button — read-only view of what the op log recorded for
        // the open page. The TUI has nothing that reads the op log's
        // history back to the user.
        //
        // Mobile's verdict was `Missing` while this client registered
        // `page_timeline` and mounted `HistorySheet.tsx` off the page
        // properties sheet — a surface `outl-mobile/CLAUDE.md` lists
        // under "Trust surfaces". The sentence sent a user who already
        // had the feature to another device to go and find it.
        //
        // Worth stating because it is the failure an exhaustive `match`
        // cannot catch: invariant 12 forces every client to *declare* a
        // verdict, and nothing forces the verdict to be true. A wrong
        // `Missing` is worse than an undeclared gap, since the text is
        // shown to the user. `outl-mobile`'s `capability_parity.rs` is
        // what fails now if this row and that client disagree again.
        Capability::PageHistory => ClientSupport {
            tui: Missing(
                "Page history isn't in the TUI — open the same page in the desktop app and \
                 use the ⏱ button to see what the op log recorded.",
            ),
            desktop: Full,
            mobile: Full,
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

        // All three clients draw a month grid; two of them let you
        // click a day.
        //
        // Desktop: `Sidebar.tsx`'s `Calendar()` — `mondayIndex` /
        // `daysInMonth` build a 7-column grid, `‹` / `›` step the
        // month, and a day cell calls `openDay()` → `openJournalFor`.
        // Mobile: `Calendar.tsx`, the same gesture by tap.
        // TUI: `view/sidebar.rs::render_calendar` paints the grid and
        // marks journal days, but nothing opens one from it — see
        // `why::TUI_CALENDAR_IS_READ_ONLY`.
        //
        // This row said `Missing` for the TUI *and* the desktop, with
        // a nudge sending desktop users to the quick switcher to do
        // what the sidebar in front of them already does. A catalog
        // that under-claims is not the safe direction of wrong: the
        // nudge is shown to the user, so a stale `Missing` actively
        // steers them away from a shipped feature.
        Capability::Calendar => ClientSupport {
            tui: Partial(why::TUI_CALENDAR_IS_READ_ONLY),
            desktop: Full,
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
        // Desktop `OutlineView.tsx` and mobile `Journal.tsx` both
        // render `<NestedPages />` from `@outl/shared/namespace` —
        // same component, same rows, and a tap opens the page. The
        // TUI paints the same list without a cursor in it; see
        // `why::TUI_NESTED_PAGES_ARE_READ_ONLY`.
        Capability::NestedPages => ClientSupport {
            tui: Partial(why::TUI_NESTED_PAGES_ARE_READ_ONLY),
            desktop: Full,
            mobile: Full,
        },
        // Mobile: the header gear opens `SettingsSheet.tsx` — "Lock
        // button order" freezes the row, "Reset button order" drops
        // the MFU counts. The other two clients have no soft-keyboard
        // bar for the setting to act on.
        // Nobody has this yet. `outl trash list` / `outl trash restore`
        // and the matching MCP tools shipped first because the primitives
        // were already there (`doctor` walks the trash, restore is a
        // `Move`); a surface in each client is the follow-up. Recorded
        // here rather than discovered by a user hunting for a recycle bin.
        // All three read the grid through `outl_md::table`, so the
        // cells and the alignment cannot disagree. The desktop and
        // mobile hand them to `<table>` inside an `overflow-x-auto`
        // wrapper (`MarkdownTable.tsx`); the TUI pads the columns and
        // paints `│` separators with a drawn rule row
        // (`view/table.rs`), and has no sideways scroll to offer.
        Capability::TableRendering => ClientSupport {
            tui: Partial(why::TUI_TABLE_NO_SCROLL),
            desktop: Full,
            mobile: Full,
        },
        // Desktop `TrashPanel.tsx` and the TUI's `/trash` overlay both
        // read `outl_actions::trash::list` and restore through
        // `trash::restore`, so neither decides for itself what can come
        // back. Mobile registers the same two commands
        // (`trash_commands!`) and has no sheet in front of them —
        // which is exactly the distinction this catalog exists to
        // record: a registered command is not a shipped capability.
        //
        // This row read `Missing` on all three, with a nudge telling a
        // TUI user to open a second terminal against a workspace their
        // own client already holds locked (issue #287).
        Capability::Trash => ClientSupport {
            tui: Full,
            desktop: Full,
            mobile: Missing(why::NO_MOBILE_TRASH_SHEET),
        },
        // Every client spawns `outl_actions::backup::spawn_auto_pass` at
        // startup — the TUI in `runtime::run_with_theme_override`, the
        // desktop in `workspace_open::start_backup`, mobile in its
        // `setup`. Only two of them have a `git` for it to fork; see
        // `why::NO_GIT_ON_THE_PHONE`.
        //
        // Recorded because the pass is silent by design (best-effort,
        // `warn!` on failure), so "are my notes being snapshotted" has
        // no surface on any client and the answer differs by platform.
        Capability::AutomaticBackups => ClientSupport {
            tui: Full,
            desktop: Full,
            mobile: Missing(why::NO_GIT_ON_THE_PHONE),
        },
        // Mobile: the bottom bar's `+` (`BottomBar.tsx` →
        // `Journal.capture.ts::captureToToday`) appends to today's
        // journal without leaving the open page.
        Capability::QuickCapture => ClientSupport {
            tui: Partial(why::TUI_CAPTURE_IS_TWO_STEPS),
            desktop: Partial(why::DESKTOP_CAPTURE_IS_TWO_STEPS),
            mobile: Full,
        },
        // Mobile: the iOS widget, the "Add to today" App Intent and the
        // iOS share extension (`gen/apple/OutlWidget`, `OutlShare`),
        // plus Android's share target, home screen widget and quick
        // settings tile (`gen/android/…/OutlShareActivity.kt`,
        // `OutlCaptureWidget.kt`, `OutlCaptureTile.kt`). Every one goes
        // through `drain_capture_inbox`.
        //
        // `Full` is about what the capability is on mobile, not about
        // one store build: the iOS extensions are unhooked from release
        // builds until their provisioning profiles exist
        // (`crates/outl-mobile/IOS-EXTENSIONS.md`), which is release
        // plumbing recorded there, and the Android APK ships all three.
        Capability::CaptureOutsideTheApp => ClientSupport {
            tui: Partial(why::TUI_CAPTURE_LANDS_AT_LAUNCH),
            desktop: Partial(why::DESKTOP_CAPTURE_HAS_NO_SYSTEM_GESTURE),
            mobile: Full,
        },
        // Mobile `TasksSheet.tsx`, desktop `TasksPanel.tsx`; both read `list_open_tasks`.
        Capability::TaskList => ClientSupport {
            tui: Missing(why::NO_TASK_LIST),
            desktop: Full,
            mobile: Full,
        },
        // Desktop `CodeFenceView` and mobile `BlockBody` both render a
        // mermaid fence through `<DiagramFence />` (Pele) unless
        // `[editor] diagrams` is `off` or a plugin transformer owns
        // the language. The TUI has no equivalent surface.
        Capability::DiagramRendering => ClientSupport {
            tui: Missing(why::TUI_KEEPS_MERMAID_SOURCE),
            desktop: Full,
            mobile: Full,
        },
        Capability::ToolbarOrderLock => ClientSupport {
            tui: NotApplicable(why::NO_SOFT_KEYBOARD_BAR),
            desktop: NotApplicable(why::NO_SOFT_KEYBOARD_BAR),
            mobile: Full,
        },
        // Desktop: `tauri.conf.json`'s `fileAssociations` puts outl in
        // the OS "Open With" list for `.md` / `.markdown` / `.txt` /
        // `.text`, and `src-tauri/src/open_with.rs` routes the macOS
        // Apple Event and the Linux / Windows `argv` into
        // `open_external_file`.
        //
        // Mobile registers the same command and no platform hands it
        // a file — the gap is the manifest, not the import, which is
        // exactly the distinction invariant 12 exists to record: a
        // registered command is not a shipped capability.
        Capability::OpenExternalFile => ClientSupport {
            tui: NotApplicable(why::NO_TUI_FILE_HANDOFF),
            desktop: Full,
            mobile: Missing(why::NO_MOBILE_SHARE_TARGET),
        },
        // Desktop `SettingsModal.tsx` and mobile `SettingsSheet.tsx`
        // both carry a Privacy section holding the toggle and the
        // shared `CRASH_REPORTS_SETTING` copy from
        // `@outl/shared/onboarding` — GUI wording lives there, not in
        // `outl_telemetry`, which owns the terminal strings. One owner
        // per medium (RFC 0377 §6). The TUI has no settings screen for
        // a toggle to live in; see
        // `why::NO_TELEMETRY_TOGGLE_IN_THE_TUI`.
        Capability::CrashReportingOptOut => ClientSupport {
            tui: Missing(why::NO_TELEMETRY_TOGGLE_IN_THE_TUI),
            desktop: Full,
            mobile: Full,
        },
        // The other half of the same promise, and a separate row
        // because a client can have either one without the other.
        // Both GUIs mount the shared `CRASH_REPORTS_NOTICE` strip at
        // their shell, dismissible and once. The terminal reports and
        // says nothing until asked; `outl telemetry status` is a pull,
        // and a pull is not a disclosure.
        Capability::CrashReportingDisclosure => ClientSupport {
            tui: Missing(why::NO_PROACTIVE_NOTICE_IN_THE_TERMINAL),
            desktop: Full,
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
    fn the_desktop_has_a_task_list() {
        assert!(matches!(
            capability_support(Capability::TaskList).desktop,
            Support::Full
        ));
    }

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
        assert_eq!(Capability::ALL.len(), 19, "Capability::ALL changed size");
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
            if CLI_ONLY.iter().any(|(declared, _)| declared == cap) {
                continue;
            }
            let s = capability_support(*cap);
            assert!(
                Client::ALL.iter().any(|c| s.get(*c).is_reachable()),
                "{cap:?} is unreachable on every client — if that is deliberate because \
                 its only surface is the CLI, add it to CLI_ONLY with the reason",
            );
        }
    }

    #[test]
    fn a_cli_only_capability_is_declared_with_a_reason_and_really_unreachable() {
        for (cap, reason) in CLI_ONLY {
            assert!(
                reason.len() > 20,
                "{cap:?} is exempted from the reachability rule without a real reason",
            );
            let s = capability_support(*cap);
            assert!(
                !Client::ALL.iter().any(|c| s.get(*c).is_reachable()),
                "{cap:?} is on CLI_ONLY but a client reaches it — drop the row, or the \
                 exemption starts hiding a real gap",
            );
        }
    }
}
