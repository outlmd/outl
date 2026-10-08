//! Reading tabular data that was never markdown.
//!
//! A spreadsheet, a SQL client, `column -t`: all of them put
//! **tab-separated** lines on the clipboard, and this is the one place
//! that decides whether such a payload is a grid at all. Both runtimes
//! answer from here — `outl_actions::paste` calls it directly, and
//! `@outl/shared/paste::looksLikeTabular` mirrors its gate so a client
//! can skip the round trip.

use super::{render_table, ColumnAlign, Table};

/// Read tab-separated text — what every spreadsheet puts on the
/// clipboard — as a table, `None` when it does not look tabular.
///
/// The gate is deliberately strict, because the cost of a false
/// positive is a user's pasted text rearranged into a grid they did
/// not ask for:
///
/// - **Two or more lines.** One line of cells has no header to rule
///   off, and a single sentence containing a tab is not a table.
/// - **The same field count on every line**, two or more. A ragged
///   count is prose or code that happens to carry tabs.
/// - **No line starts with a tab.** Tab-indented code and tab-indented
///   outlines both pass the two rules above — this is what separates
///   them from a grid. It costs the rare table whose first column is
///   empty on every row, which re-pastes fine with the first column
///   filled.
///
/// Comma-separated text is **not** read as a table on purpose: prose
/// carries commas, and `a, b` on two lines is far more often two
/// sentences than a 2×2 grid. A user with a `.csv` imports it rather
/// than pasting it.
pub fn from_delimited(text: &str, delim: char) -> Option<Table> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.len() < 2 || lines.iter().any(|l| l.starts_with(delim)) {
        return None;
    }
    let fields: Vec<Vec<&str>> = lines.iter().map(|l| l.split(delim).collect()).collect();
    let width = fields[0].len();
    if width < 2 || fields.iter().any(|row| row.len() != width) {
        return None;
    }
    let cells =
        |row: &Vec<&str>| -> Vec<String> { row.iter().map(|c| escape_cell(c.trim())).collect() };
    Some(Table {
        header: cells(&fields[0]),
        aligns: vec![ColumnAlign::None; width],
        rows: fields[1..].iter().map(cells).collect(),
    })
}

/// Escape what a cell cannot carry literally.
///
/// A `|` inside imported data would open a column that was never
/// there, so it becomes `\|`, and a `\` becomes `\\` so it cannot
/// escape that escape. A newline cannot survive a single-line
/// row at all (a spreadsheet cell can hold one) and becomes a space —
/// the only lossy step in the import, and visible rather than silent.
fn escape_cell(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len());
    for ch in cell.chars() {
        match ch {
            // A backslash already in the source data would otherwise
            // escape **our** escape: `foo\|bar` became `foo\\|bar`, an
            // even run before the pipe, which `split_cells` reads as a
            // real delimiter — three cells where the user had two, with
            // every later value under the wrong header, written to the
            // op log. `\|` is sed/grep alternation, so one column of
            // regexes triggers it; no attacker needed.
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\|"),
            '\n' | '\r' => out.push(' '),
            _ => out.push(ch),
        }
    }
    out
}

/// Tab-separated clipboard text → canonical markdown table, `None`
/// when [`from_delimited`] declines it.
///
/// The one call a paste path needs: it answers "is this tabular" and
/// "what does it look like in our dialect" in one step, so no caller
/// has to decide how to render a [`Table`] for itself.
pub fn tsv_to_markdown(text: &str) -> Option<String> {
    from_delimited(text, '\t').map(|table| {
        let md = render_table(&table);
        // The block's text carries no trailing newline: `render_table`
        // terminates every row, and the renderer adds the line break
        // between a block and its sibling.
        md.trim_end_matches('\n').to_string()
    })
}
