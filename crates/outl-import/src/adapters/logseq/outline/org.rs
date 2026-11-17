//! The org-mode constructs a Logseq markdown graph still carries.
//!
//! Two unrelated things, both spelled with org syntax, both of which
//! used to reach outl as literal text:
//!
//! - `SCHEDULED:` / `DEADLINE:` timestamps, whose **kind** was dropped
//!   (issue #578) along with the time of day and any repeater;
//! - `#+BEGIN_SRC` / `#+BEGIN_QUOTE` blocks, which the `logseq-db`
//!   adapter already turns into a fence and `> ` lines — so leaving
//!   them as text here meant one app imported two ways.

use crate::report::ImportReport;

/// Which org timestamp a line carries. Logseq draws the two
/// differently and queries them separately, so collapsing both into a
/// bare date link lost the only thing that distinguished them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StampKind {
    Scheduled,
    Deadline,
}

impl StampKind {
    /// Property key this stamp lands under.
    fn key(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Deadline => "deadline",
        }
    }
}

/// One parsed org timestamp.
pub(super) struct OrgStamp {
    kind: StampKind,
    date: chrono::NaiveDate,
    /// Time of day and/or repeater, as the source wrote it (`09:30`,
    /// `.+1w`) — empty when the stamp was a bare date.
    detail: String,
}

/// `SCHEDULED: <2026-11-12 Thu 09:30 .+1w>` / `DEADLINE: <…>`.
pub(super) fn org_stamp(trimmed: &str) -> Option<OrgStamp> {
    let (kind, rest) = if let Some(r) = trimmed.strip_prefix("SCHEDULED:") {
        (StampKind::Scheduled, r)
    } else {
        (StampKind::Deadline, trimmed.strip_prefix("DEADLINE:")?)
    };
    let inner = rest.trim_start().strip_prefix('<')?;
    let stamp = &inner[..inner.find('>')?];
    let date = chrono::NaiveDate::parse_from_str(stamp.get(..10)?, "%Y-%m-%d").ok()?;
    // Everything after the date, minus the day name org writes for the
    // reader's benefit (it is already implied by the date).
    let after = stamp[10..].trim();
    let mut tokens = after.split_whitespace();
    let detail = match tokens.next() {
        Some(first) if is_day_name(first) => tokens.collect::<Vec<_>>().join(" "),
        _ => after.to_string(),
    };
    Some(OrgStamp { kind, date, detail })
}

/// `Thu`, `Mon`… — org's reader-facing day name.
fn is_day_name(s: &str) -> bool {
    s.len() == 3 && s.chars().all(|c| c.is_ascii_alphabetic())
}

/// Record a stamp on the open block: the date link stays in the text
/// (issue #63 — a date in text surfaces in that day's journal through
/// backlinks) and the kind lands as a property (issue #578).
pub(super) fn apply_org_stamp(
    top: &mut super::RawBlock,
    stamp: OrgStamp,
    report: &mut ImportReport,
) {
    let link = format!("[[{}]]", stamp.date);
    let first = &mut top.text_lines[0];
    if !first.is_empty() {
        first.push(' ');
    }
    first.push_str(&link);

    let value = if stamp.detail.is_empty() {
        link
    } else {
        format!("{link} {}", stamp.detail)
    };
    let key = stamp.kind.key();
    // Two stamps of one kind on one block would be two properties with
    // the same key, and the op log keeps one value per key — the second
    // would silently win. Joining keeps both readable.
    match top.props.iter_mut().find(|(k, _)| k == key) {
        Some((_, existing)) => {
            existing.push_str(", ");
            existing.push_str(&value);
        }
        None => top.props.push((key.to_string(), value)),
    }
    report.org_dates_converted += 1;
}

/// `#+BEGIN_…` / `#+END_…`, the org block syntax Logseq still accepts
/// in a markdown graph.
pub(super) fn is_org_block_line(trimmed: &str) -> bool {
    let upper = trimmed.to_ascii_uppercase();
    upper.starts_with("#+BEGIN_") || upper.starts_with("#+END_")
}

/// Translate org blocks into the markdown shapes outl renders.
///
/// Logseq accepts both syntaxes in a markdown graph, and a graph that
/// came through org-mode (or used the `<` template helper) is full of
/// `#+BEGIN_SRC`. Those used to import as literal text, which is also
/// what made them **diverge from the `logseq-db` adapter**, where the
/// same constructs already become a fence and `> ` lines.
pub(super) fn convert_org_blocks(text: &str, report: &mut ImportReport) -> String {
    if !text.to_ascii_uppercase().contains("#+BEGIN_") {
        return text.to_string();
    }
    let mut out: Vec<String> = Vec::new();
    let mut quote_depth = 0usize;
    let mut converted = 0usize;
    for line in text.lines() {
        let trimmed = line.trim();
        let upper = trimmed.to_ascii_uppercase();
        if let Some(rest) = upper.strip_prefix("#+BEGIN_") {
            let kind = rest.split_whitespace().next().unwrap_or("");
            match kind {
                "QUOTE" => {
                    quote_depth += 1;
                    converted += 1;
                    continue;
                }
                "SRC" => {
                    // `#+BEGIN_SRC clojure` → ```clojure
                    let lang = trimmed
                        .split_whitespace()
                        .nth(1)
                        .map(str::to_string)
                        .unwrap_or_default();
                    out.push(format!("```{lang}"));
                    converted += 1;
                    continue;
                }
                // EXAMPLE / VERSE / EXPORT / COMMENT: preformatted text
                // with no language.
                _ => {
                    out.push("```".to_string());
                    converted += 1;
                    continue;
                }
            }
        }
        if let Some(rest) = upper.strip_prefix("#+END_") {
            let kind = rest.split_whitespace().next().unwrap_or("");
            if kind == "QUOTE" {
                quote_depth = quote_depth.saturating_sub(1);
            } else {
                out.push("```".to_string());
            }
            continue;
        }
        if quote_depth > 0 {
            out.push(format!("> {trimmed}"));
        } else {
            out.push(line.to_string());
        }
    }
    report.org_blocks_converted += converted;
    out.join("\n")
}
