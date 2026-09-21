//! Width-aware word wrapping of styled spans for the outline.
//!
//! Terminals don't reflow text on their own, so a block whose text runs
//! past the pane width would otherwise vanish off the right edge (issue
//! #99). The outline can't lean on ratatui's `Paragraph::wrap` because
//! that turns one logical line into N visual lines *after* layout, which
//! desyncs the `selected_line` index the viewport scroll depends on.
//!
//! Instead we wrap ourselves, emitting the final `Line`s up front so the
//! index stays honest. Crucially, wrapping happens **after** markdown
//! tokenization — the input is already styled [`Span`]s — so a break can
//! never land inside a `**bold**` token and turn it back into literal
//! asterisks (the failure mode a char-level wrap on the raw text hits).

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Push `guides + head + content` as one or more ratatui [`Line`]s,
/// wrapping `content` at `text_width` display columns.
///
/// `text_width` is the full drawable width of the outline panel (the
/// pane width minus the 2-col border). The first visual row carries
/// the real `head` (fold marker + bullet); continuations replace it
/// with blank padding of equal width so wrapped text stays aligned
/// under the bullet's text column. `guides` (the `│ ` rails) repeat on
/// every row so the indent structure reads the same top to bottom.
///
/// `text_width == 0` (or a prefix already wider than the pane) means
/// "don't wrap" — the caller passes 0 for headless renders, and we'd
/// rather overflow a pathologically narrow pane than loop. Cursor rows
/// *do* wrap now (#99): the cursor is baked into `content` as a styled
/// span before this call, so reflowing carries it onto its wrapped row.
/// This is the hot path for the common short block, so it stays
/// allocation-light: one `Line` straight through.
///
/// `cursor` is the style the caller painted the cursor cell with, when
/// the row carries one. The wrapper treats a space as a separator it
/// may discard, and since [#320](https://github.com/outlmd/outl/issues/320)
/// the cursor *is* the cell it lands on — so a caret on a space between
/// two words was being thrown away at a wrap boundary and the user saw
/// no cursor at all. Naming the style is what lets the wrapper tell a
/// load-bearing space from a separator; a blanket "styled spaces are
/// never separators" rule would also catch the spaces inside `**bold**`,
/// `` `code` `` and `[[a page ref]]`, which really are separators.
pub(crate) fn push_wrapped(
    guides: Vec<Span<'static>>,
    head: Vec<Span<'static>>,
    content: Vec<Span<'static>>,
    text_width: u16,
    cursor: Option<Style>,
    out: &mut Vec<Line<'static>>,
) {
    let head_w = spans_width(&head);
    let prefix_w = spans_width(&guides) + head_w;
    let avail = (text_width as usize).saturating_sub(prefix_w);
    let content_w = spans_width(&content);

    if avail == 0 || content_w <= avail {
        let mut spans = guides;
        spans.extend(head);
        spans.extend(content);
        out.push(Line::from(spans));
        return;
    }

    let cont_pad = " ".repeat(head_w);
    for (i, chunk) in wrap_spans(&content, avail, cursor).into_iter().enumerate() {
        let mut spans: Vec<Span<'static>> = guides.clone();
        if i == 0 {
            spans.extend(head.clone());
        } else {
            spans.push(Span::raw(cont_pad.clone()));
        }
        spans.extend(chunk);
        out.push(Line::from(spans));
    }
}

/// Total display width (in terminal cells) of a span sequence.
fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.as_ref().width()).sum()
}

/// Greedy width-aware word wrap of styled spans into rows no wider than
/// `max` display cells.
///
/// A "word" is a maximal run of non-space chars; single spaces between
/// them are separators that get *absorbed* at a wrap boundary (dropped,
/// not pushed to the next row) so wrapped text never leads with a stray
/// blank. A word longer than `max` is hard-broken at the cell boundary
/// so the loop always makes progress.
fn wrap_spans(
    spans: &[Span<'static>],
    max: usize,
    cursor: Option<Style>,
) -> Vec<Vec<Span<'static>>> {
    // Flatten to (char, style) so a break can fall anywhere while each
    // char keeps the style the renderer already assigned it.
    let cells: Vec<(char, Style)> = spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |ch| (ch, s.style)))
        .collect();

    let mut lines: Vec<Vec<(char, Style)>> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    let mut cur_w = 0usize;

    let mut i = 0;
    while i < cells.len() {
        let (ch, _) = cells[i];
        if ch == ' ' {
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if cur_w + w <= max {
                cur.push(cells[i]);
                cur_w += w;
            } else if carries_cursor(cells[i], cursor) {
                // The cursor is sitting on this space. Dropping it would
                // erase the caret from the screen, so it opens the next
                // row instead — which is where the insertion point is.
                lines.push(std::mem::take(&mut cur));
                cur.push(cells[i]);
                cur_w = w;
            } else {
                // The separating space overflows the row — break here and
                // drop it. The next word starts the following row flush.
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            i += 1;
            continue;
        }

        // Collect the whole word and its display width.
        let start = i;
        let mut word_w = 0usize;
        while i < cells.len() && cells[i].0 != ' ' {
            word_w += UnicodeWidthChar::width(cells[i].0).unwrap_or(0);
            i += 1;
        }
        let word = &cells[start..i];

        if cur_w + word_w <= max {
            cur.extend_from_slice(word);
            cur_w += word_w;
        } else if word_w <= max {
            // Fits on its own row — flush the current row (minus any
            // trailing separator space) and start the word fresh.
            trim_trailing_spaces(&mut cur, cursor);
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
            }
            cur.clear();
            cur.extend_from_slice(word);
            cur_w = word_w;
        } else {
            // Word longer than the whole row — hard-break it char by
            // char so the loop always makes progress.
            trim_trailing_spaces(&mut cur, cursor);
            if !cur.is_empty() {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0;
            }
            for &cell in word {
                let w = UnicodeWidthChar::width(cell.0).unwrap_or(0);
                if cur_w + w > max && !cur.is_empty() {
                    lines.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
                cur.push(cell);
                cur_w += w;
            }
        }
    }
    trim_trailing_spaces(&mut cur, cursor);
    if !cur.is_empty() {
        lines.push(cur);
    }

    lines.into_iter().map(rebuild_spans).collect()
}

/// Drop trailing space cells so a wrapped row doesn't keep the
/// separator that pushed the next word onto a new line.
///
/// A space the cursor is sitting on is not a separator — popping it
/// takes the caret off the screen (#320) — so trimming stops there.
fn trim_trailing_spaces(cells: &mut Vec<(char, Style)>, cursor: Option<Style>) {
    while let Some(&cell) = cells.last() {
        if cell.0 != ' ' || carries_cursor(cell, cursor) {
            return;
        }
        cells.pop();
    }
}

/// Whether this cell is the one the caller painted the cursor on.
fn carries_cursor(cell: (char, Style), cursor: Option<Style>) -> bool {
    cursor.is_some_and(|style| cell.1 == style)
}

/// Coalesce a `(char, style)` row back into the minimal run of
/// [`Span`]s — consecutive chars sharing a style become one span.
fn rebuild_spans(cells: Vec<(char, Style)>) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur_style: Option<Style> = None;
    for (ch, st) in cells {
        match cur_style {
            Some(s) if s == st => buf.push(ch),
            _ => {
                if let Some(s) = cur_style {
                    spans.push(Span::styled(std::mem::take(&mut buf), s));
                }
                buf.push(ch);
                cur_style = Some(st);
            }
        }
    }
    if let Some(s) = cur_style {
        spans.push(Span::styled(buf, s));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    /// Concatenate a line's span text into one `String` for assertions.
    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// Concatenate a rebuilt span row into a `String`.
    fn rebuilt_text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn wrap_breaks_on_the_last_space_within_width() {
        let rows = wrap_spans(&[Span::raw("the quick brown fox")], 10, None);
        let texts: Vec<String> = rows.iter().map(|r| rebuilt_text(r)).collect();
        // "the quick " is 10 wide; the trailing space is dropped and
        // "brown fox" starts the next row.
        assert_eq!(texts, vec!["the quick", "brown fox"]);
    }

    #[test]
    fn wrap_hard_breaks_a_word_longer_than_width() {
        let rows = wrap_spans(&[Span::raw("supercalifragilistic")], 5, None);
        let texts: Vec<String> = rows.iter().map(|r| rebuilt_text(r)).collect();
        assert_eq!(texts, vec!["super", "calif", "ragil", "istic"]);
    }

    #[test]
    fn wrap_preserves_per_span_style_across_a_break() {
        let bold = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
        // "aaaa BBBB" where "BBBB" is bold — a break between the words
        // must keep "BBBB" bold (the whole point of wrapping styled
        // spans instead of the raw string).
        let rows = wrap_spans(&[Span::raw("aaaa "), Span::styled("BBBB", bold)], 4, None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rebuilt_text(&rows[0]), "aaaa");
        assert_eq!(rebuilt_text(&rows[1]), "BBBB");
        assert_eq!(rows[1][0].style, bold);
    }

    /// #320: the cursor is the text cell it lands on, so a cursor
    /// parked on a space is not the separator this wrapper is allowed
    /// to trim off the end of a row. Popping it takes the caret off
    /// the screen entirely.
    #[test]
    fn wrap_keeps_a_trailing_space_the_cursor_is_on() {
        let caret = Style::default().add_modifier(Modifier::UNDERLINED);
        let content = [
            Span::raw("the quick"),
            Span::styled(" ", caret),
            Span::raw("brown"),
        ];
        let rows = wrap_spans(&content, 12, Some(caret));
        assert_eq!(rebuilt_text(&rows[0]), "the quick ");
        assert_eq!(rebuilt_text(&rows[1]), "brown");
        // Same input with no cursor declared: the space is a separator
        // again and goes, which is what every other row wants.
        let rows = wrap_spans(&content, 12, None);
        assert_eq!(rebuilt_text(&rows[0]), "the quick");
    }

    /// The other place a space is discarded: it overflows the row, so
    /// the row breaks and the space is skipped so the next one doesn't
    /// lead with a blank. A cursor space opens that row instead — a
    /// leading blank is exactly where the insertion point is.
    #[test]
    fn wrap_carries_an_overflowing_cursor_space_to_the_next_row() {
        let caret = Style::default().add_modifier(Modifier::UNDERLINED);
        let content = [Span::raw("aaaa"), Span::styled(" ", caret), Span::raw("bb")];
        let rows = wrap_spans(&content, 4, Some(caret));
        let texts: Vec<String> = rows.iter().map(|r| rebuilt_text(r)).collect();
        assert_eq!(texts, vec!["aaaa", " bb"]);
        let rows = wrap_spans(&content, 4, None);
        let texts: Vec<String> = rows.iter().map(|r| rebuilt_text(r)).collect();
        assert_eq!(texts, vec!["aaaa", "bb"]);
    }

    #[test]
    fn wrap_counts_wide_glyphs_as_two_cells() {
        // Each CJK glyph is 2 cells wide, so only two fit in width 5.
        let rows = wrap_spans(&[Span::raw("你好世界")], 5, None);
        let texts: Vec<String> = rows.iter().map(|r| rebuilt_text(r)).collect();
        assert_eq!(texts, vec!["你好", "世界"]);
    }

    #[test]
    fn push_wrapped_keeps_short_content_on_one_line() {
        let mut out = Vec::new();
        push_wrapped(
            vec![],
            vec![Span::raw("- ")],
            vec![Span::raw("short")],
            80,
            None,
            &mut out,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(line_text(&out[0]), "- short");
    }

    #[test]
    fn push_wrapped_reindents_continuations_under_the_text() {
        let mut out = Vec::new();
        // head "- " is 2 cells; width 8 leaves 6 cells for text.
        push_wrapped(
            vec![],
            vec![Span::raw("- ")],
            vec![Span::raw("alpha bravo charlie")],
            8,
            None,
            &mut out,
        );
        assert!(out.len() >= 2);
        // First row carries the bullet.
        assert_eq!(line_text(&out[0]), "- alpha");
        // Continuations replace "- " with two spaces so the text stays
        // in the same column.
        assert!(line_text(&out[1]).starts_with("  "));
        assert_eq!(line_text(&out[1]), "  bravo");
    }

    #[test]
    fn push_wrapped_zero_width_never_wraps() {
        // text_width 0 (headless renders) is the "don't wrap"
        // sentinel — one line straight through.
        let mut out = Vec::new();
        push_wrapped(
            vec![],
            vec![Span::raw("- ")],
            vec![Span::raw("a very long line that would otherwise wrap")],
            0,
            None,
            &mut out,
        );
        assert_eq!(out.len(), 1);
    }
}
