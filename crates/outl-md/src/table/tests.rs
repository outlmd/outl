use super::*;

fn lines(md: &str) -> Vec<&str> {
    md.lines().collect()
}

// ---- recognition -------------------------------------------------------

#[test]
fn a_header_and_a_rule_are_a_table() {
    let md = "| a | b |\n| --- | --- |";
    assert_eq!(table_span(&lines(md), 0), Some(2));
}

#[test]
fn body_rows_extend_the_span() {
    let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |";
    assert_eq!(table_span(&lines(md), 0), Some(4));
}

#[test]
fn a_header_without_a_rule_is_not_a_table() {
    // Two pipe-carrying lines that happen to sit together are prose,
    // not a grid — the rule is what declares the intent.
    let md = "a | b\nc | d";
    assert_eq!(table_span(&lines(md), 0), None);
}

#[test]
fn a_rule_alone_is_not_a_table() {
    let md = "| --- | --- |";
    assert_eq!(table_span(&lines(md), 0), None);
}

#[test]
fn outer_pipes_are_optional() {
    let md = "a | b\n--- | ---\n1 | 2";
    assert_eq!(table_span(&lines(md), 0), Some(3));
}

#[test]
fn the_span_stops_at_a_bullet_that_carries_a_pipe() {
    // A sibling block written under a table must stay a block. Reading
    // it as a fourth row would move the user's bullet inside the grid.
    let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n- item | with a pipe";
    assert_eq!(table_span(&lines(md), 0), Some(3));
}

#[test]
fn the_span_stops_at_a_blank_line() {
    let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n\n| x | y |";
    assert_eq!(table_span(&lines(md), 0), Some(3));
}

#[test]
fn the_span_stops_at_a_line_with_no_pipe() {
    let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\nplain trailing sentence";
    assert_eq!(table_span(&lines(md), 0), Some(3));
}

#[test]
fn a_span_can_start_mid_slice() {
    let md = "intro\n| a | b |\n| --- | --- |";
    assert_eq!(table_span(&lines(md), 1), Some(2));
    assert_eq!(table_span(&lines(md), 0), None);
}

#[test]
fn delimiter_row_accepts_every_colon_form() {
    assert!(is_delimiter_row("| --- | :-- | :-: | --: |"));
    assert!(is_delimiter_row("|-|-|"));
    assert!(is_delimiter_row("--- | ---"));
}

#[test]
fn delimiter_row_rejects_a_cell_that_is_not_a_rule() {
    assert!(!is_delimiter_row("| --- | x |"));
    assert!(!is_delimiter_row("| | --- |"));
    assert!(!is_delimiter_row("| :: | --- |"));
    assert!(!is_delimiter_row("| : | --- |"));
    assert!(!is_delimiter_row("no pipes here"));
}

// ---- lines another construct owns --------------------------------------
//
// A table is recognised by shape, not by a marker, so it is the one
// construct that can swallow a line meant for something else. Each of
// these was found by running shapes through `parse → render → parse`,
// and each one made the file change on a later save; the first also
// made `content_lines_missing_from` report a line the log did hold,
// which freezes the page (invariant 8).

#[test]
fn a_bullet_shaped_delimiter_row_is_not_a_delimiter_row() {
    // `- | -` satisfies "every cell is a rule" and is also a bullet.
    // The bullet wins: reading the table around it renders the bullet
    // back at indent + 1, where the next parse makes it a child.
    assert_eq!(table_span(&lines("a | b\n- | -\n1 | 2"), 0), None);
    assert_eq!(table_span(&lines("a |\n- |"), 0), None);
    // `is_delimiter_row` still answers its own narrower question — the
    // two checks are not interchangeable, which is why `table_span`
    // asks both.
    assert!(is_delimiter_row("- | -"));
}

#[test]
fn a_property_line_is_not_a_body_row() {
    // `status:: x | y` under a table is a `key:: value` line. Absorbed,
    // the reparse lifts it into `properties`, so the block's text stops
    // matching what was written.
    let md = "| a | b |\n| - | - |\nstatus:: x | y";
    assert_eq!(table_span(&lines(md), 0), Some(2));
}

#[test]
fn a_fence_opener_is_not_a_body_row() {
    // A row starting with a fence opens one in the continuation
    // grammar, and the render appends a closing fence every pass.
    let md = "| a | b |\n| - | - |\n```| x |";
    assert_eq!(table_span(&lines(md), 0), Some(2));
    assert_eq!(table_span(&lines("| a |\n| - |\n~~~| x |"), 0), Some(2));
}

#[test]
fn a_real_delimiter_row_still_passes_both_checks() {
    // The guard above must not cost the ordinary forms. `--- | ---`
    // starts with `--`, not `- `, so it is not a bullet.
    for md in [
        "| a | b |\n| --- | --- |",
        "a | b\n--- | ---",
        "| a | b |\n|-|-|",
        "| a | b | c | d |\n| --- | :-- | :-: | --: |",
    ] {
        assert_eq!(table_span(&lines(md), 0), Some(2), "rejected {md:?}");
    }
}

// ---- decomposition -----------------------------------------------------

#[test]
fn cells_are_trimmed() {
    assert_eq!(split_cells("|  a  |  b  |"), vec!["a", "b"]);
    assert_eq!(split_cells("a|b"), vec!["a", "b"]);
}

#[test]
fn an_empty_cell_survives() {
    assert_eq!(split_cells("| a |  | c |"), vec!["a", "", "c"]);
}

#[test]
fn an_escaped_pipe_does_not_split_and_keeps_its_backslash() {
    // The backslash is what makes `render_table` re-emit the source
    // form, so the cell round-trips instead of growing a column.
    assert_eq!(split_cells(r"| a \| b | c |"), vec![r"a \| b", "c"]);
}

#[test]
fn a_trailing_escaped_pipe_is_not_the_row_delimiter() {
    assert_eq!(split_cells(r"| a | b \|"), vec!["a", r"b \|"]);
}

#[test]
fn a_lone_trailing_backslash_is_content() {
    assert_eq!(split_cells(r"| a | b \"), vec!["a", r"b \"]);
}

#[test]
fn parse_reads_header_aligns_and_rows() {
    let table = parse_table("| Name | Age |\n| :--- | --: |\n| Ana | 30 |").expect("a table");
    assert_eq!(table.header, vec!["Name", "Age"]);
    assert_eq!(table.aligns, vec![ColumnAlign::Left, ColumnAlign::Right]);
    assert_eq!(table.rows, vec![vec!["Ana".to_string(), "30".into()]]);
    assert_eq!(table.columns(), 2);
}

#[test]
fn parse_declines_text_that_is_only_partly_a_table() {
    // Rendering this as a table would drop the sentence.
    assert!(parse_table("| a | b |\n| --- | --- |\nand a trailing sentence").is_none());
    assert!(parse_table("intro\n| a | b |\n| --- | --- |").is_none());
    assert!(parse_table("just prose").is_none());
    assert!(parse_table("").is_none());
}

#[test]
fn parse_ignores_surrounding_blank_lines() {
    let table = parse_table("\n| a | b |\n| --- | --- |\n\n").expect("a table");
    assert_eq!(table.header, vec!["a", "b"]);
    assert!(table.rows.is_empty());
}

#[test]
fn a_surplus_cell_is_kept_not_truncated() {
    // GFM drops the third cell. We keep it: a cell the user can see in
    // their editor and not in outl is the loss this crate exists to
    // prevent.
    let table = parse_table("| a | b |\n| --- | --- |\n| 1 | 2 | 3 |").expect("a table");
    assert_eq!(table.columns(), 3);
    assert_eq!(table.rows[0], vec!["1", "2", "3"]);
    assert_eq!(table.align(2), ColumnAlign::None);
}

#[test]
fn a_short_row_keeps_its_own_length() {
    let table = parse_table("| a | b | c |\n| --- | --- | --- |\n| 1 |").expect("a table");
    assert_eq!(table.columns(), 3);
    assert_eq!(table.rows[0], vec!["1"]);
}

// ---- emission ----------------------------------------------------------

#[test]
fn rendering_pads_every_column_to_its_widest_cell() {
    let table = parse_table("| Name | Age |\n| --- | --- |\n| Ana | 30 |\n| Roberta | 7 |")
        .expect("a table");
    assert_eq!(
        render_table(&table),
        "\
| Name    | Age |
| ------- | --- |
| Ana     | 30  |
| Roberta | 7   |
"
    );
}

#[test]
fn rendering_re_emits_the_alignment_markers() {
    let table = parse_table("| a | b | c | d |\n| --- | :-- | :-: | --: |").expect("a table");
    assert_eq!(
        render_table(&table),
        "\
| a   | b   |  c  |   d |
| --- | :-- | :-: | --: |
"
    );
}

#[test]
fn rendering_pads_a_short_row_with_empty_cells() {
    let table = parse_table("| a | b | c |\n| --- | --- | --- |\n| 1 |").expect("a table");
    assert_eq!(
        render_table(&table),
        "\
| a   | b   | c   |
| --- | --- | --- |
| 1   |     |     |
"
    );
}

#[test]
fn rendering_is_stable_under_reparse() {
    // Padding a short row changes the cell count, so `parse ∘ render`
    // is not the identity. What has to hold is that the *output* is
    // fixed from the first pass — otherwise a table on disk would
    // reflow on every save.
    for md in [
        "| a | b |\n| --- | --- |\n| 1 | 2 |",
        "| a | b | c |\n| --- | --- | --- |\n| 1 |",
        "| a | b |\n| --- | --- |\n| 1 | 2 | 3 |",
        "a | b\n--- | ---\n1 | 2",
        "| a | b |\n| --- | --- |\n| x \\| y | z |",
    ] {
        let once = render_table(&parse_table(md).expect("a table"));
        let twice = render_table(&parse_table(&once).expect("a re-read table"));
        assert_eq!(once, twice, "unstable for {md:?}");
    }
}

#[test]
fn rendering_aligns_by_display_width_not_char_count() {
    // `日本語` is 3 chars but 6 columns wide; padding by char count
    // would leave the grid three cells short.
    let table = parse_table("| k | v |\n| --- | --- |\n| 日本語 | x |").expect("a table");
    assert_eq!(
        render_table(&table),
        "\
| k      | v   |
| ------ | --- |
| 日本語 | x   |
"
    );
}

#[test]
fn rendering_never_truncates_a_cell_wider_than_its_column() {
    // Only reachable through a hand-built `Table`; the parser always
    // measures what it read. Pinned because truncating here is how a
    // table silently eats a long cell.
    let table = Table {
        header: vec!["k".into()],
        aligns: vec![ColumnAlign::None],
        rows: vec![vec!["a very long cell".into()]],
    };
    assert!(render_table(&table).contains("a very long cell"));
}

#[test]
fn rendering_an_empty_table_is_empty() {
    assert_eq!(render_table(&Table::default()), "");
}

// ---- import ------------------------------------------------------------

#[test]
fn tab_separated_text_becomes_a_table() {
    let tsv = "Name\tAge\nAna\t30\nRoberta\t7";
    assert_eq!(
        tsv_to_markdown(tsv).expect("tabular"),
        "\
| Name    | Age |
| ------- | --- |
| Ana     | 30  |
| Roberta | 7   |"
    );
}

#[test]
fn tabular_import_carries_no_trailing_newline() {
    // The block's text ends at its last row; the renderer owns the
    // line break between a block and its sibling.
    let md = tsv_to_markdown("a\tb\n1\t2").expect("tabular");
    assert!(!md.ends_with('\n'));
}

#[test]
fn a_pipe_in_imported_data_is_escaped() {
    let table = from_delimited("k\tv\na|b\tc", '\t').expect("tabular");
    assert_eq!(table.rows[0], vec![r"a\|b", "c"]);
    // And it survives the round trip back through the parser as one cell.
    let reread = parse_table(&render_table(&table)).expect("a table");
    assert_eq!(reread.rows[0], vec![r"a\|b", "c"]);
}

#[test]
fn ragged_field_counts_are_not_tabular() {
    // Prose or code that happens to carry tabs.
    assert!(tsv_to_markdown("a\tb\nc\td\te").is_none());
}

#[test]
fn a_single_line_is_not_tabular() {
    // No header to rule off.
    assert!(tsv_to_markdown("a\tb\tc").is_none());
}

#[test]
fn tab_indented_text_is_not_tabular() {
    // The regression this gate exists for: tab-indented code and
    // tab-indented outlines both have a consistent field count, and
    // rearranging either into a grid destroys it.
    assert!(tsv_to_markdown("\tif x:\n\treturn y").is_none());
    assert!(tsv_to_markdown("\tone\ttwo\n\tthree\tfour").is_none());
}

#[test]
fn text_with_no_tabs_is_not_tabular() {
    assert!(tsv_to_markdown("one line\nanother line").is_none());
}

#[test]
fn comma_separated_text_is_not_read_as_tabular() {
    // Deliberate: prose carries commas. `from_delimited` can be asked
    // for `,` explicitly (a future `outl import csv`), but no paste
    // path does.
    assert!(tsv_to_markdown("a, b\nc, d").is_none());
}

#[test]
fn an_imported_table_has_no_alignment_markers() {
    let table = from_delimited("a\tb\n1\t2", '\t').expect("tabular");
    assert_eq!(table.aligns, vec![ColumnAlign::None, ColumnAlign::None]);
}

// ---- the tokenized reading --------------------------------------------

#[test]
fn a_cells_inline_markdown_is_tokenized() {
    // The cell is the one place inline markdown would otherwise stop
    // working, because a client splitting pipes itself has no tokenizer.
    let view =
        tokenize_table("| k | v |\n| --- | --- |\n| [[page]] | **bold** |").expect("a table");
    assert!(matches!(view.rows[0][0][0], crate::InlineToken::Ref { .. }));
    assert!(matches!(
        view.rows[0][1][0],
        crate::InlineToken::Bold { .. }
    ));
}

#[test]
fn the_tokenized_reading_is_rectangular() {
    // A renderer emits `<tr>`s without counting, so every row — and
    // `aligns` — carries one entry per column.
    let view = tokenize_table("| a | b |\n| --- | --- |\n| 1 |\n| 1 | 2 | 3 |").expect("a table");
    assert_eq!(view.aligns.len(), 3);
    assert_eq!(view.header.len(), 3);
    for row in &view.rows {
        assert_eq!(row.len(), 3);
    }
}

#[test]
fn a_padded_cell_tokenizes_to_nothing() {
    let view = tokenize_table("| a | b |\n| --- | --- |\n| 1 |").expect("a table");
    assert!(view.rows[0][1].is_empty());
}

#[test]
fn an_escaped_pipe_is_shown_as_a_pipe() {
    let view = tokenize_table("| k |\n| --- |\n| a \\| b |").expect("a table");
    assert_eq!(
        view.rows[0][0],
        vec![crate::InlineToken::Plain {
            value: "a | b".to_string()
        }]
    );
}

#[test]
fn unescape_only_consumes_the_pipes_escape() {
    // `\n` and friends belong to whatever reads the cell's markdown.
    assert_eq!(unescape_cell(r"a \| b"), "a | b");
    assert_eq!(unescape_cell(r"a \n b"), r"a \n b");
    assert_eq!(unescape_cell("plain"), "plain");
    assert_eq!(unescape_cell(r"trailing \"), r"trailing \");
}

#[test]
fn prose_has_no_tokenized_table() {
    assert!(tokenize_table("a | b\nc | d").is_none());
}

// ---- the materialisation cap -------------------------------------------

#[test]
fn a_ragged_table_past_the_cell_cap_is_not_read_as_a_grid() {
    // `columns()` is the widest row and every row pads to it, so a wide
    // header over many short rows costs `columns × rows`. Measured
    // before the cap: a 17 KB block produced 1.6M cell vectors, a 4 MB
    // wire payload, and a 9 MB string the TUI rebuilt every repaint.
    let mut md = String::from("|");
    for _ in 0..4_000 {
        md.push_str(" x |");
    }
    md.push_str("\n| --- |");
    for _ in 0..400 {
        md.push_str("\n| y |");
    }
    assert!(
        parse_table(&md).is_none(),
        "4,000 x 400 must not materialise"
    );
    // Lossless: the rows are still one block, and the text is untouched.
    let lines: Vec<&str> = md.lines().collect();
    assert_eq!(table_span(&lines, 0), Some(lines.len()));
    assert!(tokenize_table(&md).is_none());
}

#[test]
fn a_table_a_human_would_write_is_under_the_cap() {
    // The cap must never be reachable by hand. 16 columns x 400 rows is
    // already an unusual dump and sits well inside it.
    let header = format!("|{}", " h |".repeat(16));
    let row = format!("|{}", " c |".repeat(16));
    let mut md = format!("{header}\n| --- |");
    for _ in 0..400 {
        md.push('\n');
        md.push_str(&row);
    }
    let table = parse_table(&md).expect("an ordinary large table still reads");
    assert_eq!(table.columns(), 16);
    assert_eq!(table.rows.len(), 400);
}

#[test]
fn a_backslash_in_imported_data_cannot_escape_our_escape() {
    // `foo\|bar` in a spreadsheet cell (sed/grep alternation — ordinary
    // data, no attacker needed). Escaping only the pipe produced
    // `foo\\|bar`: an **even** backslash run, which `split_cells` reads
    // as a real delimiter. Three cells where the user had two, every
    // later value under the wrong header, written to the op log.
    let table = from_delimited("k\tv\nfoo\\|bar\tsafe", '\t').expect("tabular");
    let reread = parse_table(&render_table(&table)).expect("a table");
    assert_eq!(reread.header.len(), 2);
    assert_eq!(reread.rows[0].len(), 2, "the row must not gain a column");
    // And it displays as the one backslash the source had.
    let view = tokenize_table(&render_table(&table)).expect("a table");
    assert_eq!(
        view.rows[0][0],
        vec![crate::InlineToken::Plain {
            value: "foo\\|bar".to_string()
        }]
    );
}

#[test]
fn unescape_collapses_a_doubled_backslash() {
    // The matching half of `escape_cell`'s doubling. Anything else keeps
    // its backslash — `\n` belongs to whatever reads the cell's markdown.
    assert_eq!(unescape_cell(r"a \\ b"), r"a \ b");
    assert_eq!(unescape_cell(r"a \| b"), "a | b");
    assert_eq!(unescape_cell(r"a \n b"), r"a \n b");
}

#[test]
fn a_cell_that_looks_like_a_property_ends_the_table() {
    // Genuinely ambiguous: `title:: x | the page name` is both a row of
    // a pipe-less-outer table and a `key:: value` line. The property
    // wins, because outl writes properties itself (`collapsed::`,
    // `remind::`) and absorbing one into a table's text would make the
    // reparse disagree with the parse.
    //
    // Cost: a table **without outer pipes** whose first cell starts
    // `key:: ` stops there, and the row becomes its own recovered block
    // (nothing is lost, and a warning says so). With outer pipes the
    // key is `| title`, which is not a valid key, so the row is kept —
    // which is the shape to prefer when writing such a table.
    let without = "k | v\n--- | ---\ntitle:: x | the page name";
    assert_eq!(table_span(&lines(without), 0), Some(2));

    let with_pipes = "| k | v |\n| --- | --- |\n| title:: x | the page name |";
    let table = parse_table(with_pipes).expect("outer pipes keep the row");
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.rows[0][0], "title:: x");
}
