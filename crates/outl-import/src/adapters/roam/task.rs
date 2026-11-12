//! Roam task markers at the head of a block.

use crate::ir::TaskState;

/// Strip the block's task marker: a leading `{{[[TODO]]}}` /
/// `{{[[DONE]]}}`, or one that sits right after a list or quote prefix
/// at the head of the block (`- {{[[TODO]]}} x`, `1. {{[[TODO]]}} x`,
/// `> {{[[TODO]]}} x`).
///
/// Roam renders the marker as the block's checkbox wherever it sits, so
/// a block pasted as a markdown list item is still a task there. The
/// prefix stays in the text as written, so `- {{[[TODO]]}} x` becomes a
/// `TODO` block reading `- x`, and a quote lands in outl's canonical
/// `TODO > x` order. A marker any later in the text stays verbatim (see
/// `inline::component`).
pub(super) fn split_task_marker(raw: &str) -> (Option<TaskState>, String) {
    let trimmed = raw.trim_start();
    if let Some((state, rest)) = strip_marker(trimmed) {
        return (Some(state), rest.trim_start().to_string());
    }
    let prefix_len = list_or_quote_prefix(trimmed);
    if prefix_len > 0 {
        let (prefix, after) = trimmed.split_at(prefix_len);
        if let Some((state, rest)) = strip_marker(after) {
            return (Some(state), format!("{prefix}{}", rest.trim_start()));
        }
    }
    (None, raw.to_string())
}

/// `Some((state, rest))` when `s` opens with a task marker.
fn strip_marker(s: &str) -> Option<(TaskState, &str)> {
    [
        ("{{[[TODO]]}}", TaskState::Todo),
        ("{{TODO}}", TaskState::Todo),
        ("{{[[DONE]]}}", TaskState::Done),
        ("{{DONE}}", TaskState::Done),
    ]
    .into_iter()
    .find_map(|(marker, state)| s.strip_prefix(marker).map(|rest| (state, rest)))
}

/// Byte length of a markdown list or quote prefix at the head of `s`,
/// trailing space included: `- `, `* `, `+ `, `> `, `1. ` or `1) `.
/// `0` when there is none.
fn list_or_quote_prefix(s: &str) -> usize {
    for p in ["- ", "* ", "+ ", "> "] {
        if s.starts_with(p) {
            return p.len();
        }
    }
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let rest = &s[digits..];
    if (1..=9).contains(&digits) && (rest.starts_with(". ") || rest.starts_with(") ")) {
        return digits + 2;
    }
    0
}
