//! UI-agnostic block layout — decomposes an [`crate::parse::OutlineNode`]'s text
//! into a sequence of "visual rows" that the TUI, future Tauri GUI,
//! and mobile clients all consume.
//!
//! Each row tells the renderer:
//!
//! - **What indent** (in nested-bullet levels) it sits at.
//! - **What kind** of row it is — bullet line, continuation,
//!   code-fence marker, or code-fence body. Renderers map those to
//!   their own primitives.
//! - **Which characters** the cursor (if any) sits on.
//!
//! The actual painting (ratatui spans, React fragments, SwiftUI
//! `AttributedString`) lives in the UI crate. This module only owns
//! the decomposition.
//!
//! Why bother splitting this out? Because the rules for what counts
//! as a code-fence opener, what column the cursor lands on, and how
//! `\n` inside a block text becomes a continuation row, must be
//! identical across every outl UI — otherwise external `.md` edits
//! would render differently between the TUI and the desktop app.

/// One visual row produced from a block's text.
///
/// Use [`block_to_rows`] to build a `Vec<BlockRow>` and then map each
/// row to your UI's primitives (a ratatui `Line`, an HTML `<div>`, an
/// `AttributedString` slice, etc.).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRow<'a> {
    /// Nesting level of the parent block. Renderers typically draw
    /// `indent` indent guides before the row content.
    pub indent: u32,
    /// What this row represents.
    pub kind: BlockRowKind,
    /// The source text of this row (no leading indent stripped from
    /// the source; that's the renderer's job).
    pub text: &'a str,
    /// If the cursor sits on this row, the **char** index inside
    /// `text`. `None` otherwise.
    pub cursor_col: Option<usize>,
}

/// Classification of a row within a block.
///
/// Renderers use this to pick the right style: e.g. a `CodeFenceBody`
/// row gets monospace + a code background; a `Bullet` row gets the
/// bullet glyph; a `Continuation` row gets indent matching the
/// bullet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockRowKind {
    /// First line of a block — the line that carries the bullet.
    Bullet,
    /// A continuation line — the user pressed Shift+Enter (or
    /// imported a multi-line bullet). Aligns under the bullet.
    Continuation,
    /// The line that *opens* or *closes* a fenced code block
    /// (`` ``` `` or `` ```lang ``).
    CodeFenceMarker,
    /// A line inside an open fence — preserved literally, not
    /// tokenized as inline markdown.
    CodeFenceBody,
    /// A row of a markdown table (header or body). Only emitted when
    /// the block's **whole** text is a table — a block that merely
    /// contains a pipe is prose, and painting it as a grid would be a
    /// lie about what the user wrote.
    ///
    /// Row 0 of a table block is still [`Self::Bullet`], for the same
    /// reason a fence opener is: the `- ` glyph is what anchors the
    /// block in the outline.
    TableRow,
    /// A table's delimiter row (`| --- | :-: |`). Carries the column
    /// alignment, not content — renderers draw it as a rule, and the
    /// GUI clients drop it entirely in favour of `<table>` borders.
    TableRule,
}

/// Convert a char index into a `(line, col)` pair using `\n` as the
/// line separator. Indices count *characters*, not bytes.
///
/// Renderers usually then convert `col` to bytes via
/// [`crate::byte_index_for_char`] before slicing the row's text.
pub fn char_to_line_col(s: &str, char_idx: usize) -> (usize, usize) {
    let mut line = 0usize;
    let mut col = 0usize;
    for (seen, ch) in s.chars().enumerate() {
        if seen == char_idx {
            return (line, col);
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Inverse of [`char_to_line_col`]: locate the char index of
/// `(line, col)` inside `s`. Lines are `\n`-separated; the column
/// counts characters into the target line.
///
/// **Column clamping (vim-style):** when the requested column is
/// past the end of its line, the cursor lands at the line's last
/// char — useful for preserving a "preferred column" while moving
/// up/down across lines of varying length.
///
/// **Line clamping:** asking for a line past the last one returns
/// the end of the string. This mirrors what most editors do when the
/// user presses `Down` on the last row.
///
/// Used by the TUI's `EditBuffer::move_up` / `move_down` so the
/// cursor positions in the editor match the line/col mapping the
/// renderer ([`block_to_rows`]) already builds — one owner for the
/// mapping, no drift between cursor and rendering.
pub fn line_col_to_char(s: &str, target_line: usize, target_col: usize) -> usize {
    let mut line = 0usize;
    let mut col = 0usize;
    let mut total = 0usize; // char count tracked inline so the
                            // fall-through doesn't pay a second pass.

    for (idx, ch) in s.chars().enumerate() {
        total = idx + 1;
        if line == target_line && col == target_col {
            return idx;
        }
        if ch == '\n' {
            // About to enter the next line — if we were on the target
            // line, the column requested was past EOL; clamp to the
            // index of this newline (i.e. one past the last char).
            if line == target_line {
                return idx;
            }
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    // Ran off the end of the string. Either the target line was the
    // last line (and the column was past its end) or the target line
    // is past the buffer entirely — both clamp to end of string.
    total
}

/// Decompose a block's `text` into visual rows.
///
/// `cursor_char`, when present, is a char index into `text` (counting
/// `\n` as one char) at which the active editing cursor sits. The
/// matching row gets `cursor_col = Some(col)`; everyone else gets
/// `None`.
///
/// Empty `text` produces a single `Bullet` row with empty content so
/// the renderer still has somewhere to draw the bullet glyph.
pub fn block_to_rows<'a>(
    text: &'a str,
    indent: u32,
    cursor_char: Option<usize>,
) -> Vec<BlockRow<'a>> {
    let lines: Vec<&str> = if text.is_empty() {
        vec![""]
    } else {
        text.split('\n').collect()
    };

    let cursor_lc = cursor_char.map(|c| char_to_line_col(text, c));

    // Whole-block check, not per-line: see `BlockRowKind::TableRow`.
    // `table_span` short-circuits on the first line for every block
    // that carries no pipe, which is almost all of them.
    let is_table = crate::table::table_span(&lines, 0) == Some(lines.len());

    let mut rows = Vec::with_capacity(lines.len());
    let mut in_fence = false;
    for (idx, line) in lines.iter().enumerate() {
        let is_marker = line.trim_start().starts_with("```");
        // The very first row of a block *always* carries the bullet —
        // even when its text happens to be a fence opener like
        // ` ```lisp `. Renderers decide how to style the fence content
        // inside a Bullet row; what we cannot give up is the visual
        // `- ` glyph that anchors the block in the outline.
        let kind = if idx == 0 {
            BlockRowKind::Bullet
        } else if is_table {
            // Position, not shape. `table_span` requires the rule at
            // index 1 and `is_table` requires the span to cover the
            // whole text, so the rule is *always* there and nowhere
            // else. Asking `is_delimiter_row` of every row instead made
            // any body row whose cells are all dash-shaped read as a
            // rule — a spreadsheet using `-` for "no data" had its data
            // row painted as `├───┼───┤`, so the cells vanished from the
            // screen while staying on disk and in the log.
            if idx == 1 {
                BlockRowKind::TableRule
            } else {
                BlockRowKind::TableRow
            }
        } else if is_marker {
            BlockRowKind::CodeFenceMarker
        } else if in_fence {
            BlockRowKind::CodeFenceBody
        } else {
            BlockRowKind::Continuation
        };
        let cursor_col = cursor_lc.and_then(|(l, c)| (l == idx).then_some(c));
        rows.push(BlockRow {
            indent,
            kind,
            text: line,
            cursor_col,
        });
        if is_marker {
            // Toggle fence state regardless of whether the marker
            // landed in row 0 (Bullet) or below — so the *next* row
            // knows it's inside the fence body.
            in_fence = !in_fence;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_yields_one_bullet_row() {
        let rows = block_to_rows("", 0, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, BlockRowKind::Bullet);
        assert_eq!(rows[0].text, "");
    }

    #[test]
    fn single_line_text_yields_one_bullet_row() {
        let rows = block_to_rows("hello", 0, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, BlockRowKind::Bullet);
        assert_eq!(rows[0].text, "hello");
    }

    #[test]
    fn multi_line_yields_bullet_then_continuation() {
        let rows = block_to_rows("first\nsecond\nthird", 2, None);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, BlockRowKind::Bullet);
        assert_eq!(rows[0].text, "first");
        assert_eq!(rows[1].kind, BlockRowKind::Continuation);
        assert_eq!(rows[1].text, "second");
        assert_eq!(rows[2].kind, BlockRowKind::Continuation);
        assert_eq!(rows[2].text, "third");
        for r in &rows {
            assert_eq!(r.indent, 2);
        }
    }

    #[test]
    fn fence_markers_and_body_classified_correctly() {
        let rows = block_to_rows("intro\n```lisp\n(+ 1 2)\n```", 0, None);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].kind, BlockRowKind::Bullet);
        assert_eq!(rows[1].kind, BlockRowKind::CodeFenceMarker);
        assert_eq!(rows[2].kind, BlockRowKind::CodeFenceBody);
        assert_eq!(rows[3].kind, BlockRowKind::CodeFenceMarker);
    }

    #[test]
    fn block_starting_with_fence_keeps_bullet_on_row_zero() {
        // Regression: a block whose first character is `` ``` `` used
        // to lose its bullet, because row 0 was classified as
        // `CodeFenceMarker`. The bullet glyph anchors the block in
        // the outline — we cannot drop it.
        let rows = block_to_rows("```lisp\n(+ 1 2)\n```", 0, None);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, BlockRowKind::Bullet);
        assert_eq!(rows[0].text, "```lisp");
        assert_eq!(rows[1].kind, BlockRowKind::CodeFenceBody);
        assert_eq!(rows[2].kind, BlockRowKind::CodeFenceMarker);
    }

    #[test]
    fn a_table_blocks_rows_are_classified() {
        let rows = block_to_rows("| a | b |\n| --- | --- |\n| 1 | 2 |", 0, None);
        assert_eq!(rows.len(), 3);
        // Row 0 keeps the bullet — the glyph anchors the block, same
        // rule as a fence opener.
        assert_eq!(rows[0].kind, BlockRowKind::Bullet);
        assert_eq!(rows[1].kind, BlockRowKind::TableRule);
        assert_eq!(rows[2].kind, BlockRowKind::TableRow);
    }

    #[test]
    fn a_body_row_of_dashes_is_not_the_rule() {
        // A spreadsheet using `-` as the "no data" placeholder. Its
        // cells are all rule-shaped, so classifying by shape painted the
        // user's data row as a horizontal rule and the values
        // disappeared from the screen.
        let rows = block_to_rows("| a | b |\n| --- | --- |\n| - | - |", 0, None);
        assert_eq!(rows[1].kind, BlockRowKind::TableRule);
        assert_eq!(rows[2].kind, BlockRowKind::TableRow, "the data row is data");
    }

    #[test]
    fn a_block_that_merely_contains_a_pipe_is_not_a_table() {
        // Painting this as a grid would be a lie about what the user
        // wrote, so every row after the first stays Continuation.
        let rows = block_to_rows("a | b\nc | d", 0, None);
        assert_eq!(rows[1].kind, BlockRowKind::Continuation);
    }

    #[test]
    fn a_table_with_a_trailing_sentence_is_not_a_table() {
        // The whole text has to be the table; otherwise the sentence
        // would render as a row.
        let rows = block_to_rows("| a |\n| --- |\nand a note", 0, None);
        assert_eq!(rows[1].kind, BlockRowKind::Continuation);
        assert_eq!(rows[2].kind, BlockRowKind::Continuation);
    }

    #[test]
    fn cursor_lands_on_the_right_row() {
        // "first\nsecond" — char index 6 is the 's' on line 1.
        let rows = block_to_rows("first\nsecond", 0, Some(6));
        assert_eq!(rows[0].cursor_col, None);
        assert_eq!(rows[1].cursor_col, Some(0));
    }

    #[test]
    fn cursor_past_end_lands_at_last_col() {
        let rows = block_to_rows("ab", 0, Some(2));
        assert_eq!(rows[0].cursor_col, Some(2));
    }

    #[test]
    fn char_to_line_col_handles_multibyte() {
        // "está" — 4 chars, 'á' is char 3.
        let (l, c) = char_to_line_col("está", 3);
        assert_eq!(l, 0);
        assert_eq!(c, 3);
    }

    #[test]
    fn char_to_line_col_across_newlines() {
        let (l, c) = char_to_line_col("first\nsecond", 7);
        assert_eq!(l, 1);
        assert_eq!(c, 1);
    }

    #[test]
    fn line_col_to_char_round_trips_with_char_to_line_col() {
        let s = "first\nsecond";
        let (l, c) = char_to_line_col(s, 7); // char `e` of "second" → (1, 1)
        assert_eq!(line_col_to_char(s, l, c), 7);
    }

    #[test]
    fn line_col_to_char_clamps_to_eol_on_overshoot() {
        // "hi\nworld" — line 0 has 2 chars; asking for col 5 should
        // land at the end of line 0 (idx 2 = the `\n`).
        assert_eq!(line_col_to_char("hi\nworld", 0, 5), 2);
    }

    #[test]
    fn line_col_to_char_clamps_to_end_when_target_line_is_last() {
        // "hello\nhi" — line 1 has 2 chars; col 4 → end of string.
        assert_eq!(line_col_to_char("hello\nhi", 1, 4), 8);
    }

    #[test]
    fn line_col_to_char_clamps_when_target_line_is_past_end() {
        // No line 5 in this string → end of string.
        assert_eq!(line_col_to_char("only\nline", 5, 0), 9);
    }

    #[test]
    fn line_col_to_char_lands_at_zero_on_origin() {
        assert_eq!(line_col_to_char("anything", 0, 0), 0);
    }
}
