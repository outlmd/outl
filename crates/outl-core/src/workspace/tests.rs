//! Unit tests for [`Workspace`](super::Workspace).
//!
//! Extracted from `workspace.rs` unchanged — every test kept its name,
//! because `cargo test <name>` and the `CLAUDE.md` citations match on
//! names. The move took the parent from 1,317 lines to 850, back under
//! the 900-line guard it had been grandfathered past.

use super::*;
use crate::content::DOC_CACHE_CAP;
use crate::fractional::Fractional;
use crate::hlc::HlcGenerator;
use crate::op::Op;
use yrs::{Doc, Text, Transact};

fn make_op(g: &HlcGenerator, op: Op) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

/// The ghost-block policy, core half (issue #213 item 1, RFC 0129).
///
/// **Never write an op for something the user did not do.** The
/// `.md` reconcile diff re-emits `Create` + `Move` + a `SetProp`
/// per property for every block on every commit; each is
/// idempotent on the tree and each still costs an fsynced append.
/// A one-block edit on an 11-block page logged 23 ops before this
/// filter existed — the slow commit, and an op log that grew by
/// the whole page per keystroke.
///
/// The filter had no direct test: it was only exercised through
/// `outl-md`'s reconcile, where a regression shows up as "commits
/// got slower", which nobody notices in CI.
#[test]
fn a_redundant_op_is_recognised_as_a_noop() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let tmp = tempfile::TempDir::new().unwrap();
    let storage =
        Box::new(crate::storage::JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap());
    let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

    let node = NodeId::new();
    let pos = Fractional::between(None, None);
    let create = Op::Create {
        node,
        parent: NodeId::root(),
        position: pos.clone(),
    };

    assert!(
        !ws.op_is_noop(&create),
        "a Create for a node that does not exist yet is real work",
    );
    ws.apply(make_op(&g, create.clone())).unwrap();
    assert!(
        ws.op_is_noop(&create),
        "re-emitting the same Create must be recognised as a no-op —              this is the filter that keeps a reconcile from logging the              whole page on every keystroke",
    );

    // A property, then the same property again.
    let set = Op::SetProp {
        node,
        key: "remind".into(),
        value: Some(crate::property::PropValue::Text("9am".into())),
        old_value: None,
    };
    assert!(!ws.op_is_noop(&set), "setting a new property is real work");
    ws.apply(make_op(&g, set.clone())).unwrap();
    assert!(
        ws.op_is_noop(&set),
        "re-setting a property to the value it already has is a no-op",
    );

    // Changing the value is NOT a no-op — guards the guard, since a
    // filter that swallows everything would pass every assertion above.
    let changed = Op::SetProp {
        node,
        key: "remind".into(),
        value: Some(crate::property::PropValue::Text("10am".into())),
        old_value: None,
    };
    assert!(
        !ws.op_is_noop(&changed),
        "a property whose value differs must still be logged",
    );
}

/// The dangerous direction: over-filtering silently loses user work.
///
/// A `Move` to a *different* parent is never a no-op, including one
/// that would form a cycle — invariant 4 says the cycle-forming move
/// is a no-op **on the materialized tree** and still goes into the
/// log, because removing it breaks the correctness of future
/// reordering. If `op_is_noop` ever started answering "true" for
/// those, the filter would drop them before they were ever written.
#[test]
fn a_move_that_changes_the_tree_is_never_filtered_out() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let tmp = tempfile::TempDir::new().unwrap();
    let storage =
        Box::new(crate::storage::JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap());
    let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

    let parent = NodeId::new();
    let child = NodeId::new();
    let pos = Fractional::between(None, None);
    for node in [parent, child] {
        ws.apply(make_op(
            &g,
            Op::Create {
                node,
                parent: NodeId::root(),
                position: pos.clone(),
            },
        ))
        .unwrap();
    }

    // child -> parent: a real move.
    let nest = Op::Move {
        node: child,
        new_parent: parent,
        position: pos.clone(),
        old_parent: NodeId::root(),
        old_position: pos.clone(),
    };
    assert!(!ws.op_is_noop(&nest), "a move to a new parent is real work");
    ws.apply(make_op(&g, nest.clone())).unwrap();
    assert!(ws.op_is_noop(&nest), "re-emitting the same move is a no-op");

    // parent -> child would form a cycle. It must NOT be filtered:
    // invariant 4 requires it in the log even though the tree
    // refuses it.
    let cycle = Op::Move {
        node: parent,
        new_parent: child,
        position: pos.clone(),
        old_parent: NodeId::root(),
        old_position: pos.clone(),
    };
    assert!(
        !ws.op_is_noop(&cycle),
        "a cycle-forming move must still be logged (invariant 4) —              filtering it out breaks the correctness of future reordering",
    );
}

/// `Op::Edit` is never filtered here, and the reason is worth pinning:
/// a Yrs delta for unchanged text is *already empty*, so the emptiness
/// check belongs upstream where the delta is built. If someone adds an
/// `Edit` arm to `op_is_noop` that inspects the bytes, they are
/// duplicating a decision that already has an owner.
#[test]
fn edit_is_left_to_the_yrs_delta_to_decide() {
    let actor = ActorId::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let storage =
        Box::new(crate::storage::JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap());
    let ws = Workspace::open_with_storage(actor, storage, None).unwrap();

    assert!(
        !ws.op_is_noop(&Op::Edit {
            node: NodeId::new(),
            text_op: Vec::new(),
        }),
        "Edit must not be filtered by op_is_noop",
    );
}

#[test]
fn open_apply_reload_preserves_state() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    // Use a shared directory: open, write, close, reopen.
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let storage1 = Box::new(crate::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
    let mut ws = Workspace::open_with_storage(actor, storage1, None).unwrap();
    let n = NodeId::new();
    ws.apply(make_op(
        &g,
        Op::Create {
            node: n,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .unwrap();
    drop(ws);

    let storage2 = Box::new(crate::storage::JsonlStorage::open(dir, actor).unwrap());
    let ws2 = Workspace::open_with_storage(actor, storage2, None).unwrap();
    assert_eq!(ws2.tree().node_count(), 1);
    assert_eq!(ws2.tree().parent(n), Some(NodeId::root()));
    assert_eq!(ws2.log().len(), 1);
}

#[test]
fn edit_dispatches_to_content_store() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let n = NodeId::new();
    ws.apply(make_op(
        &g,
        Op::Create {
            node: n,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .unwrap();

    // Build a Yrs update locally.
    let doc = Doc::new();
    let text = doc.get_or_insert_text("content");
    let mut txn = doc.transact_mut();
    text.push(&mut txn, "hello outl");
    let update_bytes = txn.encode_update_v1();
    drop(txn);

    ws.apply(make_op(
        &g,
        Op::Edit {
            node: n,
            text_op: update_bytes,
        },
    ))
    .unwrap();

    assert_eq!(ws.block_text(n).as_deref(), Some("hello outl"));
}

/// Edit one block, then create + edit a fresh one. Reopening rebuilds
/// the text of both from the log even though neither Doc was kept
/// resident across the close.
#[test]
fn reopen_rebuilds_text_without_resident_docs() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let storage = Box::new(crate::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
    let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

    let mut ids = Vec::new();
    for i in 0..5 {
        let n = NodeId::new();
        ids.push(n);
        ws.apply(make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let update = ws.build_text_replace_update(n, &format!("block {i}"));
        ws.apply(make_op(
            &g,
            Op::Edit {
                node: n,
                text_op: update,
            },
        ))
        .unwrap();
    }
    drop(ws);

    let storage = Box::new(crate::storage::JsonlStorage::open(dir, actor).unwrap());
    let ws2 = Workspace::open_with_storage(actor, storage, None).unwrap();
    for (i, n) in ids.iter().enumerate() {
        assert_eq!(
            ws2.block_text(*n).as_deref(),
            Some(format!("block {i}").as_str())
        );
    }
    // Pass 2 materializes strings and drops every Doc, so nothing is
    // resident right after open — the whole point of issue #108.
    assert_eq!(ws2.live_doc_count(), 0);
}

/// Full-replay boot must NOT materialize block text up front (#179):
/// no string is resident until the first `block_text` read, which then
/// rebuilds that one block lazily and correctly — byte-identical to
/// what the old eager pass produced.
#[test]
fn full_replay_boot_defers_block_text_materialization() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();

    let storage = Box::new(crate::storage::JsonlStorage::open(dir.clone(), actor).unwrap());
    let mut ws = Workspace::open_with_storage(actor, storage, None).unwrap();

    // Many edited blocks, plus one create-only block (no text).
    let mut edited = Vec::new();
    for i in 0..64 {
        let n = NodeId::new();
        edited.push((n, format!("block {i} ☃")));
        ws.apply(make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let update = ws.build_text_replace_update(n, &format!("block {i} ☃"));
        ws.apply(make_op(
            &g,
            Op::Edit {
                node: n,
                text_op: update,
            },
        ))
        .unwrap();
    }
    let bare = NodeId::new();
    ws.apply(make_op(
        &g,
        Op::Create {
            node: bare,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .unwrap();
    drop(ws);

    // Reopen: no snapshot (root = None) → full replay, which now
    // defers text.
    let storage = Box::new(crate::storage::JsonlStorage::open(dir, actor).unwrap());
    let ws2 = Workspace::open_with_storage(actor, storage, None).unwrap();

    // Tree structure is fully materialized; block text is not.
    assert_eq!(ws2.tree().node_count(), 65);
    assert_eq!(
        ws2.resident_text_count(),
        0,
        "boot must not eagerly materialize any block text"
    );
    assert_eq!(ws2.live_doc_count(), 0);

    // Reading a block never touched since boot rebuilds its text
    // lazily and correctly.
    let (first, first_text) = &edited[0];
    assert_eq!(ws2.block_text(*first).as_deref(), Some(first_text.as_str()));
    assert_eq!(
        ws2.resident_text_count(),
        1,
        "only the read block should now be resident"
    );

    // Every other block reads back byte-identical on demand; a
    // create-only block has no text (not a phantom empty string).
    for (n, want) in &edited {
        assert_eq!(ws2.block_text(*n).as_deref(), Some(want.as_str()));
    }
    assert_eq!(ws2.block_text(bare), None);
    // Lazy reads only ever populate the string cache, never the Doc LRU.
    assert_eq!(ws2.live_doc_count(), 0);
}

/// The live-Doc cache never grows past its cap, no matter how many
/// distinct blocks get edited in a session.
#[test]
fn doc_cache_is_bounded() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let over = DOC_CACHE_CAP + 50;
    for i in 0..over {
        let n = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let update = ws.build_text_replace_update(n, &format!("b{i}"));
        ws.apply(make_op(
            &g,
            Op::Edit {
                node: n,
                text_op: update,
            },
        ))
        .unwrap();
        assert!(ws.live_doc_count() <= DOC_CACHE_CAP);
    }
    assert_eq!(ws.live_doc_count(), DOC_CACHE_CAP);
}

/// A block evicted from the cache is rebuilt from the log on the next
/// edit, preserving its text instead of losing history.
#[test]
fn evicted_block_rebuilds_from_log() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();

    let first = NodeId::new();
    ws.apply(make_op(
        &g,
        Op::Create {
            node: first,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    ))
    .unwrap();
    let update = ws.build_text_replace_update(first, "hello");
    ws.apply(make_op(
        &g,
        Op::Edit {
            node: first,
            text_op: update,
        },
    ))
    .unwrap();

    // Edit enough other blocks to evict `first` from the cache.
    for i in 0..DOC_CACHE_CAP + 10 {
        let n = NodeId::new();
        ws.apply(make_op(
            &g,
            Op::Create {
                node: n,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let u = ws.build_text_replace_update(n, &format!("x{i}"));
        ws.apply(make_op(
            &g,
            Op::Edit {
                node: n,
                text_op: u,
            },
        ))
        .unwrap();
    }
    assert!(!ws.content.is_cached(first));

    // Rebuild on demand: appending to the evicted block keeps "hello".
    let update = ws.build_text_replace_update(first, "hello world");
    ws.apply(make_op(
        &g,
        Op::Edit {
            node: first,
            text_op: update,
        },
    ))
    .unwrap();
    assert_eq!(ws.block_text(first).as_deref(), Some("hello world"));
}
