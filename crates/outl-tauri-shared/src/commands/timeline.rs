//! A page's history, as a command.
//!
//! The op log holds every revision of every block, and until this
//! existed no GUI client could read any of it. `outl_actions::timeline`
//! owns what the events *are* — which blocks count as the page's, what
//! is deliberately not an event, why deletions are included — and this
//! is the IPC wrapper.
//!
//! **Read-only.** There is no "restore this revision" command, on
//! purpose: see `outl_actions::timeline`'s module doc.

use serde::Serialize;

use crate::helpers::{parse_node_id, with_ws};
use crate::host::AppHost;

/// Events returned when the client does not ask for a number.
///
/// The client shows a page's recent history in a side panel, and a page
/// edited daily for a year has thousands of events. The **count** is
/// never capped — [`PageTimelineDto::total`] is what the panel shows so
/// a capped list never reads as the whole history.
pub const DEFAULT_LIMIT: usize = 100;

/// One event on the wire.
///
/// Flat, with `change` as a string tag: a reader switches on `change`
/// and picks the fields that belong to it, which is a far easier shape
/// to consume than a nested enum's wrapper object. It is **not** a
/// discriminated union — every field is present on every event — so
/// nothing narrows; `change` tells you which ones are meaningful.
#[derive(Debug, Clone, Serialize)]
pub struct TimelineEventDto {
    /// Wall-clock half of the op's HLC, in milliseconds since the epoch.
    /// The client formats it; the ordering is already applied.
    ///
    /// The CLI's JSON calls the same number `physical_ms` and ships
    /// `logical` beside it, because a script may need to reproduce the
    /// real HLC order. A GUI never does — it renders the list it was
    /// handed — so this carries the rendering name and drops the
    /// counter.
    pub at_ms: u64,
    /// The device that made the change.
    pub actor: String,
    /// The block the change was on.
    pub block: String,
    /// Whether that block is in the trash today.
    pub block_deleted: bool,
    /// `created` | `edited` | `deleted` | `restored` | `moved` | `property`.
    pub change: String,
    /// `edited`: the text before. `property`: the previous value.
    pub from: Option<String>,
    /// `edited`: the text after. `property`: the new value (absent when
    /// the property was cleared).
    pub to: Option<String>,
    /// `deleted`: what the block said when it was trashed.
    pub text: Option<String>,
    /// `property`: the key.
    pub key: Option<String>,
}

/// A page's history plus the count it was cut from.
#[derive(Debug, Clone, Serialize)]
pub struct PageTimelineDto {
    /// The page's slug.
    pub slug: String,
    /// Every event the page has, before `limit`.
    pub total: usize,
    /// Whether [`Self::events`] is shorter than [`Self::total`].
    pub truncated: bool,
    /// The events, newest first.
    pub events: Vec<TimelineEventDto>,
}

/// Read `page_id`'s history, newest first.
///
/// `limit` defaults to [`DEFAULT_LIMIT`]; `0` is read as the default
/// rather than as "no events", because a client that forgets to send
/// the field should get a usable panel, not an empty one.
pub fn page_timeline<S: AppHost>(
    state: &S,
    page_id: String,
    limit: Option<usize>,
) -> Result<PageTimelineDto, String> {
    let node = parse_node_id(&page_id)?;
    let limit = match limit {
        Some(0) | None => DEFAULT_LIMIT,
        Some(n) => n,
    };
    with_ws(state, |ws| {
        let slug = outl_actions::page_meta(ws, node)
            .map(|meta| meta.slug)
            .ok_or_else(|| format!("no page {page_id}"))?;
        let timeline = outl_actions::page_timeline(ws, node, &slug, limit)
            .map_err(|e| format!("could not read the history of {slug}: {e}"))?;
        Ok(PageTimelineDto {
            slug: timeline.slug.clone(),
            total: timeline.total,
            truncated: timeline.truncated(),
            events: timeline.events.iter().map(to_dto).collect(),
        })
    })
}

fn to_dto(event: &outl_actions::TimelineEvent) -> TimelineEventDto {
    use outl_actions::Change;
    let mut dto = TimelineEventDto {
        at_ms: event.ts.physical_ms,
        actor: event.actor.0.to_string(),
        block: event.node.to_string(),
        block_deleted: event.node_deleted,
        change: String::new(),
        from: None,
        to: None,
        text: None,
        key: None,
    };
    match &event.change {
        Change::Created => dto.change = "created".into(),
        Change::Edited { from, to } => {
            dto.change = "edited".into();
            dto.from = from.clone();
            dto.to = Some(to.clone());
        }
        Change::Deleted { text } => {
            dto.change = "deleted".into();
            dto.text = Some(text.clone());
        }
        Change::Restored => dto.change = "restored".into(),
        Change::Moved => dto.change = "moved".into(),
        Change::PropertySet { key, from, to } => {
            dto.change = "property".into();
            dto.key = Some(key.clone());
            dto.from = from.clone();
            dto.to = to.clone();
        }
    }
    dto
}
