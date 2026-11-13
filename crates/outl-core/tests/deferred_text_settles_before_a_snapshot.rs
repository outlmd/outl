//! A deferred rebuild has to settle **before** a snapshot is written from
//! it.
//!
//! A snapshot boot adopts a body whose `block_text` is the text as of its
//! cutoff, then replays the delta on top. Every node the delta edited
//! therefore has a cached string that is present and *wrong*; the rebuild
//! that corrects it reads the node's full `Edit` history back from storage
//! and is deferred to first read, because doing it for every such node at
//! boot is what the launch was paying for.
//!
//! Deferring it puts a trap in exactly one place. `save_snapshot` builds
//! its body out of that same cache, so a body written after a boot where
//! nobody happened to read the block would publish the **pre-delta**
//! string as settled state — and the next boot would adopt it, with its
//! cutoff now claiming the newer edit was already folded in. No later
//! replay would correct it. That is silent content loss produced by a
//! cache, which is the one thing a cache is never allowed to do.
//!
//! `Workspace::force_materialize_pending` drains the deferred set for
//! this reason, and this is the test that fails if it stops.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use std::path::Path;
use tempfile::TempDir;

fn logop(g: &HlcGenerator, op: Op) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op,
    }
}

fn open(root: &Path, actor: ActorId) -> Workspace {
    Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(root.join("ops"), actor).expect("storage")),
        Some(root.to_path_buf()),
    )
    .expect("open")
}

#[test]
fn a_snapshot_written_after_a_deferred_boot_carries_the_settled_text() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let node = NodeId::new();

    // A block with text, and a snapshot that holds that text.
    {
        let mut ws = open(root, actor);
        ws.apply(logop(
            &g,
            Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        ))
        .unwrap();
        let edit = ws.build_text_replace_update(node, "v1");
        ws.apply(logop(
            &g,
            Op::Edit {
                node,
                text_op: edit,
            },
        ))
        .unwrap();
        ws.save_snapshot().expect("snapshot written");
    }

    // An edit above that snapshot's cutoff. This is what makes the body's
    // "v1" stale, and it is the ordinary case on any synced device.
    {
        let mut ws = open(root, actor);
        let edit = ws.build_text_replace_update(node, "v2");
        ws.apply(logop(
            &g,
            Op::Edit {
                node,
                text_op: edit,
            },
        ))
        .unwrap();
    }

    // Boot from the snapshot and write a new one **without ever reading
    // the block**, so its rebuild is still deferred when the body is
    // built. This is the whole hazard: an ordinary launch reads a journal
    // page, not every block a peer touched.
    let mut ws = open(root, actor);
    assert!(
        ws.booted_from_snapshot(),
        "the fixture has to exercise the snapshot path, or it proves nothing"
    );
    ws.save_snapshot().expect("second snapshot written");
    drop(ws);

    // A reader of that second body must see the log's text. Reading it
    // through a fresh boot rather than off the first workspace is the
    // point: this is what the *next* launch would adopt as settled.
    let fresh = open(root, actor);
    assert_eq!(
        fresh.block_text(node).as_deref(),
        Some("v2"),
        "the snapshot published the pre-delta string as settled state"
    );
}
