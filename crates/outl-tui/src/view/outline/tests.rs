//! Tests for the outline rendering in the parent module.
//!
//! They live in their own file because `outline.rs` sits at the
//! file-size ratchet and the tests are the half that keeps growing.

use super::*;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

fn test_app() -> (App, TempDir) {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let app = App::new(
        dir.path().to_path_buf(),
        ws,
        actor,
        crate::theme::default_theme(),
        false,
    )
    .unwrap();
    (app, dir)
}

/// Concatenate a rendered line's spans into one `String`.
fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Render one block at indent 0 with a leaf bullet and no auto-run,
/// the shape every wrap regression test needs. Keeps each test to
/// its `mode` + `width` instead of repeating the eight-arg call.
fn render_block_lines(app: &App, mode: RenderMode, width: u16) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    emit_block_lines(
        0,
        app.theme.bullet,
        &mode,
        false,
        FoldMarker::None,
        app,
        &mut out,
        width,
    );
    out
}

// The text used by the wrap regression tests: 43 cells of prose
// that cannot fit in a 16-cell pane, so a correct renderer must
// emit more than one visual row.
const LONG: &str = "the quick brown fox jumps over the lazy dog";

/// #99: the selected block in Normal mode used to render on a single
/// overflowing line and only wrap once the cursor moved off it. It
/// must wrap while the cursor sits on it.
#[test]
fn normal_cursor_block_wraps_to_pane_width() {
    let (app, _dir) = test_app();
    let mode = RenderMode::NormalCursor {
        text: LONG.into(),
        cursor_char: 0,
    };
    let out = render_block_lines(&app, mode, 16);
    assert!(out.len() > 1, "expected wrap, got {} line(s)", out.len());
}

/// #99: the same must hold in Insert mode, and the caret has to
/// survive the reflow (it's baked into the spans before wrapping).
/// Asserted on the caret's *style*, not on a `▏` in the text: since
/// #320 the caret marks the cell it precedes and prints no glyph of
/// its own anywhere but past the end of the line.
#[test]
fn editing_block_wraps_and_keeps_the_caret() {
    let (app, _dir) = test_app();
    let mode = RenderMode::Editing {
        text: LONG.into(),
        cursor_char: 0,
    };
    let out = render_block_lines(&app, mode, 16);
    assert!(out.len() > 1, "expected wrap, got {} line(s)", out.len());
    let caret_cells = out
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| s.style == app.theme.cursor_caret_on_char())
        .count();
    assert_eq!(caret_cells, 1, "caret lost after wrap");
}

/// The block cursor travels with its character across a wrap break:
/// a cursor on a word that lands on a continuation row still paints
/// exactly one inverted cell.
#[test]
fn block_cursor_survives_the_wrap_break() {
    let (app, _dir) = test_app();
    // Cursor on the "d" of the trailing "dog", past the first
    // 16-cell row, so it can only be drawn on a continuation row.
    let cursor_char = LONG.len() - 3;
    let mode = RenderMode::NormalCursor {
        text: LONG.into(),
        cursor_char,
    };
    let out = render_block_lines(&app, mode, 16);
    assert!(out.len() > 1, "expected wrap, got {} line(s)", out.len());
    let cursor_cells = out
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| s.style == app.theme.cursor_block)
        .count();
    assert_eq!(cursor_cells, 1, "block cursor must appear exactly once");
}

/// #320: the Insert-mode caret must not cost a terminal column.
/// It used to be a literal `▏` span spliced *between* the text's
/// characters, so every cell right of it shifted one column over
/// and the tail of the line jittered left-right as the cursor
/// moved. The row a caret sits on has to measure exactly what the
/// same row measures with no cursor on it at all.
#[test]
fn the_insert_caret_costs_no_column() {
    let (app, _dir) = test_app();
    let text = "the quick brown fox";
    let bare = render_block_lines(&app, RenderMode::Pretty { text: text.into() }, 0);
    let expected = line_text(&bare[0]).width();
    for cursor_char in 0..text.chars().count() {
        let out = render_block_lines(
            &app,
            RenderMode::Editing {
                text: text.into(),
                cursor_char,
            },
            0,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(
            line_text(&out[0]).width(),
            expected,
            "caret at char {cursor_char} moved the row: {:?}",
            line_text(&out[0]),
        );
    }
}

/// The caret is still *visible* once it stops printing a glyph:
/// it marks the character it sits before, exactly one of them.
#[test]
fn the_insert_caret_marks_the_char_it_sits_before() {
    let (app, _dir) = test_app();
    let out = render_block_lines(
        &app,
        RenderMode::Editing {
            text: "abc".into(),
            cursor_char: 1,
        },
        0,
    );
    let marked: Vec<&Span<'_>> = out[0]
        .spans
        .iter()
        .filter(|s| s.style == app.theme.cursor_caret_on_char())
        .collect();
    assert_eq!(marked.len(), 1, "caret must mark exactly one cell");
    assert_eq!(marked[0].content.as_ref(), "b");
}

/// Past the end of the text there is no character to mark, so the
/// caret keeps its `▏` glyph — appended after the last cell, it
/// has nothing to its right to shift.
#[test]
fn the_insert_caret_past_the_end_still_draws_a_glyph() {
    let (app, _dir) = test_app();
    let out = render_block_lines(
        &app,
        RenderMode::Editing {
            text: "abc".into(),
            cursor_char: 3,
        },
        0,
    );
    assert!(line_text(&out[0]).ends_with('▏'));
}

/// #320, second half: the caret is now the text cell it lands on, and
/// the wrapper is free to throw a space away — it is the separator
/// that pushed the next word onto the following row. Put the two
/// together and a caret parked on a space near a wrap boundary
/// vanished from the screen entirely.
///
/// Walked over every position rather than a hand-picked one: the
/// failing columns were 9 and 19 of `LONG` at width 16, and which
/// columns those are is a function of the wrap arithmetic, not of
/// anything a future reader would think to preserve.
#[test]
fn the_caret_survives_a_wrap_break_at_every_column() {
    let (app, _dir) = test_app();
    for mode in [
        |c| RenderMode::Editing {
            text: LONG.into(),
            cursor_char: c,
        },
        |c| RenderMode::NormalCursor {
            text: LONG.into(),
            cursor_char: c,
        },
    ] {
        for cursor_char in 0..LONG.chars().count() {
            let out = render_block_lines(&app, mode(cursor_char), 16);
            let cells = out
                .iter()
                .flat_map(|l| &l.spans)
                .filter(|s| {
                    s.style == app.theme.cursor_caret_on_char() || s.style == app.theme.cursor_block
                })
                .count();
            assert_eq!(
                cells,
                1,
                "cursor at char {cursor_char} ({:?}) is not on screen:\n{}",
                LONG.chars().nth(cursor_char).unwrap(),
                out.iter().map(line_text).collect::<Vec<_>>().join("\n"),
            );
        }
    }
}

/// A short block under the cursor still renders as a single line:
/// wrapping only kicks in past the pane width, cursor or not.
#[test]
fn short_cursor_block_stays_one_line() {
    let (app, _dir) = test_app();
    let mode = RenderMode::NormalCursor {
        text: "short".into(),
        cursor_char: 0,
    };
    let out = render_block_lines(&app, mode, 80);
    assert_eq!(out.len(), 1);
}

/// End-to-end of the #99 scenario: a page with one long block,
/// selected in Normal mode, rendered through the real
/// `render_outline` entry point into a narrow pane. The selected
/// block must occupy more than one visual line and the continuation
/// must re-indent under the bullet text.
#[test]
fn selected_block_wraps_through_render_outline() {
    let (mut app, _dir) = test_app();
    app.page = outl_md::parse::parse(&format!("- {LONG}"));
    app.selected = 0;
    app.cursor_col = 0;
    app.mode = Mode::Normal;

    let (lines, sel, _starts) = render_outline(&app.page, &app, 20);
    assert_eq!(sel, Some(0), "selected block starts at line 0");
    assert!(
        lines.len() > 1,
        "selected block must wrap, got {} line(s):\n{}",
        lines.len(),
        lines.iter().map(line_text).collect::<Vec<_>>().join("\n"),
    );
    // The continuation re-indents to the text column. Asserted as
    // the exact pad: `starts_with("  ")` holds for any pad two
    // cells or wider, so it kept passing right through #319.
    assert!(line_text(&lines[0]).contains("- "));
    let row = line_text(&lines[1]);
    let pad = row.len() - row.trim_start_matches(' ').len();
    assert_eq!(pad, 4, "continuation pad off the bullet column: {row:?}");
}

/// #319: every row a block owns starts in the block's *text*
/// column. A property row used to pad two cells and land under the
/// fold marker instead, four cells off on an `auto-run::` block.
///
/// Rendered at indent 1 so the `│ ` guides are in the comparison
/// too: both rows build them, in two separate loops.
#[test]
fn a_property_row_starts_in_its_blocks_text_column() {
    let (app, _dir) = test_app();
    let block = |key: &str, children: Vec<OutlineNode>| OutlineNode {
        text: "Define the scope".into(),
        properties: vec![(key.into(), "x".into())],
        children,
    };
    let child = block("priority", Vec::new());
    // (shape, block, the property row's first token after the pad —
    // `auto-run` carries a `property_glyph`, the others don't)
    let cases = [
        ("leaf", block("priority", Vec::new()), "priority:: "),
        ("parent ▼", block("priority", vec![child]), "priority:: "),
        (
            "auto-run ⚡",
            block("auto-run", Vec::new()),
            "▶ auto-run:: ",
        ),
    ];

    for (shape, node, token) in cases {
        let mut out = Vec::new();
        render_block(
            &node,
            1,
            &mut 0,
            &app,
            &mut out,
            &mut None,
            &mut Vec::new(),
            0,
        );
        let column_of = |line: &Line<'_>, token: &str| {
            let text = line_text(line);
            let at = text
                .find(token)
                .unwrap_or_else(|| panic!("{shape}: no {token:?} in {text:?}"));
            text[..at].width()
        };
        assert_eq!(
            column_of(&out[1], token),
            column_of(&out[0], "Define the scope"),
            "{shape}: the property row is not in the block's text column"
        );
    }
}
