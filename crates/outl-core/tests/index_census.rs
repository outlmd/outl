//! `JsonlStorage::actor_timestamps` — the metadata-only read behind every
//! vector clock.
//!
//! Two questions, and the second is the one that regresses: does it agree
//! with the `all_ops` fold it replaced, and does it stay off the two things
//! that made that fold cost a full workspace load — the node index and the
//! ops themselves?
//!
//! The cheap path exists because `peer_status` used to answer "what is my
//! sync status" by opening the whole op log: both index sidecars for every
//! actor, then every `Op::Edit`'s `text_op` parsed back out of the `.jsonl`.
//! On a 222,536-op workspace that is 132 MB of I/O per call, and on iOS it
//! ran on the main thread until the scene-update watchdog killed the app.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use outl_core::fractional::Fractional;
use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::storage::sidecar::{self, SidecarKind};
use outl_core::storage::{JsonlStorage, PageScope, Storage};

/// Write one actor's `.jsonl` by hand, so the directory starts with no
/// sidecars at all — the state a fresh install and a post-`outl compact`
/// workspace are both in.
fn write_log(ops_dir: &Path, actor: ActorId, count: usize) -> Vec<LogOp> {
    let gen = HlcGenerator::new(actor);
    let ops: Vec<LogOp> = (0..count)
        .map(|_| {
            let ts = gen.next();
            LogOp {
                ts,
                actor,
                op: Op::Create {
                    node: NodeId::new(),
                    parent: NodeId::root(),
                    position: Fractional::first(),
                },
            }
        })
        .collect();
    let body: String = ops
        .iter()
        .map(|op| format!("{}\n", serde_json::to_string(op).expect("encode")))
        .collect();
    std::fs::create_dir_all(ops_dir).expect("ops dir");
    std::fs::write(ops_dir.join(format!("ops-{actor}.jsonl")), body).expect("write log");
    ops
}

/// The census the `all_ops` fold produces: every distinct HLC, grouped by
/// the producer inside it. Written out longhand rather than shared with the
/// implementation — a shared helper would let one bug satisfy both sides.
fn fold_census(ops_dir: &Path, actor: ActorId) -> HashMap<ActorId, BTreeSet<Hlc>> {
    let storage = JsonlStorage::open(ops_dir.to_path_buf(), actor).expect("open");
    let mut out: HashMap<ActorId, BTreeSet<Hlc>> = HashMap::new();
    for op in storage.all_ops().expect("all_ops") {
        out.entry(op.ts.actor).or_default().insert(op.ts);
    }
    out
}

fn sidecar_paths(ops_dir: &Path, actor: ActorId) -> (std::path::PathBuf, std::path::PathBuf) {
    (
        sidecar::path_for(ops_dir, actor, &PageScope::Global, SidecarKind::Offset),
        sidecar::path_for(ops_dir, actor, &PageScope::Global, SidecarKind::Node),
    )
}

/// The replaced read and the replacement must see the same op log —
/// sidecars or not, and whether the `.jsonl` grew since they were written.
#[test]
fn the_index_census_matches_an_all_ops_fold() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (a, b, c) = (ActorId::new(), ActorId::new(), ActorId::new());
    write_log(&ops_dir, a, 12);
    write_log(&ops_dir, b, 5);
    write_log(&ops_dir, c, 1);

    // Cold: no sidecars anywhere, so the census falls back to the shared
    // parse-lite stream.
    let cold = JsonlStorage::actor_timestamps(&ops_dir).expect("census");

    // The fold also writes the sidecars out as a side effect, so the next
    // census takes the byte-exact-fresh arm of the ladder instead.
    let folded = fold_census(&ops_dir, a);
    let warm = JsonlStorage::actor_timestamps(&ops_dir).expect("census");

    // Grown: one more line than the sidecar accounts for — the state every
    // peer's file is in right after a sync ingest, which appends to the
    // `.jsonl` without touching the sidecars.
    let extra = write_log(&ops_dir, b, 7);
    let grown = JsonlStorage::actor_timestamps(&ops_dir).expect("census");

    assert_eq!(cold, folded, "cold census must match the all_ops fold");
    assert_eq!(warm, folded, "a fresh sidecar must not change the answer");
    assert_eq!(
        grown,
        fold_census(&ops_dir, a),
        "a grown .jsonl must be re-counted, not reported at the sidecar's extent"
    );
    assert_eq!(grown[&b].len(), extra.len());
    assert_eq!(grown.len(), 3, "every actor with a file is counted");
}

/// The node index is the expensive half of `reload` — a
/// `HashMap<NodeId, Vec<_>>` filled one `entry()` per op, and the innermost
/// frame of the iOS watchdog kill. The census has no use for it and must
/// neither read nor rebuild it.
///
/// Deleting the sidecars and asserting they stay deleted is what makes this
/// fail if the census is ever rerouted back through `JsonlStorage::open`:
/// that path rebuilds both indexes from the `.jsonl` and persists them.
#[test]
fn the_actor_census_neither_reads_nor_writes_the_node_index() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let actor = ActorId::new();
    let ops = write_log(&ops_dir, actor, 9);

    // Materialize both sidecars, then take the node one away.
    let expected = fold_census(&ops_dir, actor);
    let (offset_idx, node_idx) = sidecar_paths(&ops_dir, actor);
    assert!(node_idx.exists(), "the fold should have written it");
    std::fs::remove_file(&node_idx).expect("remove node sidecar");

    assert_eq!(JsonlStorage::actor_timestamps(&ops_dir).unwrap(), expected);
    assert!(
        !node_idx.exists(),
        "the census rebuilt the node index — it was rerouted through a full open"
    );

    // Same with no sidecar at all to lean on: the fallback streams the
    // `.jsonl` and still writes nothing.
    std::fs::remove_file(&offset_idx).expect("remove offset sidecar");
    let census = JsonlStorage::actor_timestamps(&ops_dir).expect("census");
    assert_eq!(census[&actor].len(), ops.len());
    assert!(
        !offset_idx.exists() && !node_idx.exists(),
        "a status read must not persist an index; advancing the offset sidecar \
         past the node sidecar makes the next real boot rebuild both"
    );
}

/// A workspace that has never synced has no `ops/` directory. Answering
/// "nothing" is right; creating the directory is `open`'s job.
#[test]
fn a_missing_ops_dir_is_an_empty_census_not_a_new_directory() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    assert!(JsonlStorage::actor_timestamps(&ops_dir).unwrap().is_empty());
    assert!(!ops_dir.exists(), "a read must not create the ops dir");
}

/// A file-sync conflict copy matches `ops-*.jsonl` but carries no valid
/// actor id, so there is no sidecar name to derive. `reload_global` skips
/// it; the census has to skip the same file or the two disagree about what
/// the workspace holds.
#[test]
fn a_conflict_copy_is_skipped_the_way_reload_skips_it() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (kept, stray) = (ActorId::new(), ActorId::new());
    write_log(&ops_dir, kept, 4);

    // The copy holds a DIFFERENT actor's ops, so reading it would show up as
    // an extra actor in the census rather than dedup away against the ops we
    // already counted.
    write_log(&ops_dir, stray, 3);
    std::fs::rename(
        ops_dir.join(format!("ops-{stray}.jsonl")),
        ops_dir.join(format!("ops-{stray} 2.jsonl")),
    )
    .expect("conflict copy");

    let census = JsonlStorage::actor_timestamps(&ops_dir).expect("census");
    assert_eq!(census, fold_census(&ops_dir, kept));
    assert_eq!(census[&kept].len(), 4);
    assert!(
        !census.contains_key(&stray),
        "a conflict copy has no derivable sidecar name; reload skips it, so must the census"
    );
}

// ── reading the ops the census selected, without an open ────────────────
//
// The census answers *which* ops exist. The sender then needs the op
// *bodies* for the handful it picked, and used to get them by calling
// `JsonlStorage::open` — the full index load for every actor, both
// sidecars, which is the single most expensive phase of a launch (169 MB
// and 294 ms on a real 23-actor workspace) paid per sync exchange, per
// peer, to read a few hundred ops. The census already holds each pick's
// byte offset: these two functions are what let that offset be used.

/// The offsets the census hands back have to actually address the ops, and
/// reading them must not open storage. Deleting the node sidecar and
/// asserting it stays deleted is the same signature
/// `the_actor_census_neither_reads_nor_writes_the_node_index` uses: a
/// reroute through `JsonlStorage::open` rebuilds and persists it.
#[test]
fn ops_are_read_back_from_census_offsets_without_opening_storage() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (a, b) = (ActorId::new(), ActorId::new());
    let ops_a = write_log(&ops_dir, a, 7);
    let ops_b = write_log(&ops_dir, b, 3);

    // Materialize the sidecars via the fold, then take the node one away.
    let _ = fold_census(&ops_dir, a);
    let (_, node_idx) = sidecar_paths(&ops_dir, a);
    std::fs::remove_file(&node_idx).expect("remove node sidecar");

    let census = JsonlStorage::actor_offset_indexes(&ops_dir).expect("census");
    assert_eq!(census.len(), 2);

    // Every op, addressed by the offset the census recorded for it.
    let mut picks: Vec<(ActorId, Hlc, u64)> = Vec::new();
    for (actor, index) in &census {
        for ts in index.timestamps() {
            picks.push((*actor, *ts, index.get(ts).expect("offset for a listed ts")));
        }
    }
    picks.sort_unstable_by_key(|(_, ts, _)| *ts);

    let read = JsonlStorage::read_ops_at_offsets(&ops_dir, &picks).expect("read");
    assert_eq!(read.len(), ops_a.len() + ops_b.len());
    let want: BTreeSet<Hlc> = ops_a.iter().chain(&ops_b).map(|op| op.ts).collect();
    assert_eq!(read.iter().map(|op| op.ts).collect::<BTreeSet<_>>(), want);

    assert!(
        !node_idx.exists(),
        "reading ops by offset rebuilt the node index — it went through a full open"
    );
}

/// The census is the complete op set, so "the census listed it and the file
/// will not give it back" means the log is damaged. A short read here is
/// indistinguishable from a healthy one, and the receiver would record the
/// gap as settled — the same reasoning `read_op_or_missing` carries.
#[test]
fn an_offset_that_cannot_be_read_is_an_error_not_an_omission() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let actor = ActorId::new();
    let ops = write_log(&ops_dir, actor, 4);

    let past_eof = std::fs::metadata(ops_dir.join(format!("ops-{actor}.jsonl")))
        .expect("stat")
        .len()
        + 10_000;
    let picks = [(actor, ops[0].ts, past_eof)];
    assert!(JsonlStorage::read_ops_at_offsets(&ops_dir, &picks).is_err());
}

/// `actor_timestamps` is the HLC-only projection of the same read, so the
/// two can never disagree about what the workspace holds.
#[test]
fn the_timestamp_census_is_the_projection_of_the_offset_census() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let (a, b) = (ActorId::new(), ActorId::new());
    write_log(&ops_dir, a, 11);
    write_log(&ops_dir, b, 2);

    let offsets = JsonlStorage::actor_offset_indexes(&ops_dir).expect("census");
    let projected: HashMap<ActorId, BTreeSet<Hlc>> = offsets
        .iter()
        .map(|(actor, idx)| (*actor, idx.timestamps().copied().collect()))
        .collect();

    assert_eq!(
        JsonlStorage::actor_timestamps(&ops_dir).expect("census"),
        projected
    );
}

/// `count_up_to` is the gap check's half of the vector-clock comparison
/// ("do we hold more distinct ops at or below the peer's watermark than it
/// says it has"). It used to be `BTreeSet::range(..=max).count()` at the
/// call site; keeping it on the index is what lets the census stop
/// materializing a second copy of every HLC.
#[test]
fn count_up_to_counts_the_entries_at_or_below_a_watermark() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let actor = ActorId::new();
    let ops = write_log(&ops_dir, actor, 6);

    let census = JsonlStorage::actor_offset_indexes(&ops_dir).expect("census");
    let index = &census[&actor];

    assert_eq!(index.count_up_to(ops[0].ts), 1, "inclusive of the bound");
    assert_eq!(index.count_up_to(ops[3].ts), 4);
    assert_eq!(index.count_up_to(ops[5].ts), 6);
    assert_eq!(index.len(), 6);
}

// ── the cheap read and the read it replaced must agree ──────────────────
//
// The tests above pin that the census path does not *open* storage. These
// pin that it returns the same thing the open did — which is the half
// that answers "did I break sync", because what crosses the wire is these
// op bodies, not their HLCs.

/// Ops of every variant that carries a body worth misreading, written to
/// disk the way a real log holds them.
///
/// `Op::Edit`'s payload is a Yrs `update_v1` delta, so it has to come out
/// of the real encoder — arbitrary bytes round-trip through serde and
/// prove nothing about a reader that has to replay them. The throwaway
/// in-memory workspace is what produces them, the same way
/// `block_text_is_indexed_not_scanned` does.
fn write_mixed_log(ops_dir: &Path, actor: ActorId) -> Vec<LogOp> {
    use outl_core::storage::MemoryStorage;
    use outl_core::workspace::Workspace;

    let gen = HlcGenerator::new(actor);
    let mut ws = Workspace::open_with_storage(actor, Box::new(MemoryStorage::default()), None)
        .expect("throwaway workspace");
    let mut ops: Vec<LogOp> = Vec::new();

    let page = NodeId::new();
    let push = |op: Op, ws: &mut Workspace, ops: &mut Vec<LogOp>| {
        let logged = LogOp {
            ts: gen.next(),
            actor,
            op,
        };
        ws.apply(logged.clone()).expect("apply");
        ops.push(logged);
    };

    push(
        Op::Create {
            node: page,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
        &mut ws,
        &mut ops,
    );

    let mut blocks = Vec::new();
    for _ in 0..4 {
        let node = NodeId::new();
        blocks.push(node);
        push(
            Op::Create {
                node,
                parent: page,
                position: Fractional::first(),
            },
            &mut ws,
            &mut ops,
        );
    }

    // Text long enough that a short read would be visible, and holding a
    // newline plus non-ASCII — the two things a line-oriented reader gets
    // wrong (root `CLAUDE.md` invariant 8's producer half).
    for (i, node) in blocks.iter().enumerate() {
        let text = format!("linha {i} com acento ção e emoji 🌊\n  continuação indentada");
        let update = ws.build_text_replace_update(*node, &text);
        if update.is_empty() {
            continue;
        }
        push(
            Op::Edit {
                node: *node,
                text_op: update,
            },
            &mut ws,
            &mut ops,
        );
    }

    push(
        Op::SetProp {
            node: page,
            key: "page-slug".into(),
            value: Some(outl_core::property::PropValue::Text("mixed".into())),
            old_value: None,
        },
        &mut ws,
        &mut ops,
    );
    push(
        Op::Move {
            node: blocks[0],
            new_parent: blocks[1],
            position: Fractional::first(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        },
        &mut ws,
        &mut ops,
    );

    let body: String = ops
        .iter()
        .map(|op| format!("{}\n", serde_json::to_string(op).expect("encode")))
        .collect();
    std::fs::create_dir_all(ops_dir).expect("ops dir");
    std::fs::write(ops_dir.join(format!("ops-{actor}.jsonl")), body).expect("write log");
    ops
}

/// **The equivalence that makes this safe.** Reading by census offset has
/// to produce byte-identical ops to opening the storage and reading the
/// same picks — for every variant, including an `Op::Edit` whose payload
/// is a Yrs delta and whose text carries a newline.
///
/// If this holds, the bytes that cross the wire are unchanged and sync
/// behaviour cannot have moved. If it ever stops holding, the two readers
/// disagree about the op log, which is the one failure the sync path
/// cannot absorb.
#[test]
fn reading_by_offset_returns_the_same_ops_as_opening_the_storage() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let actor = ActorId::new();
    let written = write_mixed_log(&ops_dir, actor);

    let census = JsonlStorage::actor_offset_indexes(&ops_dir).expect("census");
    let mut picks: Vec<(ActorId, Hlc, u64)> = census
        .iter()
        .flat_map(|(a, index)| {
            index
                .entries()
                .map(move |(ts, off)| (*a, *ts, *off))
                .collect::<Vec<_>>()
        })
        .collect();
    picks.sort_unstable_by_key(|(_, ts, _)| *ts);

    let by_offset = JsonlStorage::read_ops_at_offsets(&ops_dir, &picks).expect("read by offset");

    // The path this replaced: open the storage and read the same picks.
    let storage = JsonlStorage::open(ops_dir.clone(), actor).expect("open");
    let by_open = storage
        .read_ops_at(&picks.iter().map(|(a, ts, _)| (*a, *ts)).collect::<Vec<_>>())
        .expect("read through open");

    assert_eq!(
        by_offset.len(),
        written.len(),
        "every written op is addressable by its census offset"
    );
    assert_eq!(
        by_offset, by_open,
        "the offset read and the open read disagree about the op log"
    );

    // And both agree with what was actually written, so neither is
    // consistently wrong in the same way.
    let mut expected = written;
    expected.sort_by_key(|op| op.ts);
    assert_eq!(by_offset, expected);

    // An `Op::Edit`'s delta survived, which a serde round-trip of the
    // envelope alone would not have proven.
    assert!(
        by_offset.iter().any(|op| matches!(
            &op.op,
            Op::Edit { text_op, .. } if !text_op.is_empty()
        )),
        "the fixture has to contain a non-empty Yrs delta or this proves nothing"
    );
}

/// The census is read off a sidecar when one is fresh and streamed from
/// the `.jsonl` when it is not, and **the offsets have to be right in both
/// arms**. A stale-sidecar workspace is the normal state right after a
/// sync ingest, which appends without touching the sidecars.
#[test]
fn offsets_are_right_whether_the_sidecar_is_fresh_cold_or_grown() {
    let tmp = tempfile::tempdir().expect("tmp");
    let ops_dir = tmp.path().join("ops");
    let actor = ActorId::new();
    let first = write_mixed_log(&ops_dir, actor);

    // Cold: no sidecar at all, so the census streams the `.jsonl`.
    let read_back = |dir: &Path| -> Vec<LogOp> {
        let census = JsonlStorage::actor_offset_indexes(dir).expect("census");
        let mut picks: Vec<(ActorId, Hlc, u64)> = census
            .iter()
            .flat_map(|(a, i)| {
                i.entries()
                    .map(move |(ts, off)| (*a, *ts, *off))
                    .collect::<Vec<_>>()
            })
            .collect();
        picks.sort_unstable_by_key(|(_, ts, _)| *ts);
        JsonlStorage::read_ops_at_offsets(dir, &picks).expect("read")
    };
    let cold = read_back(&ops_dir);
    assert_eq!(cold.len(), first.len());

    // Fresh: the fold writes the sidecars, so the ladder takes its
    // byte-exact arm.
    let _ = fold_census(&ops_dir, actor);
    let warm = read_back(&ops_dir);
    assert_eq!(warm, cold, "a fresh sidecar must not move an offset");

    // Grown: a second actor's file appended after those sidecars were
    // written — the state every peer's file is in after an ingest.
    let peer = ActorId::new();
    let appended = write_mixed_log(&ops_dir, peer);
    let grown = read_back(&ops_dir);
    assert_eq!(
        grown.len(),
        first.len() + appended.len(),
        "a grown log is re-scanned, and its tail's offsets address real ops"
    );
    let want: BTreeSet<Hlc> = first.iter().chain(&appended).map(|op| op.ts).collect();
    assert_eq!(grown.iter().map(|op| op.ts).collect::<BTreeSet<_>>(), want);
}
