//! Tests for [`super::capability_support`].
//!
//! Its own file for the reason [`super::why`] is: the parent sits
//! against the 600-line guard and the `match` is what a reader comes
//! for. Same arrangement as `outl-actions`' `outline/tests.rs`.

use super::*;
use crate::support::Client;

#[test]
fn backlink_inline_edit_is_partial_on_the_tui_and_full_on_the_gui_clients() {
    let s = capability_support(Capability::BacklinkInlineEdit);
    // The TUI performs all nine operations this row covers and has
    // no gesture at all that opens the page a row's block lives on,
    // so the nudge has to name the route that exists. `Full` here
    // is the state this row already shipped in once, before any
    // GUI client called `edit_backlink_block` — a wrong verdict is
    // found by pressing the key.
    assert!(matches!(s.tui, Partial(_)));
    assert!(
        s.tui.nudge().is_some_and(|why| why.contains("Ctrl+P")),
        "the TUI's only route to a cited block's page is the quick switcher — the nudge \
         has to name it",
    );
    assert!(matches!(s.desktop, Full));
    // `Full` on mobile is a claim about those nine, never about a
    // one-tap finish: `Action::MarkDone` owns that one and says
    // `Missing` here, and two catalogs may not disagree about the
    // same operation.
    assert!(matches!(s.mobile, Full));
    assert!(matches!(
        crate::support::support(crate::Action::MarkDone).mobile,
        Support::Missing(_)
    ));
}

#[test]
fn only_the_desktop_toggles_a_blockquote_and_the_others_name_the_marker() {
    let s = capability_support(Capability::BlockquoteToggle);
    assert!(matches!(s.desktop, Full));
    // Both lesser states carry the *same* string, deliberately: the
    // route out is a markdown convention rather than a surface, so two
    // sentences would be two copies of one fact.
    for client in [Client::Tui, Client::Mobile] {
        let Support::Missing(why) = s.get(client) else {
            panic!("{client:?} reaches no quote gesture — the row has to say so");
        };
        assert!(
            why.contains("> "),
            "the nudge has to name the marker a user can type instead: {why:?}",
        );
    }
    assert_eq!(
        s.tui.nudge(),
        s.mobile.nudge(),
        "one route, one sentence — a per-client copy is what drifts",
    );
    // `Missing`, not `NotApplicable`: both clients render a quote, so
    // the question is answerable there and only the gesture is absent.
    assert!(!matches!(s.tui, NotApplicable(_)));
    assert!(!matches!(s.mobile, NotApplicable(_)));
}

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
    assert_eq!(Capability::ALL.len(), 26, "Capability::ALL changed size");
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
