//! Example values the wire pins serialize.
//!
//! Shared by `wire_types.rs` and `wire_plugins.rs` — a `PageView` is a
//! forty-line literal and two of them would be two things to keep in
//! step, which is the shape of problem this whole directory exists to
//! remove.
#![allow(dead_code)]

use outl_actions::{Backlink, BacklinkCrumb, OutlineNode, PageKind, PageMeta, TodoState};
use outl_md::parse::{ParseWarning, ParseWarningKind};
use outl_tauri_shared::state::{MdAheadOfLog, PageView};

// --- example values ------------------------------------------------------
//
// Every optional field is populated. A `None` behind
// `skip_serializing_if` emits no key at all, so a value built with
// defaults would silently exempt exactly the fields most likely to have
// drifted.

pub fn page_meta() -> PageMeta {
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

pub fn outline_node() -> OutlineNode {
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

pub fn backlink_crumb() -> BacklinkCrumb {
    BacklinkCrumb {
        id: "01JQ0000000000000000000002".into(),
        text: "week 34".into(),
    }
}

pub fn backlink() -> Backlink {
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

pub fn parse_warning() -> ParseWarning {
    ParseWarning {
        line: 3,
        raw: "# a heading".into(),
        kind: ParseWarningKind::UnrecognizedBlockMarker,
    }
}

pub fn md_ahead_of_log() -> MdAheadOfLog {
    MdAheadOfLog {
        path: "/w/pages/infra.md".into(),
        lines: 12,
        sample: "\"restarted the ingest worker\"".into(),
    }
}

pub fn page_view() -> PageView {
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
