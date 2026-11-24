//! The backlinks panel is rebuilt only when something it reads changed
//! (root invariant 15: every keystroke redraws, and a hub page's panel
//! is thousands of lines).
//!
//! Three nets, none covering another:
//! - *still right*: the memoised lines equal the old full render in
//!   every state a key input can move;
//! - *still cheap*: `PanelMemo::builds` counts from-scratch builds, so
//!   "a redraw that changed nothing builds nothing" is structural, not a
//!   timing;
//! - *invalidates*: each input the key holds is moved and has to miss.

use crate::actions::block::test_support::workspace_with_two_backlinks;
use crate::actions::node_id;
use crate::edit_buffer::EditBuffer;
use crate::state::{App, EditTarget, Focus, Mode};
use crate::view::backlinks::{panel, render_backlinks_inline, render_backlinks_uncached};
use outl_core::id::NodeId;

const W: u16 = 80;

fn child_of_first_row(app: &App) -> (usize, NodeId) {
    app.backlinks_for_current()
        .iter()
        .enumerate()
        .find_map(|(idx, bl)| {
            let sub = app.backlink_subtree(&bl.block_id)?;
            Some((idx, node_id(&sub.first()?.id)?))
        })
        .expect("setup: one citing block has a child")
}

fn assert_equivalent(app: &App, why: &str) {
    assert_eq!(
        render_backlinks_inline(app, W),
        render_backlinks_uncached(app, W),
        "memoised lines differ from the full render: {why}"
    );
}

#[test]
fn a_redraw_that_changed_nothing_builds_nothing() {
    let (app, _d) = workspace_with_two_backlinks();

    for _ in 0..5 {
        panel(&app, W);
    }

    assert_eq!(app.panel_memo.builds(), 1);
    assert_equivalent(&app, "idle redraws");
}

#[test]
fn typing_in_the_outline_never_rebuilds_the_panel() {
    let (mut app, _d) = workspace_with_two_backlinks();
    app.focus = Focus::Outline;
    panel(&app, W);

    for n in 0..20 {
        app.mode = Mode::Insert {
            target: EditTarget::CurrentPage,
            block_path: vec![0],
            buffer: EditBuffer::from_text(&"x".repeat(n)),
            original_text: String::new(),
        };
        app.cursor_col = n;
        panel(&app, W);
    }

    assert_eq!(app.panel_memo.builds(), 1);
    assert_equivalent(&app, "after typing above the panel");
}

#[test]
fn every_input_the_key_holds_misses_and_stays_equivalent() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, child) = child_of_first_row(&app);
    panel(&app, W);
    let step = |app: &App, why: &str| {
        let before = app.panel_memo.builds();
        render_backlinks_inline(app, W);
        assert_eq!(app.panel_memo.builds(), before + 1, "{why} must miss");
        assert_equivalent(app, why);
    };

    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };
    step(&app, "focus moved onto a child");

    app.cursor_col = 3;
    step(&app, "the block cursor moved");

    app.mode = Mode::Insert {
        target: EditTarget::SourcePage {
            page: NodeId::new(),
            block: child,
        },
        block_path: Vec::new(),
        buffer: EditBuffer::from_text("half-typed"),
        original_text: String::new(),
    };
    step(&app, "an in-place edit began");
    let drawn = render_backlinks_inline(&app, W).0;
    assert!(drawn
        .iter()
        .any(|l| l.spans.iter().any(|s| s.content.contains("half-typed"))));

    app.mode = Mode::Normal;
    app.backlinks_newest_first = !app.backlinks_newest_first;
    step(&app, "the sort direction flipped");

    app.bump_render_epoch();
    step(&app, "the index or theme changed");

    // Width: a different wrap.
    let before = render_backlinks_inline(&app, 30);
    assert_eq!(before, render_backlinks_uncached(&app, 30));
}

#[test]
fn a_refetched_source_page_shows_its_new_text() {
    let (app, d) = workspace_with_two_backlinks();
    let first = render_backlinks_inline(&app, W);
    std::fs::write(
        d.path().join("pages").join("source.md"),
        "- mentions [[target]] and now says more\n- second [[target]] mention\n",
    )
    .unwrap();

    app.reindex_backlinks_for_slug("source");

    let again = render_backlinks_inline(&app, W);
    assert_ne!(first, again, "the row's text changed, so must the lines");
    assert!(again
        .0
        .iter()
        .any(|l| l.spans.iter().any(|s| s.content.contains("now says more"))));
    assert_equivalent(&app, "after the page was refetched");
}
