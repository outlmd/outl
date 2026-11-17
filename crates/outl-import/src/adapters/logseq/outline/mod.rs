//! Logseq outline parsing: one `.md` file → page properties + a typed
//! block tree.
//!
//! Line-level constructs owned here:
//!
//! - leading `key:: value` lines and `#+KEY: value` directives →
//!   page properties (`#+title:` overrides the display title)
//! - free text before the first bullet → one block (Logseq's
//!   "pre-block"), never dropped
//! - `key:: value` under a bullet → block property
//!   (`id::` → UID, `collapsed:: true` → fold flag, `logseq.*` →
//!   dropped + counted); a `((uuid))` in a property *value* becomes a
//!   ref placeholder, so it resolves to a real handle like one in prose
//! - every Logseq task marker → [`TaskState`]; `[#A]` → `priority::`
//! - `SCHEDULED:`/`DEADLINE:` → a `[[YYYY-MM-DD]]` date link in the
//!   text **and** a `scheduled::` / `deadline::` property naming which
//!   one it was (issue #578)
//! - `:LOGBOOK: … :END:` drawers → dropped + counted
//! - `#+BEGIN_SRC`/`#+BEGIN_QUOTE` org blocks → a fenced code block /
//!   `> ` quote lines, the same shapes the `logseq-db` adapter
//!   produces for the same constructs

use super::inline;
use crate::adapters::asset_scan::scan_assets;
use crate::adapters::scan::{is_uid, parse_prop_line, parse_whole_fence};
use crate::ir::{placeholder_ref, AssetRef, BlockContent, ImportBlock, TaskState};
use crate::report::ImportReport;
use org::{apply_org_stamp, convert_org_blocks, is_org_block_line, org_stamp};
use std::path::Path;

/// One bullet mid-parse, before inline tokenization.
pub(super) struct RawBlock {
    level: usize,
    pub(super) text_lines: Vec<String>,
    uid: Option<String>,
    collapsed: bool,
    pub(super) props: Vec<(String, String)>,
    task: Option<TaskState>,
    children: Vec<ImportBlock>,
}

/// Parse a whole file into page properties + an outline tree.
pub(super) fn parse_outline(
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
            // `#+BEGIN_QUOTE` and friends are content, not a directive:
            // they open an org block, which the pre-block below keeps.
            if !is_org_block_line(trimmed) {
                if let Some((k, v)) = directive.split_once(':') {
                    page_props.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
                    report.artifacts_stripped += 1;
                    idx += 1;
                    continue;
                }
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

    // 2. Free text between the properties and the first bullet.
    //    Logseq shows this as a block (its "pre-block"); the parser used
    //    to `continue` past every one of those lines, so a file written
    //    or edited outside Logseq lost its text with nothing in the
    //    report to show for it.
    let mut roots: Vec<ImportBlock> = Vec::new();
    let mut pre: Vec<String> = Vec::new();
    while idx < lines.len() {
        let trimmed = lines[idx].trim_start();
        if bullet_content(trimmed).is_some() {
            break;
        }
        pre.push(lines[idx].to_string());
        idx += 1;
    }
    if pre.iter().any(|l| !l.trim().is_empty()) {
        let raw = RawBlock {
            level: 0,
            text_lines: trim_blank_edges(pre),
            uid: None,
            collapsed: false,
            props: Vec::new(),
            task: None,
            children: Vec::new(),
        };
        roots.push(finalize(raw, base_dir, assets, report));
        report.blocks_from_pre_text += 1;
    }

    // 3. Bullets. Tabs count one level each; space runs divide by the
    //    smallest indent step observed in the file (Logseq default 2).
    let space_unit = detect_space_unit(&lines[idx..]);
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

        // Continuation line for the innermost open block. Nothing can
        // reach here with an empty stack: step 2 consumed every line
        // before the first bullet.
        let Some(top) = stack.last_mut() else {
            continue;
        };
        if trimmed == ":LOGBOOK:" {
            in_logbook = true;
            report.artifacts_stripped += 1;
            continue;
        }
        if let Some(stamp) = org_stamp(trimmed) {
            apply_org_stamp(top, stamp, report);
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
                _ => top.props.push((k, refs_in_prop_value(&v, report))),
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

/// Blocks in a subtree, root included — the adapter's half of the
/// reconciliation denominator.
pub(super) fn count_blocks(b: &ImportBlock) -> usize {
    1 + b.children.iter().map(count_blocks).sum::<usize>()
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
    let joined = raw.text_lines.join("\n");
    // Org blocks only translate outside a markdown fence: inside one,
    // `#+BEGIN_SRC` is the code someone is quoting.
    let text = if joined.contains("```") {
        joined
    } else {
        convert_org_blocks(&joined, report)
    };
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

/// Drop leading and trailing blank lines, keeping the ones inside.
fn trim_blank_edges(mut lines: Vec<String>) -> Vec<String> {
    while lines.first().is_some_and(|l| l.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    lines
}

/// Every Logseq task marker, longest spelling of a family first.
///
/// `WAIT`, `IN-PROGRESS` and `STARTED` are the ones the first version
/// missed: Logseq accepts all three (`frontend.util.marker`), and a
/// block starting with one used to keep the word as literal text and
/// import as a plain, untracked block — the task state gone, and not
/// even counted.
const TASK_MARKERS: &[(&str, TaskState)] = &[
    ("TODO", TaskState::Todo),
    ("DOING", TaskState::Doing),
    ("IN-PROGRESS", TaskState::Doing),
    ("STARTED", TaskState::Doing),
    ("NOW", TaskState::Now),
    ("LATER", TaskState::Later),
    ("WAITING", TaskState::Waiting),
    ("WAIT", TaskState::Waiting),
    ("DONE", TaskState::Done),
    ("CANCELLED", TaskState::Canceled),
    ("CANCELED", TaskState::Canceled),
];

/// Leading Logseq task keyword → state + remaining text.
fn split_task(content: &str) -> (Option<TaskState>, &str) {
    for (kw, state) in TASK_MARKERS {
        if let Some(rest) = content.strip_prefix(kw) {
            if rest.is_empty() {
                return (Some(*state), "");
            }
            if let Some(r) = rest.strip_prefix(' ') {
                return (Some(*state), r);
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

/// A `((uuid))` inside a property value → a ref placeholder.
///
/// A property value never reaches the inline tokenizer (it is a
/// property, not prose), so a `related:: ((uuid))` used to be written
/// out verbatim and stayed inert text forever: the uid is not one of
/// outl's handles. The resolve pass's file-fallback sweep already knows
/// how to rewrite a marker that landed outside block text, so emitting
/// the placeholder here is all it takes to make the ref real.
fn refs_in_prop_value(value: &str, report: &mut ImportReport) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    let mut found = false;
    while let Some(open) = rest.find("((") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("))") else {
            break;
        };
        let uid = &after[..close];
        out.push_str(&rest[..open]);
        if is_uid(uid) {
            out.push_str(&placeholder_ref(uid));
            found = true;
        } else {
            out.push_str(&rest[open..open + 2 + close + 2]);
        }
        rest = &after[close + 2..];
    }
    if !found {
        return value.to_string();
    }
    report.refs_in_prop_values += 1;
    out.push_str(rest);
    out
}

/// True when a line both opens and closes a fence (` ```x``` `).
fn fence_closes_inline(s: &str) -> bool {
    let t = s.trim_start();
    t.len() > 3 && t.ends_with("```") && t.starts_with("```")
}

mod org;
#[cfg(test)]
mod tests;
