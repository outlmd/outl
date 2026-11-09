//! Root `CLAUDE.md` invariant 3 for `trash::restore`: re-delivering the
//! restore op, or replaying the whole log a second time, changes
//! neither the materialized state nor the log.
//!
//! Three paths re-deliver an op in production — iroh ingest, a `file`
//! transport that syncs the same `ops-<actor>.jsonl` twice, and a boot
//! replay over a log a crash left partly applied — so this is not a
//! theoretical property.
//!
//! `trash/replay.rs` is read-only and memoises per call, so it has no
//! state to go stale between two `list` calls; what these exercise is
//! the write side (`Workspace::apply`'s ts dedup and `do_op(Move)`'s
//! idempotence) reached through `restore` rather than directly.

mod trash_restore;

use outl_actions::block::{append_block, delete as delete_block};
use outl_actions::page::{open_or_create, PageKind};
use outl_actions::refusal::Surface;
use outl_actions::trash;
use trash_restore::{log_ids, ops, replay, replica, shape};

#[test]
fn replaying_one_restore_op_twice_moves_nothing_and_logs_nothing() {
    let (mut ws, hlc) = replica();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let block = append_block(&mut ws, &hlc, Some(page), Some("texto")).expect("block");
    delete_block(&mut ws, &hlc, block).expect("delete");
    trash::restore(&mut ws, &hlc, block).expect("restore");

    let restore_op = ops(&ws)
        .last()
        .cloned()
        .expect("the restore is the last op");
    let before_tree = shape(&ws);
    let before_log = log_ids(&ws);

    ws.apply(restore_op.clone()).expect("re-apply");
    ws.apply(restore_op).expect("re-apply again");

    assert_eq!(shape(&ws), before_tree, "the tree moved on a re-delivery");
    assert_eq!(log_ids(&ws), before_log, "the log grew on a re-delivery");
}

/// A whole log, delivered once, twice, and backwards: one tree.
///
/// The second pass is the shape a boot replay over an already-applied
/// log has; the reverse pass forces `apply_op`'s undo/replay loop
/// through every op rather than letting them land in ts order.
#[test]
fn replaying_a_log_holding_restores_twice_lands_on_one_tree() {
    let (mut ws, hlc) = replica();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let first = append_block(&mut ws, &hlc, Some(page), Some("a")).expect("a");
    let second = append_block(&mut ws, &hlc, Some(page), Some("b")).expect("b");
    delete_block(&mut ws, &hlc, first).expect("delete a");
    delete_block(&mut ws, &hlc, second).expect("delete b");
    trash::restore(&mut ws, &hlc, first).expect("restore a");
    trash::restore(&mut ws, &hlc, second).expect("restore b");
    let log = ops(&ws);

    let once = replay(&log);
    let mut twice = replay(&log);
    for op in &log {
        twice.apply(op.clone()).expect("second pass");
    }
    assert_eq!(
        shape(&twice),
        shape(&once),
        "a second pass changed the tree"
    );
    assert_eq!(
        log_ids(&twice),
        log_ids(&once),
        "a second pass grew the log"
    );

    let mut backwards = log.clone();
    backwards.reverse();
    assert_eq!(
        shape(&replay(&backwards)),
        shape(&once),
        "reverse delivery produced a different tree"
    );

    assert!(
        trash::list(&once, Surface::Cli).is_empty(),
        "both blocks were restored, so nothing is left to restore"
    );
    assert_eq!(
        trash::list(&once, Surface::Cli).len(),
        trash::list(&once, Surface::Cli).len(),
        "and the derived listing is stable across repeated reads"
    );
}
