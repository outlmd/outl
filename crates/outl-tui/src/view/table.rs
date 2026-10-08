//! Painting a markdown table as a grid.
//!
//! The dialect stores a table as the pipe rows the user wrote (see
//! [`outl_md::table`]); this module is the TUI's *pretty* reading of
//! them — the same relationship `render_pretty_block_text` has with a
//! `TODO ` prefix or a `> ` quote marker, and for the same reason: the
//! source is what gets edited, the glyph is what gets read.
//!
//! Three steps, each of which has to stay in this order:
//!
//! 1. [`aligned_grid`] re-emits the block through
//!    [`outl_md::render_table`], so every column is padded to its
//!    widest cell. That function is the single owner of column width —
//!    a second opinion here would drift from the markdown a tabular
//!    paste writes.
//! 2. Because every row is then the same length with its separators in
//!    fixed positions, a rule row can be drawn by substituting
//!    characters in place ([`render_rule`]) and it lines up with the
//!    rows above and below it for free.
//! 3. Cell contents go through the inline tokenizer, so a `[[ref]]` or
//!    `**bold**` inside a cell renders like it does anywhere else.
//!
//! **Nothing here runs while the cursor is on the block.** The
//! cursor-bearing render is raw (`highlight_inline`), because a glyph
//! that is not in the source breaks column-to-byte alignment for the
//! caret — the same rule `render_markdown_inline`'s doc states.

use crate::icons::IconSet;
use crate::theme::Theme;
use crate::view::inline::render_markdown_inline;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

/// The block's text re-emitted as a column-aligned grid, `None` when
/// the block is not a table.
///
/// The trailing newline [`outl_md::render_table`] writes is dropped: a
/// block's text ends at its last row, and keeping it would add an empty
/// visual row under every table.
pub(crate) fn aligned_grid(text: &str) -> Option<String> {
    let table = outl_md::parse_table(text)?;
    Some(
        outl_md::render_table_with(&table, shown_width)
            .trim_end_matches('\n')
            .to_string(),
    )
}

/// How wide a cell will be **after** [`render_row`] paints it.
///
/// Padding by source width is what broke the grid: this module paints
/// `Avelino` for `[[Avelino]]`, `bold` for `**bold**`, `🎉` for
/// `:tada:` and `|` for `\|`, each narrower than the source, so every
/// `│` after such a cell sat left of the ones above and below it.
///
/// `plain_text` is the same flattening the pretty render performs, and
/// `unescape_cell` the same unescape, so the two agree by construction
/// for every token but one.
///
/// **Known gap:** a `((blk-…))` resolves to the source block's text
/// when painted, which needs the workspace index; `plain_text` drops it
/// to nothing. A cell that is only a block ref therefore still
/// misaligns. Closing it means handing the index to the measure, which
/// is a wider change than the one this function is part of.
fn shown_width(cell: &str) -> usize {
    UnicodeWidthStr::width(outl_md::plain_text(&outl_md::unescape_cell(cell)).as_str())
}

/// One table row: cell contents tokenized, separators drawn as `│`.
///
/// `line` must come from [`aligned_grid`] — the padding is what makes
/// the separators line up between rows.
pub(crate) fn render_row(
    line: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
    icons: &IconSet,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    let flush = |cell: &mut String, out: &mut Vec<Span<'static>>| {
        if !cell.is_empty() {
            // `outl_md::unescape_cell` is the single owner of "what does
            // a cell show": pretty render shows a literal pipe, not its
            // escape, and the GUI clients' `tokenize_table` asks the
            // same function.
            let shown = outl_md::unescape_cell(cell);
            out.extend(render_markdown_inline(&shown, theme, index, icons));
            cell.clear();
        }
    };
    for ch in line.chars() {
        if escaped {
            // Keep the escape in the accumulator — `unescape_cell`
            // resolves it at flush, so the split and the display cannot
            // disagree about which backslashes were syntax.
            cell.push('\\');
            cell.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '|' => {
                flush(&mut cell, &mut out);
                out.push(Span::styled("│", theme.dim));
            }
            _ => cell.push(ch),
        }
    }
    if escaped {
        cell.push('\\');
    }
    flush(&mut cell, &mut out);
    out
}

/// A delimiter row, drawn as a rule rather than as `| --- | :-: |`.
///
/// Substitution in place, one output character per input character, so
/// the junctions sit exactly under the `│` of the row above. The
/// alignment colons are deliberately not shown: they describe the
/// column, and the column's own padding already shows it.
pub(crate) fn render_rule(line: &str, theme: &Theme) -> Vec<Span<'static>> {
    let total = line.chars().count();
    let drawn: String = line
        .chars()
        .enumerate()
        .map(|(idx, ch)| match ch {
            '|' if idx == 0 => '├',
            '|' if idx + 1 == total => '┤',
            '|' => '┼',
            _ => '─',
        })
        .collect();
    vec![Span::styled(drawn, theme.dim)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_table_is_padded_to_a_grid() {
        let grid = aligned_grid("| Name | Age |\n| --- | --- |\n| Roberta | 7 |").expect("a table");
        assert_eq!(
            grid,
            "| Name    | Age |\n| ------- | --- |\n| Roberta | 7   |"
        );
    }

    #[test]
    fn a_grid_carries_no_trailing_blank_row() {
        let grid = aligned_grid("| a |\n| --- |").expect("a table");
        assert!(!grid.ends_with('\n'));
    }

    #[test]
    fn prose_with_a_pipe_is_not_a_grid() {
        assert!(aligned_grid("a | b\nc | d").is_none());
        assert!(aligned_grid("plain text").is_none());
    }

    #[test]
    fn a_cells_escaped_pipe_is_painted_as_a_pipe() {
        let theme = crate::theme::default_theme();
        let index = outl_md::index::WorkspaceIndex::default();
        let icons = IconSet::default();
        let spans = render_row(r"| a \| b |", &theme, &index, &icons);
        let painted: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(painted, "│ a | b │");
    }

    #[test]
    fn a_cell_carrying_inline_markdown_still_lines_up() {
        // The grid is padded at source width and painted at rendered
        // width, so `[[Avelino]]` (11 columns of source, 7 painted) used
        // to shift every `│` after it four columns left of the rows
        // above and below.
        let theme = crate::theme::default_theme();
        let index = outl_md::index::WorkspaceIndex::default();
        let icons = IconSet::default();
        let grid = aligned_grid("| Page | n |\n| --- | --- |\n| [[Avelino]] | 1 |\n| plain | 2 |")
            .expect("a table");
        let painted: Vec<String> = grid
            .lines()
            .enumerate()
            .map(|(idx, line)| {
                // Index 1, the same rule `block_to_rows` follows: the
                // delimiter row is at a position, not a shape.
                let spans = if idx == 1 {
                    render_rule(line, &theme)
                } else {
                    render_row(line, &theme, &index, &icons)
                };
                spans.iter().map(|s| s.content.as_ref()).collect()
            })
            .collect();
        let widths: Vec<usize> = painted
            .iter()
            .map(|l| UnicodeWidthStr::width(l.as_str()))
            .collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "rows disagree on width: {widths:?}\n{}",
            painted.join("\n")
        );
    }

    #[test]
    fn a_rule_row_lines_up_with_the_row_above_it() {
        let theme = crate::theme::default_theme();
        let grid = aligned_grid("| Name | Age |\n| --- | --- |\n| Ana | 30 |").expect("a table");
        let mut rows = grid.lines();
        let header = rows.next().expect("header");
        let rule = rows.next().expect("rule");
        let drawn = render_rule(rule, &theme);
        let painted: String = drawn.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(painted, "├──────┼─────┤");
        // One output char per input char is what buys the alignment.
        assert_eq!(painted.chars().count(), header.chars().count());
        // And the junctions sit under the header's separators.
        for (idx, (a, b)) in header.chars().zip(painted.chars()).enumerate() {
            assert_eq!(
                a == '|',
                matches!(b, '├' | '┼' | '┤'),
                "column {idx} disagrees"
            );
        }
    }
}
