//! Logseq adapter (markdown graph directory).
//!
//! A Logseq graph is `pages/` + `journals/` of bullet-outlined
//! markdown. Unlike the old string pipeline, this adapter **parses
//! the outline** — every bullet becomes a typed [`ImportBlock`], so
//! `id::` UIDs map to real `((blk-XXXXXX))` handles, `collapsed::`
//! survives as `Op::SetCollapsed`, and per-block properties land as
//! outl property lines instead of stray text.
//!
//! Line-level constructs owned here:
//!
//! - `key:: value` under a bullet → block property
//!   (`id::` → UID, `collapsed:: true` → fold flag, `logseq.*` →
//!   dropped + counted)
//! - `TODO`/`DOING`/`NOW`/`LATER`/`WAITING`/`DONE`/`CANCELED` →
//!   [`TaskState`]; `[#A]` priority → `priority::` property
//! - `SCHEDULED:`/`DEADLINE:` lines → `[[YYYY-MM-DD]]` date link
//!   appended to the block text (issue #63 model)
//! - `:LOGBOOK: … :END:` drawers → dropped + counted
//! - leading `key:: value` lines and `#+KEY: value` directives →
//!   page properties (`#+title:` overrides the display title)
//! - filename decoding: `%2F` → `/`, `___` → space; journal stems
//!   `2026_05_25` → ISO date

pub(crate) mod inline;
#[cfg(test)]
mod tests;

use crate::adapter::{ImportError, SourceAdapter};
use crate::adapters::asset_scan::scan_assets;
use crate::adapters::scan::{parse_prop_line, parse_whole_fence};
use crate::ir::{
    AssetRef, BlockContent, ImportBlock, ImportGraph, ImportPage, PageBody, PageName, TaskState,
};
use crate::report::ImportReport;
use outl_md::slug::slugify;
use std::path::{Path, PathBuf};

/// The Logseq adapter.
pub struct LogseqAdapter;

impl SourceAdapter for LogseqAdapter {
    fn id(&self) -> &'static str {
        "logseq"
    }

    fn detect(src: &Path) -> bool {
        src.is_dir()
            && (src.join("logseq").is_dir()
                || (src.join("pages").is_dir() && src.join("journals").is_dir()))
    }

    fn parse(&self, src: &Path, report: &mut ImportReport) -> Result<ImportGraph, ImportError> {
        if !src.is_dir() {
            return Err(ImportError::Parse(format!(
                "logseq source must be the graph directory (got {})",
                src.display()
            )));
        }
        // A DB-version graph holds db.sqlite and none of pages/ + journals/;
        // scanning it as a file graph would report 0 pages with no error.
        if super::logseq_db::is_db_graph_dir(src) {
            return Err(ImportError::Parse(super::logseq_db::db_graph_dir_message(
                src,
            )));
        }

        let mut graph = ImportGraph::default();
        for (dir, is_journal) in [("pages", false), ("journals", true)] {
            let d = src.join(dir);
            if !d.is_dir() {
                continue;
            }
            let mut files = md_files_shallow(&d, report);
            files.sort();
            for f in &files {
                convert_file(f, is_journal, &mut graph, report)?;
            }
        }

        // Assets referenced from blocks (`![](../assets/x.png)`) are
        // scanned during `parse_outline` and copied into the workspace's
        // `assets/` dir by the emit stage — no skip warning needed.
        Ok(graph)
    }
}

/// Depth-1 `.md` listing; `.org` files are counted as skipped.
fn md_files_shallow(dir: &Path, report: &mut ImportReport) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match path.extension().and_then(|x| x.to_str()) {
            Some("md") => out.push(path),
            Some("org") => report.skip(
                path.display().to_string(),
                "org-mode files are not supported — convert the graph to markdown \
                 in Logseq first",
                0,
            ),
            _ => {}
        }
    }
    out
}

/// One Logseq `.md` file → one [`ImportPage`].
fn convert_file(
    src: &Path,
    is_journal: bool,
    graph: &mut ImportGraph,
    report: &mut ImportReport,
) -> Result<(), ImportError> {
    let text = std::fs::read_to_string(src).map_err(|e| ImportError::io(src, e))?;
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let decoded = decode_page_name(stem);

    let label = decoded.clone();
    // Relative asset links (`![](../assets/x.png)`) resolve against the
    // `.md` file's own directory (`pages/` or `journals/`), so
    // `../assets/x.png` lands on `<graph>/assets/x.png`.
    let base_dir = src.parent().unwrap_or(src).to_path_buf();
    let (mut props, blocks) = parse_outline(&text, &base_dir, &mut graph.assets, report);

    // `title::` / `#+title:` overrides the display name; the file
    // keeps its filename-derived slug so existing `[[refs]]` resolve.
    let mut stem_override = None;
    let title_prop = props
        .iter()
        .position(|(k, _)| k == "title")
        .map(|i| props.remove(i).1);

    let name = if is_journal {
        match outl_actions::parse_flexible_date(&stem.replace('_', "-")) {
            Some(d) => PageName::Journal(d),
            None => {
                report.warn(
                    &label,
                    "journal filename doesn't parse as a date — imported as a regular page",
                );
                PageName::Named(decoded)
            }
        }
    } else {
        match title_prop {
            Some(t) if t != decoded => {
                stem_override = Some(slugify(&decoded));
                PageName::Named(t)
            }
            _ => PageName::Named(decoded),
        }
    };

    graph.pages.push(ImportPage {
        name,
        props,
        body: PageBody::Outline(blocks),
        stem_override,
        created: None,
        edited: None,
        source_uid: None,
    });
    Ok(())
}

/// Decode Logseq's filename encoding: `%2F` → `/`, `___` → space.
fn decode_page_name(stem: &str) -> String {
    stem.replace("%2F", "/").replace("___", " ")
}

// --- outline parsing -------------------------------------------------------

/// One bullet mid-parse, before inline tokenization.
struct RawBlock {
    level: usize,
    text_lines: Vec<String>,
    uid: Option<String>,
    collapsed: bool,
    props: Vec<(String, String)>,
    task: Option<TaskState>,
    children: Vec<ImportBlock>,
}

/// Parse a whole file into page properties + an outline tree.
///
/// `base_dir` / `assets` thread through to [`finalize`], where each
/// block's text is scanned for asset links before inline tokenization.
fn parse_outline(
    text: &str,
    base_dir: &Path,
    assets: &mut Vec<AssetRef>,
    report: &mut ImportReport,
) -> (Vec<(String, String)>, Vec<ImportBlock>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut idx = 0usize;

    // 1. Page-property region: leading `key:: value` lines and `#+`
    //    directives before the first bullet.
    let mut page_props: Vec<(String, String)> = Vec::new();
    while idx < lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() {
            idx += 1;
            continue;
        }
        if let Some(directive) = trimmed.strip_prefix("#+") {
            if let Some((k, v)) = directive.split_once(':') {
                page_props.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                report.artifacts_stripped += 1;
                idx += 1;
                continue;
            }
        }
        if !trimmed.starts_with("- ") && trimmed != "-" {
            if let Some((k, v)) = parse_prop_line(trimmed) {
                if k == "id" {
                    report.artifacts_stripped += 1;
                } else {
                    page_props.push((k, v));
                }
                idx += 1;
                continue;
            }
        }
        break;
    }

    // 2. Bullets. Tabs count one level each; space runs divide by the
    //    smallest indent step observed in the file (Logseq default 2).
    let space_unit = detect_space_unit(&lines[idx..]);
    let mut roots: Vec<ImportBlock> = Vec::new();
    let mut stack: Vec<RawBlock> = Vec::new();
    let mut in_fence = false;
    let mut in_logbook = false;

    while idx < lines.len() {
        let line = lines[idx];
        idx += 1;
        let trimmed = line.trim_start();

        // Fence bodies swallow every classification below.
        if in_fence {
            if let Some(top) = stack.last_mut() {
                top.text_lines
                    .push(strip_indent(line, top.level + 1, space_unit));
            }
            if trimmed.starts_with("```") {
                in_fence = false;
            }
            continue;
        }
        if in_logbook {
            if trimmed == ":END:" {
                in_logbook = false;
            }
            continue;
        }

        if let Some(content) = bullet_content(trimmed) {
            let level = indent_level(line, space_unit);
            close_to(&mut stack, &mut roots, level, base_dir, assets, report);

            let (task, rest) = split_task(content);
            if let Some(state) = task {
                report.count_task(state.report_key());
            }
            let (priority, rest) = split_priority(rest);
            let mut block = RawBlock {
                level,
                text_lines: vec![rest.to_string()],
                uid: None,
                collapsed: false,
                props: Vec::new(),
                task,
                children: Vec::new(),
            };
            if let Some(p) = priority {
                block.props.push(("priority".to_string(), p.to_string()));
            }
            if rest.trim_start().starts_with("```") && !fence_closes_inline(rest) {
                in_fence = true;
            }
            stack.push(block);
            continue;
        }

        // Continuation line for the innermost open block.
        let Some(top) = stack.last_mut() else {
            continue; // Stray text before the first bullet — page-prop pass ended.
        };
        if trimmed == ":LOGBOOK:" {
            in_logbook = true;
            report.artifacts_stripped += 1;
            continue;
        }
        if let Some(date) = org_stamp_date(trimmed) {
            // SCHEDULED:/DEADLINE: line → [[date]] appended to the
            // block text (issue #63: dates in text surface in the
            // daily via backlinks).
            let first = &mut top.text_lines[0];
            first.push(' ');
            first.push_str(&format!("[[{date}]]"));
            report.org_dates_converted += 1;
            continue;
        }
        if let Some((k, v)) = parse_prop_line(trimmed) {
            match k.as_str() {
                "id" => {
                    top.uid = Some(v);
                    report.artifacts_stripped += 1;
                }
                "collapsed" if v == "true" => top.collapsed = true,
                "collapsed" => {}
                _ if k.starts_with("logseq.") => report.artifacts_stripped += 1,
                _ => top.props.push((k, v)),
            }
            continue;
        }
        if trimmed.starts_with("```") {
            in_fence = !fence_closes_inline(trimmed);
            top.text_lines
                .push(strip_indent(line, top.level + 1, space_unit));
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        top.text_lines
            .push(strip_indent(line, top.level + 1, space_unit));
    }

    close_to(&mut stack, &mut roots, 0, base_dir, assets, report);
    (page_props, roots)
}

/// Pop every open block at `level` or deeper, finalizing each into
/// its parent (or the root list).
fn close_to(
    stack: &mut Vec<RawBlock>,
    roots: &mut Vec<ImportBlock>,
    level: usize,
    base_dir: &Path,
    assets: &mut Vec<AssetRef>,
    report: &mut ImportReport,
) {
    while stack.last().is_some_and(|b| b.level >= level) {
        let raw = stack.pop().expect("checked by is_some_and");
        let block = finalize(raw, base_dir, assets, report);
        match stack.last_mut() {
            Some(parent) => parent.children.push(block),
            None => roots.push(block),
        }
    }
}

/// Raw text lines → typed content + assembled [`ImportBlock`].
fn finalize(
    raw: RawBlock,
    base_dir: &Path,
    assets: &mut Vec<AssetRef>,
    report: &mut ImportReport,
) -> ImportBlock {
    let text = raw.text_lines.join("\n");
    let content = if let Some((lang, body)) = parse_whole_fence(&text) {
        BlockContent::Code { lang, body }
    } else if text.contains("```") {
        BlockContent::Verbatim(text)
    } else {
        // Scan for asset links first so the emitted placeholder survives
        // tokenization as inert text; a fenced block is skipped above so
        // a `[x](y)` inside code is never mistaken for a file link.
        let scanned = scan_assets(&text, base_dir, assets);
        BlockContent::Inline(inline::tokenize(&scanned, report))
    };
    ImportBlock {
        uid: raw.uid,
        content,
        children: raw.children,
        task: raw.task,
        heading: None,
        collapsed: raw.collapsed,
        props: raw.props,
        created: None,
        edited: None,
    }
}

// --- line classification ---------------------------------------------------

/// `- content` / bare `-` → the content after the marker.
fn bullet_content(trimmed: &str) -> Option<&str> {
    if let Some(rest) = trimmed.strip_prefix("- ") {
        return Some(rest);
    }
    (trimmed == "-").then_some("")
}

/// Leading whitespace → indent level. Tabs are one level each; space
/// runs divide by `space_unit`.
fn indent_level(line: &str, space_unit: usize) -> usize {
    let mut tabs = 0usize;
    let mut spaces = 0usize;
    for c in line.chars() {
        match c {
            '\t' => tabs += 1,
            ' ' => spaces += 1,
            _ => break,
        }
    }
    tabs + spaces / space_unit
}

/// Smallest nonzero space-indent step across bullet lines (tab-only
/// files fall back to 2, which is then never consulted).
fn detect_space_unit(lines: &[&str]) -> usize {
    lines
        .iter()
        .filter(|l| l.trim_start().starts_with("- "))
        .map(|l| l.chars().take_while(|c| *c == ' ').count())
        .filter(|n| *n > 0)
        .min()
        .unwrap_or(2)
        .max(1)
}

/// Drop up to `levels` of indentation from a continuation line.
fn strip_indent(line: &str, levels: usize, space_unit: usize) -> String {
    let mut budget = levels * space_unit;
    let mut idx = 0usize;
    for c in line.chars() {
        let cost = match c {
            '\t' => space_unit,
            ' ' => 1,
            _ => break,
        };
        if budget < cost {
            break;
        }
        budget -= cost;
        idx += c.len_utf8();
    }
    line[idx..].to_string()
}

/// Leading Logseq task keyword → state + remaining text.
fn split_task(content: &str) -> (Option<TaskState>, &str) {
    for (kw, state) in [
        ("TODO", TaskState::Todo),
        ("DOING", TaskState::Doing),
        ("NOW", TaskState::Now),
        ("LATER", TaskState::Later),
        ("WAITING", TaskState::Waiting),
        ("DONE", TaskState::Done),
        ("CANCELLED", TaskState::Canceled),
        ("CANCELED", TaskState::Canceled),
    ] {
        if let Some(rest) = content.strip_prefix(kw) {
            if rest.is_empty() {
                return (Some(state), "");
            }
            if let Some(r) = rest.strip_prefix(' ') {
                return (Some(state), r);
            }
        }
    }
    (None, content)
}

/// Leading `[#A] ` priority marker → letter + remaining text.
fn split_priority(content: &str) -> (Option<char>, &str) {
    let Some(rest) = content.strip_prefix("[#") else {
        return (None, content);
    };
    let mut chars = rest.chars();
    let letter = chars.next();
    if let (Some(l), Some(']')) = (letter, chars.next()) {
        if l.is_ascii_uppercase() {
            let after = &rest[2..];
            return (Some(l), after.strip_prefix(' ').unwrap_or(after));
        }
    }
    (None, content)
}

/// `SCHEDULED: <2026-11-12 Thu …>` / `DEADLINE: <…>` → the ISO date.
fn org_stamp_date(trimmed: &str) -> Option<chrono::NaiveDate> {
    let rest = trimmed
        .strip_prefix("SCHEDULED:")
        .or_else(|| trimmed.strip_prefix("DEADLINE:"))?;
    let inner = rest.trim_start().strip_prefix('<')?;
    let stamp = &inner[..inner.find('>')?];
    chrono::NaiveDate::parse_from_str(stamp.get(..10)?, "%Y-%m-%d").ok()
}

/// True when a line both opens and closes a fence (` ```x``` `).
fn fence_closes_inline(s: &str) -> bool {
    let t = s.trim_start();
    t.len() > 3 && t.ends_with("```") && t.starts_with("```")
}
