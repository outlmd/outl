//! Routing and merging, without a workspace.
//!
//! The merge's ordering and de-duplication rules — the part that makes
//! per-page sharding invisible to replay — used to be reachable only
//! through `Workspace::apply` against a real on-disk shard layout, and
//! so had no direct test at all.

use std::collections::BTreeMap;

use super::{Route, StorageRouter};
use crate::fractional::Fractional;
use crate::hlc::HlcGenerator;
use crate::id::{ActorId, NodeId};
use crate::log::OpLog;
use crate::op::{LogOp, Op};
use crate::storage::MemoryStorage;
use crate::tree::Tree;

fn router() -> StorageRouter {
    StorageRouter::new(Box::new(MemoryStorage::default()))
}

/// A `SetCollapsed` op naming `node` — the smallest op that carries a
/// node, so routing has something to resolve.
fn op_for(hlc: &HlcGenerator, node: NodeId) -> LogOp {
    let ts = hlc.next();
    LogOp {
        actor: ts.actor,
        ts,
        op: Op::SetCollapsed {
            node,
            value: true,
            old_value: false,
        },
    }
}

/// A tree of `root → child → grandchild`, so slug resolution has a
/// parent chain longer than one hop to walk.
fn three_deep(hlc: &HlcGenerator) -> (Tree, NodeId, NodeId) {
    let mut tree = Tree::new();
    let mut log = OpLog::new();
    let root = NodeId::new();
    let child = NodeId::new();
    let grandchild = NodeId::new();
    for (node, parent) in [(root, NodeId::root()), (child, root), (grandchild, child)] {
        let ts = hlc.next();
        tree.apply_op(
            &mut log,
            LogOp {
                actor: ts.actor,
                ts,
                op: Op::Create {
                    node,
                    parent,
                    position: Fractional::between(None, None),
                },
            },
        );
    }
    (tree, root, grandchild)
}

#[test]
fn an_unregistered_node_routes_global() {
    assert_eq!(
        router().route(&Tree::new(), Some(NodeId::new())),
        Route::Global
    );
}

#[test]
fn an_op_with_no_node_routes_global() {
    assert_eq!(router().route(&Tree::new(), None), Route::Global);
}

#[test]
fn a_registered_page_root_routes_to_its_shard() {
    let mut r = router();
    let root = NodeId::new();
    r.register_root(root, "infra");
    r.register_page("infra", Box::new(MemoryStorage::default()));
    assert_eq!(
        r.route(&Tree::new(), Some(root)),
        Route::Page("infra".into())
    );
}

/// A page whose root is registered but whose shard is not open must
/// still persist. Dropping the op would lose it; the global file is what
/// the merge reads anyway.
#[test]
fn a_known_page_with_no_open_shard_falls_back_to_global() {
    let mut r = router();
    let root = NodeId::new();
    r.register_root(root, "infra");
    assert_eq!(r.route(&Tree::new(), Some(root)), Route::Global);
}

#[test]
fn a_write_to_a_vanished_shard_still_persists() {
    let mut r = router();
    let hlc = HlcGenerator::new(ActorId::new());
    let op = op_for(&hlc, NodeId::new());

    r.append_op(&Route::Page("gone".into()), &op)
        .expect("the op must land somewhere");

    assert_eq!(
        r.all_ops().expect("read back").len(),
        1,
        "an op routed to a missing shard belongs in the global file, not \
         in nothing"
    );
}

#[test]
fn merged_reads_are_hlc_ordered_across_shards() {
    let mut r = router();
    r.register_page("a", Box::new(MemoryStorage::default()));
    r.register_page("b", Box::new(MemoryStorage::default()));
    let hlc = HlcGenerator::new(ActorId::new());

    // Written out of shard order on purpose, so only the sort can
    // produce the right sequence.
    let ops: Vec<LogOp> = (0..4).map(|_| op_for(&hlc, NodeId::new())).collect();
    r.append_op(&Route::Global, &ops[0]).unwrap();
    r.append_op(&Route::Page("a".into()), &ops[1]).unwrap();
    r.append_op(&Route::Page("b".into()), &ops[2]).unwrap();
    r.append_op(&Route::Page("a".into()), &ops[3]).unwrap();

    let read = r.all_ops().expect("read back");
    let timestamps: Vec<_> = read.iter().map(|o| o.ts).collect();
    let mut sorted = timestamps.clone();
    sorted.sort();
    assert_eq!(
        timestamps, sorted,
        "the merge must reproduce the order a single-file log would have \
         had — that is what makes sharding invisible to replay"
    );
    assert_eq!(read.len(), 4);
}

/// An op can legitimately live in two shards (a page registered after
/// some of its ops were already written globally). The CRDT is
/// idempotent per op, but the log must not double-count.
#[test]
fn an_op_present_in_two_shards_is_read_once() {
    let mut r = router();
    r.register_page("a", Box::new(MemoryStorage::default()));
    let hlc = HlcGenerator::new(ActorId::new());
    let op = op_for(&hlc, NodeId::new());

    r.append_op(&Route::Global, &op).unwrap();
    r.append_op(&Route::Page("a".into()), &op).unwrap();

    assert_eq!(
        r.all_ops().expect("read back").len(),
        1,
        "the same HLC appearing in two shards is one op"
    );
}

#[test]
fn ops_for_node_spans_every_shard() {
    let mut r = router();
    r.register_page("a", Box::new(MemoryStorage::default()));
    let hlc = HlcGenerator::new(ActorId::new());
    let node = NodeId::new();

    r.append_op(&Route::Global, &op_for(&hlc, node)).unwrap();
    r.append_op(&Route::Page("a".into()), &op_for(&hlc, node))
        .unwrap();

    assert_eq!(r.ops_for_node(node).expect("read back").len(), 2);
}

#[test]
fn the_cutoff_keeps_the_latest_timestamp_per_actor() {
    let mut r = router();
    r.register_page("a", Box::new(MemoryStorage::default()));
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let node = NodeId::new();

    let early = op_for(&hlc, node);
    let late = op_for(&hlc, node);
    // The later op goes to the *page* shard, so a cutoff that only read
    // the global file would be stale — and a stale cutoff makes the next
    // snapshot boot replay ops it already has.
    r.append_op(&Route::Global, &early).unwrap();
    r.append_op(&Route::Page("a".into()), &late).unwrap();

    assert_eq!(
        r.last_ts_per_actor().expect("read back").get(&actor),
        Some(&late.ts)
    );
}

#[test]
fn the_per_actor_delta_spans_every_shard() {
    let mut r = router();
    r.register_page("a", Box::new(MemoryStorage::default()));
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let node = NodeId::new();

    let first = op_for(&hlc, node);
    r.append_op(&Route::Global, &first).unwrap();
    r.append_op(&Route::Page("a".into()), &op_for(&hlc, node))
        .unwrap();

    let mut cutoff = BTreeMap::new();
    cutoff.insert(actor, first.ts);

    assert_eq!(
        r.ops_since_per_actor(&cutoff).expect("read back").len(),
        1,
        "only the op after the cutoff, and it lives in the page shard"
    );
}

#[test]
fn slug_resolution_walks_up_to_the_registered_root() {
    let hlc = HlcGenerator::new(ActorId::new());
    let (tree, root, grandchild) = three_deep(&hlc);
    let mut r = router();
    r.register_root(root, "infra");

    assert_eq!(r.slug_for_node(&tree, grandchild).as_deref(), Some("infra"));
}

#[test]
fn a_chain_that_reaches_no_registered_root_resolves_to_nothing() {
    let hlc = HlcGenerator::new(ActorId::new());
    let (tree, _root, grandchild) = three_deep(&hlc);
    assert_eq!(router().slug_for_node(&tree, grandchild), None);
}

#[test]
fn the_nearest_registered_root_wins() {
    // A page root nested under another registered root must claim its
    // own subtree — otherwise every op in the workspace would route to
    // whichever ancestor happened to be registered.
    let hlc = HlcGenerator::new(ActorId::new());
    let (tree, root, grandchild) = three_deep(&hlc);
    let child = tree.parent(grandchild).expect("grandchild has a parent");

    let mut r = router();
    r.register_root(root, "outer");
    r.register_root(child, "inner");

    assert_eq!(r.slug_for_node(&tree, grandchild).as_deref(), Some("inner"));
}
