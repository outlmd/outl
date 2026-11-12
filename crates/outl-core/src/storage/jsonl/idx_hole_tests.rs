//! A sidecar must never lose entries to a concurrent persist, and one that
//! already has a hole must be repairable.
//!
//! See `idx_lock.rs` for the race. The census (`actor_timestamps`) is what a
//! sync peer compares counts against, so it is the observable here.

use std::path::{Path, PathBuf};

use super::*;
use crate::fractional::Fractional;
use crate::hlc::HlcGenerator;
use crate::op::Op;
use crate::storage::sidecar::{self, SidecarKind};
use crate::storage::Storage;
use tempfile::TempDir;

fn batch(g: &HlcGenerator, n: usize) -> Vec<LogOp> {
    (0..n)
        .map(|_| {
            let ts = g.next();
            LogOp {
                ts,
                actor: ts.actor,
                op: Op::Create {
                    node: NodeId::new(),
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            }
        })
        .collect()
}

fn census_count(dir: &Path, actor: ActorId) -> usize {
    JsonlStorage::actor_timestamps(dir)
        .unwrap()
        .get(&actor)
        .map_or(0, |set| set.len())
}

fn sidecar_paths(dir: &Path, actor: ActorId) -> (PathBuf, PathBuf) {
    (
        sidecar::path_for(dir, actor, &PageScope::Global, SidecarKind::Offset),
        sidecar::path_for(dir, actor, &PageScope::Global, SidecarKind::Node),
    )
}

/// The writer appends between a second opener's read of the grown tail and its
/// rename. Without the re-index under the lock, the renamed sidecar lacks the
/// writer's batch, and the next append makes the hole look exact.
#[test]
fn a_persist_racing_an_append_leaves_no_hole() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let (idx_path, node_idx_path) = sidecar_paths(&dir, actor);
    let jsonl_path = dir.join(format!("ops-{actor}.jsonl"));

    let mut writer = JsonlStorage::open(dir.clone(), actor).unwrap();
    writer.append_ops(&batch(&g, 3)).unwrap();

    // Leave the sidecars behind the log, as they are for an opener that
    // arrives after the writer has grown the file.
    let stale_idx = std::fs::read(&idx_path).unwrap();
    let stale_node = std::fs::read(&node_idx_path).unwrap();
    writer.append_ops(&batch(&g, 3)).unwrap();
    std::fs::write(&idx_path, stale_idx).unwrap();
    std::fs::write(&node_idx_path, stale_node).unwrap();

    // t1: the opener indexes the grown tail in memory.
    let (mut offset_index, mut node_index, grew) =
        JsonlStorage::load_unpersisted_indexes(&jsonl_path, &idx_path, &node_idx_path)
            .expect("stale sidecars are trusted as grown");
    assert!(grew);

    // The writer lands another batch before the opener's rename.
    writer.append_ops(&batch(&g, 3)).unwrap();

    // t2: the opener persists.
    JsonlStorage::persist_indexes_locked(
        &jsonl_path,
        &idx_path,
        &node_idx_path,
        &mut offset_index,
        &mut node_index,
    );

    // Later appends go to the renamed file and make its max-offset record
    // valid, which is what let the ladder accept the hole as exact.
    writer.append_ops(&batch(&g, 3)).unwrap();

    assert_eq!(census_count(&dir, actor), 12, "census must see every op");
    let reopened = JsonlStorage::open(dir, actor).unwrap();
    assert_eq!(reopened.all_ops().unwrap().len(), 12);
}

/// A hole already on disk passes the freshness ladder; the rebuild fixes it.
#[test]
fn a_holed_sidecar_is_rebuilt_from_the_jsonl() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let (idx_path, _) = sidecar_paths(&dir, actor);

    let mut writer = JsonlStorage::open(dir.clone(), actor).unwrap();
    writer.append_ops(&batch(&g, 6)).unwrap();
    drop(writer);

    let text = std::fs::read_to_string(&idx_path).unwrap();
    let kept: Vec<&str> = text
        .lines()
        .enumerate()
        .filter(|(i, _)| *i != 2 && *i != 3)
        .map(|(_, l)| l)
        .collect();
    std::fs::write(&idx_path, kept.join("\n") + "\n").unwrap();

    assert_eq!(
        census_count(&dir, actor),
        4,
        "fixture: the ladder trusts the holed sidecar"
    );

    let rebuilt = JsonlStorage::rebuild_own_indexes(&dir, actor).unwrap();
    assert_eq!(rebuilt.len(), 6);
    assert_eq!(census_count(&dir, actor), 6);
}
