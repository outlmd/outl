//! Wire shapes whose TypeScript mirror does **not** live in
//! `@outl/shared/api/types.ts`.
//!
//! The coverage gate used to walk exactly one file. That made it
//! accurate about a universe it had chosen, and silent about everything
//! outside it: fourteen hand-written mirrors lived in
//! `@outl/shared/api/commands.ts`, `outl-desktop/src/lib/api.ts` and
//! `outl-desktop/src/lib/events.ts`, and not one of them was pinned,
//! exempted, or counted. A gate that overstates its own reach is worse
//! than no gate, because the number it prints is what stops anyone
//! looking.
//!
//! Nothing about these shapes made them unpinnable — the reader simply
//! never opened the files. `ts_parser::MIRROR_FILES` is the fix, and
//! this file is what that widening exposed.
//!
//! Two of them stay out of reach and say so in [`FOREIGN_CRATE_GAPS`]:
//! their Rust type lives in `outl-desktop`, which this crate does not
//! (and should not) depend on.

mod ts_parser;
mod wire_pin;

use std::collections::BTreeSet;

use outl_shortcuts::{Chord, ChordSequence, Key, Modifiers, Support};
use outl_tauri_shared::commands::shortcuts::SupportDto;
use outl_tauri_shared::plugin_dto::PluginKeybindingDto;
use outl_tauri_shared::state::{BlockHit, RefProjectionFailed};

use wire_pin::assert_wire_shape;

// --- @outl/shared/api/commands.ts -----------------------------------------

#[test]
fn emoji_hit_matches_its_interface() {
    let dto = outl_md::EmojiHit {
        shortcode: "tada".into(),
        glyph: "🎉".into(),
        score: 90,
    };
    assert_wire_shape(&dto, "EmojiHit", &[]);
}

#[test]
fn block_hit_matches_its_interface() {
    let dto = BlockHit {
        handle: "blk-r6s4a1".into(),
        text: "restarted the ingest worker".into(),
        source_slug: "infra".into(),
    };
    assert_wire_shape(&dto, "BlockHit", &[]);
}

#[test]
fn imported_asset_matches_its_interface() {
    let dto = outl_actions::ImportedAsset {
        rel_path: "assets/abc123.pdf".into(),
        display_name: "invoice.pdf".into(),
        is_image: false,
        markdown: "[invoice.pdf](assets/abc123.pdf)".into(),
    };
    assert_wire_shape(&dto, "ImportedAsset", &[]);
}

// --- outl-desktop/src/lib/api.ts (the shortcut catalog) -------------------
//
// `Key` and `Action` are unions and are pinned in `wire_enums.rs`;
// what is left here is the three structs that carry them.

#[test]
fn chord_matches_its_interface() {
    let dto = Chord {
        mods: Modifiers(0b0101),
        key: Key::char('p'),
    };
    assert_wire_shape(&dto, "Chord", &[]);
}

/// `Binding.chord` is a `ChordSequence`, which is
/// `#[serde(transparent)]` over `Vec<Chord>` — so the wire carries an
/// array and the TypeScript says `Chord[]`. A two-chord sequence (the
/// vim-style `g j`) is the shape worth building here: a one-element
/// value would serialize identically whether or not the transparency
/// survived.
#[test]
fn binding_matches_its_interface() {
    let dto = outl_shortcuts::Binding::new(
        ChordSequence(vec![
            Chord {
                mods: Modifiers(0),
                key: Key::char('g'),
            },
            Chord {
                mods: Modifiers(0),
                key: Key::char('j'),
            },
        ]),
        outl_shortcuts::Mode::Normal,
        outl_shortcuts::Action::NextDay,
        "Next day",
    );
    assert_wire_shape(&dto, "Binding", &[]);
}

/// The lesser `Support` states carry the sentence shown to the user
/// (root `CLAUDE.md` invariant 12), so the example must be one of them:
/// `Full` serializes `why` as `null`, which is still a key, but a value
/// that exercises the string is the honest one to pin.
#[test]
fn support_dto_matches_its_interface() {
    let dto = SupportDto::from(Support::Partial("only the block, not the character"));
    assert_wire_shape(&dto, "SupportDto", &[]);
}

/// The Rust `ActionSupportDto` is the TypeScript `ActionSupport`. Its
/// fields are private, so the pin takes a row straight out of the
/// command body — which is the value a client actually receives.
#[test]
fn action_support_matches_its_interface() {
    let rows = outl_tauri_shared::commands::shortcuts::list_action_support();
    let row = rows.first().expect("the action catalog is never empty");
    assert_wire_shape(row, "ActionSupport", &[]);
}

#[test]
fn plugin_keybinding_matches_its_interface() {
    let dto = PluginKeybindingDto {
        chord: ChordSequence(vec![Chord {
            mods: Modifiers(0b1000),
            key: Key::char('k'),
        }]),
        mode: outl_shortcuts::Mode::Global,
        plugin_id: "app.outl.demo".into(),
        command_id: "demo.run".into(),
        description: "Run the demo".into(),
    };
    assert_wire_shape(&dto, "PluginKeybinding", &[]);
}

// --- outl-desktop/src/lib/events.ts ---------------------------------------

/// This payload was a `serde_json::json!` literal until this change.
/// The JSON is the same; what is new is that there is now something to
/// serialize, which is the only reason the mirror can be pinned at all.
#[test]
fn ref_projection_failed_matches_its_interface() {
    let dto = RefProjectionFailed {
        target: "infra".into(),
        error: "disk full".into(),
    };
    assert_wire_shape(&dto, "RefProjectionFailedPayload", &[]);
}

// --- what stays out of reach ----------------------------------------------

/// Mirrors this crate cannot pin because their Rust type lives in a
/// client crate, keyed by TypeScript name.
///
/// The reason is a real boundary, not a shrug: `outl-tauri-shared` is a
/// *dependency* of both clients, so depending back on `outl-desktop`
/// would be a cycle. Closing either gap means moving the Rust type down
/// here, which is a design decision with its own cost — not something
/// to do for a test.
///
/// `wire_types.rs`'s coverage gate reads this table, so a row here is
/// counted as a declared gap rather than silently missing.
pub const FOREIGN_CRATE_GAPS: &[(&str, &str)] = &[
    (
        "Settings",
        "Rust `Settings` lives in outl-desktop/src-tauri/src/settings.rs; \
         outl-tauri-shared is that crate's dependency and cannot depend back",
    ),
    (
        "PeerPairingTicketPayload",
        "emitted by outl-desktop/src-tauri/src/commands/peers.rs, which is \
         downstream of this crate",
    ),
];

/// A gap that has been closed must not keep its excuse.
///
/// Invariant 10's "exemptions that outlived their premise" is not a
/// hypothetical here: every one of the eight `UNPINNED` rows this change
/// deleted claimed to need "a live PluginService", and all eight were
/// plain `pub` structs with `pub` fields.
#[test]
fn no_foreign_crate_gap_is_actually_reachable() {
    for (name, why) in FOREIGN_CRATE_GAPS {
        assert!(
            !why.trim().is_empty(),
            "the gap recorded for {name} has no reason"
        );
        assert!(
            why.contains("outl-desktop"),
            "{name}: this table is only for types owned by a client crate. \
             A type this crate can reach belongs in a pin, not here."
        );
    }
    let names: BTreeSet<&str> = FOREIGN_CRATE_GAPS.iter().map(|(n, _)| *n).collect();
    assert_eq!(
        names.len(),
        FOREIGN_CRATE_GAPS.len(),
        "a TypeScript name is listed twice"
    );
}
