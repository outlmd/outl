use super::*;
use outl_actions::Backlink;
use outl_core::id::ActorId;
use outl_core::workspace::Workspace;
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

fn test_app() -> (App, TempDir) {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let app = App::new_for_tests(
        dir.path().to_path_buf(),
        ws,
        actor,
        crate::theme::default_theme(),
        false,
    )
    .unwrap();
    (app, dir)
}

fn row_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// A citing block carrying a real ULID id, which `node`'s fixed
/// `blk-test01` is not — `node_id` refuses that, so the id comparison
/// under test would answer "no" for every row.
fn citing_row() -> (Backlink, outl_core::id::NodeId) {
    let id = outl_core::id::NodeId::new();
    let mut source_block = node("cites the page", Vec::new());
    source_block.id = id.to_string();
    (backlink(source_block), id)
}

/// The in-flight cross-page Insert state, naming `editing` as the block
/// under the caret — what `enter_insert_backlink` leaves behind.
fn editing_mode(editing: outl_core::id::NodeId) -> Mode {
    Mode::Insert {
        target: EditTarget::SourcePage {
            page: outl_core::id::NodeId::new(),
            block: editing,
        },
        block_path: Vec::new(),
        buffer: crate::edit_buffer::EditBuffer::from_text("half-typed"),
        original_text: "cites the page".to_string(),
    }
}

fn render_focused(app: &App, bl: &Backlink) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    render_backlink_node(
        &bl.source_block,
        0,
        crate::actions::node_id(&bl.source_block.id),
        app,
        &mut out,
        &mut None,
        0,
    );
    out
}

/// Typing into a backlink row draws the live buffer, and **which** row
/// that is comes from the block's id.
#[test]
fn the_edited_backlink_row_draws_the_live_buffer() {
    let (mut app, _dir) = test_app();
    let (bl, id) = citing_row();
    app.mode = editing_mode(id);

    let out = render_focused(&app, &bl);

    assert!(
        row_text(&out[0]).contains("half-typed"),
        "the row the target names must render the in-flight buffer, got {:?}",
        row_text(&out[0])
    );
}

/// And a row the target does *not* name keeps its own text. The
/// predicate this replaced compared a reconstructed `block_path`, so a
/// block that merely shared a path answered yes.
#[test]
fn a_row_the_target_does_not_name_keeps_its_own_text() {
    let (mut app, _dir) = test_app();
    let (bl, _own_id) = citing_row();
    app.mode = editing_mode(outl_core::id::NodeId::new());

    let out = render_focused(&app, &bl);

    let row = row_text(&out[0]);
    assert!(row.contains("cites the page"), "got {row:?}");
    assert!(
        !row.contains("half-typed"),
        "another block's buffer was painted onto this row: {row:?}"
    );
}

fn node(text: &str, properties: Vec<(String, String)>) -> OutlineNode {
    OutlineNode {
        id: "blk-test01".into(),
        text: text.into(),
        todo: None,
        collapsed: false,
        properties,
        tokens: Vec::new(),
        table: None,
        children: Vec::new(),
    }
}

fn backlink(source_block: OutlineNode) -> Backlink {
    Backlink {
        block_id: source_block.id.clone(),
        block_text: source_block.text.clone(),
        todo: None,
        source_page: None,
        source_block,
        source_block_path: Vec::new(),
        ancestors: Vec::new(),
        source_path: None,
    }
}

/// Zooming into a block is a request for that subtree and nothing
/// else. The backlinks belong to the **page**, so under a zoom —
/// where the header is a block — they are an answer to a question
/// nobody asked, about something other than what is on screen.
///
/// Driven through the pending-index notice because that is the one
/// section state reachable without building an index: it proves the
/// renderer asks `page_sections_visible`, which a test of the
/// predicate alone cannot.
#[test]
fn a_zoom_takes_the_pages_backlinks_off_screen() {
    let (mut app, _dir) = test_app();
    let (_tx, rx) = std::sync::mpsc::channel();
    app.backlink_index_rx = Some(rx);
    assert!(
        !render_backlinks_inline(&app, 40).0.is_empty(),
        "the section is on screen on the full page"
    );

    app.zoom_stack.push(vec![0]);

    assert!(
        render_backlinks_inline(&app, 40).0.is_empty(),
        "a zoom hides the page's backlinks"
    );
}

/// The breadcrumb is the one line on the panel that used to be
/// drawn as source: a branch reading `#alinhamento [[buser/…]] …`
/// above a citing block that reads as prose. Dimming it was never
/// a reason not to parse it.
#[test]
fn a_crumb_trail_renders_its_markdown_instead_of_the_source() {
    let (app, _dir) = test_app();
    let branch = "#alinhamento [[buser/team/vendas]] internet 0800";
    let crumbs = vec![BacklinkCrumb {
        id: "blk-anc001".into(),
        text: branch.into(),
        tokens: outl_md::tokenize_owned(branch),
    }];

    let text = row_text(&render_crumb_trail(&crumbs, &app));

    assert!(text.contains("buser/team/vendas"), "{text:?}");
    assert!(text.contains("#alinhamento"), "{text:?}");
    assert!(!text.contains("[["), "{text:?}");
    assert!(!text.contains("]]"), "{text:?}");
}

/// Dim is the **line** style, so prose takes it while a ref keeps
/// the hue the tokenizer's render gave it. A per-span dim would
/// flatten the trail back to one colour and lose what the tokens
/// are; styling nothing would make it as loud as the block below.
#[test]
fn a_crumb_trail_dims_its_prose_and_keeps_its_ref_colour() {
    let (app, _dir) = test_app();
    let text = "ver [[hub]]";
    let crumbs = vec![BacklinkCrumb {
        id: "blk-anc001".into(),
        text: text.into(),
        tokens: outl_md::tokenize_owned(text),
    }];

    let line = render_crumb_trail(&crumbs, &app);

    assert_eq!(line.style, app.theme.dim);
    let hub = line
        .spans
        .iter()
        .find(|s| s.content.as_ref() == "hub")
        .expect("the ref renders as its name");
    assert_eq!(hub.style, app.theme.ref_link);
}

/// The separator carries the same `›` every GUI client draws
/// (`CRUMB_SEPARATOR` in `@outl/shared/markdown`).
#[test]
fn a_crumb_trail_joins_its_steps_with_one_separator() {
    let (app, _dir) = test_app();
    let crumb = |text: &str| BacklinkCrumb {
        id: text.into(),
        text: text.into(),
        tokens: outl_md::tokenize_owned(text),
    };

    let text = row_text(&render_crumb_trail(
        &[crumb("parent"), crumb("child")],
        &app,
    ));

    assert_eq!(text, "  parent › child");
}

/// #319, third caller. `render_block` has the same rule and its own
/// test; this loop is a separate copy of the geometry, and it is the
/// copy that had already drifted (it never drew `property_glyph`).
/// A regression here would be invisible to the outline's test.
#[test]
fn a_backlink_property_row_starts_in_its_blocks_text_column() {
    let (app, _dir) = test_app();
    let bl = backlink(node(
        "cites the page",
        vec![("priority".into(), "high".into())],
    ));

    let mut out = Vec::new();
    render_backlink_node(&bl.source_block, 1, None, &app, &mut out, &mut None, 0);

    let column_of = |line: &Line<'_>, token: &str| {
        let text = row_text(line);
        let at = text
            .find(token)
            .unwrap_or_else(|| panic!("no {token:?} in {text:?}"));
        text[..at].width()
    };
    assert_eq!(
        column_of(&out[1], "priority:: "),
        column_of(&out[0], "cites the page"),
        "the backlink property row is not in the block's text column"
    );
}

/// The outline and this mini-outline are one render now
/// (`push_property_row`), so a `remind::` reads the same in both
/// panes. It did not before: this loop never drew the glyph, and
/// nothing failed when the two disagreed.
#[test]
fn a_backlink_property_row_draws_the_same_glyph_the_outline_does() {
    let (app, _dir) = test_app();
    let bl = backlink(node(
        "cites the page",
        vec![("remind".into(), "9am".into())],
    ));

    let mut out = Vec::new();
    render_backlink_node(&bl.source_block, 0, None, &app, &mut out, &mut None, 0);

    let mut expected = Vec::new();
    push_property_row(0, "remind", "9am", false, &app, &mut expected, 0);
    assert_eq!(row_text(&out[1]), row_text(&expected[0]));
    assert!(row_text(&out[1]).contains('⏰'), "{:?}", row_text(&out[1]));
}

/// A property value too wide for the pane wraps instead of being
/// clipped at the right edge with nothing to say it was.
#[test]
fn a_long_property_value_wraps_instead_of_being_cut() {
    let (app, _dir) = test_app();
    let bl = backlink(node(
        "cites the page",
        vec![(
            "template".into(),
            "the quick brown fox jumps over the lazy dog".into(),
        )],
    ));

    let mut out = Vec::new();
    render_backlink_node(&bl.source_block, 0, None, &app, &mut out, &mut None, 24);

    let rows: Vec<String> = out[1..].iter().map(row_text).collect();
    assert!(rows.len() > 1, "expected a wrap, got {rows:?}");
    assert!(
        rows.concat().contains("lazy dog"),
        "the tail of the value was dropped: {rows:?}"
    );
}
