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

/// True when the payload carries a markdown table anywhere in it.
///
/// The second gate on the tree-conversion pipeline, beside
/// [`looks_like_outline`]. Without it a pasted table — from a README, a
/// web page, an assistant's reply — reaches `split_paragraphs` and
/// lands as one block **per row**, which is the shape the user then has
/// to undo by hand.
///
/// Asks `outl_md::table_span` at every line rather than looking for a
/// `|`, so the answer cannot disagree with what the parser will do with
/// the same text. A line of prose carrying a pipe is not a table to
/// either of them.
///
/// Mirrored in TypeScript (`@outl/shared/paste::looksLikeTable`) for
/// the same reason `looks_like_outline` is: the client gates the Tauri
/// round-trip before the browser splices the text in place. **Keep both
/// in lockstep.**
pub fn looks_like_table(s: &str) -> bool {
    let lines: Vec<&str> = s.lines().collect();
    (0..lines.len()).any(|start| outl_md::table_span(&lines, start).is_some())
}

/// True when the payload is tab-separated tabular data.
///
/// Delegates to `outl_md::from_delimited`, which is the single owner of
/// that gate (two lines or more, the same field count on every line,
/// no line starting with a tab) and also does the conversion — asking
/// it rather than re-deriving the rule is what keeps a client's "should
/// I route this" from disagreeing with the pipeline's "is this a
/// table".
pub fn looks_like_tabular(s: &str) -> bool {
    outl_md::from_delimited(s, '\t').is_some()
}

/// Whether the payload carries structure [`super::paste_markdown`] will
/// act on — an outline, a markdown table, or tabular data.
///
/// The question a client asks **before** calling the pipeline: a
/// payload with no structure is better spliced where the caret already
/// is (the TUI's Insert mode, the browser's native paste) than sent on
/// a round trip that would only hand it back unchanged.
///
/// One predicate rather than three at each call site, because three
/// copies of "is this worth converting" is how one client learns about
/// tables and another doesn't. Mirrored by
/// `@outl/shared/paste::choosePasteRoute`'s `structured` arm.
pub fn looks_structured(s: &str) -> bool {
    looks_like_outline(s) || looks_like_table(s) || looks_like_tabular(s)
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
    fn table_detector_finds_a_table_anywhere_in_the_payload() {
        assert!(looks_like_table("| a | b |\n| --- | --- |"));
        assert!(looks_like_table(
            "some prose\n\n| a | b |\n| --- | --- |\n| 1 | 2 |"
        ));
        assert!(looks_like_table("a | b\n--- | ---"));
    }

    #[test]
    fn table_detector_is_false_on_prose_that_carries_a_pipe() {
        // The false positive that would matter: this must stay on the
        // paragraph path, one block per line.
        assert!(!looks_like_table("run `a | b` in the shell"));
        assert!(!looks_like_table("a | b\nc | d"));
        assert!(!looks_like_table("plain words"));
        assert!(!looks_like_table(""));
    }

    #[test]
    fn structured_covers_all_three_shapes_and_nothing_else() {
        assert!(looks_structured("- a bullet"));
        assert!(looks_structured("| a | b |\n| --- | --- |"));
        assert!(looks_structured("Route\tPax\nSP\t1203"));
        // Prose stays unstructured: a round trip would hand it back
        // unchanged, so the caller splices it where the caret is.
        assert!(!looks_structured("one sentence"));
        assert!(!looks_structured("a | b\nc | d"));
        assert!(!looks_structured("\tif x:\n\treturn y"));
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
