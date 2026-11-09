//! Root `CLAUDE.md` invariant 5, asked of `trash::restore`: a restore
//! that cannot land must refuse, and refuse having written nothing.
//!
//! Restore is a `Move` out of `TRASH_ROOT`, so there is always a
//! plausible place to put the block — `root()`, the nearest live
//! ancestor, today's journal. Every one of those is content the user
//! cannot find again, reported as a restore that worked. The only
//! acceptable answer is a refusal that names where the content still
//! is, so the user can act on it.
//!
//! `trash::landing_for` is the single owner of that verdict
//! (`crates/outl-actions/src/trash.rs`), and `list` asks the same
//! function per row — so these assert the mutation, and
//! `a_listing_carries_the_same_refusal_restore_would_return` in
//! `src/trash/tests.rs` is what keeps the listing from drifting off it.

mod trash_restore;

use outl_actions::block::{append_block, delete as delete_block};
use outl_actions::page::{open_or_create, PageKind};
use outl_actions::refusal::Surface;
use outl_actions::trash;
use outl_actions::tree::{children_of, is_trashed};
use outl_core::id::NodeId;
use trash_restore::{log_ids, replica, shape};

/// Delete a block, delete its page, restore the block.
///
/// The realistic way in: the user tidies up, then changes their mind
/// about one block. The page root is in the trash, so there is nowhere
/// to land — and `root()` is exactly the plausible wrong answer, since
/// it is a legal `move_under` destination and the block would vanish
/// from every page listing.
#[test]
fn restoring_into_a_deleted_page_is_refused_not_orphaned() {
    let (mut ws, hlc) = replica();
    let page = open_or_create(&mut ws, &hlc, "arquivo", "Arquivo", PageKind::Page).expect("page");
    let block = append_block(&mut ws, &hlc, Some(page), Some("o bloco")).expect("block");

    delete_block(&mut ws, &hlc, block).expect("delete the block");
    delete_block(&mut ws, &hlc, page).expect("delete the page");

    let before_tree = shape(&ws);
    let before_log = log_ids(&ws);

    let err = trash::restore(&mut ws, &hlc, block).expect_err("the page is in the trash");

    assert!(
        err.to_string().contains(&page.to_string()),
        "the refusal has to name where the content still is: {err}"
    );
    assert!(is_trashed(&ws, block), "the block stayed in the trash");
    assert!(
        !children_of(&ws, NodeId::root())
            .iter()
            .any(|(node, _)| *node == block),
        "nothing landed under root() — that is the orphan this refuses"
    );
    assert_eq!(shape(&ws), before_tree, "the tree is untouched");
    assert_eq!(
        log_ids(&ws),
        before_log,
        "and no op was appended: a refusal is not a mutation"
    );
}

/// The same question asked of a block *buried* inside a trashed
/// subtree.
///
/// Deleting a subtree is one `Move` on its root, so the log holds no
/// move that put the child in the trash and there is no origin to fold.
/// `restore` is reachable with that id from the CLI, the MCP tools and
/// any client holding a stale listing, so "the listing never offers it"
/// is not on its own a defence.
#[test]
fn a_block_buried_in_a_trashed_subtree_has_no_landing_and_is_refused() {
    let (mut ws, hlc) = replica();
    let page = open_or_create(&mut ws, &hlc, "notas", "Notas", PageKind::Page).expect("page");
    let section = append_block(&mut ws, &hlc, Some(page), Some("seção")).expect("section");
    let child = append_block(&mut ws, &hlc, Some(section), Some("filho")).expect("child");

    delete_block(&mut ws, &hlc, section).expect("delete the section");
    assert!(is_trashed(&ws, child), "the child rode along");

    let before_tree = shape(&ws);
    let before_log = log_ids(&ws);

    let err = trash::restore(&mut ws, &hlc, child)
        .expect_err("no move in the log put the child itself in the trash");

    assert_eq!(shape(&ws), before_tree, "nothing moved");
    assert_eq!(log_ids(&ws), before_log, "nothing was logged");
    assert!(
        trash::list(&ws, Surface::Cli)
            .iter()
            .all(|entry| entry.node != child),
        "and a listing never offered it: {err}"
    );
}
