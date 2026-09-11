//! Read-path robustness for [`JsonlStorage`]: what a damaged `.jsonl`
//! costs, and what it must never cost.
//!
//! Sibling of `tests.rs` (which covers the happy path, the LRU bound and
//! the scope layouts) because these cases share one thesis and one setup
//! style: **a damaged log may cost you the damaged bytes, never the
//! healthy bytes after them, and never quietly** — invariant #5's read
//! side.
//!
//! Two failure shapes, two required behaviours:
//!
//! - The **index build** must skip a bad record and keep going. A `break`
//!   there is the worst of the family: the resulting index never learns
//!   about the ops past the damage, so `read_op_or_missing` has nothing
//!   to report, no `StorageError::MissingOp` is raised, and the workspace
//!   boots short with a clean bill of health.
//! - The **index-driven collectors** must error rather than return a
//!   short set, because the index knowing an op the disk won't return is
//!   corruption — and the next snapshot's cutoff comes from that index.

use super::*;
use crate::fractional::Fractional;
use crate::hlc::HlcGenerator;
use crate::op::Op;
use crate::storage::sidecar;
use crate::storage::{NodeIndex, OffsetIndex, PageScope, Storage};
use tempfile::TempDir;

fn mk_create(g: &HlcGenerator) -> LogOp {
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
}

/// A `Read` that hands back one prepared chunk per call, except on the
/// call at `fail_at` (0-based), where it reports a device error **without
/// consuming anything** — the shape of a transient mid-file read failure.
struct FlakyReader {
    chunks: Vec<Vec<u8>>,
    next: usize,
    calls: usize,
    fail_at: usize,
}

impl std::io::Read for FlakyReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let call = self.calls;
        self.calls += 1;
        if call == self.fail_at {
            return Err(std::io::Error::other("simulated device read error"));
        }
        let Some(chunk) = self.chunks.get(self.next) else {
            return Ok(0);
        };
        self.next += 1;
        let n = chunk.len().min(out.len());
        out[..n].copy_from_slice(&chunk[..n]);
        Ok(n)
    }
}

/// A read error partway through the index build must cost that record and
/// nothing after it.
///
/// This is the one a `break` hides completely. A short offset index does
/// not *know* about the ops past the damage, so `read_op_or_missing` has
/// nothing to report and no `StorageError::MissingOp` is ever raised — the
/// workspace boots short with a clean bill of health.
#[test]
fn index_rebuild_skips_a_read_error_and_keeps_indexing() {
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);
    let ops: Vec<LogOp> = (0..4).map(|_| mk_create(&g)).collect();
    let chunks: Vec<Vec<u8>> = ops
        .iter()
        .map(|op| format!("{}\n", serde_json::to_string(op).unwrap()).into_bytes())
        .collect();

    // Fails on the read that would have delivered the third line.
    let mut reader = std::io::BufReader::new(FlakyReader {
        chunks,
        next: 0,
        calls: 0,
        fail_at: 2,
    });
    let mut hlc = OffsetIndex::new();
    let mut node = NodeIndex::new();
    let saw_io_error = JsonlStorage::index_stream(
        &mut reader,
        std::path::Path::new("flaky.jsonl"),
        &mut hlc,
        &mut node,
    );

    assert!(
        saw_io_error,
        "the pass must report that it hit a read error"
    );
    for op in &ops {
        assert!(
            hlc.get(&op.ts).is_some(),
            "every op must be indexed; a `break` on the error stopped at 2 of 4"
        );
    }
}

/// The consecutive-error cap still bounds a genuinely dead file handle:
/// skipping is for a one-off bad record, not for spinning forever.
#[test]
fn index_rebuild_gives_up_on_an_endlessly_failing_reader() {
    struct AlwaysFails;
    impl std::io::Read for AlwaysFails {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("gone"))
        }
    }

    let mut reader = std::io::BufReader::new(AlwaysFails);
    let mut hlc = OffsetIndex::new();
    let mut node = NodeIndex::new();
    // Terminates (the assertion is that this returns at all).
    let saw_io_error = JsonlStorage::index_stream(
        &mut reader,
        std::path::Path::new("dead.jsonl"),
        &mut hlc,
        &mut node,
    );
    assert!(saw_io_error);
    assert_eq!(hlc.len(), 0);
}

/// A corrupt line in the MIDDLE of the log must not stop the index build.
/// The index is what defines the known op set, so ops after the damage
/// have to be in it — otherwise they are invisible to every index-driven
/// read and to the snapshot cutoff alike.
///
/// A parse failure is a `RecordRead::Skip`, so this passed before the
/// `break` was replaced too. It is here as the end-to-end statement of the
/// property (index → `ops_since` → `ops_for_node`) that the unit test
/// above pins at the loop level for the *read-error* shape.
#[test]
fn index_rebuild_covers_ops_after_a_corrupt_middle_line() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let ops: Vec<LogOp> = {
        let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
        let ops: Vec<LogOp> = (0..7).map(|_| mk_create(&g)).collect();
        for op in &ops {
            storage.append_op(op).unwrap();
        }
        ops
    };

    // Corrupt the middle line and drop the index sidecars so the next open
    // has to rebuild from the `.jsonl`.
    let path = tmp.path().join(format!("ops-{actor}.jsonl"));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines[3] = "{ not json at all".to_string();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    for path in sidecar::paths_for(tmp.path(), actor, &PageScope::Global) {
        let _ = std::fs::remove_file(path);
    }

    let storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();

    // `ops_since` is index-driven: it can only return what the index knows.
    let seen = storage.ops_since(Hlc::new(0, 0, actor)).unwrap();
    let seen_ts: Vec<_> = seen.iter().map(|o| o.ts).collect();
    for op in ops.iter().skip(4) {
        assert!(
            seen_ts.contains(&op.ts),
            "ops after the corrupt line must be indexed and readable, got {seen_ts:?}"
        );
    }
    assert_eq!(seen.len(), 6, "exactly the one corrupt record is lost");
    // And the per-node index reaches past the damage too.
    if let Op::Create { node, .. } = ops[6].op {
        assert_eq!(storage.ops_for_node(node).unwrap().len(), 1);
    } else {
        unreachable!("fixture builds Create ops");
    }
}

/// `ops_for_node` is the fourth index-driven collector. A short read here
/// does not shorten a list anyone inspects — it rebuilds the block's Yrs
/// `Doc` from partial `Edit` history and yields text the user never
/// wrote (#129). It must be an error.
#[test]
fn ops_for_node_surfaces_missing_ops_instead_of_dropping_them() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let g = HlcGenerator::new(actor);

    let mut storage = JsonlStorage::open(tmp.path().to_path_buf(), actor).unwrap();
    let create = mk_create(&g);
    let node = match create.op {
        Op::Create { node, .. } => node,
        _ => unreachable!(),
    };
    storage.append_op(&create).unwrap();
    for _ in 0..3 {
        let ts = g.next();
        storage
            .append_op(&LogOp {
                ts,
                actor,
                op: Op::Edit {
                    node,
                    text_op: vec![1, 2, 3],
                },
            })
            .unwrap();
    }

    // Truncate the file behind the index — a partial sync / bad sector.
    // The node index still lists all four ops.
    std::fs::write(tmp.path().join(format!("ops-{actor}.jsonl")), "").unwrap();
    Storage::resize_cache(&mut storage, 1);

    let err = storage
        .ops_for_node(node)
        .expect_err("partial Edit history rebuilds the block's text WRONG — never return it");
    assert!(
        err.to_string().contains("offset index"),
        "the error must name the index/file disagreement, got: {err}"
    );
}
