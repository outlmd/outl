//! Tests for the cross-client sync engine.
//!
//! Split out of `sync.rs` to keep that file under the size ratchet
//! recorded in `.github/file-size-baseline.txt`.

use super::*;
use tempfile::TempDir;

/// A generator for the tests that only need `reload_workspace` to have
/// one, and never look at what it issued.
fn gen_for(actor: ActorId) -> HlcGenerator {
    HlcGenerator::new(actor)
}

#[test]
fn snapshot_returns_empty_when_no_ops_dir() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let engine = SyncEngine::new(tmp.path().to_path_buf(), actor);
    assert!(engine.snapshot().is_empty());
}

#[test]
fn snapshot_lists_ops_files_and_skips_others() {
    let tmp = TempDir::new().unwrap();
    let ops = tmp.path().join("ops");
    std::fs::create_dir(&ops).unwrap();
    std::fs::write(ops.join("ops-A.jsonl"), b"x").unwrap();
    std::fs::write(ops.join("ops-B.jsonl"), b"yz").unwrap();
    std::fs::write(ops.join("README.md"), b"hello").unwrap();

    let actor = ActorId::new();
    let engine = SyncEngine::new(tmp.path().to_path_buf(), actor);
    let snap = engine.snapshot();
    assert_eq!(snap.len(), 2);
    assert_eq!(snap[0].name, "ops-A.jsonl");
    assert_eq!(snap[0].size, 1);
    assert_eq!(snap[1].name, "ops-B.jsonl");
    assert_eq!(snap[1].size, 2);
}

#[test]
fn reload_workspace_opens_empty_workspace_when_no_ops() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let engine = SyncEngine::new(tmp.path().to_path_buf(), actor);
    let ws = engine
        .reload_workspace(&gen_for(actor))
        .expect("should open clean");
    // Materialised tree starts empty.
    assert_eq!(
        crate::tree::children_of(&ws, outl_core::id::NodeId::root()).len(),
        0
    );
}

/// Regression: a small (well under the old 10k-op threshold) workspace
/// whose on-disk snapshot gets rejected by the convergence guard once
/// must NOT be stuck full-replaying on every subsequent incremental
/// reload. This is the routine two-actor case — see
/// `snapshot_late_op.rs::late_low_hlc_op_from_unseen_actor_survives_snapshot_boot`
/// for why a legitimate peer op can sort below another actor's cutoff.
#[test]
fn reload_workspace_refreshes_snapshot_after_guard_rejection_even_below_threshold() {
    use outl_core::fractional::Fractional;
    use outl_core::hlc::Hlc;
    use outl_core::id::NodeId;
    use outl_core::op::{LogOp, Op};
    use outl_core::storage::{JsonlStorage, Storage};

    fn hlc(physical_ms: u64, actor: ActorId) -> Hlc {
        Hlc {
            physical_ms,
            logical: 0,
            actor,
        }
    }

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let ops_dir = root.join("ops");
    let actor_a = ActorId::new();
    let actor_b = ActorId::new();

    // Actor A creates one node at a HIGH physical time and snapshots.
    let mut ws = Workspace::open_with_storage(
        actor_a,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor_a).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    ws.set_snapshot_policy(false, 0);
    let n_a = NodeId::new();
    ws.apply(LogOp {
        ts: hlc(10_000, actor_a),
        actor: actor_a,
        op: Op::Create {
            node: n_a,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    })
    .unwrap();
    ws.save_snapshot().unwrap();
    drop(ws);

    // Actor B's op arrives via sync with a LOW physical time (B was
    // offline / its clock lags), sitting below A's cutoff — the
    // convergence guard must reject the stale snapshot for this boot.
    let n_b = NodeId::new();
    {
        let mut storage_b = JsonlStorage::open(ops_dir.clone(), actor_b).unwrap();
        storage_b
            .append_op(&LogOp {
                ts: hlc(5, actor_b),
                actor: actor_b,
                op: Op::Create {
                    node: n_b,
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            })
            .unwrap();
    }

    let engine = SyncEngine::new(root.to_path_buf(), actor_a);

    // Well under the old 10_000-op "worth the write" gate.
    let clock = gen_for(actor_a);
    let ws1 = engine.reload_workspace(&clock).expect("first reload");
    assert!(ws1.tree().contains(n_a));
    assert!(ws1.tree().contains(n_b));
    assert!(
        !ws1.booted_from_snapshot(),
        "first boot must full-replay: the stale snapshot's cutoff sits above B's late op"
    );
    drop(ws1);

    // No new ops landed since. A fresh snapshot persisted after the
    // first reload's full replay should let this second reload adopt
    // it directly instead of full-replaying again.
    let ws2 = engine.reload_workspace(&clock).expect("second reload");
    assert!(ws2.tree().contains(n_a));
    assert!(ws2.tree().contains(n_b));
    assert!(
        ws2.booted_from_snapshot(),
        "second reload must adopt the refreshed snapshot instead of full-replaying forever"
    );
}

/// The site this test guards: `reproject_page` used to call the
/// unconditional writer, so a page a peer never touched — its `.md`
/// merely holding content no op has seen — got flattened by the very
/// next reload. Root `CLAUDE.md` invariant 8.
#[test]
fn reproject_page_refuses_a_frozen_page_instead_of_deleting_it() {
    use crate::block::append_block;
    use crate::journal::{apply_page_md_with_sidecar, page_md_path};
    use crate::page::{open_or_create, page_meta, PageKind};
    use outl_core::hlc::HlcGenerator;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);

    // Write-side workspace: create the page, project it once while
    // it is healthy, then drop it — the next reload below mirrors
    // "another process/device reopens after a peer merge".
    let mut ws = Workspace::open_with_storage(
        actor,
        Box::new(JsonlStorage::open(ops_dir.clone(), actor).unwrap()),
        Some(root.to_path_buf()),
    )
    .unwrap();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
    apply_page_md_with_sidecar(&ws, root, page).unwrap();
    let md_path = page_md_path(root, &page_meta(&ws, page).unwrap());
    drop(ws);

    // Simulate the state a `reconcile_md` that missed invariant 8
    // leaves behind: content on disk the op log never recorded, with
    // the sidecar re-stamped to call those exact bytes faithful.
    let mut md = std::fs::read_to_string(&md_path).unwrap();
    md.push_str("- only ever on disk\n");
    std::fs::write(&md_path, &md).unwrap();
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);
    let mut sidecar = outl_md::sidecar::read(&sidecar_path).unwrap();
    sidecar.last_synced_hash = outl_md::sidecar::file_hash(&md);
    outl_md::sidecar::write(&sidecar_path, &sidecar).unwrap();

    let engine = SyncEngine::new(root.to_path_buf(), actor);
    let fresh = engine.reload_workspace(&hlc).expect("reload after merge");

    match engine.reproject_page(&fresh, page) {
        Err(ActionError::PageMarkdownAheadOfLog { sample, .. }) => assert!(
            sample.contains("only ever on disk"),
            "the error must name the content at risk, got {sample:?}"
        ),
        other => panic!("expected PageMarkdownAheadOfLog, got {other:?}"),
    }
    let after = std::fs::read_to_string(&md_path).unwrap();
    assert!(
        after.contains("only ever on disk"),
        "a refused reprojection must never delete the unlogged content: {after:?}"
    );
}

/// The reload path's own seeding, distinct from what `outl-core`'s
/// `seed_clock` tests already own.
///
/// Boot seeding cannot cover this: a peer whose clock runs ahead of ours
/// lands its ops *after* we booted, and `reload_workspace` is the only
/// place this process learns about them. Without a seed there, the next
/// local edit is stamped below the peer's op — which converges fine and
/// costs the paper's undo/redo window over every newer entry, on the
/// foreground thread, on every subsequent edit.
///
/// Asserted against **the log's tail**, never against a second
/// generator's reading: two generators built in the same millisecond
/// legitimately agree, so comparing them would be flaky by construction.
/// The tail answers the real question — did our op land at the end, or
/// did it force a reorder?
#[test]
fn a_reload_lifts_the_clock_above_a_peer_op_stamped_ahead_of_our_wall_clock() {
    use outl_core::fractional::Fractional;
    use outl_core::id::NodeId;
    use outl_core::op::{LogOp, Op};
    use outl_core::storage::Storage;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let ops_dir = root.join("ops");
    let me = ActorId::new();
    let peer = ActorId::new();

    // Our clock, as a long-running client holds it: seeded at boot from a
    // log that did not yet contain the peer's op.
    let hlc = HlcGenerator::new(me);
    {
        let mut ws = Workspace::open_with_storage(
            me,
            Box::new(JsonlStorage::open(ops_dir.clone(), me).unwrap()),
            Some(root.to_path_buf()),
        )
        .unwrap();
        ws.set_snapshot_policy(false, 0);
        ws.apply(LogOp {
            ts: hlc.next(),
            actor: me,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        })
        .unwrap();
    }

    // The peer's op arrives by sync, stamped three days ahead of our wall
    // clock. Nothing pathological: NTP correction on either side, a VM
    // resuming with a stale clock, a restored backup.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_millis() as u64;
    let peer_ts = Hlc {
        physical_ms: now_ms + outl_core::hlc::MAX_CLOCK_SKEW_MS / 2,
        logical: 0,
        actor: peer,
    };
    {
        let mut peer_storage = JsonlStorage::open(ops_dir.clone(), peer).unwrap();
        peer_storage
            .append_op(&LogOp {
                ts: peer_ts,
                actor: peer,
                op: Op::Create {
                    node: NodeId::new(),
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            })
            .unwrap();
    }

    let engine = SyncEngine::new(root.to_path_buf(), me);
    let mut fresh = engine
        .reload_workspace(&hlc)
        .expect("reload merges the peer's ops");
    assert_eq!(
        fresh.log().last().map(|o| o.ts),
        Some(peer_ts),
        "precondition: the peer's op is the newest thing the merged log holds"
    );

    // The next local edit, stamped from the same generator the client
    // kept holding across the reload.
    let mine = LogOp {
        ts: hlc.next(),
        actor: me,
        op: Op::Create {
            node: NodeId::new(),
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    };
    let mine_ts = mine.ts;
    fresh.apply(mine).unwrap();

    assert!(
        mine_ts > peer_ts,
        "a local op stamped after the reload must sort above the peer op it merged: \
         {mine_ts:?} vs {peer_ts:?}"
    );
    assert_eq!(
        fresh.log().last().map(|o| o.ts),
        Some(mine_ts),
        "our op must land at the tail; anywhere else means it forced the undo/redo \
         window back over every newer entry"
    );
}
