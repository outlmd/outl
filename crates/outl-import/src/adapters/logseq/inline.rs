//! Logseq inline dialect scanner.
//!
//! Logseq markdown is CommonMark-flavoured, so — unlike Roam — there
//! are **no formatting swaps** (`__x__` already means bold, `==x==`
//! is already the highlight form). What needs translation or
//! resolution:
//!
//! - `((uuid))` → [`Inline::BlockRef`]
//! - `[alias](((uuid)))` → aliased [`Inline::BlockRef`]
//! - `{{embed ((uuid))}}` / `{{embed [[Page]]}}` → [`Inline::Embed`]
//!   (space-separated — Logseq's form, not Roam's colon form)
//! - `{{query …}}` (sexp DSL) and other `{{…}}` components →
//!   [`Inline::Component`] verbatim, counted
//! - `$$…$$` → LaTeX component, verbatim + counted
//! - `#[[Multi Word]]` → `[[Multi Word]]` (outl tags are single-token)
//! - `` `code` `` spans are consumed atomically so nothing rewrites
//!   inside them

use crate::adapters::scan::{alias_link, balanced, balanced_braces, is_uid};
use crate::ir::{ComponentKind, EmbedTarget, Inline};
use crate::report::ImportReport;

/// Tokenize one Logseq block text (task marker already stripped).
pub(crate) fn tokenize(text: &str, report: &mut ImportReport) -> Vec<Inline> {
    let mut out: Vec<Inline> = Vec::new();
    let mut buf = String::new();
    let mut i = 0usize;

    while i < text.len() {
        let rest = &text[i..];

        // `code` span — protected from everything below.
        if let Some(r) = rest.strip_prefix('`') {
            if let Some(close) = r.find('`') {
                flush(&mut buf, &mut out);
                out.push(Inline::CodeSpan(r[..close].to_string()));
                i += 1 + close + 1;
                continue;
            }
        }

        // {{…}} — embed / query / cloze / renderer / anything.
        if rest.starts_with("{{") {
            if let Some(len) = balanced_braces(rest) {
                component(&rest[..len], &mut buf, &mut out, report);
                i += len;
                continue;
            }
        }

        // $$latex$$ — verbatim + counted.
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

        // #[[Multi Word]] → [[Multi Word]].
        if rest.starts_with("#[[") {
            if let Some(len) = balanced(&rest[1..], "[[", "]]") {
                buf.push_str(&rest[1..1 + len]);
                report.tags_multiword_to_page_ref += 1;
                i += 1 + len;
                continue;
            }
        }

        // [[Page]] — passthrough, consumed atomically.
        if rest.starts_with("[[") {
            if let Some(len) = balanced(rest, "[[", "]]") {
                buf.push_str(&rest[..len]);
                i += len;
                continue;
            }
        }

        // ((uuid)) block ref.
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

        // [alias](target) — only a ((uuid)) target needs a token.
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

/// Classify one `{{…}}` component. Logseq's head is space-separated
/// (`{{embed ((uuid))}}`), not colon-separated like Roam's.
fn component(raw: &str, buf: &mut String, out: &mut Vec<Inline>, report: &mut ImportReport) {
    let inner = raw[2..raw.len() - 2].trim();
    let (head_raw, tail) = match inner.find(char::is_whitespace) {
        Some(p) => (&inner[..p], inner[p + 1..].trim()),
        None => (inner, ""),
    };
    let head = head_raw.to_ascii_lowercase();

    match head.as_str() {
        "embed" => {
            let block_uid = tail
                .strip_prefix("((")
                .and_then(|t| t.strip_suffix("))"))
                .map(str::trim)
                .filter(|u| is_uid(u));
            if let Some(uid) = block_uid {
                flush(buf, out);
                out.push(Inline::Embed(EmbedTarget::Block(uid.to_string())));
                return;
            }
            if let Some(name) = tail.strip_prefix("[[").and_then(|t| t.strip_suffix("]]")) {
                flush(buf, out);
                out.push(Inline::Embed(EmbedTarget::Page(name.to_string())));
                return;
            }
            report.count_component("embed-unparsed");
            push_component(buf, out, ComponentKind::Other, raw);
        }
        // Logseq queries are the sexp DSL — no safe translation to
        // outl's ` ```query ` directives, so they stay verbatim.
        "query" => {
            report.count_component("query");
            push_component(buf, out, ComponentKind::Query, raw);
        }
        other => {
            // Alias the video family so the report key stays stable
            // regardless of which spelling the source used.
            let other = if matches!(other, "youtube" | "twitter" | "tweet") {
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
