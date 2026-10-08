//! Pipe-delimited markdown tables in the outl dialect.
//!
//! A table is **not** a new block kind, and there is no `Op::Table`.
//! It is a block whose `text` carries the pipe rows, exactly like a
//! fenced code block and a `> ` quote: the dialect stays
//! one-block-per-bullet and the op log never learns a new shape, so
//! nothing here can disagree with [`mod@crate::parse`] about what a block
//! is. On disk a table reads
//!
//! ```text
//! - | Name | Age |
//!   | ---- | --: |
//!   | Ana  |  30 |
//! ```
//!
//! and the block's text is the three rows joined by `\n` — which the
//! existing continuation grammar already round-trips.
//!
//! ## What this module owns
//!
//! - **Recognition.** [`table_span`] answers "do the lines starting
//!   here form a table", which is what lets [`mod@crate::parse`] claim a
//!   pasted table as one block instead of one recovered block per row.
//! - **Decomposition.** [`parse_table`] turns a block's text into
//!   cells plus per-column alignment, so every client renders the same
//!   grid from the same reading.
//! - **Emission.** [`render_table`] writes the canonical, column-padded
//!   form. Used where outl *creates* a table (a tabular paste), never
//!   to reformat what the user typed — see the note below.
//! - **Import.** [`from_delimited`] reads the tab-separated text a
//!   spreadsheet puts on the clipboard.
//!
//! ## Why the parser does not reformat
//!
//! [`parse_table`] keeps the user's bytes. Re-emitting every table
//! through [`render_table`] on read would be a whitespace-only rewrite
//! of a file the user hand-wrote, and the reconcile that follows turns
//! that into an `Op::Edit` per table — churn in the log for a space
//! nobody typed (the same reasoning as `pending_blanks` in
//! [`mod@crate::parse`]). Alignment is a **render** decision: the TUI pads
//! to a grid when it paints, the GUI clients hand the cells to
//! `<table>`, and the bytes on disk stay the ones that were written.
//!
//! ## Nothing is dropped
//!
//! GFM truncates a row that carries more cells than the header. This
//! module does not: [`Table::columns`] is the widest row, short rows
//! pad with empty cells, and a surplus cell keeps its content. A cell
//! the user can see in their editor and cannot see in outl is the
//! failure mode this crate exists to prevent.

use serde::{Deserialize, Serialize};

mod import;
mod render;

pub use import::{from_delimited, tsv_to_markdown};
pub use render::{render_table, render_table_with};

/// Most cells a table may materialise before it stops being read as a
/// grid.
///
/// **A cap, because `Table::columns` is the widest row and every other
/// row pads to it.** That is the deliberate "nothing is dropped" choice
/// (see the module doc), and it costs `columns × rows` — so a *ragged*
/// table is quadratic in a way a rectangular one is not. Measured: a
/// 17 KB block holding a 4,000-cell header over 400 one-cell rows
/// materialises 1.6M `Vec<InlineToken>`, a 4 MB wire payload, and a 9 MB
/// string that the TUI rebuilds **every repaint**. At 109 KB it is
/// gigabytes. The `.md` arrives over iroh / iCloud / Syncthing from
/// another device, so opening the page is the whole interaction, and it
/// re-triggers on every projection.
///
/// Refusing here is **lossless and already a defined outcome**: the
/// block's text is untouched (`table_span` / `take_table` still claim
/// it as one block), and a `None` from [`parse_table`] is what every
/// consumer already handles — the client falls back to `tokens` and
/// shows the pipe rows verbatim, which is what it did before tables
/// were modelled.
///
/// 65,536 is far past any hand-written table (16 columns × 4,096 rows)
/// and far below where the padding hurts.
const MAX_TABLE_CELLS: usize = 65_536;

/// Per-column alignment, read off the delimiter row's colons.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnAlign {
    /// `---` — no colon. Renderers use their own default (left, in
    /// every client outl ships); kept distinct from [`Self::Left`] so
    /// [`render_table`] re-emits the delimiter the user wrote.
    #[default]
    None,
    /// `:---`
    Left,
    /// `:---:`
    Center,
    /// `---:`
    Right,
}

/// A markdown table decomposed into cells.
///
/// Cell strings are **trimmed but otherwise verbatim**: an escaped
/// pipe stays `\|` so [`render_table`] re-emits the source form, and
/// inline markdown inside a cell (`[[ref]]`, `**bold**`) is left for
/// [`crate::tokenize_owned`] — this module does not tokenize.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    /// Header cells, in column order.
    pub header: Vec<String>,
    /// Alignment per column, from the delimiter row. May be shorter
    /// than [`Self::columns`] when a body row is wider than the header.
    pub aligns: Vec<ColumnAlign>,
    /// Body rows. A row may be shorter or longer than the header —
    /// see the module doc on why neither is truncated.
    pub rows: Vec<Vec<String>>,
}

impl Table {
    /// Column count: the widest of the header, the delimiter row and
    /// every body row.
    pub fn columns(&self) -> usize {
        let widest_row = self.rows.iter().map(Vec::len).max().unwrap_or(0);
        self.header.len().max(self.aligns.len()).max(widest_row)
    }

    /// Alignment of column `col`, [`ColumnAlign::None`] past the end
    /// of the delimiter row.
    pub fn align(&self, col: usize) -> ColumnAlign {
        self.aligns.get(col).copied().unwrap_or_default()
    }
}

/// Whether `line` could be a table row: non-blank, carrying an
/// unescaped `|`, and **not already claimed by another construct of
/// the outline grammar**.
///
/// That last clause is the whole function. A table is recognised by
/// shape rather than by a marker, so it is the one construct that can
/// swallow a line meant for something else — and every such line is a
/// line the next parse reads differently from the one that wrote it.
/// Three of them, each found by running shapes through
/// `parse → render → parse`:
///
/// - **A bullet.** `- item | with a pipe` written under a table is a
///   sibling block. Absorbed, it renders back at `indent + 1`, where
///   the next parse reads it as a *child* — so the user's block moves
///   inside the table and the file keeps changing on every save.
/// - **A property.** `status:: x | y` is a `key:: value` line. Absorbed
///   into the text, the reparse lifts it into `properties`, so the
///   block's text differs from the one that was written and the
///   reconcile emits an `Op::Edit` plus a `SetProp` for a line nobody
///   touched.
/// - **A fence opener.** A row starting with ```` ``` ```` opens a
///   fence in the continuation grammar, and the render appends a
///   closing fence on every pass — unbounded growth.
///
/// The first of those is also an **invariant 8** bug, not just churn:
/// `- | -` passes as a delimiter row, and once the table is read with
/// the bullet inside it, `content_lines_missing_from` reports a line
/// the log *does* hold. That withholds `last_synced_hash` and freezes a
/// page with nothing wrong with it — the exact false positive the
/// corpus gate's third property exists to catch.
///
/// So the rule is: **ask this of the delimiter row too**, not only of
/// the header and the body. `is_delimiter_row` answers a question about
/// a cell's shape (`-`, `:-`, `-:`) and `- | -` satisfies it, which is
/// why the two checks are not interchangeable.
fn is_row(line: &str) -> bool {
    let trimmed = line.trim();
    // Cheapest and most discriminating first: almost no line in a
    // workspace carries a pipe, and `block_to_rows` asks this of every
    // visible block on every frame. `parse_property_line` allocates two
    // `String`s when it matches, so running it ahead of this allocated
    // on every `collapsed::` and `remind::` line the outline drew.
    if !has_unescaped_pipe(trimmed) {
        return false;
    }
    if trimmed == "-" || trimmed.starts_with("- ") {
        return false;
    }
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        return false;
    }
    crate::parse::parse_property_line(trimmed).is_none()
}

fn has_unescaped_pipe(s: &str) -> bool {
    let mut escaped = false;
    for ch in s.chars() {
        match ch {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '|' => return true,
            _ => {}
        }
    }
    false
}

/// Whether `line` is a table's delimiter row (`| --- | :-: |`).
///
/// Requires at least one cell and every cell to be a rule: optional
/// leading / trailing colon around one or more dashes. A row of plain
/// text is not one, which is what makes [`table_span`] refuse to read
/// two unrelated pipe-carrying lines as a table.
pub fn is_delimiter_row(line: &str) -> bool {
    let cells = split_cells(line);
    !cells.is_empty() && cells.iter().all(|cell| align_of_rule(cell).is_some())
}

/// Alignment a delimiter cell declares, `None` when it is not a rule.
fn align_of_rule(cell: &str) -> Option<ColumnAlign> {
    let cell = cell.trim();
    let left = cell.starts_with(':');
    let right = cell.ends_with(':') && cell.len() > 1;
    let dashes = &cell[usize::from(left)..cell.len() - usize::from(right)];
    if dashes.is_empty() || !dashes.bytes().all(|b| b == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => ColumnAlign::Center,
        (true, false) => ColumnAlign::Left,
        (false, true) => ColumnAlign::Right,
        (false, false) => ColumnAlign::None,
    })
}

/// How many lines starting at `start` form a table, `None` when they
/// do not.
///
/// A table is a row, a delimiter row, then every following row until
/// the first line that is not one. Two lines is therefore the minimum
/// — a header with a rule under it and no body is still a table, and
/// the user is mid-typing.
///
/// Called by [`mod@crate::parse`] at the point where a line carries no
/// bullet, so the header line is never a block marker; every other line
/// — the delimiter row included — is re-checked (see `is_row`).
pub fn table_span(lines: &[&str], start: usize) -> Option<usize> {
    if !is_row(lines.get(start)?) {
        return None;
    }
    // Both questions, not just the second: see `is_row`'s doc for the
    // three lines a delimiter-shape check alone lets through, and why
    // `- | -` is the one that reaches invariant 8.
    let rule = lines.get(start + 1)?;
    if !is_row(rule) || !is_delimiter_row(rule) {
        return None;
    }
    let mut span = 2;
    while lines.get(start + span).is_some_and(|line| is_row(line)) {
        span += 1;
    }
    Some(span)
}

/// Claim a markdown table starting at `lines[*i]` as one block,
/// advancing the cursor past it. `None` when the lines are not a table.
///
/// `levels` is the indentation the renderer will put back in front of
/// each row (see `crate::render::write_block_text`), so it is
/// stripped here — storing it inside the text makes the renderer write
/// it *after* its own marker, which is how a block's text grows an
/// indent per save.
///
/// Every row is **trimmed**, which is the one place this departs from
/// the verbatim recovery around it. It has to: the continuation arm
/// trims (`node.text.push_str(next_stripped)`), so a row stored with
/// its trailing whitespace would come back different on the next parse
/// and the file would never reach a fixpoint. The first save normalises
/// trailing space inside a table; every save after it is a no-op.
///
/// A table whose rows arrive **over-indented before their parent
/// bullet** is deliberately not claimed here — that arm cannot tell a
/// consistently-indented grid from three unrelated lines, so those rows
/// keep today's one-verbatim-block-per-row recovery. Pinned by
/// `an_over_indented_table_falls_back_to_verbatim_recovery`.
pub(crate) fn take_table(
    lines: &[&str],
    i: &mut usize,
    levels: usize,
) -> Option<crate::parse::OutlineNode> {
    let span = table_span(lines, *i)?;
    let text = lines[*i..*i + span]
        .iter()
        .map(|line| crate::parse::strip_indent_levels(line, levels).trim())
        .collect::<Vec<_>>()
        .join("\n");
    *i += span;
    Some(crate::parse::OutlineNode {
        text,
        properties: Vec::new(),
        children: Vec::new(),
    })
}

/// Read a block's whole `text` as a table, `None` when it is not one.
///
/// Every line must take part: a block whose text is a table plus a
/// trailing sentence is not a table, because rendering it as one would
/// drop the sentence. Leading and trailing blank lines are ignored —
/// they carry no cells either way.
pub fn parse_table(text: &str) -> Option<Table> {
    // Cheap reject before the line split allocates. Every block a
    // projection walks asks this question, and almost none of them
    // carry a pipe at all.
    if !text.contains('|') {
        return None;
    }
    let lines: Vec<&str> = text
        .lines()
        .skip_while(|line| line.trim().is_empty())
        .collect();
    let body_len = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|last| last + 1)?;
    let lines = &lines[..body_len];
    if table_span(lines, 0)? != lines.len() {
        return None;
    }
    let table = Table {
        header: split_cells(lines[0]),
        aligns: split_cells(lines[1])
            .iter()
            .map(|cell| align_of_rule(cell).unwrap_or_default())
            .collect(),
        rows: lines[2..].iter().map(|line| split_cells(line)).collect(),
    };
    // Splitting the cells is linear; **materialising** them is not, so
    // the cap is checked once here, in the single owner of the reading,
    // rather than in each of the four consumers. See `MAX_TABLE_CELLS`.
    if table.columns().saturating_mul(table.rows.len() + 1) > MAX_TABLE_CELLS {
        return None;
    }
    Some(table)
}

/// Split one table row into its cells, trimmed.
///
/// Outer pipes are optional (GFM allows `a | b`); an escaped `\|`
/// splits nothing and keeps its backslash, so the cell re-emits as the
/// user wrote it.
pub fn split_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if !has_unescaped_pipe(trimmed) {
        return Vec::new();
    }
    let body = strip_outer_pipes(trimmed);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut escaped = false;
    for ch in body.chars() {
        if escaped {
            cur.push('\\');
            cur.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '|' => cells.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(ch),
        }
    }
    if escaped {
        // A trailing lone backslash is content, not an escape.
        cur.push('\\');
    }
    cells.push(cur.trim().to_string());
    cells
}

fn strip_outer_pipes(trimmed: &str) -> &str {
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    match inner.strip_suffix('|') {
        // `a\|` ends with an escaped pipe — that is a cell's last
        // character, not the row's closing delimiter.
        Some(without) if !without.ends_with('\\') => without,
        _ => inner,
    }
}

/// A cell as it should be **shown**: the escape in front of a literal
/// pipe is syntax, not content.
///
/// One owner, because both renders ask it — the TUI while it walks an
/// aligned row, and [`tokenize_table`] before it hands a cell to the
/// tokenizer. A client that showed `a \| b` where the other showed
/// `a | b` is invariant 13's shape one construct over.
pub fn unescape_cell(cell: &str) -> String {
    if !cell.contains('\\') {
        return cell.to_string();
    }
    let mut out = String::with_capacity(cell.len());
    let mut escaped = false;
    for ch in cell.chars() {
        if escaped {
            // Ours to consume: the pipe's escape, and a doubled
            // backslash (the producer writes `\\` so a literal `\` in
            // imported data cannot escape the pipe's escape — see
            // `import::escape_cell`). Everything else belongs to
            // whatever reads the cell's markdown, `\n` included.
            if ch != '|' && ch != '\\' {
                out.push('\\');
            }
            out.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    if escaped {
        out.push('\\');
    }
    out
}

/// A table whose cells are pre-tokenized inline markdown, for a client
/// that renders from tokens instead of from text.
///
/// Same bargain as [`crate::parse::OutlineNode`]'s `tokens`: the
/// backend runs the one canonical tokenizer so no client keeps a second
/// one. Without it a GUI would have to split the pipes itself and would
/// render a cell's `[[ref]]` as literal text — the cell is the one place
/// inline markdown would otherwise stop working.
///
/// **Rectangular**: every row and the header are padded to
/// [`Table::columns`], and `aligns` with it, so a renderer can emit
/// `<tr>`s without counting. [`Table`] keeps the ragged reading, which
/// is what preserves a surplus cell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TableView {
    /// Alignment per column, one entry per column.
    pub aligns: Vec<ColumnAlign>,
    /// Header cells, tokenized.
    pub header: Vec<Vec<crate::InlineToken>>,
    /// Body rows, tokenized.
    pub rows: Vec<Vec<Vec<crate::InlineToken>>>,
}

/// Read a block's text as a [`TableView`], `None` when it is not a
/// table.
///
/// The one call a DTO needs: it answers "is this block a table" and
/// "what does a renderer draw" together, so no caller decides for
/// itself which cells exist.
pub fn tokenize_table(text: &str) -> Option<TableView> {
    let table = parse_table(text)?;
    let cols = table.columns();
    let row = |cells: &[String]| -> Vec<Vec<crate::InlineToken>> {
        (0..cols)
            .map(|col| {
                let cell = cells.get(col).map(String::as_str).unwrap_or("");
                crate::tokenize_owned(&cell_for_tokenizer(cell))
            })
            .collect()
    };
    Some(TableView {
        aligns: (0..cols).map(|col| table.align(col)).collect(),
        header: row(&table.header),
        rows: table.rows.iter().map(|cells| row(cells)).collect(),
    })
}

/// A cell's source as the tokenizer should read it.
///
/// [`unescape_cell`] everywhere except inside a code span, where a
/// backslash is literal: there only the pipe's own escape is consumed
/// (GFM does the same), so `` `C:\\server` `` keeps both backslashes.
/// The span boundaries mirror `emphasis::try_code`: one backtick, closed
/// by the next one, non-empty.
fn cell_for_tokenizer(cell: &str) -> String {
    if !cell.contains('`') {
        return unescape_cell(cell);
    }
    let mut out = String::with_capacity(cell.len());
    let mut rest = cell;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        match after.find('`') {
            Some(close) if close > 0 => {
                out.push_str(&unescape_cell(&rest[..open]));
                out.push('`');
                out.push_str(&after[..close].replace("\\|", "|"));
                out.push('`');
                rest = &after[close + 1..];
            }
            _ => break,
        }
    }
    out.push_str(&unescape_cell(rest));
    out
}

/// Every line of every table in `blocks`, counted, for reconcile's
/// bulk-delete guard.
///
/// A sidecar written before tables were modelled holds each row as its
/// own block, so the pass consolidating them orphans every one; on a
/// page that is mostly the table, `OrphanGuard` would refuse forever.
/// The exemption is positive evidence: an orphan's text has to be a
/// table line on disk right now, consumed once per occurrence.
///
/// Uses [`table_span`] rather than [`parse_table`]: the rendering cap
/// says nothing about whether a line is on disk, and applying it here
/// refused the migration of exactly the tables `take_table` still
/// consolidates.
#[derive(Debug, Default)]
pub(crate) struct TableLines(std::cell::RefCell<std::collections::HashMap<String, usize>>);

impl TableLines {
    pub(crate) fn from_blocks(blocks: &[crate::parse::OutlineNode]) -> Self {
        fn walk(
            blocks: &[crate::parse::OutlineNode],
            counts: &mut std::collections::HashMap<String, usize>,
        ) {
            for block in blocks {
                if block.text.contains('|') {
                    let lines: Vec<&str> = block.text.lines().collect();
                    if !lines.is_empty() && table_span(&lines, 0) == Some(lines.len()) {
                        for line in lines {
                            let line = line.trim();
                            if !line.is_empty() {
                                *counts.entry(line.to_string()).or_default() += 1;
                            }
                        }
                    }
                }
                walk(&block.children, counts);
            }
        }
        let mut counts = std::collections::HashMap::new();
        walk(blocks, &mut counts);
        Self(std::cell::RefCell::new(counts))
    }

    /// Whether `text` is a table line on disk not yet claimed by
    /// another orphan. Each line on disk exempts at most one orphan, so
    /// one row cannot vouch for a thousand deleted duplicates.
    pub(crate) fn consume(&self, text: &str) -> bool {
        let needle = text.trim();
        if needle.is_empty() {
            return false;
        }
        let mut counts = self.0.borrow_mut();
        match counts.get_mut(needle) {
            Some(n) if *n > 0 => {
                *n -= 1;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests;
