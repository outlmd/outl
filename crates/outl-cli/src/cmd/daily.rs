//! `outl daily …` — journal helpers.
//!
//! Date parsing accepts ISO (`YYYY-MM-DD`), natural shortcuts
//! (`today`, `yesterday`, `tomorrow`), and the Roam-style "April 22nd,
//! 2026" form so callers can quote what the user typed verbatim.

use std::path::Path;

use chrono::{Duration, NaiveDate};
use clap::Subcommand;
use serde_json::{json, Value};

use outl_actions::{
    append_block, apply_page_md_with_sidecar_guarded, journal_slug, open_journal, open_today,
    page_meta, project_outline, render_page_md, today,
};

use crate::human::print_outline_tree;
use crate::output::{codes, emit, ApiError};
use crate::ws::{self, WsCtx};

/// `outl daily …` subcommands.
#[derive(Subcommand, Debug)]
pub enum DailyCommand {
    /// Open (or create) today's journal and return it.
    Today {
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Get a specific journal by date.
    Get {
        /// Date (ISO, natural, or `April 22nd, 2026`).
        date: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// Append a block to a journal.
    Append {
        /// Block text.
        #[arg(long)]
        text: String,
        /// Target date (defaults to today).
        #[arg(long)]
        date: Option<String>,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
    /// List journals in a date range.
    Range {
        /// First date (inclusive).
        #[arg(long)]
        from: String,
        /// Last date (inclusive).
        #[arg(long)]
        to: String,
        /// Force JSON output.
        #[arg(long)]
        json: bool,
    },
}

/// Run a `outl daily …` invocation.
pub fn run(cmd: &DailyCommand, path: &Path) -> i32 {
    match cmd {
        DailyCommand::Today { json } => {
            let result = ws::open(path).and_then(|mut ctx| today_handler(&mut ctx));
            emit(*json, result, print_journal)
        }
        DailyCommand::Get { date, json } => {
            let result = ws::open(path).and_then(|mut ctx| get(&mut ctx, date));
            emit(*json, result, print_journal)
        }
        DailyCommand::Append { text, date, json } => {
            let result = ws::open(path).and_then(|mut ctx| append(&mut ctx, date.as_deref(), text));
            emit(*json, result, |v| {
                println!(
                    "appended block {} to {}",
                    v.get("block_id").and_then(Value::as_str).unwrap_or("?"),
                    v.get("date").and_then(Value::as_str).unwrap_or("?")
                );
            })
        }
        DailyCommand::Range { from, to, json } => {
            let result = ws::open(path).and_then(|ctx| range(&ctx, from, to));
            emit(*json, result, |v| {
                if let Some(items) = v.get("journals").and_then(Value::as_array) {
                    for j in items {
                        let date = j.get("date").and_then(Value::as_str).unwrap_or("?");
                        let has_blocks = j
                            .get("outline")
                            .and_then(Value::as_array)
                            .map(|a| !a.is_empty())
                            .unwrap_or(false);
                        let marker = if has_blocks { "•" } else { " " };
                        println!("{marker} {date}");
                    }
                }
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Open today's journal, render its projection, return meta + outline.
pub fn today_handler(ctx: &mut WsCtx) -> Result<Value, ApiError> {
    let id = open_today(&mut ctx.workspace, &ctx.hlc).map_err(ApiError::internal)?;
    apply_page_md_with_sidecar_guarded(&ctx.workspace, &ctx.root, id)?;
    journal_payload(ctx, id, today())
}

/// Open the journal for `date`.
pub fn get(ctx: &mut WsCtx, date: &str) -> Result<Value, ApiError> {
    let parsed = parse_date(date)?;
    let id = open_journal(&mut ctx.workspace, &ctx.hlc, parsed).map_err(ApiError::internal)?;
    apply_page_md_with_sidecar_guarded(&ctx.workspace, &ctx.root, id)?;
    journal_payload(ctx, id, parsed)
}

/// Append a block to a journal (today by default).
pub fn append(ctx: &mut WsCtx, date: Option<&str>, text: &str) -> Result<Value, ApiError> {
    let parsed = match date {
        Some(d) => parse_date(d)?,
        None => today(),
    };
    let journal_id =
        open_journal(&mut ctx.workspace, &ctx.hlc, parsed).map_err(ApiError::internal)?;
    let block_id = append_block(&mut ctx.workspace, &ctx.hlc, Some(journal_id), Some(text))
        .map_err(ApiError::internal)?;
    apply_page_md_with_sidecar_guarded(&ctx.workspace, &ctx.root, journal_id)?;
    Ok(json!({
        "date": journal_slug(parsed),
        "block_id": block_id.to_string(),
        "text": text,
    }))
}

/// Range of journals between two dates. Days that do not have a
/// materialised journal yet are still listed with an empty outline.
pub fn range(ctx: &WsCtx, from: &str, to: &str) -> Result<Value, ApiError> {
    let start = parse_date(from)?;
    let end = parse_date(to)?;
    if end < start {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            "`--from` must be <= `--to`".to_string(),
        ));
    }
    let mut entries = Vec::new();
    let mut cursor = start;
    while cursor <= end {
        let slug = journal_slug(cursor);
        let value = match outl_actions::find_by_slug(&ctx.workspace, &slug) {
            Some(id) => {
                let outline = project_outline(&ctx.workspace, id);
                let meta = page_meta(&ctx.workspace, id);
                json!({
                    "date": slug,
                    "exists": true,
                    "meta": meta,
                    "outline": serde_json::to_value(&outline).map_err(ApiError::internal)?,
                })
            }
            None => json!({
                "date": slug,
                "exists": false,
            }),
        };
        entries.push(value);
        cursor += Duration::days(1);
    }
    Ok(json!({ "journals": entries }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn journal_payload(
    ctx: &WsCtx,
    id: outl_core::id::NodeId,
    date: NaiveDate,
) -> Result<Value, ApiError> {
    let meta = page_meta(&ctx.workspace, id)
        .ok_or_else(|| ApiError::new(codes::INTERNAL, "journal meta missing".to_string()))?;
    let outline = project_outline(&ctx.workspace, id);
    let md = render_page_md(&ctx.workspace, id);
    Ok(json!({
        "date": journal_slug(date),
        "meta": meta,
        "outline": serde_json::to_value(&outline).map_err(ApiError::internal)?,
        "md": md,
    }))
}

/// Parse the variety of date forms we accept.
///
/// Keyword shortcuts (`today` / `yesterday` / `tomorrow`) are resolved
/// here; every literal spelling (ISO, `2026/04/22`, Roam's
/// `April 22nd, 2026`, …) is handled by the shared
/// [`outl_actions::parse_flexible_date`].
pub fn parse_date(s: &str) -> Result<NaiveDate, ApiError> {
    let trimmed = s.trim();
    let lower = trimmed.to_lowercase();
    match lower.as_str() {
        "today" => return Ok(today()),
        "yesterday" => return Ok(today() - Duration::days(1)),
        "tomorrow" => return Ok(today() + Duration::days(1)),
        _ => {}
    }
    if let Some(d) = outl_actions::parse_flexible_date(trimmed) {
        return Ok(d);
    }
    Err(ApiError::new(
        codes::INVALID_DATE,
        format!("could not parse date `{s}` — try YYYY-MM-DD"),
    ))
}

// ---------------------------------------------------------------------------
// Human formatters
// ---------------------------------------------------------------------------

fn print_journal(v: &Value) {
    let date = v.get("date").and_then(Value::as_str).unwrap_or("?");
    println!("{date}");
    if let Some(outline) = v.get("outline").and_then(Value::as_array) {
        print_outline_tree(outline, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_parses() {
        assert_eq!(
            parse_date("2026-05-31").unwrap(),
            NaiveDate::from_ymd_opt(2026, 5, 31).unwrap()
        );
    }

    #[test]
    fn keywords_parse() {
        let t = parse_date("today").unwrap();
        assert_eq!(t, today());
        let y = parse_date("yesterday").unwrap();
        assert_eq!(y, today() - Duration::days(1));
    }

    #[test]
    fn roam_form_parses() {
        let d = parse_date("April 22nd, 2026").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 4, 22).unwrap());
        let d = parse_date("October 3rd, 2025").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2025, 10, 3).unwrap());
        let d = parse_date("July 11th, 2026").unwrap();
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 7, 11).unwrap());
    }

    #[test]
    fn garbage_rejected() {
        assert!(parse_date("not a date").is_err());
    }
}
