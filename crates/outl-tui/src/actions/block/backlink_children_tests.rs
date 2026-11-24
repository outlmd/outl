//! The children of a citing block are shown, walkable (`j`/`k`) and
//! editable in the backlinks panel. The cursor names one by id
//! (`Focus::Backlink { idx, node }`) and every gesture acts on it.

use crate::actions::block::test_support::{
    workspace_with_a_self_backlink, workspace_with_two_backlinks,
};
use crate::actions::block::InsertCursor;
use crate::actions::node_id;
use crate::edit_buffer::EditBuffer;
use crate::state::{App, Focus, Mode};
use crate::view::backlinks::render_backlinks_inline;
use outl_core::id::NodeId;

const CHILD_TEXT: &str = "a child of the first citing block";

/// The row of the citing block that has a child, and that child.
fn row_with_a_child(app: &App) -> (usize, NodeId) {
    app.backlinks_for_current()
        .iter()
        .enumerate()
        .find_map(|(idx, bl)| {
            let sub = app.backlink_subtree(&bl.block_id)?;
            Some((idx, node_id(&sub.first()?.id)?))
        })
        .expect("setup: one citing block has a child")
}

fn lines(app: &App) -> (Vec<String>, Option<usize>) {
    let (out, sel) = render_backlinks_inline(app, 80);
    let text = out
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect();
    (text, sel)
}

#[test]
fn children_are_drawn_indented_below_their_citing_block() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, _) = row_with_a_child(&app);
    app.focus = Focus::Backlink {
        idx: row,
        node: None,
    };

    let (text, _) = lines(&app);

    let parent = text
        .iter()
        .position(|l| l.contains("mentions [[target]]") || l.contains("mentions target"))
        .expect("the citing block is drawn");
    let child = text
        .iter()
        .position(|l| l.contains(CHILD_TEXT))
        .expect("the child is drawn");
    assert!(child > parent, "child below its citing block");
    assert!(
        text[child].find(CHILD_TEXT) > text[parent].find("mentions"),
        "child is indented: {:?} vs {:?}",
        text[child],
        text[parent]
    );
}

#[test]
fn only_the_focused_node_is_highlighted() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, child) = row_with_a_child(&app);

    app.focus = Focus::Backlink {
        idx: row,
        node: None,
    };
    let (text, sel_root) = lines(&app);
    let child_line = text.iter().position(|l| l.contains(CHILD_TEXT)).unwrap();
    let root_line = sel_root.expect("a focused row has a selected line");
    assert!(root_line < child_line, "focus on the citing block");

    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };
    let (_, sel_child) = lines(&app);
    assert_eq!(sel_child, Some(child_line), "focus on the child");
}

#[test]
fn j_and_k_walk_root_then_children_then_next_row() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, child) = row_with_a_child(&app);
    app.focus = Focus::Backlink {
        idx: row,
        node: None,
    };

    app.move_selection(1);
    assert_eq!(
        app.focus,
        Focus::Backlink {
            idx: row,
            node: Some(child)
        }
    );
    assert_eq!(app.current_block_text(), CHILD_TEXT);

    app.move_selection(1);
    if row == 0 {
        assert_eq!(app.focus, Focus::Backlink { idx: 1, node: None });
        app.move_selection(-1);
        assert_eq!(
            app.focus,
            Focus::Backlink {
                idx: 0,
                node: Some(child)
            },
            "k from the next row lands on the previous row's last stop"
        );
    }
    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };
    app.move_selection(-1);
    assert_eq!(
        app.focus,
        Focus::Backlink {
            idx: row,
            node: None
        }
    );
}

#[test]
fn the_focus_survives_a_refetch_of_the_source_page() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, child) = row_with_a_child(&app);
    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };

    app.reindex_backlinks_for_slug("source");

    assert_eq!(
        app.focus,
        Focus::Backlink {
            idx: row,
            node: Some(child)
        }
    );
    let target = app.focused_backlink_target().expect("still resolves");
    assert_eq!(target.block, child, "the target is the child, by id");
}

#[test]
fn editing_a_child_commits_on_the_child_id() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, child) = row_with_a_child(&app);
    let parent = node_id(&app.backlinks_for_current()[row].block_id).unwrap();
    let parent_before = app.workspace.block_text(parent);
    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };

    app.enter_insert(InsertCursor::Start);
    let Mode::Insert { buffer, .. } = &mut app.mode else {
        panic!("`i` on a child must enter Insert, got {:?}", app.mode);
    };
    assert_eq!(buffer.as_string(), CHILD_TEXT, "seeded from the child");
    *buffer = EditBuffer::from_text("child, edited");
    app.commit_insert();

    assert_eq!(
        app.workspace.block_text(child).as_deref(),
        Some("child, edited")
    );
    assert_eq!(
        app.workspace.block_text(parent),
        parent_before,
        "parent untouched"
    );
    assert_eq!(
        app.focus,
        Focus::Backlink {
            idx: row,
            node: Some(child)
        },
        "the cursor stays on the child after the refetch"
    );
}

#[test]
fn o_on_a_child_opens_insert_on_the_new_sibling() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, child) = row_with_a_child(&app);
    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };

    app.create_block_below();

    let Mode::Insert { target, .. } = &app.mode else {
        let sub = app.backlink_subtree(&app.backlinks_for_current()[row].block_id);
        panic!(
            "`o` inside a shown subtree opens Insert, got {:?} status {:?} sub {:?}",
            app.mode,
            app.status,
            sub.map(|s| s.iter().map(|n| n.text.clone()).collect::<Vec<_>>())
        );
    };
    let crate::state::EditTarget::SourcePage { block, .. } = target else {
        panic!("the edit targets the source page, got {target:?}");
    };
    assert_ne!(*block, child, "on the new block, not the old one");
    assert!(matches!(&app.focus, Focus::Backlink { node: Some(n), .. } if n == block));
}

#[test]
fn o_on_a_citing_block_still_declines_insert() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, _) = row_with_a_child(&app);
    app.focus = Focus::Backlink {
        idx: row,
        node: None,
    };

    app.create_block_below();

    assert!(
        matches!(app.mode, Mode::Normal),
        "a sibling of the citing block cites nothing"
    );
    assert!(
        app.status.contains("not editable from the panel"),
        "{:?}",
        app.status
    );
}

#[test]
fn a_gesture_on_a_child_acts_on_the_child() {
    let (mut app, _d) = workspace_with_two_backlinks();
    let (row, child) = row_with_a_child(&app);
    app.focus = Focus::Backlink {
        idx: row,
        node: Some(child),
    };

    app.delete_current();

    assert!(
        !outl_actions::is_live(&app.workspace, child),
        "the child went to the trash"
    );
    let parent = node_id(&app.backlinks_for_current()[row].block_id).unwrap();
    assert!(
        outl_actions::is_live(&app.workspace, parent),
        "the citing block did not"
    );
    assert_eq!(
        app.focus,
        Focus::Backlink {
            idx: row,
            node: None
        },
        "a node the subtree lost falls back to the citing block"
    );
}

#[test]
fn a_self_citing_page_with_no_children_behaves_as_before() {
    let (app, _d, _p) = workspace_with_a_self_backlink();
    let t = app.focused_backlink_target().expect("resolves");
    let bl = &app.backlinks_for_current()[0];
    assert_eq!(t.block.to_string(), bl.block_id);
}
