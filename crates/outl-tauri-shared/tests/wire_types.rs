//! The Tauri wire contract has two declarations. This makes them agree.
//!
//! `crates/outl-frontend-shared/src/api/types.ts` opens by stating the
//! rule this file enforces:
//!
//! > Every shape here mirrors a `serde`-serialized Rust type. Adding a
//! > field on the Rust side means extending the interface here in the
//! > same change — backend and frontend share the wire format, never a
//! > generator.
//!
//! That decision stands; nothing here generates anything. What was
//! missing is the part that makes a hand-written mirror safe: something
//! that **fails** when the two halves stop matching. Until now, adding a
//! field to a Rust DTO and forgetting `types.ts` compiled cleanly in
//! both languages and surfaced as `undefined` at runtime, on a device,
//! in whichever client happened to read it first.
//!
//! This is the same shape as `outl-theme`'s `the_theme_tokens_match_the_palette`
//! (root `CLAUDE.md` invariant 13), one layer up: that test pins the CSS
//! token names against `Palette`'s fields, this one pins the TypeScript
//! interfaces against the JSON the backend actually emits. That test
//! caught a real bug on its first run, which is the argument for this
//! one.
//!
//! **The Rust side is the serialized value, not the struct definition.**
//! Every case below builds a real instance and runs it through
//! `serde_json`, so `#[serde(rename)]`, `rename_all`, `flatten` and
//! `skip_serializing_if` are all accounted for by construction rather
//! than by a parser guessing at attributes.
//!
//! **The struct literals are exhaustive on purpose.** Adding a field to
//! a Rust DTO breaks *this file's* compilation, which is the point: the
//! author is standing in the one place that also names the TypeScript
//! interface they have to update.
//!
//! ## Adding a DTO
//!
//! Build a value with every optional field populated (a `None` behind
//! `skip_serializing_if` emits no key, and a key that is never emitted
//! cannot be compared), then call [`assert_wire_shape`].

mod ts_parser;

use std::collections::BTreeSet;

use outl_actions::{Backlink, BacklinkCrumb, OutlineNode, PageKind, PageMeta, TodoState};
use outl_md::parse::{ParseWarning, ParseWarningKind};
use outl_tauri_shared::commands::exec::{EmbedContent, RunCodeBlockReply};
use outl_tauri_shared::commands::peers::{PeerDto, PeerStatusDto};
use outl_tauri_shared::commands::property::PropertyKey;
use outl_tauri_shared::commands::reminders::{ReminderDto, ReminderSettingsDto, SnoozePresetDto};
use outl_tauri_shared::commands::theme::ThemeConfigDto;
use outl_tauri_shared::commands::timeline::{PageTimelineDto, TimelineEventDto};
use outl_tauri_shared::state::{
    BacklinksReply, CreateBlockReply, CutBlockReply, MdAheadOfLog, PageView, ProjectionWriteFailed,
    TemplateDto, WorkspaceSummary,
};
use serde::Serialize;

use ts_parser::{interface_fields, source, wire_keys};

/// Compare what `value` serializes to against what `export interface
/// <ts_name>` declares.
///
/// `rust_only` names fields the backend emits that the frontend
/// deliberately does not model. Each entry is a decision someone made
/// and had to write down here — which is the difference between a
/// declared asymmetry and a drift nobody noticed.
fn assert_wire_shape<T: Serialize>(value: &T, ts_name: &str, rust_only: &[&str]) {
    let src = source();
    let declared = interface_fields(&src, ts_name);
    let emitted = wire_keys(&serde_json::to_value(value).expect("DTO serializes"));

    let exempt: BTreeSet<String> = rust_only.iter().map(|s| (*s).to_string()).collect();
    for name in &exempt {
        assert!(
            emitted.contains(name),
            "{ts_name}: `{name}` is listed as backend-only but the backend does not emit it — \
             drop the exemption"
        );
    }

    let missing_in_ts: Vec<&String> = emitted
        .difference(&declared)
        .filter(|f| !exempt.contains(*f))
        .collect();
    let missing_in_rust: Vec<&String> = declared.difference(&emitted).collect();

    assert!(
        missing_in_ts.is_empty(),
        "{ts_name}: the backend emits {missing_in_ts:?}, which `types.ts` does not declare.\n\
         The frontend reads those as `undefined`. Add them to `export interface {ts_name}`, \
         or list them in `rust_only` with a reason if the frontend deliberately ignores them."
    );
    assert!(
        missing_in_rust.is_empty(),
        "{ts_name}: `types.ts` declares {missing_in_rust:?}, which the backend never emits.\n\
         The frontend is modelling a field that is always `undefined` — either the Rust DTO \
         lost it, or the interface should."
    );
}

// --- example values ------------------------------------------------------
//
// Every optional field is populated. A `None` behind
// `skip_serializing_if` emits no key at all, so a value built with
// defaults would silently exempt exactly the fields most likely to have
// drifted.

fn page_meta() -> PageMeta {
    PageMeta {
        id: "01JQ0000000000000000000000".into(),
        slug: "infra".into(),
        title: "Infra".into(),
        kind: PageKind::Page,
        icon: Some("🛠".into()),
        pinned: true,
        page_type: Some("person".into()),
    }
}

fn outline_node() -> OutlineNode {
    OutlineNode {
        id: "01JQ0000000000000000000001".into(),
        text: "restarted the ingest worker".into(),
        todo: Some(TodoState::Todo),
        collapsed: true,
        properties: vec![("owner".into(), "avelino".into())],
        tokens: Vec::new(),
        children: Vec::new(),
    }
}

fn backlink_crumb() -> BacklinkCrumb {
    BacklinkCrumb {
        id: "01JQ0000000000000000000002".into(),
        text: "week 34".into(),
    }
}

fn backlink() -> Backlink {
    Backlink {
        block_id: "01JQ0000000000000000000003".into(),
        block_text: "see [[infra]]".into(),
        todo: Some(TodoState::Done),
        source_page: Some(page_meta()),
        source_block: outline_node(),
        source_block_path: vec![0, 2],
        ancestors: vec![backlink_crumb()],
        source_path: Some("/w/pages/journal.md".into()),
    }
}

fn parse_warning() -> ParseWarning {
    ParseWarning {
        line: 3,
        raw: "# a heading".into(),
        kind: ParseWarningKind::UnrecognizedBlockMarker,
    }
}

fn md_ahead_of_log() -> MdAheadOfLog {
    MdAheadOfLog {
        path: "/w/pages/infra.md".into(),
        lines: 12,
        sample: "\"restarted the ingest worker\"".into(),
    }
}

fn page_view() -> PageView {
    PageView {
        page: page_meta(),
        outline: vec![outline_node()],
        backlinks: vec![backlink()],
        backlinks_order: outl_config::BacklinksOrder::Newest,
        page_properties: vec![("icon".into(), "🛠".into())],
        warnings: vec![parse_warning()],
        md_ahead_of_log: Some(md_ahead_of_log()),
        md_ahead_of_log_checked: true,
        projection_error: Some("disk full".into()),
    }
}

// --- the pins ------------------------------------------------------------

#[test]
fn page_meta_matches_its_interface() {
    assert_wire_shape(&page_meta(), "PageMeta", &[]);
}

/// The Rust `OutlineNode` is the TypeScript `BlockNode`. The names differ by
/// history, not by shape.
#[test]
fn outline_node_matches_block_node() {
    assert_wire_shape(&outline_node(), "BlockNode", &[]);
}

#[test]
fn backlink_crumb_matches_its_interface() {
    assert_wire_shape(&backlink_crumb(), "BacklinkCrumb", &[]);
}

#[test]
fn backlink_matches_its_interface() {
    // `block_text` is the one declared asymmetry: the CLI / MCP JSON
    // envelope consumes it (`outl page rename` returns it inside
    // `affected_refs`), the GUI clients render `source_block.tokens`
    // instead. Documented on the Rust field; pinned here so removing it
    // from `types.ts` cannot look like an accident.
    assert_wire_shape(&backlink(), "Backlink", &["block_text"]);
}

#[test]
fn parse_warning_matches_its_interface() {
    assert_wire_shape(&parse_warning(), "ParseWarning", &[]);
}

#[test]
fn page_view_matches_its_interface() {
    assert_wire_shape(&page_view(), "PageView", &[]);
}

#[test]
fn md_ahead_of_log_matches_its_interface() {
    assert_wire_shape(&md_ahead_of_log(), "MdAheadOfLog", &[]);
}

#[test]
fn projection_write_failed_matches_its_interface() {
    let dto = ProjectionWriteFailed {
        page_id: "01JQ0000000000000000000000".into(),
        md_ahead_of_log: Some(md_ahead_of_log()),
        error: "refused: page markdown ahead of log".into(),
    };
    assert_wire_shape(&dto, "ProjectionWriteFailed", &[]);
}

#[test]
fn create_block_reply_matches_its_interface() {
    let dto = CreateBlockReply {
        view: page_view(),
        new_id: "01JQ0000000000000000000004".into(),
    };
    assert_wire_shape(&dto, "CreateBlockReply", &[]);
}

#[test]
fn cut_block_reply_matches_its_interface() {
    let dto = CutBlockReply {
        markdown: "- cut me\n".into(),
        view: page_view(),
    };
    assert_wire_shape(&dto, "CutBlockReply", &[]);
}

#[test]
fn workspace_summary_matches_its_interface() {
    let dto = WorkspaceSummary {
        blocks: 66_000,
        ops: 120_000,
        actor: "01JQ0000000000000000000005".into(),
        storage_root: "/w".into(),
        ready: true,
    };
    assert_wire_shape(&dto, "WorkspaceSummary", &[]);
}

/// `Palette` is the widest hand-mirrored shape in the contract (~45
/// fields) and the one invariant 13 already cares about. The CSS-token
/// test in `outl-theme` pins the token *names* against the struct; this
/// pins the TypeScript interface against the JSON, closing the third
/// side of the same triangle.
#[test]
fn palette_matches_its_interface() {
    assert_wire_shape(&outl_theme::presets::outl(), "Palette", &[]);
}

// --- the rest of the wire ------------------------------------------------

#[test]
fn peer_dto_matches_its_interface() {
    let dto = PeerDto {
        node_id: "abc123".into(),
        alias: Some("laptop".into()),
        added_at: "2026-09-09T09:00:00Z".into(),
    };
    assert_wire_shape(&dto, "PeerDto", &[]);
}

#[test]
fn peer_status_dto_matches_its_interface() {
    let dto = PeerStatusDto {
        node_id: "abc123".into(),
        alias: Some("phone".into()),
        online: true,
        rtt_ms: Some(42),
    };
    assert_wire_shape(&dto, "PeerStatusDto", &[]);
}

#[test]
fn property_key_matches_its_interface() {
    let dto = PropertyKey {
        key: "owner".into(),
        uses: 12,
    };
    assert_wire_shape(&dto, "PropertyKey", &[]);
}

#[test]
fn theme_config_matches_its_interface() {
    let dto = ThemeConfigDto {
        preset: "outl".into(),
        preset_dark: "outl".into(),
        mode: "system".into(),
    };
    assert_wire_shape(&dto, "ThemeConfig", &[]);
}

#[test]
fn snooze_preset_matches_its_interface() {
    let dto = SnoozePresetDto {
        id: "1h".into(),
        label: "Snooze 1h".into(),
    };
    assert_wire_shape(&dto, "SnoozePreset", &[]);
}

#[test]
fn reminder_settings_matches_its_interface() {
    let dto = ReminderSettingsDto {
        enabled: true,
        quiet_hours: "22:00-07:00".into(),
    };
    assert_wire_shape(&dto, "ReminderSettings", &[]);
}

#[test]
fn reminder_matches_its_interface() {
    let dto = ReminderDto {
        block_id: "01JQ0000000000000000000001".into(),
        page_slug: "2026-09-09".into(),
        page_title: "September 9th, 2026".into(),
        text: "TODO call the bank".into(),
        plain_text: "call the bank".into(),
        rule: "10am".into(),
        anchor_date: "2026-09-09".into(),
        done: false,
        next_fire: Some("2026-09-09T10:00:00".into()),
        snoozed_until: Some("2026-09-09T11:00:00".into()),
        urgency: outl_actions::reminders::Urgency::Soon,
    };
    // `plain_text` is the second declared asymmetry, and this pin found
    // it on its first run: the backend emits it and no frontend reads
    // it, because the only consumer is the OS notification body, built
    // in Rust (`deliver_due_reminders` on both clients). The panel
    // renders `text`.
    //
    // The reason lives here and not as a comment in `types.ts` on
    // purpose — one owner, and this is the thing that fails if someone
    // adds the field on either side.
    assert_wire_shape(&dto, "Reminder", &["plain_text"]);
}

#[test]
fn template_dto_matches_its_interface() {
    let dto = TemplateDto {
        name: "standup".into(),
        slug: "templates/standup".into(),
        duplicate: true,
    };
    assert_wire_shape(&dto, "TemplateDto", &[]);
}

#[test]
fn exec_output_matches_its_interface() {
    let dto = outl_actions::ExecOutputDto {
        stdout: "42\n".into(),
        stderr: String::new(),
        duration_ms: 7,
        exit: "Ok".into(),
    };
    assert_wire_shape(&dto, "ExecOutputDto", &[]);
}

#[test]
fn run_code_block_reply_matches_its_interface() {
    let dto = RunCodeBlockReply {
        language: "python".into(),
        result_ok: Some(outl_actions::ExecOutputDto {
            stdout: "42\n".into(),
            stderr: String::new(),
            duration_ms: 7,
            exit: "Ok".into(),
        }),
        error: Some("boom".into()),
        view: page_view(),
    };
    assert_wire_shape(&dto, "RunCodeBlockReply", &[]);
}

/// The Rust `EmbedContent` is the TypeScript `ResolvedBlock` — one
/// command (`resolve_embeds`) serves both `((ref))` and `!((embed))`.
#[test]
fn embed_content_matches_resolved_block() {
    let dto = EmbedContent {
        handle: "blk-abc123".into(),
        text: "the source block".into(),
        page_slug: "infra".into(),
        status: Some("TODO".into()),
        children: vec![outline_node()],
    };
    assert_wire_shape(&dto, "ResolvedBlock", &[]);
}

#[test]
fn timeline_event_matches_its_interface() {
    let dto = TimelineEventDto {
        at_ms: 1_757_000_000_000,
        actor: "01JQ0000000000000000000005".into(),
        block: "01JQ0000000000000000000001".into(),
        block_deleted: false,
        change: "edited".into(),
        from: Some("before".into()),
        to: Some("after".into()),
        text: Some("trashed text".into()),
        key: Some("owner".into()),
    };
    assert_wire_shape(&dto, "TimelineEvent", &[]);
}

#[test]
fn page_timeline_matches_its_interface() {
    let dto = PageTimelineDto {
        slug: "infra".into(),
        total: 120,
        truncated: true,
        events: Vec::new(),
    };
    assert_wire_shape(&dto, "PageTimeline", &[]);
}

#[test]
fn backlinks_reply_matches_page_backlinks() {
    let dto = BacklinksReply {
        backlinks: vec![backlink()],
        backlinks_order: outl_config::BacklinksOrder::Oldest,
    };
    assert_wire_shape(&dto, "PageBacklinks", &[]);
}

// --- coverage -------------------------------------------------------------

/// Interfaces this file deliberately does not pin, each with a reason.
///
/// Adding a row is the escape hatch; leaving one out is the default. The
/// point is that "nobody pinned it" stops being invisible.
const UNPINNED: &[(&str, &str)] = &[
    // The plugin surface is owned by `outl-plugins` and reaches the wire
    // through `plugin_dto.rs`. Building one of these needs a live
    // `PluginService` on its own thread (Boa's `Context` is `!Send`),
    // which is a different kind of test than "does this struct's JSON
    // match that interface". Pin them when the plugin DTOs get a
    // constructible shape.
    ("PluginCommand", "needs a live PluginService to construct"),
    ("PluginRunReply", "needs a live PluginService to construct"),
    (
        "PluginSettingsField",
        "needs a live PluginService to construct",
    ),
    (
        "PluginSyncHooksReply",
        "needs a live PluginService to construct",
    ),
    (
        "PluginToolbarButton",
        "needs a live PluginService to construct",
    ),
    (
        "PluginTransformer",
        "needs a live PluginService to construct",
    ),
    (
        "PluginTransformResult",
        "needs a live PluginService to construct",
    ),
    ("RegistryItem", "needs a live PluginService to construct"),
];

/// Every `export interface` in `types.ts` is pinned, or says why not.
///
/// The pins above are only as good as their coverage. Eleven interfaces
/// were pinned when this file was written and `types.ts` declared
/// thirty-four — so the next DTO to drift would have been one of the
/// twenty-three nobody was watching, and nothing would have failed. That
/// is the same by-omission gap `command_parity.rs` exists to close, one
/// file over.
#[test]
fn every_wire_interface_is_pinned_or_declared_unpinned() {
    let src = source();
    let declared: BTreeSet<String> = src
        .lines()
        .filter_map(|line| line.strip_prefix("export interface "))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(str::to_string)
        .collect();
    assert!(
        declared.len() > 20,
        "only {} interfaces parsed out of types.ts — the parser is reading \
         the wrong shape and this test proves nothing",
        declared.len()
    );

    let pinned = pinned_interface_names();
    let exempt: BTreeSet<&str> = UNPINNED.iter().map(|(name, _)| *name).collect();

    let unwatched: Vec<&String> = declared
        .iter()
        .filter(|n| !pinned.contains(n.as_str()) && !exempt.contains(n.as_str()))
        .collect();
    assert!(
        unwatched.is_empty(),
        "these wire interfaces have no pin and no exemption: {unwatched:?}.\n\
         A field added to the Rust side of one of these reaches the \
         frontend as `undefined` with nothing failing. Add an \
         `assert_wire_shape` case, or a row in UNPINNED with the reason."
    );

    for (name, why) in UNPINNED {
        assert!(
            declared.contains(*name),
            "UNPINNED names {name}, which types.ts does not declare — stale row"
        );
        assert!(
            !pinned.contains(*name),
            "{name} is both pinned and listed as unpinned — drop the UNPINNED row"
        );
        assert!(
            !why.trim().is_empty(),
            "the exemption for {name} has no reason"
        );
    }
}

/// Interface names this file passes to `assert_wire_shape`.
///
/// Read out of this file's own source rather than tracked by hand: a
/// list maintained beside the calls is a list that drifts from them.
fn pinned_interface_names() -> BTreeSet<String> {
    let src = std::fs::read_to_string(file!())
        .or_else(|_| {
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/wire_types.rs"))
        })
        .expect("this test file is readable");
    let mut names = BTreeSet::new();
    for (idx, _) in src.match_indices("assert_wire_shape(") {
        let rest = &src[idx..];
        let Some(open) = rest.find('"') else { continue };
        let after = &rest[open + 1..];
        let Some(close) = after.find('"') else {
            continue;
        };
        names.insert(after[..close].to_string());
    }
    names
}
