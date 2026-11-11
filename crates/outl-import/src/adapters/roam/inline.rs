//! Roam inline dialect scanner.
//!
//! Only constructs that need *translation or resolution* become
//! tokens; everything else — `[[page]]`, `#tag`, `**bold**`, links,
//! images — is already valid outl markdown and flows through as
//! [`Inline::Text`] verbatim (outl's own tokenizer picks it up after
//! emission).
//!
//! Transforms owned here (dialect knowledge):
//! - `((uid))` → [`Inline::BlockRef`]
//! - `[alias](((uid)))` → aliased [`Inline::BlockRef`]
//! - `{{[[embed]]: …}}` / `{{embed: …}}` → [`Inline::Embed`]
//! - `{{[[TODO]]}}` / `{{[[DONE]]}}` mid-text → kept verbatim, counted
//! - other `{{…}}` → [`Inline::Component`] verbatim, counted
//! - `$$…$$` → LaTeX component, verbatim + counted
//! - `__italic__` → `*italic*` (Roam `__` is italic; CommonMark bold)
//! - `^^highlight^^` → `==highlight==` (inert today; lights up if
//!   outl ever gains `==` highlight)
//! - `#[[Multi Word]]` → kept verbatim: outl reads it as `#` plus the
//!   `[[Multi Word]]` page ref, so the link survives and so does the text
//! - `` `code` `` spans are consumed atomically so none of the above
//!   rewrites fire inside them

use crate::adapters::scan::{alias_link, balanced, balanced_braces, is_uid, truncate};
use crate::ir::{ComponentKind, EmbedTarget, Inline};
use crate::report::ImportReport;

/// Tokenize one Roam block string (task marker already stripped).
pub(super) fn tokenize(text: &str, page: &str, report: &mut ImportReport) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut buf = String::new();
    let mut i = 0usize;

    while i < text.len() {
        let rest = &text[i..];

        // `code` span — protected from every rewrite below.
        if let Some(r) = rest.strip_prefix('`') {
            if let Some(close) = r.find('`') {
                flush(&mut buf, &mut out);
                out.push(Inline::CodeSpan(r[..close].to_string()));
                i += 1 + close + 1;
                continue;
            }
        }

        // {{…}} — embed / TODO / DONE / query / any component. Roam
        // queries nest SINGLE-brace clauses (`{and: …}`) inside the
        // double-brace component, so this counts individual braces.
        if rest.starts_with("{{") {
            if let Some(len) = balanced_braces(rest) {
                component(&rest[..len], page, &mut buf, &mut out, report);
                i += len;
                continue;
            }
        }

        // $$latex$$ — no outl equivalent, verbatim + counted.
        if let Some(r) = rest.strip_prefix("$$") {
            if let Some(close) = r.find("$$") {
                let raw = &rest[..2 + close + 2];
                flush(&mut buf, &mut out);
                report.count_component("latex");
                out.push(Inline::Component {
                    kind: ComponentKind::Other,
                    raw: raw.to_string(),
                });
                i += raw.len();
                continue;
            }
        }

        // ^^highlight^^ → ==highlight==.
        if let Some(r) = rest.strip_prefix("^^") {
            if let Some(close) = r.find("^^") {
                buf.push_str("==");
                buf.push_str(&r[..close]);
                buf.push_str("==");
                i += 2 + close + 2;
                continue;
            }
        }

        // __italic__ (Roam) → *italic* (CommonMark).
        if let Some(r) = rest.strip_prefix("__") {
            if let Some(close) = r.find("__") {
                buf.push('*');
                buf.push_str(&r[..close]);
                buf.push('*');
                i += 2 + close + 2;
                continue;
            }
        }

        // #[[Multi Word]]: kept as written. outl tags are single-token,
        // but its tokenizer reads this as `#` plus the `[[Multi Word]]`
        // page ref, so the link resolves without rewriting the user's
        // text. Consumed atomically like `[[Page]]` below.
        if rest.starts_with("#[[") {
            if let Some(len) = balanced(&rest[1..], "[[", "]]") {
                buf.push_str(&rest[..1 + len]);
                i += 1 + len;
                continue;
            }
        }

        // [[Page]] — passthrough, consumed atomically so an inner
        // `((` / `{{` never triggers the scanners on a partial slice.
        if rest.starts_with("[[") {
            if let Some(len) = balanced(rest, "[[", "]]") {
                buf.push_str(&rest[..len]);
                i += len;
                continue;
            }
        }

        // ((uid)) block ref.
        if let Some(r) = rest.strip_prefix("((") {
            if let Some(close) = r.find("))") {
                let uid = &r[..close];
                if is_uid(uid) {
                    flush(&mut buf, &mut out);
                    out.push(Inline::BlockRef {
                        uid: uid.to_string(),
                        alias: None,
                    });
                    i += 2 + close + 2;
                    continue;
                }
            }
        }

        // [alias](target) — only a ((uid)) target needs a token; links
        // and `[alias]([[Page]])` (issue #191) pass through verbatim.
        if rest.starts_with('[') && !rest.starts_with("[[") {
            if let Some((alias, target, len)) = alias_link(rest) {
                let inner = target.strip_prefix("((").and_then(|t| t.strip_suffix("))"));
                if let Some(uid) = inner.filter(|u| is_uid(u)) {
                    flush(&mut buf, &mut out);
                    out.push(Inline::BlockRef {
                        uid: uid.to_string(),
                        alias: Some(alias.to_string()),
                    });
                } else {
                    buf.push_str(&rest[..len]);
                }
                i += len;
                continue;
            }
        }

        let ch = rest.chars().next().expect("rest is non-empty");
        buf.push(ch);
        i += ch.len_utf8();
    }

    flush(&mut buf, &mut out);
    out
}

/// Flush the pending text run into the token list.
fn flush(buf: &mut String, out: &mut Vec<Inline>) {
    if !buf.is_empty() {
        out.push(Inline::Text(std::mem::take(buf)));
    }
}

/// Classify one `{{…}}` component (braces included in `raw`).
fn component(
    raw: &str,
    page: &str,
    buf: &mut String,
    out: &mut Vec<Inline>,
    report: &mut ImportReport,
) {
    let inner = raw[2..raw.len() - 2].trim();

    // Mid-text task markers stay exactly as written (start-of-block
    // markers are stripped upstream before tokenize runs).
    //
    // outl models one task per block, driven by the marker at the
    // block's head, so a marker buried mid-text can't become one without
    // moving the user's words around. Rewriting it to a bare `TODO` /
    // `DONE` word kept the word and threw away the markup that says it
    // was a checkbox, and with it the `[[TODO]]` / `[[DONE]]` page ref
    // Roam files the block under. Verbatim keeps both. Still counted,
    // because the block does not answer `outl query --kind=task`; the
    // aggregate warning is emitted once per import (a 10k-DONE graph
    // would otherwise produce 10k warnings).
    if matches!(inner, "[[TODO]]" | "TODO" | "[[DONE]]" | "DONE") {
        report.tasks_midtext_literal += 1;
        buf.push_str(raw);
        return;
    }

    let (head_raw, tail) = match inner.find(':') {
        Some(p) => (&inner[..p], Some(inner[p + 1..].trim())),
        None => (inner, None),
    };
    let head = head_raw
        .trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .trim()
        .to_ascii_lowercase();

    match head.as_str() {
        "embed" => {
            if let Some(t) = tail {
                let block_uid = t
                    .strip_prefix("((")
                    .and_then(|x| x.strip_suffix("))"))
                    .map(str::trim)
                    .filter(|u| is_uid(u));
                if let Some(uid) = block_uid {
                    flush(buf, out);
                    out.push(Inline::Embed(EmbedTarget::Block(uid.to_string())));
                    return;
                }
                let page_name = t.strip_prefix("[[").and_then(|x| x.strip_suffix("]]"));
                if let Some(name) = page_name {
                    flush(buf, out);
                    out.push(Inline::Embed(EmbedTarget::Page(name.to_string())));
                    return;
                }
            }
            report.count_component("embed-unparsed");
            report.warn(page, format!("embed kept verbatim: {}", truncate(raw, 60)));
            push_component(buf, out, ComponentKind::Other, raw);
        }
        // Query components are pushed silently — the block-level
        // conversion decides between ` ```query ` translation and
        // verbatim (and does the counting there).
        "query" => {
            flush(buf, out);
            out.push(Inline::Component {
                kind: ComponentKind::Query,
                raw: raw.to_string(),
            });
        }
        other => {
            // Alias the video family so the report key stays stable
            // regardless of which spelling the source used.
            let other = if matches!(other, "youtube") {
                "video"
            } else {
                other
            };
            let key = if !other.is_empty()
                && other.len() <= 24
                && other
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                other
            } else {
                "other"
            };
            report.count_component(key);
            push_component(buf, out, ComponentKind::Other, raw);
        }
    }
}

/// Push a verbatim component token.
fn push_component(buf: &mut String, out: &mut Vec<Inline>, kind: ComponentKind, raw: &str) {
    flush(buf, out);
    out.push(Inline::Component {
        kind,
        raw: raw.to_string(),
    });
}
