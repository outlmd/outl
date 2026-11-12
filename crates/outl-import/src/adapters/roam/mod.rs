//! Roam Research adapter (JSON backup format).
//!
//! Roam's "Export → JSON" produces an array of pages, each with a
//! recursive tree of blocks:
//!
//! ```jsonc
//! [
//!   {
//!     "title": "Avelino",
//!     "children": [
//!       {
//!         "string": "first block",
//!         "uid": "abcd",
//!         "children": [ { "string": "child", "uid": "efgh" } ]
//!       }
//!     ]
//!   }
//! ]
//! ```
//!
//! The adapter's whole job is JSON → typed IR. Inline dialect
//! translation lives in the `inline` submodule; everything downstream (placeholder
//! emission, handle resolution) is the shared emitter's business.

mod inline;
mod task;
#[cfg(test)]
mod tests;

use crate::adapter::{ImportError, SourceAdapter};
use crate::adapters::asset_scan::scan_assets;
use crate::adapters::scan::{parse_prop_line, parse_whole_fence};
use crate::ir::{
    AssetRef, BlockContent, ComponentKind, ImportBlock, ImportGraph, ImportPage, Inline, PageBody,
    PageName,
};
use crate::report::ImportReport;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use std::path::Path;
use task::split_task_marker;

/// Top-level Roam page from the JSON backup.
#[derive(Debug, Deserialize)]
struct RoamPage {
    title: String,
    #[serde(default)]
    children: Vec<RoamBlock>,
    /// Milliseconds since epoch. Roam stamps pages as well as blocks;
    /// dropping these lost the creation date of every page.
    #[serde(default, rename = "create-time")]
    create_time: Option<i64>,
    /// Milliseconds since epoch.
    #[serde(default, rename = "edit-time")]
    edit_time: Option<i64>,
}

/// One block in the Roam tree.
#[derive(Debug, Deserialize)]
struct RoamBlock {
    #[serde(default)]
    string: String,
    #[serde(default)]
    uid: String,
    #[serde(default)]
    children: Vec<RoamBlock>,
    /// `heading: 1..3` — Roam's per-block heading level.
    #[serde(default)]
    heading: Option<u8>,
    /// `open: false` marks a folded block (EDN exports always carry
    /// it; JSON backups sometimes do).
    #[serde(default)]
    open: Option<bool>,
    /// Milliseconds since epoch.
    #[serde(default, rename = "create-time")]
    create_time: Option<i64>,
    /// Milliseconds since epoch.
    #[serde(default, rename = "edit-time")]
    edit_time: Option<i64>,
}

/// The Roam adapter.
pub struct RoamAdapter;

impl SourceAdapter for RoamAdapter {
    fn id(&self) -> &'static str {
        "roam"
    }

    fn detect(src: &Path) -> bool {
        src.is_file() && src.extension().and_then(|e| e.to_str()) == Some("json")
    }

    fn parse(&self, src: &Path, report: &mut ImportReport) -> Result<ImportGraph, ImportError> {
        let text = std::fs::read_to_string(src).map_err(|e| ImportError::io(src, e))?;
        let mut pages: Vec<RoamPage> = serde_json::from_str(&text)
            .map_err(|e| ImportError::Parse(format!("roam backup JSON: {e}")))?;

        // Roam images are firebase-hosted URLs, so relative resolution
        // rarely fires; the backup file's dir is the base_dir anyway.
        let base_dir = src.parent().unwrap_or(src).to_path_buf();
        // Denominators for the report's reconciliation, counted off the
        // deserialized JSON — before any pipeline decision can hide a
        // loss from both the numerator and the denominator.
        report.source_pages += pages.len();
        report.source_blocks += pages
            .iter()
            .map(|p| count_blocks(&p.children))
            .sum::<usize>();

        // After the source counts: a folded table's cells are reported
        // as folded, not lost.
        table::fold_tables(&mut pages, report);

        let mut graph = ImportGraph::default();
        // Whether each emitted page is a journal NOT titled the way Roam
        // titles its own daily notes (`2024-03-19` next to `March 19th,
        // 2024`). Both land on one outl journal; Roam's own daily page
        // goes first so the merged page does not depend on the order the
        // backup happened to list them in.
        let mut secondary_journal: Vec<bool> = Vec::new();
        for (idx, page) in pages.iter().enumerate() {
            if page.title.trim().is_empty() {
                // Roam can emit empty pages on edge cases. Dropping the
                // page drops its whole subtree, so say so out loud
                // instead of `continue`-ing in silence.
                let dropped = count_blocks(&page.children);
                report.skip(
                    format!("{}#page[{idx}]", src.display()),
                    format!("page title is empty — {dropped} block(s) dropped with it"),
                    dropped,
                );
                continue;
            }
            let name = match outl_actions::parse_flexible_date(&page.title) {
                Some(d) => {
                    secondary_journal.push(page.title.trim() != roam_daily_title(d));
                    PageName::Journal(d)
                }
                None => {
                    secondary_journal.push(false);
                    PageName::Named(page.title.clone())
                }
            };
            let mut blocks: Vec<ImportBlock> = page
                .children
                .iter()
                .map(|b| convert_block(b, &page.title, &base_dir, &mut graph.assets, report))
                .collect();
            // Lift only the *leading* run of pure-attribute blocks
            // (`icon::`, `page-type::`, …) into the page header — that's how
            // Roam models page attributes, at the head of the page. A
            // pure-prop block that appears AFTER real content stays where the
            // user put it: promoting it would reorder the outline and pull
            // content out of place (the mid-page case the reviewer flagged).
            let mut props: Vec<(String, String)> = Vec::new();
            while blocks.first().is_some_and(is_pure_prop_block) {
                let mut b = blocks.remove(0);
                // `title::` is the emitter's own header line (it owns the page
                // name); a promoted `title::` attribute would emit a second,
                // conflicting one, so drop it here.
                b.props.retain(|(k, _)| k != "title");
                props.append(&mut b.props);
                // A source block that stops being a block. Counted so
                // the reconciliation closes on it instead of reading
                // the promotion as lost content.
                report.blocks_lifted_to_props += 1;
            }
            graph.pages.push(ImportPage {
                name,
                props,
                body: PageBody::Outline(blocks),
                stem_override: None,
                created: page.create_time.and_then(ms_to_datetime),
                edited: page.edit_time.and_then(ms_to_datetime),
            });
        }
        // Stable: only same-date journals change places relative to
        // each other, every other page keeps the backup's order.
        let mut keyed: Vec<(bool, ImportPage)> = secondary_journal
            .into_iter()
            .zip(graph.pages.drain(..))
            .collect();
        keyed.sort_by_key(|(secondary, _)| *secondary);
        graph.pages = keyed.into_iter().map(|(_, p)| p).collect();

        // One aggregate warning, never one per occurrence: a real graph
        // carries tens of thousands of `{{[[DONE]]}}` markers and a
        // per-hit warning would bury every other finding.
        let midtext = report.tasks_midtext_literal;
        if midtext > 0 {
            report.warn(
                GRAPH_SCOPE,
                format!(
                    "{midtext} mid-block TODO/DONE markers kept verbatim as text \
                     (task state not preserved: outl tracks one task per block)"
                ),
            );
        }
        Ok(graph)
    }
}

// `{{[[table]]}}` subtrees → one pipe-table block, run by `parse`.
mod table;

/// Warning scope for findings that belong to the whole import rather
/// than to one page.
const GRAPH_SCOPE: &str = "(graph)";

/// Total blocks in a Roam subtree, counted recursively — the source
/// side of the report's reconciliation.
fn count_blocks(blocks: &[RoamBlock]) -> usize {
    blocks
        .iter()
        .map(|b| 1 + count_blocks(&b.children))
        .sum::<usize>()
}

/// One Roam block → IR block (recursive).
fn convert_block(
    b: &RoamBlock,
    page: &str,
    base_dir: &Path,
    assets: &mut Vec<AssetRef>,
    report: &mut ImportReport,
) -> ImportBlock {
    let (task, after_task) = split_task_marker(&b.string);
    if let Some(state) = task {
        report.count_task(state.report_key());
    }
    let after_task = after_task.as_str();

    // A fenced block is opaque: its body legitimately carries `::`
    // (`foo::bar`, `use a::b`), so property extraction must skip it and
    // let `convert_content` own the fence. Everything else may hold
    // `key:: value` attribute lines to lift out of the text.
    let is_fence = parse_whole_fence(after_task).is_some() || after_task.contains("```");
    let (props, collapsed_prop, text) = if is_fence {
        (Vec::new(), false, after_task.to_string())
    } else {
        split_block_props(after_task)
    };

    ImportBlock {
        uid: (!b.uid.is_empty()).then(|| b.uid.clone()),
        content: convert_content(&text, page, base_dir, assets, report),
        children: b
            .children
            .iter()
            .map(|c| convert_block(c, page, base_dir, assets, report))
            .collect(),
        task,
        heading: b.heading.filter(|h| (1..=3).contains(h)),
        collapsed: b.open == Some(false) || collapsed_prop,
        props,
        created: b.create_time.and_then(ms_to_datetime),
        edited: b.edit_time.and_then(ms_to_datetime),
    }
}

/// Split a Roam block string into `key:: value` property lines and the
/// remaining prose. Roam attributes live *inside* `:block/string` —
/// often a whole block of them (`icon:: 🏢\npage-type:: company`) or
/// mixed with text (`[[2023-01-31]] #1on1\ncollapsed:: true`) — so a
/// property line can sit on any line of the block.
///
/// Two classes of line are handled differently:
///
/// - **Structural** (`collapsed::`) is *always* stripped from the text:
///   `collapsed:: true` flips the returned fold flag (its own IR field).
///   `id::` (Logseq residue: the block's ref identity is the Roam JSON
///   `uid`) is **not** structural here. It is in the Roam block's text,
///   Roam shows it as an attribute, and dropping it lost the only copy
///   of the id that pre-Roam links point at, so it stays like any other
///   user attribute.
/// - **User attributes** (`icon::`, `work::`, …) are lifted into `props`
///   **only when the block is nothing but attribute lines**. A block
///   that still has prose keeps its attribute lines *in the text*, in
///   the order the user wrote them: outl's renderer escapes a
///   `key:: value` continuation line, so it stays text, the way Roam
///   shows it, and a `((uid))` in it resolves like any other in the text
///   (the Omnivore `note:: ((uid))` shape). Roam's page-attribute blocks
///   (`icon::` at the head of a contact page) are pure-attribute, so this
///   still captures the case that matters.
fn split_block_props(raw: &str) -> (Vec<(String, String)>, bool, String) {
    // Fast path: no `::` at all → nothing to lift, keep the string byte
    // for byte (the common case, and it preserves exact whitespace).
    if !raw.contains("::") {
        return (Vec::new(), false, raw.to_string());
    }
    let mut collapsed = false;
    let mut user_props: Vec<(String, String)> = Vec::new();
    let mut kept: Vec<&str> = Vec::new(); // text + user-prop lines, in order
    let mut has_prose = false;
    for line in raw.split('\n') {
        match parse_prop_line(line.trim()) {
            Some((k, v)) if k == "collapsed" => collapsed |= v == "true",
            Some((k, v)) => {
                user_props.push((k, v));
                kept.push(line);
            }
            None => {
                if !line.trim().is_empty() {
                    has_prose = true;
                }
                kept.push(line);
            }
        }
    }
    if has_prose {
        // Prose present → leave user attributes in the text for outl's
        // parser to lift and resolve. Only the structural lines were
        // stripped from `kept`.
        (Vec::new(), collapsed, kept.join("\n"))
    } else {
        // Pure-attribute block → lift the props; the text is now empty.
        (user_props, collapsed, String::new())
    }
}

/// A top-level block that is *only* attribute lines — no prose, no
/// children, no task/heading. Roam models page attributes as such
/// blocks at the head of a page (`icon::`, `page-type::`, `related::`),
/// so they belong in the page's `title::`-style property header, not as
/// a stray empty bullet. Promoting them is what lets outl's index see
/// `page-type::` / `icon::` for the sidebar, type filter, and `@`
/// mention autocomplete.
fn is_pure_prop_block(b: &ImportBlock) -> bool {
    b.task.is_none()
        && b.heading.is_none()
        && b.children.is_empty()
        && !b.props.is_empty()
        && content_is_blank(&b.content)
}

/// True when the block carries no visible text — only whitespace runs.
fn content_is_blank(content: &BlockContent) -> bool {
    match content {
        BlockContent::Inline(toks) => toks
            .iter()
            .all(|t| matches!(t, Inline::Text(s) if s.trim().is_empty())),
        BlockContent::Code { .. } | BlockContent::Verbatim(_) => false,
    }
}

/// Roam block string → IR content.
fn convert_content(
    raw: &str,
    page: &str,
    base_dir: &Path,
    assets: &mut Vec<AssetRef>,
    report: &mut ImportReport,
) -> BlockContent {
    if let Some((lang, body)) = parse_whole_fence(raw) {
        return BlockContent::Code { lang, body };
    }
    if raw.contains("```") {
        // Mixed text + fence in one block string — preserve verbatim
        // rather than guessing at a split. Never drop content.
        return BlockContent::Verbatim(raw.to_string());
    }

    let pre = convert_org_dates(raw, report);
    let pre = pdf::link_pdf_components(&pre, report);
    // Pull image/file links out to placeholders before tokenizing; a
    // remote `![](https://…)` becomes an `AssetSource::Remote` ref.
    let pre = scan_assets(&pre, base_dir, assets);
    let toks = inline::tokenize(&pre, page, report);

    // A block that IS a single query component becomes a ` ```query `
    // fence when the Roam query translates; otherwise it stays a
    // verbatim component (counted here — tokenize pushes queries
    // silently so this decision point owns the counting).
    if let Some(raw_query) = whole_block_query(&toks) {
        if let Some(body) = translate_query(raw_query) {
            report.queries_translated += 1;
            return BlockContent::Code {
                lang: Some("query".to_string()),
                body,
            };
        }
    }
    for t in &toks {
        if matches!(
            t,
            Inline::Component {
                kind: ComponentKind::Query,
                ..
            }
        ) {
            report.count_component("query");
        }
    }

    BlockContent::Inline(toks)
}

// `{{[[pdf]]: url}}` → `[name](url)`, run by `convert_content`.
mod pdf;

/// `Some(raw)` when the token list is exactly one query component
/// (plus whitespace).
fn whole_block_query(toks: &[Inline]) -> Option<&str> {
    let mut query: Option<&str> = None;
    for t in toks {
        match t {
            Inline::Component {
                kind: ComponentKind::Query,
                raw,
            } if query.is_none() => query = Some(raw),
            Inline::Text(s) if s.trim().is_empty() => {}
            _ => return None,
        }
    }
    query
}

/// Best-effort `{{[[query]]: {and: …}}}` → ` ```query ` DSL body.
///
/// Only the flat `{and: [[ref]] …}` form translates — `[[TODO]]` /
/// `[[DONE]]` become `status:` directives, every other ref becomes a
/// `text:` substring match (which is exactly "blocks referencing this
/// page"). Nested clauses (`{or:}`, `{between:}`, `{not:}`) have no
/// DSL equivalent and fall back to the verbatim component.
fn translate_query(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix("{{")?.strip_suffix("}}")?;
    let tail = inner[inner.find(':')? + 1..].trim();
    let and_body = tail.strip_prefix("{and:")?.strip_suffix('}')?.trim();
    if and_body.contains('{') {
        return None; // Nested clause — not translatable.
    }

    let mut lines: Vec<String> = Vec::new();
    let mut rest = and_body;
    while let Some(open) = rest.find("[[") {
        let after = &rest[open + 2..];
        let close = after.find("]]")?;
        let name = after[..close].trim();
        match name {
            "TODO" => lines.push("status: todo".to_string()),
            "DONE" => lines.push("status: done".to_string()),
            _ => {
                if outl_actions::parse_flexible_date(name).is_some() {
                    // Absolute dates have no DSL equivalent (`since:`
                    // is relative-only) — don't mistranslate.
                    return None;
                }
                lines.push(format!("text: [[{name}]]"));
            }
        }
        rest = &after[close + 2..];
    }
    if lines.is_empty() {
        return None;
    }
    Some(lines.join("\n"))
}

/// `DEADLINE: <2025-11-12 Wed>` / `SCHEDULED: <…>` →
/// `DEADLINE: [[2025-11-12]]` / `SCHEDULED: [[…]]`.
///
/// Follows the issue #63 model: a date in the block text is a
/// `[[date]]` link, surfacing the block in that day's journal via
/// backlinks; notification scheduling is a separate opt-in property.
///
/// Only the `<…>` stamp is rewritten. The keyword stays (it is what
/// says the date is a deadline and not a mention), and so does anything
/// in the stamp past the date and weekday, such as a time or a repeater
/// (`<2025-11-12 Wed 10:00 +1w>` → `[[2025-11-12]] 10:00 +1w`). The
/// weekday alone is dropped: the date already names it.
fn convert_org_dates(raw: &str, report: &mut ImportReport) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    loop {
        let hit = ["DEADLINE:", "SCHEDULED:"]
            .iter()
            .filter_map(|kw| rest.find(kw).map(|p| (p, *kw)))
            .min_by_key(|(p, _)| *p);
        let Some((pos, kw)) = hit else {
            out.push_str(rest);
            return out;
        };
        let after_kw = &rest[pos + kw.len()..];
        let after_sp = after_kw.trim_start();
        let converted = after_sp
            .strip_prefix('<')
            .and_then(|inner| inner.find('>').map(|close| &inner[..close]))
            .and_then(|stamp| {
                let date_part = stamp.get(..10)?;
                let date = chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()?;
                Some((date, stamp[10..].trim()))
            });
        match converted {
            Some((date, tail)) => {
                // Keyword and the whitespace after it, as written.
                let ws = after_kw.len() - after_sp.len();
                out.push_str(&rest[..pos + kw.len() + ws]);
                out.push_str(&format!("[[{date}]]"));
                let extra = strip_weekday(tail, date);
                if !extra.is_empty() {
                    out.push(' ');
                    out.push_str(extra);
                }
                report.org_dates_converted += 1;
                // Skip: `<…>`.
                let angle_len = after_sp.find('>').expect("checked above") + 1;
                rest = &rest[pos + kw.len() + ws + angle_len..];
            }
            None => {
                out.push_str(&rest[..pos + kw.len()]);
                rest = &rest[pos + kw.len()..];
            }
        }
    }
}

/// The title Roam gives the daily note of `d` (`March 19th, 2024`).
fn roam_daily_title(d: chrono::NaiveDate) -> String {
    use chrono::Datelike;
    let day = d.day();
    let suffix = match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{} {day}{suffix}, {}", d.format("%B"), d.year())
}

/// Drop a leading weekday name (`Wed`, `Wednesday`, any case) from an
/// org stamp's tail, but only when it is the weekday of `date`: a
/// mismatched one is information, not redundancy.
fn strip_weekday(tail: &str, date: chrono::NaiveDate) -> &str {
    let word_end = tail
        .find(|c: char| !c.is_alphabetic())
        .unwrap_or(tail.len());
    let word = &tail[..word_end];
    let full = date.format("%A").to_string();
    let short = date.format("%a").to_string();
    if !word.is_empty() && (word.eq_ignore_ascii_case(&full) || word.eq_ignore_ascii_case(&short)) {
        tail[word_end..].trim_start()
    } else {
        tail
    }
}

/// Roam timestamps are milliseconds since epoch.
fn ms_to_datetime(ms: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single()
}
