//! End-to-end behaviour of the `ops/` index sidecars: which files a real
//! `JsonlStorage` writes, which ones it reads, and what compaction leaves
//! behind.
//!
//! The unit tests in `storage/sidecar/` pin the naming rules. These pin
//! that the storage layer actually goes through them — a correct rule
//! nothing calls is how the undotted generation survived a rename.

use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::sidecar::{self, SidecarKind};
use outl_core::storage::{JsonlStorage, PageScope, Storage};
use tempfile::TempDir;

fn create(g: &HlcGenerator, node: NodeId) -> LogOp {
    let ts = g.next();
    LogOp {
        ts,
        actor: ts.actor,
        op: Op::Create {
            node,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    }
}

fn names_in(dir: &std::path::Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// The whole basis for the GC calling `ops-<actor>.idx` dead is that
/// nothing reads it. So: plant a *correct-looking but wrong* undotted
/// index next to a real log, and prove a boot ignores it completely.
///
/// If someone reintroduces an undotted reader, these poisoned offsets
/// become live and this test fails — before the GC turns into a
/// data-losing bug.
#[test]
fn an_undotted_sidecar_is_never_read() {
    let tmp = TempDir::new().unwrap();
    let ops = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut storage = JsonlStorage::open(ops.clone(), actor).unwrap();
    let written: Vec<LogOp> = (0..5).map(|_| create(&g, NodeId::new())).collect();
    storage.append_ops(&written).unwrap();
    drop(storage);

    // A plausible index that points everywhere except the truth.
    let poison: String = written
        .iter()
        .map(|op| {
            format!(
                "{{\"ts\":{},\"offset\":999999}}\n",
                serde_json::to_string(&op.ts).unwrap()
            )
        })
        .collect();
    std::fs::write(ops.join(format!("ops-{actor}.idx")), &poison).unwrap();
    std::fs::write(ops.join(format!("ops-{actor}.nodes.idx")), &poison).unwrap();

    let reopened = JsonlStorage::open(ops.clone(), actor).unwrap();
    let read = reopened.all_ops().unwrap();
    assert_eq!(
        read.len(),
        written.len(),
        "the undotted sidecar must not be consulted — if it were, these offsets would break \
         the read, and the GC that deletes it would be deleting a live cache"
    );

    // And the file the boot *did* use is the dotted one.
    for kind in SidecarKind::ALL {
        assert!(
            sidecar::path_for(&ops, actor, &PageScope::Global, kind).exists(),
            "the dotted sidecar is the one the reader composes"
        );
    }
}

/// Under `PerPage`, one actor owns many `.jsonl` files inside
/// `ops/<actor>/`. Each must get its own index, or two pages' byte
/// offsets end up interleaved in one file.
#[test]
fn per_page_shards_write_separate_sidecars_on_disk() {
    let tmp = TempDir::new().unwrap();
    let ops = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    for slug in ["home", "work"] {
        let mut shard = JsonlStorage::open_with_scope_cap(
            ops.clone(),
            actor,
            PageScope::PerPage(slug.into()),
            0,
        )
        .unwrap();
        shard.append_op(&create(&g, NodeId::new())).unwrap();
    }

    let shard_dir = ops.join(actor.to_string());
    let names = names_in(&shard_dir);
    for slug in ["home", "work"] {
        assert!(names.contains(&format!("{slug}.jsonl")), "{names:?}");
        assert!(names.contains(&format!(".{slug}.idx")), "{names:?}");
        assert!(names.contains(&format!(".{slug}.nodes.idx")), "{names:?}");
    }
    assert!(
        !names.contains(&format!(".ops-{actor}.idx")),
        "an actor-named sidecar in a per-page directory is shared by every page of that \
         actor — offsets into two different `.jsonl` files in one index: {names:?}"
    );
}

/// A page shard reads back exactly its own ops after both shards have
/// written — the behavioural consequence of the test above.
#[test]
fn a_per_page_shard_reads_back_only_its_own_ops() {
    let tmp = TempDir::new().unwrap();
    let ops = tmp.path().to_path_buf();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut home_ops = Vec::new();
    {
        let mut home = JsonlStorage::open_with_scope_cap(
            ops.clone(),
            actor,
            PageScope::PerPage("home".into()),
            0,
        )
        .unwrap();
        for _ in 0..3 {
            let op = create(&g, NodeId::new());
            home.append_op(&op).unwrap();
            home_ops.push(op);
        }
    }
    {
        let mut work = JsonlStorage::open_with_scope_cap(
            ops.clone(),
            actor,
            PageScope::PerPage("work".into()),
            0,
        )
        .unwrap();
        for _ in 0..4 {
            work.append_op(&create(&g, NodeId::new())).unwrap();
        }
    }

    let home = JsonlStorage::open_with_scope_cap(ops, actor, PageScope::PerPage("home".into()), 0)
        .unwrap();
    let read = home.all_ops().unwrap();
    assert_eq!(read.len(), home_ops.len());
    for op in &home_ops {
        assert!(read.iter().any(|r| r.ts == op.ts));
    }
}

/// Compaction renumbers every byte offset in a `.jsonl`, so a sidecar it
/// does not know to delete is a wrong offset on the next cold read — and
/// the dead generations hold offsets into that same file.
#[test]
fn compaction_leaves_no_index_sidecar_behind() {
    use outl_core::storage::compact::{apply_compaction, plan_compaction, CompactOptions};

    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    let ops = root.join("ops");
    std::fs::create_dir_all(root.join(".outl")).unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut storage = JsonlStorage::open(ops.clone(), actor).unwrap();
    let parent = NodeId::new();
    let mut batch = vec![create(&g, parent)];
    for _ in 0..4 {
        let node = NodeId::new();
        let ts = g.next();
        batch.push(LogOp {
            ts,
            actor: ts.actor,
            op: Op::Create {
                node,
                parent,
                position: Fractional::first(),
            },
        });
        // A `Move` restating the placement its own `Create` just made:
        // the one shape compaction can prove inert.
        let ts = g.next();
        batch.push(LogOp {
            ts,
            actor: ts.actor,
            op: Op::Move {
                node,
                new_parent: parent,
                position: Fractional::first(),
                old_parent: NodeId::root(),
                old_position: Fractional::first(),
            },
        });
    }
    storage.append_ops(&batch).unwrap();
    drop(storage);

    // Every dead generation compaction must also clear.
    std::fs::write(ops.join(format!("ops-{actor}.idx")), "stale\n").unwrap();
    std::fs::write(ops.join(format!("ops-{actor}.nodes.idx")), "stale\n").unwrap();
    let old_tmp = ops.join(format!(".ops-{actor}.idx.tmp.{}", ulid::Ulid::new()));
    std::fs::write(&old_tmp, "half written\n").unwrap();
    filetime::set_file_mtime(
        &old_tmp,
        filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - std::time::Duration::from_secs(60 * 60 * 48),
        ),
    )
    .unwrap();

    let plan = plan_compaction(&root, &CompactOptions { horizon_ms: 0 }).unwrap();
    assert!(!plan.is_empty(), "the fixture must produce droppable ops");
    apply_compaction(&root, &plan).unwrap();

    let leftovers: Vec<String> = names_in(&ops)
        .into_iter()
        .filter(|n| n.contains(".idx"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "compaction renumbered every offset; nothing index-shaped may survive it: {leftovers:?}"
    );
}
