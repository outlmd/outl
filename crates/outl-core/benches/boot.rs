//! Benchmarks workspace **boot** — the path a user waits on.
//!
//! Every other number in this suite is a microbenchmark; this one is
//! the latency somebody actually feels, and it is the reason
//! [RFC 0128](../../../docs/rfcs/0128-boot-and-memory-at-scale.md) and
//! [RFC 0137](../../../docs/rfcs/0137-storage-scale.md) exist. The
//! reference workspace is 217,811 ops across 20 actors, so the `218k`
//! tier is production, not a stress test.
//!
//! Four boot modes are measured, because the crate has four and they
//! differ by orders of magnitude:
//!
//! - **`replay_memory`** — full op replay with no disk at all
//!   (`MemoryStorage`). Isolates CRDT + `ContentStore` cost from I/O.
//! - **`replay_cold`** — `JsonlStorage` with no `.idx` sidecars and no
//!   snapshot. The worst case: parse every line, rebuild both indexes,
//!   replay everything. This is a first open, and a boot after the
//!   snapshot is invalidated.
//! - **`replay_warm_index`** — the `.idx` sidecars present, no
//!   snapshot. Isolates what the persisted offset/node index buys.
//! - **`snapshot`** — snapshot present, so boot decodes materialized
//!   state and replays only the per-actor delta above the cutoff.
//!   The path a returning user takes, and the one to protect.
//!
//! The gap between `replay_cold` and `snapshot` is the headline
//! number; the gap between `replay_memory` and `replay_cold` is how
//! much of boot is I/O and parsing rather than the algorithm.
//!
//! Run:
//! ```text
//! cargo bench -p outl-core --bench boot
//! cargo bench -p outl-core --bench boot -- snapshot
//! ```

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::storage::{JsonlStorage, MemoryStorage, Storage};
use outl_core::workspace::Workspace;
use std::hint::black_box;
use tempfile::TempDir;

#[path = "common.rs"]
mod common;

use common::{actors, log_op, node};

/// Op-log sizes. `218k` is the reference workspace.
///
/// Boot is measured in tens of milliseconds upward, so criterion's
/// default 100 samples would make this bench take many minutes.
/// `sample_size` is lowered per group instead.
const BOOT_SIZES: &[(&str, usize)] = &[("10k", 10_000), ("50k", 50_000), ("218k", 217_811)];

/// How many actors interleave in the generated log. The reference
/// workspace has 20, and the count matters: `ops_since_per_actor`
/// keys the snapshot cutoff by actor, so a one-actor log would hide
/// whatever that map costs.
const ACTOR_COUNT: usize = 20;

/// Generate `n` ops shaped like a real workspace: mostly `Create` and
/// `Edit`, some `Move` and `SetProp`, spread over `ACTOR_COUNT`
/// actors in strictly increasing HLC order.
///
/// Strictly increasing is deliberate — this bench measures *boot*, and
/// a boot replays a log that is already ordered. The cost of
/// out-of-order arrival is `tree.rs`'s `apply_op/reorder` group.
fn synth_ops(n: usize) -> Vec<outl_core::op::LogOp> {
    let acts = actors(ACTOR_COUNT);
    let mut ops = Vec::with_capacity(n);
    for i in 0..n {
        let a = acts[i % ACTOR_COUNT];
        let ms = i as u64 + 1;
        let op = match i % 10 {
            // 40% creates — the tree has to exist before it can be edited.
            0..=3 => Op::Create {
                node: node(i),
                parent: if i < ACTOR_COUNT {
                    NodeId::root()
                } else {
                    node(i - ACTOR_COUNT)
                },
                position: outl_core::fractional::Fractional::first(),
            },
            // 40% edits — the tier that drives `ContentStore` work.
            4..=7 => Op::Edit {
                node: node(i / 2),
                text_op: vec![(i % 251) as u8; 48],
            },
            8 => Op::SetProp {
                node: node(i / 2),
                key: "title".to_string(),
                value: Some(outl_core::property::PropValue::Text(format!("page {i}"))),
                old_value: None,
            },
            _ => Op::Move {
                node: node(i / 2),
                new_parent: NodeId::root(),
                position: outl_core::fractional::Fractional::last(),
                old_parent: NodeId::root(),
                old_position: outl_core::fractional::Fractional::first(),
            },
        };
        ops.push(log_op(ms, 0, a, op));
    }
    ops
}

/// Write `ops` into a fresh `TempDir` as `ops-<actor>.jsonl` files and
/// return the dir. Kept alive by the caller — dropping it deletes the
/// fixture.
fn seed_ops_dir(ops: &[outl_core::op::LogOp]) -> (TempDir, ActorId) {
    let dir = TempDir::new().expect("tempdir");
    let ops_dir = dir.path().join("ops");
    std::fs::create_dir_all(&ops_dir).expect("mkdir ops");

    let primary = ops[0].actor;
    // Group by actor so each writes its own file, as production does.
    let mut by_actor: std::collections::HashMap<ActorId, Vec<outl_core::op::LogOp>> =
        std::collections::HashMap::new();
    for op in ops {
        by_actor.entry(op.actor).or_default().push(op.clone());
    }
    for (a, actor_ops) in by_actor {
        let mut storage = JsonlStorage::open(ops_dir.clone(), a).expect("open jsonl");
        storage.append_ops(&actor_ops).expect("append batch");
    }
    (dir, primary)
}

/// Remove the `.idx` / `.nodes.idx` sidecars so the next open has to
/// rebuild them — the cold path.
fn drop_indexes(ops_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(ops_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') && name.ends_with(".idx") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn bench_replay_memory(c: &mut Criterion) {
    let mut group = c.benchmark_group("boot/replay_memory");
    group.sample_size(10);
    for &(tier, n) in BOOT_SIZES {
        let ops = synth_ops(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(tier), &ops, |b, ops| {
            b.iter_batched(
                || {
                    let mut s = MemoryStorage::default();
                    s.append_ops(ops).expect("seed memory storage");
                    s
                },
                |s| {
                    let ws = Workspace::open_with_storage(ops[0].actor, Box::new(s), None)
                        .expect("open");
                    black_box(ws.tree().node_count())
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

fn bench_replay_disk(c: &mut Criterion) {
    let mut group = c.benchmark_group("boot/replay_disk");
    group.sample_size(10);

    for &(tier, n) in BOOT_SIZES {
        let ops = synth_ops(n);
        let (dir, primary) = seed_ops_dir(&ops);
        let ops_dir = dir.path().join("ops");

        group.throughput(Throughput::Elements(n as u64));

        // Cold: rebuild both indexes from the `.jsonl` every time.
        group.bench_function(BenchmarkId::new("cold_no_index", tier), |b| {
            b.iter_batched(
                || drop_indexes(&ops_dir),
                |()| {
                    let s = JsonlStorage::open(ops_dir.clone(), primary).expect("open jsonl");
                    let ws = Workspace::open_with_storage(primary, Box::new(s), None)
                        .expect("open workspace");
                    black_box(ws.tree().node_count())
                },
                criterion::BatchSize::PerIteration,
            );
        });

        // Warm: `.idx` sidecars already on disk. One open first so
        // they exist, then measure repeated opens.
        {
            let s = JsonlStorage::open(ops_dir.clone(), primary).expect("open jsonl");
            let _ = Workspace::open_with_storage(primary, Box::new(s), None).expect("prime index");
        }
        group.bench_function(BenchmarkId::new("warm_index", tier), |b| {
            b.iter(|| {
                let s = JsonlStorage::open(ops_dir.clone(), primary).expect("open jsonl");
                let ws = Workspace::open_with_storage(primary, Box::new(s), None)
                    .expect("open workspace");
                black_box(ws.tree().node_count())
            });
        });
    }
    group.finish();
}

/// Snapshot boot: the returning-user path. A snapshot is written once,
/// then every measured open decodes it and replays only the delta.
fn bench_snapshot_boot(c: &mut Criterion) {
    let mut group = c.benchmark_group("boot/snapshot");
    group.sample_size(10);

    for &(tier, n) in BOOT_SIZES {
        let ops = synth_ops(n);
        let (dir, primary) = seed_ops_dir(&ops);
        let root = dir.path().to_path_buf();
        let ops_dir = root.join("ops");

        // Prime: open once and write the snapshot.
        {
            let s = JsonlStorage::open(ops_dir.clone(), primary).expect("open jsonl");
            let mut ws = Workspace::open_with_storage(primary, Box::new(s), Some(root.clone()))
                .expect("open workspace");
            ws.save_snapshot().expect("write snapshot");
        }

        group.throughput(Throughput::Elements(n as u64));
        group.bench_function(BenchmarkId::from_parameter(tier), |b| {
            b.iter(|| {
                let s = JsonlStorage::open(ops_dir.clone(), primary).expect("open jsonl");
                let ws = Workspace::open_with_storage(primary, Box::new(s), Some(root.clone()))
                    .expect("open workspace");
                black_box(ws.tree().node_count())
            });
        });
    }
    group.finish();
}

/// `block_text` on a snapshotless boot is **lazy** (#179): the string
/// is rebuilt from the log on first read. This measures the first-read
/// cost, which is the one a user pays when scrolling into cold
/// content, and `resident_text_count` confirms the laziness is real
/// rather than assumed.
fn bench_lazy_block_text(c: &mut Criterion) {
    let mut group = c.benchmark_group("boot/lazy_block_text");
    group.sample_size(10);

    for &(tier, n) in &BOOT_SIZES[..2] {
        let ops = synth_ops(n);
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::new("first_read_all", tier), &ops, |b, ops| {
            b.iter_batched(
                || {
                    let mut s = MemoryStorage::default();
                    s.append_ops(ops).expect("seed");
                    Workspace::open_with_storage(ops[0].actor, Box::new(s), None).expect("open")
                },
                |ws| {
                    let mut found = 0usize;
                    for i in 0..n / 2 {
                        if ws.block_text(node(i)).is_some() {
                            found += 1;
                        }
                    }
                    black_box((found, ws.resident_text_count()))
                },
                criterion::BatchSize::LargeInput,
            );
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_replay_memory,
    bench_replay_disk,
    bench_snapshot_boot,
    bench_lazy_block_text,
);
criterion_main!(benches);
