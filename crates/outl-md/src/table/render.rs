//! Writing a [`Table`](super::Table) back out as markdown.
//!
//! Split from the parent module at the parse/emit seam: everything here
//! answers "what does this table look like on a line", and nothing here
//! reads a `.md`.
//!
//! **Only used where outl *creates* a table** — a tabular paste, the
//! TUI painting a grid. The parser deliberately does not route a
//! hand-written table through this, because re-emitting it
//! column-padded on read is a whitespace-only rewrite that the
//! reconcile turns into an `Op::Edit` per table. See the parent
//! module's "Why the parser does not reformat".

use unicode_width::UnicodeWidthStr;

use super::{ColumnAlign, Table};

/// Narrowest delimiter cell GFM accepts once a colon is in play
/// (`:-:`), and the floor [`render_table`] pads every column to so a
/// one-character column still reads as a rule rather than a dash.
const MIN_RULE_WIDTH: usize = 3;

/// Render a table to canonical markdown: one `| cell | cell |` line
/// per row, every column padded to its widest cell.
///
/// Padding uses display width, not byte or char count, so a column of
/// CJK or emoji lines up in a monospaced grid.
///
/// Short rows are padded with empty cells to [`Table::columns`], which
/// means `parse_table(render_table(t))` can carry more cells per row
/// than `t` did. `render_table` of that reading is identical, so the
/// output is stable from the first pass — pinned by
/// `rendering_is_stable_under_reparse`.
pub fn render_table(table: &Table) -> String {
    render_table_with(table, UnicodeWidthStr::width)
}

/// [`render_table`] with the caller's own idea of how wide a cell is.
///
/// Padding has to measure **what the reader will see**, and that is not
/// always the source. A renderer that paints inline markdown shows
/// `Avelino` for `[[Avelino]]` (4 columns narrower) and `|` for `\|`
/// (one narrower), so padding by source width shifted every `│` after
/// such a cell and the grid stopped lining up — while the module doc
/// claimed it did.
///
/// The alignment itself stays here, so there is still one owner of it;
/// only the measurement is the caller's. `measure` is asked for each
/// cell exactly as it appears in the [`Table`], escapes included.
pub fn render_table_with(table: &Table, measure: impl Fn(&str) -> usize) -> String {
    let cols = table.columns();
    if cols == 0 {
        return String::new();
    }
    let widths = column_widths(table, cols, &measure);

    let mut out = String::new();
    write_row(&mut out, &table.header, &widths, table, &measure);
    out.push('|');
    for (col, width) in widths.iter().enumerate() {
        out.push(' ');
        out.push_str(&rule_for(table.align(col), *width));
        out.push_str(" |");
    }
    out.push('\n');
    for row in &table.rows {
        write_row(&mut out, row, &widths, table, &measure);
    }
    out
}

fn column_widths(table: &Table, cols: usize, measure: &impl Fn(&str) -> usize) -> Vec<usize> {
    let mut widths = vec![MIN_RULE_WIDTH; cols];
    let rows = std::iter::once(&table.header).chain(table.rows.iter());
    for row in rows {
        for (col, cell) in row.iter().enumerate() {
            let width = measure(cell);
            if width > widths[col] {
                widths[col] = width;
            }
        }
    }
    widths
}

fn write_row(
    out: &mut String,
    row: &[String],
    widths: &[usize],
    table: &Table,
    measure: &impl Fn(&str) -> usize,
) {
    out.push('|');
    for (col, width) in widths.iter().enumerate() {
        let cell = row.get(col).map(String::as_str).unwrap_or("");
        out.push(' ');
        out.push_str(&pad(cell, *width, table.align(col), measure));
        out.push_str(" |");
    }
    out.push('\n');
}

/// Pad `cell` to `width` display columns according to `align`.
///
/// A cell wider than `width` is returned untouched — truncating is how
/// a table silently eats a long cell, and a grid one column too wide
/// is strictly better than content nobody can read.
fn pad(cell: &str, width: usize, align: ColumnAlign, measure: &impl Fn(&str) -> usize) -> String {
    let slack = width.saturating_sub(measure(cell));
    if slack == 0 {
        return cell.to_string();
    }
    match align {
        ColumnAlign::Right => format!("{}{cell}", " ".repeat(slack)),
        ColumnAlign::Center => {
            let left = slack / 2;
            format!("{}{cell}{}", " ".repeat(left), " ".repeat(slack - left))
        }
        // `None` renders like `Left`: every client outl ships reads an
        // unmarked column as left-aligned, and the delimiter row is
        // what preserves the distinction on disk.
        ColumnAlign::None | ColumnAlign::Left => format!("{cell}{}", " ".repeat(slack)),
    }
}

fn rule_for(align: ColumnAlign, width: usize) -> String {
    let width = width.max(MIN_RULE_WIDTH);
    match align {
        ColumnAlign::None => "-".repeat(width),
        ColumnAlign::Left => format!(":{}", "-".repeat(width - 1)),
        ColumnAlign::Right => format!("{}:", "-".repeat(width - 1)),
        ColumnAlign::Center => format!(":{}:", "-".repeat(width - 2)),
    }
}
