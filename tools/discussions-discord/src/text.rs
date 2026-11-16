//! Discord's length limits, counted the way Discord counts.

pub const CONTENT_LIMIT: usize = 2000;
pub const THREAD_NAME_LIMIT: usize = 100;
pub const ELLIPSIS: char = '…';

/// Length in UTF-16 code units, so an emoji outside the BMP is 2.
pub fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Cut `text` to at most `limit` UTF-16 units, ending in an ellipsis.
pub fn truncate(text: &str, limit: usize) -> String {
    if units(text) <= limit {
        return text.to_string();
    }
    if limit == 0 {
        return String::new();
    }
    // One unit is kept for the ellipsis (it is in the BMP, so exactly 1).
    let mut used = 0;
    let mut end = 0;
    for (idx, ch) in text.char_indices() {
        let width = ch.len_utf16();
        if used + width + 1 > limit {
            break;
        }
        used += width;
        end = idx + ch.len_utf8();
    }
    format!("{}{ELLIPSIS}", text[..end].trim_end())
}

pub fn thread_name(number: u64, title: &str) -> String {
    let one_line = title.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&format!("#{number} {one_line}"), THREAD_NAME_LIMIT)
}

/// `head`, an excerpt and `tail`, never longer than `limit`.
///
/// The excerpt gives way first: the link line is what a reader acts on, so it
/// is never the part that gets cut.
pub fn build_content(head: &str, excerpt: &str, tail: &str, limit: usize) -> String {
    let excerpt = excerpt.replace("\r\n", "\n");
    let excerpt = excerpt.trim();
    let fixed = format!("{head}\n\n{tail}");
    // The two newlines that separate the excerpt from `head` and `tail`.
    let budget = limit.saturating_sub(units(&fixed) + 2);
    if excerpt.is_empty() || budget < 2 {
        return fixed;
    }
    format!("{head}\n\n{}\n\n{tail}", truncate(excerpt, budget))
}
