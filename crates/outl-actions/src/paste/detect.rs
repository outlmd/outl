//! Reading the shape of the clipboard payload, before anything is
//! written.
//!
//! Two questions, both asked of the **raw** text and both answered
//! without mutating it: is this an outline, and where does plain text
//! break into blocks. They live together because they are the pair that
//! decides which pipeline [`super::paste_markdown`] runs, and because
//! getting either wrong lands mangled text in a user's page rather than
//! failing.

/// True when at least one non-blank line starts with `- ` or is just `-`.
///
/// This is the canonical detector that gates the tree-conversion
/// pipeline. Mobile mirrors it in TypeScript
/// (`crates/outl-mobile/src/lib/paste.ts::looksLikeOutline`) so the
/// client can avoid a Tauri round-trip when the user pastes plain
/// text. The two implementations **must stay in lockstep** — if you
/// extend this to recognise `*` bullets, ordered lists, or anything
/// else, update the JS mirror in the same PR and add the case to
/// `paste.test.ts`.
///
/// Exposed `pub` so UI clients can branch *before* invoking
/// [`super::paste_markdown`]: a TUI in Insert mode, for example, wants to
/// splice plain text into the live edit buffer instead of going
/// through the full paste pipeline.
pub fn looks_like_outline(s: &str) -> bool {
    s.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed == "-" || trimmed.starts_with("- ")
    })
}

/// Split pasted plain text into one block per non-blank line.
///
/// In a `text/plain` clipboard a paragraph is a **single line** — the
/// visual wrap of a long sentence carries no newline — so a chat reply or
/// an email arrives as one line per paragraph, separated by `\n` (and
/// only sometimes by a blank `\n\n`). Splitting on every non-blank line,
/// dropping the blank ones, turns that into one block per paragraph for
/// either separator. Text that is genuinely one unit but carries hard
/// line breaks (a code listing) is the rare exception — paste that
/// "without formatting" (`paste_plain`) to keep it as a single block.
///
/// Returns an empty `Vec` for all-blank input. Internal to the paste
/// pipeline.
pub(crate) fn split_paragraphs(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_detector_true_on_bullet_lines() {
        assert!(looks_like_outline("- foo"));
        assert!(looks_like_outline("  - nested"));
        assert!(looks_like_outline("preface\n- bullet"));
    }

    #[test]
    fn outline_detector_false_on_plain_text() {
        assert!(!looks_like_outline("just words"));
        assert!(!looks_like_outline("multi\nline\ntext"));
        assert!(!looks_like_outline(""));
    }

    #[test]
    fn split_paragraphs_is_one_per_nonblank_line() {
        // One block per non-blank line, for either `\n` or `\n\n`
        // separators (in a text/plain clipboard a paragraph is a line).
        let text = "Para one\nPara two\n\nPara three\n";
        assert_eq!(
            split_paragraphs(text),
            vec![
                "Para one".to_string(),
                "Para two".to_string(),
                "Para three".to_string(),
            ]
        );
        assert!(split_paragraphs("   \n  \n").is_empty());
        assert_eq!(split_paragraphs("solo").len(), 1);
    }

    #[test]
    fn split_paragraphs_handles_crlf() {
        // Windows clipboards separate lines with `\r\n`; `str::lines()`
        // strips the `\r`, so no carriage return leaks into a block.
        assert_eq!(
            split_paragraphs("a\r\nb\r\nc"),
            vec!["a".to_string(), "b".into(), "c".into()]
        );
        // A blank CRLF line is dropped like any other blank.
        assert_eq!(
            split_paragraphs("a\r\n\r\nb"),
            vec!["a".to_string(), "b".into()]
        );
    }
}
