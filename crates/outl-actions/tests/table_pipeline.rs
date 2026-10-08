//! A table written on disk reaches a client as a table.
//!
//! The unit tests pin each hop (`outl_md::table` reads the cells,
//! `parse` claims the rows as one block, `project_node` attaches the
//! tokenized reading). What they cannot pin is the hop *count*: a
//! table has to survive `.md` → `reconcile_md` → op log → tree →
//! `OutlineNode`, and every one of those is a place a multi-line block
//! has historically been split, truncated or reflowed.
//!
//! The specific failure this guards against is the one issue #329
//! describes: a table arriving as one block per row. That is visible on
//! disk (five bullets where there was a grid) and in the op log (five
//! `Create`s), and neither a parser test nor a renderer test can see
//! it, because both of them stop before the log.

use std::path::Path;
use std::str::FromStr;

use outl_actions::{find_by_slug, list_pages, project_outline, PasteAnchor};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

const TABLE: &str = "\
| Route   | Pax | Owner       |
| ------- | --: | :---------- |
| SP → RJ | 1203 | [[avelino]] |
| BH \\| CWB | 12 |             |
";

/// Write `md` as `pages/report.md`, reconcile it into a fresh
/// workspace, and return the workspace, its root, and a clock bound to
/// the **same** actor.
///
/// The actor has to match: `Workspace`'s batch buffer asserts that
/// every op in it came from the local actor, so a test that mints a
/// second `ActorId` for its clock panics inside the first batched
/// mutation rather than failing on the thing it meant to check.
fn ingest(md: &str) -> (Workspace, HlcGenerator, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("pages")).expect("pages dir");
    std::fs::create_dir_all(root.join("ops")).expect("ops dir");
    std::fs::write(root.join("pages/report.md"), md).expect("write page");

    let actor = ActorId::new();
    let storage = JsonlStorage::open(root.join("ops"), actor).expect("storage");
    let mut workspace =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
            .expect("workspace");
    let hlc = HlcGenerator::new(actor);

    // What the file watcher and `outl serve` run on a changed `.md`.
    outl_md::reconcile_md(&mut workspace, &hlc, &root.join("pages/report.md"), None)
        .expect("reconcile");
    (workspace, hlc, dir)
}

/// Root-level blocks of the one page, as a client sees them.
fn blocks(workspace: &Workspace) -> Vec<outl_actions::outline::OutlineNode> {
    let page = find_by_slug(workspace, "report").expect("the page is in the tree");
    project_outline(workspace, page)
}

#[test]
fn a_table_on_disk_reaches_a_client_as_one_block_carrying_a_table() {
    let (workspace, _hlc, _dir) = ingest(TABLE);
    let blocks = blocks(&workspace);
    assert_eq!(
        blocks.len(),
        1,
        "a table is one block, not one per row — got {:?}",
        blocks.iter().map(|b| &b.text).collect::<Vec<_>>()
    );

    let table = blocks[0].table.as_ref().expect("the DTO carries the table");
    assert_eq!(table.aligns.len(), 3);
    assert_eq!(table.header.len(), 3);
    assert_eq!(table.rows.len(), 2);
}

#[test]
fn a_cells_ref_arrives_tokenized_not_as_literal_text() {
    // The whole reason the backend reads the grid: a client that split
    // the pipes itself has no tokenizer, so `[[avelino]]` would reach
    // the user as four brackets.
    let (workspace, _hlc, _dir) = ingest(TABLE);
    let table = blocks(&workspace)[0]
        .table
        .clone()
        .expect("the DTO carries the table");
    let owner = &table.rows[0][2];
    assert!(
        matches!(owner.as_slice(), [outl_md::InlineToken::Ref { value }] if value == "avelino"),
        "expected one Ref token, got {owner:?}"
    );
}

#[test]
fn an_escaped_pipe_stays_one_cell_through_the_log() {
    let (workspace, _hlc, _dir) = ingest(TABLE);
    let table = blocks(&workspace)[0]
        .table
        .clone()
        .expect("the DTO carries the table");
    // Four cells would mean the escape was lost somewhere between disk
    // and the tree, and the row would have gained a column.
    assert_eq!(table.rows[1].len(), 3);
    assert_eq!(
        table.rows[1][0],
        vec![outl_md::InlineToken::Plain {
            value: "BH | CWB".to_string()
        }]
    );
}

#[test]
fn ingesting_a_table_raises_no_parse_warning() {
    // Before #329 this page produced one `unrecognized_block_marker`
    // per row: the content was safe and the file looked broken.
    let parsed = outl_md::parse(TABLE);
    assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
}

#[test]
fn a_sibling_bullet_under_a_table_stays_its_own_block() {
    let md = format!("{TABLE}\n- a sibling | with a pipe\n");
    let (workspace, _hlc, _dir) = ingest(&md);
    let blocks = blocks(&workspace);
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].table.is_some());
    assert!(
        blocks[1].table.is_none(),
        "a bullet is a block, whatever pipes it carries"
    );
    assert_eq!(blocks[1].text, "a sibling | with a pipe");
}

#[test]
fn the_page_settles_on_disk_after_one_pass() {
    // The first write normalises the rows under a bullet; a second
    // reconcile must change nothing. A page that reflows on every save
    // emits an `Op::Edit` per pass forever (corpus-gate property 2, at
    // the pipeline level rather than the parser's).
    let (mut workspace, hlc, dir) = ingest(TABLE);
    let root: &Path = dir.path();
    let page = root.join("pages/report.md");
    let after_first = std::fs::read_to_string(&page).expect("read");

    outl_md::reconcile_md(&mut workspace, &hlc, &page, None).expect("second pass");
    assert_eq!(
        std::fs::read_to_string(&page).expect("read"),
        after_first,
        "the file moved on the second pass"
    );
    assert_eq!(blocks(&workspace).len(), 1);
}

#[test]
fn tabular_data_pasted_into_a_page_lands_as_one_table_block() {
    // The other half of #329: a spreadsheet's clipboard, through the
    // real paste entry point, into a page that is then projected.
    let (mut workspace, hlc, _dir) = ingest("- host\n");
    let host = outl_core::id::NodeId(
        ulid::Ulid::from_str(&blocks(&workspace)[0].id).expect("block id is a ULID"),
    );

    outl_actions::paste_markdown(
        &mut workspace,
        &hlc,
        PasteAnchor::AsLastChildOf(host),
        "Route\tPax\nSP → RJ\t1203\nBH → CWB\t12",
    )
    .expect("paste");

    let children = project_outline(&workspace, host);
    assert_eq!(
        children.len(),
        1,
        "tabular data is one table — got {:?}",
        children.iter().map(|b| &b.text).collect::<Vec<_>>()
    );
    let table = children[0].table.as_ref().expect("a table");
    assert_eq!(table.rows.len(), 2);
    assert_eq!(
        table.header[0],
        vec![outl_md::InlineToken::Plain {
            value: "Route".to_string()
        }]
    );
    // And the page it belongs to is still the only one.
    assert_eq!(list_pages(&workspace).len(), 1);
}
