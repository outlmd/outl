//! Per-client support for every [`Action`] in the catalog.
//!
//! ## Why this exists
//!
//! [`crate::Action`] names an operation; [`crate::default_bindings`]
//! names the chord that reaches it. Neither says whether the client
//! the user is actually looking at *does* anything when that chord
//! fires. That fact had no owner, so it accumulated in three places
//! that disagreed:
//!
//! - `docs/shortcuts.md` claimed the desktop binds `y r`
//!   ([`Action::CopyBlockRef`]) and `:`
//!   ([`Action::OpenCommandPalette`]). Neither has a handler — the
//!   keys do nothing.
//! - The same doc listed mobile undo / redo as "toolbar". Mobile has
//!   neither (issue #14), and `Journal.tsx` says so in a comment.
//! - The desktop dispatcher logged `console.warn` for an unhandled
//!   action, in a comment that called DevTools output something
//!   "the user sees".
//!
//! Three hand-maintained copies of one fact, each stale in a
//! different direction. This module is the single owner, and
//! [`support`] is an exhaustive `match` — a new `Action` variant does
//! not compile until every client has declared what it does with it.
//!
//! ## What "supported" means here
//!
//! The question is the user's, not the dispatcher's: **can I do this
//! thing on this client?** A gesture, a toolbar button and a chord
//! are all [`Support::Full`], because the user does not care which
//! one carried it. What earns a lesser state is the user reaching for
//! something and not getting it.
//!
//! ## The nudge
//!
//! [`Support::nudge`] is the text a client shows when the user
//! reaches for an action it cannot deliver. Writing it *here* rather
//! than in the client is the point: the reason a thing is missing is
//! the same reason on every client that lacks it, and a client that
//! invents its own wording drifts from the catalog the same way the
//! doc did.

use crate::Action;

/// A client that paints an editor and dispatches [`Action`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Client {
    /// `outl-tui` — the terminal client.
    Tui,
    /// `outl-desktop` — Tauri + Solid, macOS / Linux / Windows.
    Desktop,
    /// `outl-mobile` — Tauri 2, iOS + Android.
    Mobile,
}

impl Client {
    /// Every client, for exhaustive iteration in tests and in the
    /// parity table generator.
    pub const ALL: [Client; 3] = [Client::Tui, Client::Desktop, Client::Mobile];
}

/// What a given client does when a given [`Action`] fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// The client performs the action. How it is reached — chord,
    /// button, gesture — is deliberately not part of this state.
    Full,

    /// The platform performs it, not outl. `Backspace` on an empty
    /// `<textarea>`, the OS text clipboard, the on-screen keyboard.
    ///
    /// Distinct from [`Support::Full`] because a client test that
    /// asserts "every `Full` action has a handler" must not demand
    /// one here — there is nothing to hand off to.
    Native(&'static str),

    /// Reachable, but not with the full semantics the action names.
    /// The string says what is missing, and is shown to the user.
    Partial(&'static str),

    /// Not implemented here yet, and it should be. The string is
    /// shown to the user, so it says what they can do instead —
    /// never "unimplemented".
    Missing(&'static str),

    /// Cannot exist on this client by construction, so no amount of
    /// work would add it. The string says why.
    NotApplicable(&'static str),
}

impl Support {
    /// Whether the user gets the behaviour at all, by any route.
    pub fn is_reachable(self) -> bool {
        matches!(
            self,
            Support::Full | Support::Native(_) | Support::Partial(_)
        )
    }

    /// Text to show the user when they reach for this action and the
    /// client cannot fully deliver it. `None` when there is nothing
    /// to explain.
    ///
    /// A client that swallows the keystroke silently — or logs to a
    /// console the user never opens — leaves them unable to tell a
    /// bug from a gap. That is the failure this replaces.
    pub fn nudge(self) -> Option<&'static str> {
        match self {
            Support::Full | Support::Native(_) => None,
            Support::Partial(why) | Support::Missing(why) | Support::NotApplicable(why) => {
                Some(why)
            }
        }
    }

    /// Stable lowercase tag, for the parity table and the wire form.
    pub fn kind(self) -> &'static str {
        match self {
            Support::Full => "full",
            Support::Native(_) => "native",
            Support::Partial(_) => "partial",
            Support::Missing(_) => "missing",
            Support::NotApplicable(_) => "n/a",
        }
    }
}

/// One [`Action`]'s support across every client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientSupport {
    /// `outl-tui`.
    pub tui: Support,
    /// `outl-desktop`.
    pub desktop: Support,
    /// `outl-mobile`.
    pub mobile: Support,
}

impl ClientSupport {
    /// Support for one client.
    pub fn get(&self, client: Client) -> Support {
        match client {
            Client::Tui => self.tui,
            Client::Desktop => self.desktop,
            Client::Mobile => self.mobile,
        }
    }
}

/// Shorthand so the table below reads as three columns.
macro_rules! row {
    ($tui:expr, $desktop:expr, $mobile:expr) => {
        ClientSupport {
            tui: $tui,
            desktop: $desktop,
            mobile: $mobile,
        }
    };
}

use Support::{Full, Missing, Native, NotApplicable, Partial};

/// Reasons repeated across many rows, named once so a re-wording
/// cannot land on some rows and miss others.
mod why {
    /// The desktop has no character cursor inside a block, so every
    /// vim op that addresses a column has nowhere to land.
    /// See RFC 0070 and `outl-desktop/CLAUDE.md` → "Vim parity".
    pub const NO_CHAR_CURSOR: &str =
        "This vim op needs a character cursor inside the block, which only the TUI has. \
         Edit the block and use the arrow keys, or run it from the TUI.";

    /// Mobile drives the outline by touch; there is no selection to
    /// move and no modal Normal state to move it in.
    pub const TOUCH_ONLY: &str =
        "Not on mobile — tap the block you want instead of moving a selection.";

    /// Mobile has no keyboard chord surface for vim-modal state.
    pub const NO_VIM_MODE: &str = "Mobile has no vim modes — it edits directly on tap.";
}

/// Per-client support for every action in the catalog.
///
/// **This `match` is exhaustive on purpose.** Adding an [`Action`]
/// variant breaks the build here until the three clients have
/// declared what they do with it, which is the whole mechanism: the
/// gap gets recorded when it is created, by the person who created
/// it, instead of being discovered later by a user pressing a key
/// that does nothing.
pub fn support(action: Action) -> ClientSupport {
    match action {
        // ── chrome ───────────────────────────────────────────────
        Action::OpenPicker => row!(Full, Full, Full),
        Action::OpenCommandPalette => row!(
            Full,
            Missing(
                "The command palette isn't on the desktop yet — use the quick switcher \
                 (Cmd/Ctrl+P) or the slash menu inside a block."
            ),
            Missing("The command palette isn't on mobile yet — use the slash menu inside a block.")
        ),
        Action::ToggleHelp => row!(Full, Full, Full),
        Action::ToggleSidebar => row!(
            Full,
            Full,
            NotApplicable("Mobile is single-pane — the page switcher replaces the sidebar.")
        ),
        Action::ToggleBacklinks => row!(Full, Full, Full),
        Action::OpenSettings => row!(Full, Full, Full),
        Action::Quit => row!(
            Full,
            Full,
            NotApplicable("Mobile apps are backgrounded by the OS, not quit from inside.")
        ),

        // ── navigation ───────────────────────────────────────────
        Action::OpenToday => row!(Full, Full, Full),
        Action::PrevDay => row!(Full, Full, Full),
        Action::NextDay => row!(Full, Full, Full),
        Action::SelectionDown => row!(Full, Full, NotApplicable(why::TOUCH_ONLY)),
        Action::SelectionUp => row!(Full, Full, NotApplicable(why::TOUCH_ONLY)),
        Action::OpenRefUnderCursor => row!(Full, Full, Full),

        // ── entering insert ──────────────────────────────────────
        Action::EnterInsert => row!(Full, Full, Full),
        Action::EnterInsertAtStart => row!(Full, Full, Full),
        Action::EnterInsertAfter => row!(
            Full,
            Partial("No character cursor on the desktop, so `a` behaves like `i`."),
            NotApplicable(why::NO_VIM_MODE)
        ),
        Action::EnterInsertAtEnd => row!(Full, Full, Full),

        // ── char-cursor vim ops (TUI only — RFC 0070) ────────────
        Action::DeleteCharUnderCursor => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::DeleteCharBeforeCursor => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::DeleteToEndOfBlock => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::ChangeToEndOfBlock => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::SubstituteChar => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::ReplaceChar => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::FindCharForward => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::FindCharBackward => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::ToggleCharCase => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::CursorWordEnd => {
            row!(Full, Missing(why::NO_CHAR_CURSOR), NotApplicable(why::NO_VIM_MODE))
        }
        Action::SubstituteBlock => row!(Full, Full, NotApplicable(why::NO_VIM_MODE)),

        // ── folding / viewport / zoom ────────────────────────────
        // RFC 0254 phase 4b: mobile drives `FoldAll`/`UnfoldAll` by
        // looping the existing `set_block_collapsed` command over
        // `flattenParents`/`flattenAll` (`Journal.tsx::applyCollapsedToAll`),
        // exactly mirroring the desktop's own client-side loop — there
        // is no bulk-fold backend op on any client, by design (each
        // flip is its own `Op::SetCollapsed` so concurrent flips
        // converge). Reached from two header buttons rather than a
        // chord; `Support` doesn't care which.
        Action::UnfoldAll => row!(Full, Full, Full),
        Action::FoldAll => row!(Full, Full, Full),
        Action::CenterViewport => row!(
            Full,
            Full,
            NotApplicable("Mobile scrolls by touch — there is no cursor to centre on.")
        ),
        Action::ZoomIn => row!(Full, Full, Full),
        Action::ZoomOut => row!(Full, Full, Full),

        // ── workspace search from the outline ────────────────────
        Action::SearchWordForward => row!(
            Full,
            Partial("The desktop seeds the quick switcher instead of jumping between hits."),
            Partial(
                "Mobile has no word-under-cursor to search from and no forward/backward \
                 stepping between hits — open the page switcher's Blocks tab (issue #19) \
                 and type the word instead."
            )
        ),
        Action::SearchWordBackward => row!(
            Full,
            Partial("The desktop seeds the quick switcher instead of jumping between hits."),
            Partial(
                "Mobile has no word-under-cursor to search from and no forward/backward \
                 stepping between hits — open the page switcher's Blocks tab (issue #19) \
                 and type the word instead."
            )
        ),

        // ── block structure ──────────────────────────────────────
        Action::NewBlockBelow => row!(Full, Full, Full),
        // RFC 0254 phase 4b: the backend already supports `beforeId`
        // (the same floor-slot `create_block` the desktop's `O` uses)
        // — mobile only lacked the affordance. Reached from the
        // block long-press menu ("New block above"), not a chord.
        Action::NewBlockAbove => row!(Full, Full, Full),
        Action::IndentBlock => row!(Full, Full, Full),
        Action::OutdentBlock => row!(Full, Full, Full),
        Action::MoveBlockUp => row!(Full, Full, Full),
        Action::MoveBlockDown => row!(Full, Full, Full),
        Action::DeleteBlock => row!(Full, Full, Full),
        Action::ToggleCollapsed => row!(Full, Full, Full),
        Action::ToggleTodo => row!(Full, Full, Full),
        // RFC 0254 phase 4b closes issue #18 for mobile only — the
        // long-press menu's "Copy block ref" reads the same
        // `((blk-XXXXXX))` handle the TUI's `y r` copies (off the
        // workspace index, never re-derived) and puts it on the OS
        // clipboard. Desktop stays `Missing`; it wasn't in this
        // phase's scope.
        Action::CopyBlockRef => row!(
            Full,
            Missing(
                "Copying a block ref isn't on the desktop yet — open the block's \
                 properties, or copy the handle from the TUI with `y r`."
            ),
            Full
        ),

        // ── page operations ──────────────────────────────────────
        Action::DeletePage => row!(Full, Full, Full),

        // ── reminders ────────────────────────────────────────────
        Action::InsertRemind => row!(Full, Full, Full),
        Action::InsertRemindNag => row!(
            Full,
            Full,
            Missing("The nag preset isn't on mobile — add a reminder, then edit the rule.")
        ),
        Action::OpenReminders => row!(Full, Full, Full),
        Action::SnoozeReminder => row!(Full, Full, Full),

        // ── properties ───────────────────────────────────────────
        Action::AddProperty => row!(Full, Full, Full),
        Action::OpenProperties => row!(Full, Full, Full),
        // RFC 0254 phase 4b: `pinned::` was already an `Op::SetProp`-
        // backed page property (every property editor could already
        // write it) — the gap was a write affordance on the two GUI
        // clients. Mobile's page switcher now has one (a pin toggle
        // per row, pinned pages sorted first); desktop's own gap
        // stays `Missing` — out of this phase's scope.
        Action::TogglePin => row!(
            Full,
            Missing("Pinning a page isn't on the desktop yet — pin it from the TUI with `g P`."),
            Full
        ),

        // ── block clipboard (view-mode cut / copy / paste) ───────
        //
        // RFC 0254 phase 4b: mobile's long-press "Cut block" renders
        // the subtree to markdown and deletes the source in one
        // round-trip (`cutBlock`), arming the same clipboard "Paste
        // block" reads. Deliberately `Partial`, not `Full` — the
        // catalog's own doc comment on `CutBlock` promises the paste
        // keeps the block's identity (`Op::Move`, refs/backlinks stay
        // valid), which is what the desktop's move-based cut does.
        // Mobile's paste mints a fresh id instead (same as its
        // `CopyBlock`/`PasteBlock` below), so a `((blk-…))` ref to the
        // cut block goes stale across the round-trip — a real,
        // user-visible gap the nudge names rather than overstating.
        Action::CutBlock => row!(
            Missing("Cutting a whole block isn't in the TUI yet — use `d d`, then paste."),
            Full,
            Partial(
                "Mobile's cut duplicates the block with a fresh id instead of moving it, so any \
                 ((blk-…)) refs pointing at it go stale — copy the ref first (long-press → \
                 \"Copy block ref\") if something else links to it."
            )
        ),
        Action::CopyBlock => row!(
            Missing("Copying a whole block + subtree isn't in the TUI yet — use `y y` for the block alone."),
            Full,
            Full
        ),
        Action::PasteBlock => row!(
            Missing("Block-clipboard paste isn't in the TUI yet — `p` pastes the OS clipboard."),
            Full,
            Full
        ),

        // ── insert-mode commits ──────────────────────────────────
        Action::ExitInsert => row!(Full, Full, Full),
        Action::CommitAndContinue => row!(Full, Full, Full),
        // RFC 0254 phase 4b: mobile's `EditableTextarea` now checks
        // for an empty value on `Backspace` before its bracket-pair
        // logic and routes to the same swipe-to-delete handler
        // (confirms first when the block has descendants — a stricter
        // guard than the action's name requires, never a lesser one).
        Action::DeleteEmptyBlock => row!(
            Full,
            Native("Backspace in an empty textarea is handled by the editor itself."),
            Full
        ),

        // ── visual / range ───────────────────────────────────────
        //
        // RFC 0254 phase 3: mobile gained a touch-native multi-select
        // (long-press a block's context menu → "Select blocks", then
        // tap another block to grow the contiguous range up or down —
        // `crates/outl-mobile/src/lib/block-selection.ts`). It
        // dispatches the same range actions the desktop's Visual mode
        // does, so every row below except `EnterVisual` itself is
        // `Full`. `EnterVisual` stays `Partial`: mobile deliberately
        // never grows a modal Visual *state* (no keyboard, no modes —
        // same reasoning as `EnterInsertAfter` above), it only grows a
        // selection, so claiming `Full` would overstate what changed.
        Action::EnterVisual => row!(
            Full,
            Partial(
                "The desktop enters a range selection rather than a full modal Visual state."
            ),
            Partial(
                "Mobile enters a touch-native block selection (long-press a block, then \
                 \"Select blocks\") rather than a modal Visual state."
            )
        ),
        Action::YankCurrentBlock => row!(Full, Full, Full),
        Action::YankRange => row!(Full, Full, Full),
        Action::DeleteRange => row!(Full, Full, Full),
        Action::SelectRangeDown => row!(Full, Full, Full),
        Action::SelectRangeUp => row!(Full, Full, Full),
        Action::MoveVisualRangeUp => row!(Full, Full, Full),
        Action::MoveVisualRangeDown => row!(Full, Full, Full),

        Action::ReselectLastVisual => row!(Full, Full, Full),
        Action::IndentVisualRange => row!(Full, Full, Full),
        Action::OutdentVisualRange => row!(Full, Full, Full),

        // ── code execution ───────────────────────────────────────
        Action::RunCodeBlock => row!(Full, Full, Full),

        // ── inline markdown wrappers ─────────────────────────────
        Action::WrapBold => row!(Full, Full, Full),
        Action::WrapItalic => row!(Full, Full, Full),
        Action::WrapCode => row!(Full, Full, Full),
        Action::WrapStrike => row!(Full, Full, Full),
        Action::InsertLink => row!(Full, Full, Full),

        // ── undo / redo ──────────────────────────────────────────
        // RFC 0254 phase 1: the undo/redo logic was already shared
        // (`outl_actions::history`) — it was trapped behind a
        // desktop-only Tauri command. Moving the command body into
        // `outl-tauri-shared` and registering it on mobile (the
        // keyboard toolbar's Undo / Redo buttons) closed issue #14.
        Action::Undo => row!(Full, Full, Full),
        Action::Redo => row!(Full, Full, Full),
    }
}

/// Render the parity table that `docs/client-parity.md` carries, so
/// the doc is a projection of the code rather than a second copy of
/// it. The delimiters let the doc keep prose around the table.
///
/// Test-only: this crate links into the mobile binary, so the doc
/// generator does not belong in its public surface. Regenerating the
/// doc is a `cargo test` away either way — see
/// `the_parity_doc_matches_the_code`.
#[cfg(test)]
pub(crate) fn parity_table_markdown() -> String {
    let mut out = String::from("| Action | TUI | Desktop | Mobile |\n|---|---|---|---|\n");
    for action in Action::ALL {
        let s = support(*action);
        out.push_str(&format!(
            "| `{:?}` | {} | {} | {} |\n",
            action,
            cell(s.tui),
            cell(s.desktop),
            cell(s.mobile),
        ));
    }
    out
}

/// Words that leak developer vocabulary into a user-facing nudge —
/// the state `support` / `capability_support` replaced was
/// `console.warn("no handler for action X")`, accurate and useless
/// to the person holding the keyboard. Shared by
/// [`crate::capability_support`]'s own nudge test rather than
/// copied, so the banned list can't drift between the two catalogs.
#[cfg(test)]
pub(crate) const DEV_WORDS: [&str; 7] = [
    "unimplemented",
    "not implemented",
    "todo",
    "fixme",
    "no handler",
    "handler",
    "dispatcher",
];

/// Shared by [`crate::capability_support`]'s own table renderer, so
/// both generated tables format a [`Support`] cell identically.
#[cfg(test)]
pub(crate) fn cell(s: Support) -> String {
    match s {
        Support::Full => "✅".to_string(),
        Support::Native(why) => format!("✅ _native — {why}_"),
        Support::Partial(why) => format!("⚠️ {why}"),
        Support::Missing(why) => format!("❌ {why}"),
        Support::NotApplicable(why) => format!("— _{why}_"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_client_declares_support_for_every_action() {
        // The `match` in `support` is exhaustive, so this cannot fail
        // for a missing arm — it fails for a missing *variant* in
        // `Action::ALL`, which would silently shrink every other test
        // in this module and the parity table with it.
        for action in Action::ALL {
            let s = support(*action);
            for client in Client::ALL {
                let _ = s.get(client);
            }
        }
        assert_eq!(Action::ALL.len(), 78, "Action::ALL changed size");
    }

    #[test]
    fn every_degraded_state_explains_itself() {
        for action in Action::ALL {
            let s = support(*action);
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
                    "{action:?} on {client:?} has an empty reason",
                );
            }
        }
    }

    #[test]
    fn nudges_are_written_for_the_user_not_the_developer() {
        // A nudge that leaks developer vocabulary is the same
        // failure with a nicer transport, so pin the vocabulary
        // rather than trusting review to catch it. `DEV_WORDS` lives
        // above `cell`, module-level, so `capability_support`'s own
        // nudge test reuses this exact list instead of copying it.
        for action in Action::ALL {
            let s = support(*action);
            for client in Client::ALL {
                let Some(why) = s.get(client).nudge() else {
                    continue;
                };
                let lower = why.to_lowercase();
                for bad in DEV_WORDS {
                    assert!(
                        !lower.contains(bad),
                        "nudge for {action:?} on {client:?} says {bad:?} — \
                         write what the user can do instead: {why:?}",
                    );
                }
                assert!(
                    why.len() > 20,
                    "nudge for {action:?} on {client:?} is too terse to help: {why:?}",
                );
            }
        }
    }

    #[test]
    fn no_action_is_out_of_reach_on_every_client() {
        // An action no client performs is a chord the catalog hands
        // out and nothing honours — dead weight that still shadows a
        // key. If one lands here, either wire it somewhere or drop
        // the variant.
        for action in Action::ALL {
            let s = support(*action);
            assert!(
                Client::ALL.iter().any(|c| s.get(*c).is_reachable()),
                "{action:?} is unreachable on every client",
            );
        }
    }

    #[test]
    fn every_bound_action_is_listed_in_all() {
        // `default_bindings` is the other half of the catalog. An
        // action it binds but `Action::ALL` omits would keep its
        // chord and drop out of the parity table — visible to the
        // user, invisible to us. Exactly the shape this module
        // exists to close.
        for b in crate::default_bindings() {
            assert!(
                Action::ALL.contains(&b.action),
                "{:?} is bound to {:?} but missing from Action::ALL",
                b.action,
                b.chord,
            );
        }
    }

    #[test]
    fn the_parity_doc_matches_the_code() {
        // `docs/client-parity.md` carries two generated tables now —
        // one per `outl_shortcuts` catalog (`Action` via `support`,
        // `Capability` via `capability_support`) — each inside its
        // own `BEGIN GENERATED` / `END GENERATED` marker pair. Both
        // are checked here, unconditionally: a marker pair this test
        // doesn't know about is a marker pair nothing regenerates,
        // which is exactly how a second, hand-maintained table would
        // survive a regen and drift — so a table lives inside a pair
        // this list names, or it isn't the canonical one.
        //
        // When this fails, the code is right and the doc is stale —
        // rerun with `OUTL_UPDATE_PARITY_DOC=1` (below) rather than
        // editing either table by hand.
        //
        // The doc this replaces was hand-written and wrong in three
        // places at once: it claimed the desktop bound `y r` and `:`
        // (no handler for either) and listed mobile undo / redo as
        // "toolbar" (issue #14, still open). A table a human
        // maintains is a table that disagrees with the build.
        let regions: [(&str, &str, String); 2] = [
            (
                "<!-- BEGIN GENERATED: client-parity -->",
                "<!-- END GENERATED: client-parity -->",
                parity_table_markdown(),
            ),
            (
                "<!-- BEGIN GENERATED: capability-parity -->",
                "<!-- END GENERATED: capability-parity -->",
                crate::capability_support::capability_parity_table_markdown(),
            ),
        ];

        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/client-parity.md");
        let doc =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

        // Regenerate rather than hand-edit:
        //     OUTL_UPDATE_PARITY_DOC=1 cargo test -p outl-shortcuts
        if std::env::var_os("OUTL_UPDATE_PARITY_DOC").is_some() {
            let mut updated = String::new();
            let mut cursor = 0;
            for (begin, end, expected) in &regions {
                let begin_at = doc[cursor..]
                    .find(begin)
                    .unwrap_or_else(|| panic!("{path} has no {begin} marker"))
                    + cursor;
                let content_start = begin_at + begin.len();
                let end_at = doc[content_start..]
                    .find(end)
                    .unwrap_or_else(|| panic!("{path} has no {end} marker"))
                    + content_start;
                updated.push_str(&doc[cursor..begin_at]);
                updated.push_str(begin);
                updated.push_str("\n\n");
                updated.push_str(expected.trim());
                updated.push('\n');
                updated.push_str(end);
                cursor = end_at + end.len();
            }
            updated.push_str(&doc[cursor..]);
            std::fs::write(path, updated).expect("cannot write the parity doc");
            return;
        }

        let mut cursor = 0;
        for (begin, end, expected) in &regions {
            let begin_at = doc[cursor..]
                .find(begin)
                .unwrap_or_else(|| panic!("{path} has no {begin} marker"))
                + cursor;
            let content_start = begin_at + begin.len();
            let end_at = doc[content_start..]
                .find(end)
                .unwrap_or_else(|| panic!("{path} has no {end} marker"))
                + content_start;
            let in_doc = doc[content_start..end_at].trim();
            assert_eq!(
                in_doc,
                expected.trim(),
                "docs/client-parity.md's {begin} … {end} region is out of date \n\
                 — regenerate with `OUTL_UPDATE_PARITY_DOC=1 cargo test -p outl-shortcuts`",
            );
            cursor = end_at + end.len();
        }
    }
}
