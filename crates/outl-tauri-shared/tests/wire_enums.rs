//! The half of the wire contract the struct pins could not reach.
//!
//! `wire_types.rs` compares a serialized struct's **keys** against an
//! `export interface`. Its `wire_keys` helper panics on anything that
//! is not a JSON object — and a `serde` enum serializes to a *string*,
//! or to an object whose interesting part is a tag rather than a key
//! set. So every enum on the wire was unpinnable by construction, and
//! twelve of them were unpinned.
//!
//! That is not a theoretical gap. `outl_md::ParseWarningKind` had six
//! variants; `types.ts` declared **one**:
//!
//! ```text
//! export type ParseWarningKind = "unrecognized_block_marker";
//! ```
//!
//! The five `Remind*` variants had shipped, were produced by
//! `outl_md::remind`, and reached `PageView.warnings` — and the comment
//! above that line claimed future variants "land here in lockstep".
//! Nothing could have failed, so nothing did.
//!
//! ## What a pin here actually proves
//!
//! Two things, and the second is the one that matters:
//!
//! 1. **The wire tags match.** Every variant is serialized through
//!    `serde_json`, so `rename_all`, `#[serde(rename)]`, `tag` and
//!    `content` are accounted for by construction rather than by a
//!    parser guessing at attributes — the same rule `wire_types.rs`
//!    follows.
//! 2. **The variant list is exhaustive.** A pin that compares a
//!    hand-written subset against TypeScript proves nothing: that is
//!    precisely the state `ParseWarningKind` was already in, one
//!    language over. Every list below is built by `wire_pin`'s
//!    `wire_variants!`, whose `match` stops this file compiling when a
//!    variant is added in Rust — in the one place that also names the
//!    TypeScript union to extend.
//!
//! ## Adding an enum
//!
//! Build its variants with `wire_variants!` and call the `wire_pin`
//! assertion that matches its wire form:
//!
//! - serializes to a string -> `assert_string_enum`
//! - serializes to a tagged object -> `assert_tagged_enum`
//! - a hand-written tag on a struct field (no Rust enum on the wire)
//!   -> `assert_field_tags`
//! - a tag only observable through a carrier, or through no Rust type
//!   at all -> `assert_union_tags` / `assert_union_variant_tags`

mod ts_parser;
mod wire_pin;

use std::collections::BTreeSet;

use wire_pin::{
    assert_field_tags, assert_string_enum, assert_tagged_enum, assert_union_tags,
    assert_union_variant_tags, wire_variants,
};

// --- string enums ---------------------------------------------------------

/// The pin this whole file exists for.
///
/// Six variants in `outl_md::ParseWarningKind`, one in `types.ts`, and
/// nothing in either language that could notice.
#[test]
fn parse_warning_kind_matches_its_union() {
    use outl_md::parse::ParseWarningKind as K;
    let variants = wire_variants!(K;
        K::UnrecognizedBlockMarker => K::UnrecognizedBlockMarker,
        K::RemindMissingAnchor => K::RemindMissingAnchor,
        K::RemindInvalidTime => K::RemindInvalidTime,
        K::RemindInvalidInterval => K::RemindInvalidInterval,
        K::RemindInvalidStop => K::RemindInvalidStop,
        K::RemindMaxClamped => K::RemindMaxClamped,
    );
    assert_string_enum(&variants, "ParseWarningKind");
}

#[test]
fn page_kind_matches_its_union() {
    use outl_actions::PageKind as K;
    let variants = wire_variants!(K;
        K::Page => K::Page,
        K::Journal => K::Journal,
    );
    assert_string_enum(&variants, "PageKind");
}

#[test]
fn backlinks_order_matches_its_union() {
    use outl_config::BacklinksOrder as O;
    let variants = wire_variants!(O;
        O::Newest => O::Newest,
        O::Oldest => O::Oldest,
    );
    assert_string_enum(&variants, "BacklinksOrder");
}

#[test]
fn reminder_urgency_matches_its_union() {
    use outl_actions::reminders::Urgency as U;
    let variants = wire_variants!(U;
        U::Overdue => U::Overdue,
        U::Soon => U::Soon,
        U::Later => U::Later,
        U::Finished => U::Finished,
    );
    assert_string_enum(&variants, "ReminderUrgency");
}

#[test]
fn plugin_field_kind_matches_its_union() {
    use outl_plugins::FieldKind as F;
    let variants = wire_variants!(F;
        F::String => F::String,
        F::Integer => F::Integer,
        F::Number => F::Number,
        F::Boolean => F::Boolean,
        F::Json => F::Json,
    );
    assert_string_enum(&variants, "PluginFieldKind");
}

/// The Rust `Mode` is the TypeScript `ShortcutMode`. Both the shortcut
/// catalog (`Binding.mode`) and a plugin keybinding
/// (`PluginKeybinding.mode`) carry it.
#[test]
fn shortcut_mode_matches_its_union() {
    use outl_shortcuts::Mode as M;
    let variants = wire_variants!(M;
        M::Global => M::Global,
        M::Normal => M::Normal,
        M::Insert => M::Insert,
        M::Visual => M::Visual,
        M::Overlay => M::Overlay,
    );
    assert_string_enum(&variants, "ShortcutMode");
}

/// `TodoState` has no `Serialize` derive — `OutlineNode.todo` goes
/// through a `serialize_with`. So the pin serializes the carrier and
/// reads the field back, which is the wire form either way and stays
/// correct if that custom serializer ever changes.
#[test]
fn todo_state_matches_its_union() {
    use outl_actions::TodoState as T;
    let variants = wire_variants!(T;
        T::Todo => T::Todo,
        T::Doing => T::Doing,
        T::Done => T::Done,
    );
    let tags: Vec<String> = variants
        .iter()
        .map(|state| {
            let node = outl_actions::OutlineNode {
                id: "01JQ0000000000000000000001".into(),
                text: "x".into(),
                todo: Some(*state),
                collapsed: false,
                properties: Vec::new(),
                tokens: Vec::new(),
                children: Vec::new(),
            };
            match serde_json::to_value(&node).expect("node serializes")["todo"] {
                serde_json::Value::String(ref s) => s.clone(),
                ref other => panic!("OutlineNode.todo serialized to {other}, not a string"),
            }
        })
        .collect();
    assert_union_tags(&tags.into_iter().collect(), "TodoState");
}

// --- tagged enums ---------------------------------------------------------

#[test]
fn inline_token_matches_its_union() {
    use outl_md::InlineToken as T;
    let inner = || vec![T::Plain { value: "x".into() }];
    let variants = wire_variants!(T;
        T::Plain { .. } => T::Plain { value: "x".into() },
        T::Bold { .. } => T::Bold { inner: inner() },
        T::Italic { .. } => T::Italic { inner: inner() },
        T::Strike { .. } => T::Strike { inner: inner() },
        T::Highlight { .. } => T::Highlight { inner: inner() },
        T::Code { .. } => T::Code { value: "x".into() },
        T::Link { .. } => T::Link { value: "x".into(), href: "https://outl.app".into() },
        T::Image { .. } => T::Image { alt: "x".into(), href: "assets/a.png".into() },
        T::Ref { .. } => T::Ref { value: "infra".into() },
        T::Tag { .. } => T::Tag { value: "#infra".into() },
        T::BlockRef { .. } => T::BlockRef { value: "blk-abc123".into() },
        T::Embed { .. } => T::Embed { value: "blk-abc123".into() },
        T::Emoji { .. } => T::Emoji { shortcode: "tada".into(), glyph: "🎉".into() },
    );
    assert_tagged_enum(&variants, "InlineToken", "kind");
}

#[test]
fn sync_progress_matches_its_union() {
    use outl_actions::SyncProgress as P;
    let peer = || "abc123".to_string();
    let variants = wire_variants!(P;
        P::Connecting { .. } => P::Connecting { peer: peer() },
        P::Snapshot { .. } => P::Snapshot { peer: peer(), received: 1, total: 2 },
        P::Asset { .. } => P::Asset { peer: peer(), received: 1, total: 2 },
        P::ReceivedOps { .. } => P::ReceivedOps { peer: peer(), count: 3, nodes: Vec::new() },
        P::PushedOps { .. } => P::PushedOps { peer: peer(), count: 4 },
        P::Synced { .. } => P::Synced { peer: peer() },
        P::Interrupted { .. } => P::Interrupted { peer: peer(), reason: "slept".into() },
        P::Failed { .. } => P::Failed { peer: peer(), error: "boom".into() },
    );
    assert_tagged_enum(&variants, "SyncProgress", "phase");
}

/// `Key` is `#[serde(tag = "kind", content = "value")]`, so a
/// data-carrying variant emits `{kind, value}` and a unit variant emits
/// `{kind}` alone. The per-variant field comparison is what makes that
/// difference visible.
#[test]
fn chord_key_matches_its_union() {
    use outl_shortcuts::Key as K;
    let variants = wire_variants!(K;
        K::Char(_) => K::Char('a'),
        K::Enter => K::Enter,
        K::Esc => K::Esc,
        K::Tab => K::Tab,
        K::Backspace => K::Backspace,
        K::Delete => K::Delete,
        K::Up => K::Up,
        K::Down => K::Down,
        K::Left => K::Left,
        K::Right => K::Right,
        K::Home => K::Home,
        K::End => K::End,
        K::PageUp => K::PageUp,
        K::PageDown => K::PageDown,
        K::Space => K::Space,
        K::Function(_) => K::Function(1),
    );
    assert_tagged_enum(&variants, "Key", "kind");
}

/// `Action` brings its own exhaustive list: `outl_shortcuts::Action::ALL`
/// is pinned against the enum inside that crate (root `CLAUDE.md`
/// invariant 12 makes a new variant a compile error there), so this pin
/// reuses it instead of keeping a second copy that could go stale in a
/// different direction than the first.
///
/// The desktop dispatcher narrows on `action.kind`, which is why the
/// mirror must be complete: an unlisted variant is not a type error in
/// TypeScript, it is a chord that silently resolves to nothing.
#[test]
fn action_matches_its_union() {
    assert_tagged_enum(outl_shortcuts::Action::ALL, "Action", "kind");
}

// --- hand-written tags ----------------------------------------------------

/// `TimelineEventDto.change` is a `String` the DTO builder writes by
/// hand, so `serde` proves nothing about it and the contract lives
/// entirely in the TypeScript union.
///
/// The tags come out of `commands::timeline::to_dto` — the real
/// producer — rather than a table retyped here, because a second table
/// is a second owner and the point of a pin is to have one.
#[test]
fn timeline_change_tags_match_their_union() {
    use outl_actions::Change as C;
    let changes = wire_variants!(C;
        C::Created => C::Created,
        C::Edited { .. } => C::Edited { from: Some("a".into()), to: "b".into() },
        C::Deleted { .. } => C::Deleted { text: "gone".into() },
        C::Restored => C::Restored,
        C::Moved => C::Moved,
        C::PropertySet { .. } => C::PropertySet {
            key: "owner".into(),
            from: None,
            to: Some("avelino".into()),
        },
    );
    let actor = outl_core::ActorId::new();
    let tags: Vec<String> = changes
        .into_iter()
        .map(|change| {
            let event = outl_actions::TimelineEvent {
                ts: outl_core::Hlc::new(1_757_000_000_000, 0, actor),
                actor,
                node: outl_core::NodeId::new(),
                node_deleted: false,
                change,
            };
            outl_tauri_shared::commands::timeline::to_dto(&event).change
        })
        .collect();
    let refs: Vec<&str> = tags.iter().map(String::as_str).collect();
    assert_field_tags(&refs, "TimelineEvent", "change");
}

/// `ThemeConfigDto.mode` is the same shape of hand-written tag, mapped
/// off `outl_config::ThemeMode` inside the command body.
#[test]
fn theme_mode_tags_match_their_union() {
    use outl_config::ThemeMode as M;
    let modes = wire_variants!(M;
        M::Light => M::Light,
        M::Dark => M::Dark,
        M::Auto => M::Auto,
    );
    let tags: Vec<String> = modes
        .into_iter()
        .map(|mode| {
            let mut cfg = outl_config::Config::default();
            cfg.theme.mode = mode;
            outl_tauri_shared::commands::theme::theme_config_dto(&cfg).mode
        })
        .collect();
    let refs: Vec<&str> = tags.iter().map(String::as_str).collect();
    assert_field_tags(&refs, "ThemeConfig", "mode");
}

/// `SupportDto.kind` comes from `Support::kind()`, a `&'static str`.
/// Five states, and the desktop's help overlay switches on all of them.
#[test]
fn support_kind_tags_match_their_union() {
    use outl_shortcuts::Support as S;
    let variants = wire_variants!(S;
        S::Full => S::Full,
        S::Native(_) => S::Native("the platform does it"),
        S::Partial(_) => S::Partial("only the block, not the character"),
        S::Missing(_) => S::Missing("not here yet"),
        S::NotApplicable(_) => S::NotApplicable("cannot exist here"),
    );
    let tags: Vec<&str> = variants.iter().map(|s| s.kind()).collect();
    assert_field_tags(&tags, "SupportDto", "kind");
}

/// The deep-link payload is the one wire shape with **no Rust type at
/// all**: both clients' `lib.rs` build it with `serde_json::json!`, and
/// this crate cannot reach either.
///
/// What it can reach is `outl_actions::DeepLinkTarget`, the enum those
/// two builders match on — so the pin catches the drift that actually
/// happens (a variant added upstream, the union not extended) while
/// being honest that the tag spellings below are a second copy of a
/// mapping this crate does not own. `wire_types.rs`'s `UNTYPED_EVENTS`
/// records the gap; closing it means giving the payload a real DTO in
/// a crate both clients already depend on.
#[test]
fn deep_link_targets_match_their_union() {
    use outl_actions::DeepLinkTarget as T;
    let tags = wire_variants!(T;
        T::Today => "today",
        T::Daily(_) => "daily",
        T::Page(_) => "page",
    );
    let emitted: BTreeSet<String> = tags.iter().map(|t| (*t).to_string()).collect();
    assert_union_variant_tags(&emitted, "DeepLinkNavigate", "kind");
}
