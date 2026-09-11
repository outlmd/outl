//! The read-and-decide half of compaction. Writes nothing.
//!
//! # The predicate
//!
//! Let the **merged log** be every op in every `ops/ops-<actor>.jsonl`,
//! sorted by [`Hlc`] — which orders on `(physical_ms, logical, actor)`,
//! so the actor tiebreak is never skipped. Replaying that sequence
//! through [`Tree::do_op`] is the canonical materialization (the CRDT's
//! SEC property: any delivery order converges to it).
//!
//! A `Move{node: n, new_parent: P, position: X}` at `t2`, written by
//! actor `A`, is dropped iff **all six** hold:
//!
//! 1. **Pair.** The record immediately before it *in the merged HLC
//!    order* is `Create{node: n, parent: P, position: X}` at `t1`, by
//!    the same actor `A`, with `P` and `X` equal field-for-field.
//! 2. **First appearance.** `t1` is the minimum HLC among every op
//!    naming `n` anywhere in the merged log. So the `Create` is the one
//!    that actually created `n`, not an idempotent re-`Create` over a
//!    node that already existed somewhere else — the trashed-and-restored
//!    block, above all.
//! 3. **Sole `Create`.** No other `Create` anywhere in the merged log
//!    names `n`. Two `Create`s for one id means the id is *derivable*
//!    rather than minted (`NodeId::from_slug`, a template result block),
//!    and a device that has never exchanged an op with us can then mint
//!    it at an HLC below `t1`.
//! 4. **Not a page root.** `P != NodeId::root()`. Page and journal roots
//!    are the derivable-id family we know about; condition 3 only
//!    catches the ones already contested in *this* log.
//! 5. **Replay-verified inert.** In the replay, the tree state
//!    immediately before `t2` has `nodes[n] == (P, X)` — so applying the
//!    `Move` provably changes nothing. Conditions 1+2 imply this; it is
//!    asserted directly because this is the check that actually decides,
//!    and it is `do_op`'s own semantics rather than a restatement of
//!    them.
//! 6. **Settling horizon.** `t2` is at least [`CompactOptions::horizon_ms`]
//!    older than the newest op in the log.
//!
//! # Why that is sound
//!
//! *Against the log we hold*, conditions 1 and 5 are a decision
//! procedure, not a heuristic. Applying ops in HLC order makes the final
//! tree a pure function of the sequence; an op whose application leaves
//! the state unchanged can be deleted from the sequence and every later
//! op still sees the identical state. Induction gives an identical final
//! tree. `Op::Move` is the only variant that can change `nodes[n]` once
//! `n` exists, and `old_parent` / `old_position` are read by nothing but
//! `undo_op`, which never sees an op that is not in the log.
//!
//! *Against ops we do not hold* the argument is causal, and narrower.
//! Only an op naming `n` with HLC below `t2` can invalidate the drop.
//!
//! - Below `t1`: a `Move` there is inert (the node does not exist yet),
//!   so only a second `Create` matters — which for an unguessable
//!   128-bit `NodeId` requires having received ours. Conditions 3 and 4
//!   exclude the id families that *are* guessable.
//! - Inside `(t1, t2)`: a peer must have received our `Create` (emitted
//!   at `t1`) and emitted its own `Move` on `n` stamped inside a gap in
//!   which this device wrote nothing at all (condition 1), and still not
//!   delivered it to us by compaction time (or condition 1 would see
//!   it). Condition 6 additionally requires that op to have been in
//!   flight for longer than the horizon.
//!
//! That residual is stated rather than hidden: see
//! [RFC 0256](../../../../docs/rfcs/0256-op-log-compaction.md).
//!
//! # Measured
//!
//! On the reference workspace (217,811 ops, 20 actors, 2,574 pages) the
//! predicate drops 62,209 of 65,707 `Move` ops — 18.26 MB, 22.6% of the
//! log — and a full replay of the compacted log materializes a
//! byte-identical tree, property table and collapsed set.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{op_node, LogOp, Op};
use crate::tree::Tree;

use super::{ActorSaving, CompactError, CompactReport};

use super::rewrite::{read_actor_files, ActorFile, Line};

/// How much recent history compaction leaves alone, in milliseconds.
///
/// Thirty days. The number is not about the ops themselves — they are
/// inert the moment they are written — it is about the peers that have
/// not delivered theirs yet. An op still in flight after a month means a
/// device that has been offline for a month, which is the only way the
/// residual risk in this module's predicate can be realised.
pub const DEFAULT_HORIZON_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// Knobs for [`plan_compaction`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactOptions {
    /// Leave every op newer than `newest_op - horizon_ms` alone.
    /// `0` disables the horizon (used by tests and by
    /// `outl compact --no-horizon`).
    pub horizon_ms: u64,
}

impl Default for CompactOptions {
    fn default() -> Self {
        Self {
            horizon_ms: DEFAULT_HORIZON_MS,
        }
    }
}

/// A decided-but-unwritten compaction.
///
/// Holds the HLCs to drop per actor file, plus each file's byte length
/// at plan time so [`super::apply_compaction`] can refuse a log that
/// moved underneath it.
#[derive(Clone, Debug)]
pub struct CompactPlan {
    pub(super) drops: BTreeMap<ActorId, BTreeSet<Hlc>>,
    pub(super) sizes: BTreeMap<ActorId, u64>,
    pub(super) report: CompactReport,
}

impl CompactPlan {
    /// What this plan would remove. Never `None`; an empty plan reports
    /// zeroes against the real totals.
    pub fn report(&self) -> &CompactReport {
        &self.report
    }

    /// Whether the plan drops nothing.
    pub fn is_empty(&self) -> bool {
        self.report.ops_dropped == 0
    }

    /// The same plan, narrowed to rewrite only `actor`'s file.
    ///
    /// This is how a device compacts *its own* history without touching a
    /// peer's `ops-<actor>.jsonl` — see [`super::apply_compaction_as`] for
    /// why that distinction is the difference between reclaiming bytes and
    /// deleting another device's unshipped ops.
    ///
    /// The totals (`ops_total`, `bytes_total`) still describe the whole
    /// log, because the decision was made against the whole log. Only the
    /// drops narrow.
    pub fn restricted_to(&self, actor: ActorId) -> CompactPlan {
        let mut drops: BTreeMap<ActorId, BTreeSet<Hlc>> = BTreeMap::new();
        if let Some(hlcs) = self.drops.get(&actor) {
            drops.insert(actor, hlcs.clone());
        }
        let mut report = self.report.clone();
        for saving in &mut report.actors {
            if saving.actor != actor {
                saving.ops_dropped = 0;
                saving.bytes_dropped = 0;
            }
        }
        report.ops_dropped = report.actors.iter().map(|a| a.ops_dropped).sum();
        report.bytes_dropped = report.actors.iter().map(|a| a.bytes_dropped).sum();
        CompactPlan {
            drops,
            sizes: self.sizes.clone(),
            report,
        }
    }

    /// Actors this plan would rewrite that are **not** `own` — i.e. the
    /// files belonging to other devices. Empty is the safe answer.
    pub fn foreign_actors(&self, own: ActorId) -> Vec<ActorId> {
        self.touched().filter(|actor| *actor != own).collect()
    }

    /// Actor files this plan would rewrite.
    pub(super) fn touched(&self) -> impl Iterator<Item = ActorId> + '_ {
        self.drops
            .iter()
            .filter(|(_, hlcs)| !hlcs.is_empty())
            .map(|(actor, _)| *actor)
    }
}

/// Read `<root>/ops` and decide what is droppable. Writes nothing.
///
/// Refuses with [`CompactError::Busy`] while any other `outl` process
/// holds the workspace. Reading is harmless, but reading a file that is
/// being appended to is not: a `write(2)` this reader catches mid-flight
/// shows up as a truncated record, and `read_one` turns any record it
/// cannot parse into [`CompactError::DamagedLog`], whose message tells
/// the user to run `outl doctor` on a log that was never damaged. Nothing
/// is written either way, so the cost is only the false alarm — and a
/// false corruption alarm on the op log is an expensive thing to spend.
pub fn plan_compaction(root: &Path, opts: &CompactOptions) -> Result<CompactPlan, CompactError> {
    let ops_dir = root.join("ops");
    // Held for the whole read, not probed and released: the point is that
    // no append can happen *while* the bytes are being parsed.
    let _idle = super::rewrite::idle_workspace_probe(root)?;
    let files = read_actor_files(&ops_dir)?;
    Ok(decide(&files, opts))
}

/// One op as the decision pass sees it: its position in the merged HLC
/// order, plus enough provenance to route a drop back to a file line.
struct Record<'a> {
    op: &'a LogOp,
    /// Actor whose *file* carries it (not necessarily `op.actor`, for a
    /// log that a sync transport mirrored).
    file_actor: ActorId,
    /// Bytes the physical line occupies.
    bytes: u64,
    /// Whether the line carries exactly this one op. A glued line (two
    /// JSON values with no separating newline, the signature of an
    /// interleaved append) is never droppable: its bytes belong to
    /// several ops at once.
    sole_on_line: bool,
}

fn decide(files: &[ActorFile], opts: &CompactOptions) -> CompactPlan {
    let mut records: Vec<Record<'_>> = Vec::new();
    let mut ops_total = 0usize;
    let mut bytes_total = 0u64;
    // A blank line carries no op and the rewrite drops it, so its bytes
    // are reclaimed too. Counted separately because they belong to no
    // `Hlc` and must never inflate `ops_dropped`.
    let mut blank_bytes: HashMap<ActorId, u64> = HashMap::new();
    for file in files {
        for line in &file.lines {
            if line.ops.is_empty() {
                *blank_bytes.entry(file.actor).or_insert(0) += line.bytes;
            }
            let sole = line.ops.len() == 1;
            for op in &line.ops {
                records.push(Record {
                    op,
                    file_actor: file.actor,
                    bytes: line.bytes,
                    sole_on_line: sole,
                });
            }
            ops_total += line.ops.len();
            bytes_total += line.bytes;
        }
    }
    // The merged total order. `Hlc: Ord` is
    // (physical_ms, logical, actor) — the actor tiebreak is part of the
    // type, so there is no way to compare two HLCs without it here.
    records.sort_by_key(|r| r.op.ts);

    let first_seen = first_appearance(&records);
    let create_count = create_counts(&records);
    let cutoff = horizon_cutoff(&records, opts.horizon_ms);

    let mut drops: BTreeMap<ActorId, BTreeSet<Hlc>> = BTreeMap::new();
    let mut dropped_per_actor: HashMap<ActorId, (usize, u64)> = HashMap::new();
    let mut tree = Tree::new();
    let mut seen: BTreeSet<Hlc> = BTreeSet::new();

    for i in 0..records.len() {
        let rec = &records[i];
        // Two files carrying the same op (a mirrored log) dedup here,
        // exactly as `Tree::apply_op` would.
        if !seen.insert(rec.op.ts) {
            continue;
        }
        if droppable(&records, i, &tree, &first_seen, &create_count, cutoff) {
            drops.entry(rec.file_actor).or_default().insert(rec.op.ts);
            let entry = dropped_per_actor.entry(rec.file_actor).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += rec.bytes;
        }
        // Advance the replay whether or not we dropped: a dropped op is
        // by construction one whose application changes nothing, so the
        // state the next op sees is the same either way.
        let mut applied = rec.op.clone();
        tree.do_op(&mut applied);
    }

    let mut actors: Vec<ActorSaving> = files
        .iter()
        .map(|file| {
            let (ops_dropped, bytes_dropped) = dropped_per_actor
                .get(&file.actor)
                .copied()
                .unwrap_or((0, 0));
            // Blank lines go only where a rewrite actually happens: a
            // file with nothing droppable is never opened, so its blank
            // lines survive and claiming them would over-declare.
            let blanks = if ops_dropped > 0 {
                blank_bytes.get(&file.actor).copied().unwrap_or(0)
            } else {
                0
            };
            ActorSaving {
                actor: file.actor,
                ops_total: file.lines.iter().map(|l| l.ops.len()).sum(),
                ops_dropped,
                bytes_total: file.lines.iter().map(|l| l.bytes).sum(),
                bytes_dropped: bytes_dropped + blanks,
            }
        })
        .collect();
    actors.sort_by_key(|a| a.actor.0);

    let report = CompactReport {
        ops_total,
        ops_dropped: actors.iter().map(|a| a.ops_dropped).sum(),
        bytes_total,
        bytes_dropped: actors.iter().map(|a| a.bytes_dropped).sum(),
        actors,
        backup_dir: None,
    };
    CompactPlan {
        drops,
        sizes: files.iter().map(|f| (f.actor, f.bytes)).collect(),
        report,
    }
}

/// The six conditions, in the order that rejects fastest.
fn droppable(
    records: &[Record<'_>],
    i: usize,
    tree: &Tree,
    first_seen: &HashMap<NodeId, Hlc>,
    create_count: &HashMap<NodeId, usize>,
    cutoff: Option<u64>,
) -> bool {
    let rec = &records[i];
    let Op::Move {
        node,
        new_parent,
        position,
        ..
    } = &rec.op.op
    else {
        return false;
    };
    if !rec.sole_on_line {
        return false;
    }
    // (4) page and journal roots are `NodeId::from_slug` — derivable by
    // a device that has never seen one of our ops.
    if *new_parent == NodeId::root() {
        return false;
    }
    // (6) settling horizon.
    if cutoff.is_some_and(|c| rec.op.ts.physical_ms > c) {
        return false;
    }
    // (3) sole Create anywhere in the merged log.
    if create_count.get(node).copied().unwrap_or(0) != 1 {
        return false;
    }
    // (1) the immediately preceding record in the merged HLC order is
    // this node's Create, by the same actor, to the same place.
    let Some(prev) = i.checked_sub(1).map(|j| &records[j]) else {
        return false;
    };
    let Op::Create {
        node: cnode,
        parent,
        position: cpos,
    } = &prev.op.op
    else {
        return false;
    };
    if cnode != node || parent != new_parent || cpos != position {
        return false;
    }
    if prev.op.actor != rec.op.actor || prev.file_actor != rec.file_actor {
        return false;
    }
    // (2) that Create is the node's first appearance in the whole log.
    if first_seen.get(node) != Some(&prev.op.ts) {
        return false;
    }
    // (5) and the replay agrees the Move changes nothing.
    tree.parent(*node) == Some(*new_parent) && tree.position(*node) == Some(position)
}

fn first_appearance(records: &[Record<'_>]) -> HashMap<NodeId, Hlc> {
    let mut out: HashMap<NodeId, Hlc> = HashMap::new();
    for rec in records {
        if let Some(node) = op_node(&rec.op.op) {
            out.entry(node)
                .and_modify(|ts| {
                    if rec.op.ts < *ts {
                        *ts = rec.op.ts;
                    }
                })
                .or_insert(rec.op.ts);
        }
    }
    out
}

fn create_counts(records: &[Record<'_>]) -> HashMap<NodeId, usize> {
    let mut out: HashMap<NodeId, usize> = HashMap::new();
    for rec in records {
        if let Op::Create { node, .. } = &rec.op.op {
            *out.entry(*node).or_insert(0) += 1;
        }
    }
    out
}

/// The cutoff for condition 6, or `None` when the horizon is disabled or
/// the log is empty.
///
/// It is the horizon subtracted from the newest op **or now, whichever is
/// earlier** — and that clamp is the whole point of this function.
///
/// `physical_ms` is not a number this device chose. `hlc.rs` is
/// hand-rolled with no drift bound (there is no `uhlc` dependency), and
/// `HlcGenerator::observe` adopts a peer's higher `physical_ms`
/// unconditionally. So one device with a wrong clock puts an op years in
/// the future into the merged log, and a cutoff anchored on "the newest
/// op" then sits years in the past: every real op falls below it,
/// condition 6 stops rejecting anything, and a run the user deliberately
/// did **not** give `--no-horizon` behaves exactly as if they had.
/// Losing a safety margin is bad; losing it silently, on the one
/// condition that protects against undelivered peer ops, is worse.
///
/// The clamp is strictly more conservative: when the log's newest op is
/// in the past — every healthy workspace — `min` returns it unchanged.
fn horizon_cutoff(records: &[Record<'_>], horizon_ms: u64) -> Option<u64> {
    if horizon_ms == 0 {
        return None;
    }
    let newest = records.last()?.op.ts.physical_ms;
    Some(newest.min(wall_clock_ms()).saturating_sub(horizon_ms))
}

/// Milliseconds since the Unix epoch, floored at zero.
///
/// A machine whose own clock is set before 1970 gets a cutoff of 0, which
/// disables *dropping* rather than disabling the horizon — the refusing
/// direction, as everywhere else in this module.
fn wall_clock_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

/// Path of one actor's op log under `ops_dir` (the `Global` layout).
pub(super) fn ops_path(ops_dir: &Path, actor: ActorId) -> PathBuf {
    ops_dir.join(format!("ops-{actor}.jsonl"))
}

/// Lines are re-read at apply time; this keeps the two passes agreeing
/// on what "a line" is.
pub(super) fn line_is_droppable(line: &Line, drops: &BTreeSet<Hlc>) -> bool {
    line.ops.len() == 1 && drops.contains(&line.ops[0].ts)
}
