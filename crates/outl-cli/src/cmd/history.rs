//! `outl page history` / `outl block history` — reading the op log's past.
//!
//! Both subcommands render the same events through the same two
//! functions, so the page view and the block view can never disagree
//! about how a change is described. `outl_actions::timeline` owns what
//! the events *are*; this module owns only how they read.
//!
//! Read-only. Nothing here writes an op — see
//! `outl_actions::timeline`'s module doc for why restoring a revision is
//! deliberately not part of this.

use std::path::Path;

use chrono::{Local, TimeZone};
use serde_json::{json, Value};

use outl_actions::{
    block_timeline, find_by_slug, page_slug_of, page_timeline, Change, TimelineEvent,
};

use crate::output::{codes, emit, ApiError};
use crate::ws;

/// Events to show when `--limit` is not given.
///
/// A page edited daily for a year has thousands; printing all of them by
/// default buries the recent change somebody opened the history to find.
/// The count is never capped — the listing says how many it left out.
pub const DEFAULT_LIMIT: usize = 50;

/// How many characters of a block's text to show on one line before
/// eliding. Wide enough to recognise the block, narrow enough that one
/// event stays one line on a normal terminal.
const SNIPPET: usize = 72;

/// `outl page history <slug>`.
pub fn run_page(path: &Path, slug: &str, limit: usize, json_out: bool) -> i32 {
    let result = ws::open(path).and_then(|ctx| {
        let root = find_by_slug(&ctx.workspace, slug).ok_or_else(|| {
            ApiError::new(codes::PAGE_NOT_FOUND, format!("page `{slug}` not found"))
        })?;
        let timeline = page_timeline(&ctx.workspace, root, slug, limit)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        Ok(json!({
            "slug": timeline.slug,
            "page_root": timeline.page_root.to_string(),
            "total": timeline.total,
            "showing": timeline.events.len(),
            "truncated": timeline.truncated(),
            "events": timeline.events.iter().map(event_json).collect::<Vec<_>>(),
        }))
    });
    emit(json_out, result, print_history)
}

/// `outl block history <id>`.
pub fn run_block(path: &Path, id: &str, limit: usize, json_out: bool) -> i32 {
    let result = ws::open(path).and_then(|ctx| {
        let node = super::block::parse_id(id)?;
        let mut events =
            block_timeline(&ctx.workspace, node).map_err(|e| ApiError::internal(e.to_string()))?;
        // A block with no ops is a block that does not exist. Saying
        // "no history" would read as "this block was never edited",
        // which is a different and reassuring answer.
        if events.is_empty() && ctx.workspace.tree().parent(node).is_none() {
            return Err(ApiError::new(
                codes::BLOCK_NOT_FOUND,
                format!("no block `{id}` in this workspace"),
            ));
        }
        // Count before the cut. Reporting the truncated length as the
        // total is what makes a partial listing read as a whole history.
        let total = events.len();
        events.truncate(limit);
        Ok(json!({
            "block": node.to_string(),
            "page": page_slug_of(&ctx.workspace, node),
            "total": total,
            "showing": events.len(),
            "truncated": events.len() < total,
            "events": events.iter().map(event_json).collect::<Vec<_>>(),
        }))
    });
    emit(json_out, result, print_history)
}

/// One event as a flat JSON object.
///
/// Flat rather than a nested enum: `change` is a string tag and the
/// fields that belong to it sit beside it, so a `jq` or MCP consumer
/// reads `.change` and picks the keys it wants without matching on a
/// wrapper object shape.
fn event_json(event: &TimelineEvent) -> Value {
    let mut value = json!({
        "at": local_time(event.ts.physical_ms),
        "physical_ms": event.ts.physical_ms,
        "logical": event.ts.logical,
        "actor": event.actor.0.to_string(),
        "block": event.node.to_string(),
        "block_deleted": event.node_deleted,
    });
    let map = value.as_object_mut().expect("built as an object");
    match &event.change {
        Change::Created => {
            map.insert("change".into(), json!("created"));
        }
        Change::Edited { from, to } => {
            map.insert("change".into(), json!("edited"));
            map.insert("from".into(), json!(from));
            map.insert("to".into(), json!(to));
        }
        Change::Deleted { text } => {
            map.insert("change".into(), json!("deleted"));
            map.insert("text".into(), json!(text));
        }
        Change::Restored => {
            map.insert("change".into(), json!("restored"));
        }
        Change::Moved => {
            map.insert("change".into(), json!("moved"));
        }
        Change::PropertySet { key, from, to } => {
            map.insert("change".into(), json!("property"));
            map.insert("key".into(), json!(key));
            map.insert("from".into(), json!(from));
            map.insert("to".into(), json!(to));
        }
    }
    value
}

/// The HLC's wall-clock half in the user's local zone.
///
/// The `logical` counter and the actor tiebreak are what *order* two
/// events; this is only what to show a human. Both are in the JSON so a
/// caller that needs the real ordering has it.
fn local_time(physical_ms: u64) -> String {
    // `as i64` is not a substitute for this: it wraps, and a wrapped
    // `u64::MAX` renders as a perfectly plausible 1969 date rather than
    // reaching the fallback below. A wrong date that looks right is the
    // one output this command must never produce.
    let Ok(millis) = i64::try_from(physical_ms) else {
        return format!("ms:{physical_ms}");
    };
    match Local.timestamp_millis_opt(millis) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M").to_string(),
        // An HLC whose physical half is nonsense (a device with a broken
        // clock, an op hand-written into the log) still has a real
        // ordering. Show the raw number rather than dropping the event.
        _ => format!("ms:{physical_ms}"),
    }
}

/// Human rendering, shared by both subcommands.
fn print_history(value: &Value) {
    let Some(events) = value.get("events").and_then(Value::as_array) else {
        return;
    };

    if let Some(slug) = value.get("slug").and_then(Value::as_str) {
        println!("history of `{slug}`");
    } else if let Some(block) = value.get("block").and_then(Value::as_str) {
        let page = value
            .get("page")
            .and_then(Value::as_str)
            .map(|p| format!(" (in `{p}`)"))
            .unwrap_or_default();
        println!("history of block {block}{page}");
    }

    let total = value.get("total").and_then(Value::as_u64).unwrap_or(0);
    let showing = value.get("showing").and_then(Value::as_u64).unwrap_or(0);

    if has_no_history(events.len(), total) {
        println!("  nothing in the op log for this yet");
        return;
    }

    for event in events {
        print_event(event);
    }

    println!();
    if value.get("truncated").and_then(Value::as_bool) == Some(true) {
        // Never let a capped listing read as the whole history.
        println!("showing the {showing} most recent of {total} events — `--limit` for more");
    } else {
        println!("{total} event(s)");
    }
}

/// Whether to say "nothing in the op log" rather than print a listing.
///
/// Both halves are load-bearing. An empty listing with a **non-zero**
/// total is `--limit 0`, and answering "no history" for a page with 174
/// events is the same lie the truncation footer exists to prevent.
fn has_no_history(shown: usize, total: u64) -> bool {
    shown == 0 && total == 0
}

fn print_event(event: &Value) {
    let at = event.get("at").and_then(Value::as_str).unwrap_or("?");
    let change = event.get("change").and_then(Value::as_str).unwrap_or("?");
    let gone = if event.get("block_deleted").and_then(Value::as_bool) == Some(true) {
        " [deleted]"
    } else {
        ""
    };
    let field = |key: &str| event.get(key).and_then(Value::as_str).unwrap_or("");

    match change {
        "edited" => {
            println!("  {at}  edited{gone}");
            if let Some(from) = event.get("from").and_then(Value::as_str) {
                println!("      - {}", snippet(from));
            }
            println!("      + {}", snippet(field("to")));
        }
        "deleted" => {
            // The text is the point of the whole command — never elide
            // this one into a bare "deleted".
            println!("  {at}  deleted");
            println!("      - {}", snippet(field("text")));
        }
        "property" => {
            let key = field("key");
            let to = event.get("to").and_then(Value::as_str);
            match to {
                Some(v) => println!("  {at}  {key}:: {}{gone}", snippet(v)),
                None => println!("  {at}  {key}:: cleared{gone}"),
            }
        }
        "restored" => println!("  {at}  restored from trash{gone}"),
        // `created` / `moved` render as their own tag, which is exactly
        // what this arm does — and a `Change` variant added later gets
        // a readable row instead of being dropped.
        other => println!("  {at}  {other}{gone}"),
    }
}

/// First line of `text`, truncated on a char boundary.
fn snippet(text: &str) -> String {
    let line = text.lines().next().unwrap_or("");
    if line.chars().count() <= SNIPPET {
        return line.to_string();
    }
    let cut: String = line.chars().take(SNIPPET).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_with_no_events_reports_no_history() {
        assert!(has_no_history(0, 0));
    }

    /// `--limit 0` empties the listing without emptying the history.
    #[test]
    fn an_empty_listing_with_a_nonzero_total_is_not_no_history() {
        assert!(!has_no_history(0, 174));
    }

    #[test]
    fn a_non_empty_listing_is_never_no_history() {
        assert!(!has_no_history(3, 174));
    }

    #[test]
    fn a_snippet_keeps_a_short_line_whole() {
        assert_eq!(snippet("short"), "short");
    }

    #[test]
    fn a_snippet_shows_only_the_first_line() {
        assert_eq!(snippet("first\nsecond"), "first");
    }

    /// Truncating mid-codepoint panics on a byte slice. Accents are the
    /// common case in the workspace this was built for.
    #[test]
    fn a_snippet_truncates_multibyte_text_without_panicking() {
        let text = "ç".repeat(SNIPPET + 10);
        let out = snippet(&text);
        assert_eq!(out.chars().count(), SNIPPET + 1);
        assert!(out.ends_with('…'));
    }

    /// A clock so broken chrono refuses it still leaves the event
    /// orderable, so it has to render rather than disappear.
    #[test]
    fn an_impossible_timestamp_still_renders() {
        assert_eq!(local_time(u64::MAX), format!("ms:{}", u64::MAX));
    }
}
