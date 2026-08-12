//! Reading and writing the op log on behalf of the sync wire.
//!
//! `engine_sync` owns what crosses the wire; this module owns what the wire
//! reads from and writes to disk. The split matters because the durability
//! rules here are not protocol rules: they hold no matter which version of the
//! wire calls them, and they are the reason a sync can be interrupted at any
//! point without corrupting anything.
//!
//! Two invariants live here, both paid for in production incidents:
//!
//! - **Every append is serialized**, in-process by [`AppendLock`] and
//!   across processes by an advisory flock on `ops/.append.lock`. Two
//!   concurrent `write_all`s interleave at the syscall layer and glue two ops
//!   onto one line (`…}}}{"ts":…`); that was found on a real iCloud workspace,
//!   45 glued lines among 5261 ops.
//! - **A batch is flushed and `sync_data`'d before anyone is told it landed.**
//!   The confirmation the responder sends is only worth the fsync that
//!   precedes it.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write as _;

use anyhow::{Context, Result};
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::LogOp;
use tracing::{info, warn};

use crate::engine::AppendLock;
use crate::protocol::ActorClock;

/// Per-actor census of the local op log: the set of DISTINCT HLCs held for
/// each actor. Distinct because historic unsynchronized concurrent pulls left
/// duplicated append lines on real disks — a raw line count would inflate the
/// `count` and mask a genuine gap on the advertising side.
pub(crate) fn actor_census(
    ops_dir: &std::path::Path,
    actor: ActorId,
) -> Result<HashMap<ActorId, BTreeSet<Hlc>>> {
    let storage = JsonlStorage::open(ops_dir.to_path_buf(), actor)
        .context("open JsonlStorage for actor census")?;
    let mut census: HashMap<ActorId, BTreeSet<Hlc>> = HashMap::new();
    for op in storage.all_ops().context("load all ops for census")? {
        census.entry(op.actor).or_default().insert(op.ts);
    }
    Ok(census)
}

/// Compute our vector clock (per-actor max HLC + distinct-op count) from the
/// local op log. Derived from `all_ops` — the `Storage` trait stays untouched;
/// max + count both fall out of the same census pass.
pub(crate) fn local_vector_clock(
    ops_dir: &std::path::Path,
    actor: ActorId,
) -> Result<HashMap<ActorId, ActorClock>> {
    Ok(actor_census(ops_dir, actor)?
        .into_iter()
        .filter_map(|(actor_id, hlcs)| {
            let max = *hlcs.last()?;
            Some((
                actor_id,
                ActorClock {
                    max,
                    count: hlcs.len() as u64,
                },
            ))
        })
        .collect())
}

/// Collect the ops the peer is missing, given the peer's vector clock.
///
/// Per actor in OUR log:
/// - peer has no entry → send everything we hold for that actor;
/// - peer has `(max_r, count_r)` → let `below` = how many DISTINCT ops of
///   that actor we hold with `ts <= max_r`. If `below > count_r` the peer
///   has a GAP below its own watermark (an op landed ahead of a pending
///   backlog, so the bare max lies about what's underneath) → send the
///   actor's FULL log; the receiver's ingest dedup absorbs the overlap.
///   Otherwise → fast path, send only `ts > max_r`.
///
/// Theoretical limitation (accepted): two replicas holding EQUAL counts of
/// DIFFERENT subsets below the same max are indistinguishable to this check,
/// so the fast path under-sends between them. Convergence is still
/// guaranteed via the origin: each op's authoring device holds its own
/// actor's complete log, so any replica with a partial subset trips the
/// `below > count` check the next time it syncs (directly or transitively)
/// with the origin's full prefix.
pub(crate) fn ops_missing_for(
    ops_dir: &std::path::Path,
    actor: ActorId,
    peer_clock: &HashMap<ActorId, ActorClock>,
) -> Result<Vec<LogOp>> {
    let census = actor_census(ops_dir, actor)?;

    // Actors whose FULL log must cross (unknown to the peer, or gap detected
    // below the peer's watermark).
    let full_resend: HashSet<ActorId> = census
        .iter()
        .filter_map(|(actor_id, hlcs)| match peer_clock.get(actor_id) {
            None => Some(*actor_id),
            Some(peer) => {
                let below = hlcs.range(..=peer.max).count() as u64;
                (below > peer.count).then(|| {
                    info!(
                        actor = %actor_id,
                        below,
                        peer_count = peer.count,
                        "gap below peer watermark detected; resending full actor log"
                    );
                    *actor_id
                })
            }
        })
        .collect();

    let storage =
        JsonlStorage::open(ops_dir.to_path_buf(), actor).context("open storage for delta")?;
    let all_ops = storage.all_ops().context("load all ops")?;
    Ok(all_ops
        .into_iter()
        .filter(|op| {
            if full_resend.contains(&op.actor) {
                return true;
            }
            match peer_clock.get(&op.actor) {
                None => true,
                Some(peer) => op.ts > peer.max,
            }
        })
        .collect())
}

/// Cross-process append lock on `<ops_dir>/.append.lock` (advisory flock).
///
/// The in-process [`AppendLock`] serializes appends within ONE transport, but
/// a device legitimately runs several processes with their own transports at
/// once (GUI + MCP server + `outl sync`), each holding its own `AppendLock`.
/// Two of them ingesting concurrently interleave whole batches on the same
/// `ops-<actor>.jsonl` — observed in production as timestamp retrocessions in
/// a peer's file, the out-of-order delivery that broke the max-HLC watermark.
/// This flock is the cross-process half: acquired AFTER the in-process lock,
/// held across open+write+flush+sync of every per-actor file in the batch.
///
/// Placement: a dotfile inside `ops/`, following the existing
/// `ops/.lock-<actor>` precedent (`outl_core::lock::ActorWriteLock`). Some
/// file transports (iCloud) drop dotted paths — harmless here: flock state is
/// kernel-local (it never syncs anyway) and the file is empty and recreated
/// on demand, so losing it costs nothing.
///
/// Acquisition blocks (`fs2::lock_exclusive`, a blocking `flock(2)`), so it
/// must run on a blocking thread — see `write_deduped_batch`'s `spawn_blocking`
/// caller. Drop releases via the closed fd.
struct OpsDirAppendLock {
    _file: std::fs::File,
}

impl OpsDirAppendLock {
    fn acquire(ops_dir: &std::path::Path) -> Result<Self> {
        let path = ops_dir.join(".append.lock");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open append lock {}", path.display()))?;
        // `fs2::lock_exclusive` (blocking `flock(2)` via libc), NOT
        // `std::fs::File::lock()`, which is hardcoded to return `Unsupported`
        // on Android and would break inbound sync there. Same as `peers_lock`.
        fs2::FileExt::lock_exclusive(&file)
            .with_context(|| format!("flock append lock {}", path.display()))?;
        Ok(Self { _file: file })
    }
}

/// Blocking half of the ingest: dedup against the on-disk log and append.
///
/// Runs under BOTH locks — the caller holds the in-process [`AppendLock`],
/// and this function takes the cross-process [`OpsDirAppendLock`] — so the
/// read-present/append pair is atomic against every other transport writer
/// on this device. Loading the present `(actor, ts)` set inside the flock is
/// what makes the dedup race-free: nothing can append between the read and
/// the write.
///
/// Dedup drops ops already on disk AND duplicates within the batch itself,
/// so a full-actor resend (gap fallback) or two concurrent pulls of the same
/// backlog never append the same op twice. Historic duplicated lines from
/// pre-lock pulls stay on disk (reload dedups by op id on apply) but no new
/// ones are minted. Returns how many ops were actually appended.
fn write_deduped_batch(
    ops_dir: &std::path::Path,
    local_actor: ActorId,
    candidates: &[LogOp],
) -> Result<usize> {
    std::fs::create_dir_all(ops_dir)
        .with_context(|| format!("create ops dir {}", ops_dir.display()))?;
    let _flock = OpsDirAppendLock::acquire(ops_dir)?;

    let storage = JsonlStorage::open(ops_dir.to_path_buf(), local_actor)
        .context("open storage for ingest dedup")?;
    let mut present: HashSet<(ActorId, Hlc)> = storage
        .all_ops()
        .context("load present ops for dedup")?
        .iter()
        .map(|op| (op.actor, op.ts))
        .collect();
    drop(storage);

    let mut per_actor: HashMap<ActorId, Vec<u8>> = HashMap::new();
    let mut applied = 0usize;
    for op in candidates {
        // Already on disk, or a duplicate earlier in this same batch.
        if !present.insert((op.actor, op.ts)) {
            continue;
        }
        let line = match crate::protocol::encode_op(op) {
            Ok(l) => l,
            Err(e) => {
                warn!("re-encode received op: {e}");
                continue;
            }
        };
        let entry = per_actor.entry(op.actor).or_default();
        entry.extend_from_slice(&line);
        entry.push(b'\n');
        applied += 1;
    }

    for (actor_id, lines) in per_actor {
        let path = ops_dir.join(format!("ops-{actor_id}.jsonl"));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open ops file {}", path.display()))?;
        file.write_all(&lines)
            .with_context(|| format!("append ops to {}", path.display()))?;
        file.flush()
            .with_context(|| format!("flush ops to {}", path.display()))?;
        // Durably land the batch before releasing the locks so a concurrent
        // reader (or the next writer) never observes a partial line.
        file.sync_data()
            .with_context(|| format!("fsync ops to {}", path.display()))?;
    }

    Ok(applied)
}

/// Apply a batch of received ops: drop ones with a future HLC, dedup against
/// the on-disk log (and within the batch), bucket per actor, append to
/// `ops/ops-<actor>.jsonl`, and fire `peer_ready_tx` if any op landed. Shared
/// by both sync directions so the gate + dedup + append logic lives in
/// exactly one place. Returns the number of ops actually applied (gate-passed
/// AND not already present), so the caller's log reflects real progress.
///
/// **All file writes happen under `append_lock` + the cross-process
/// [`OpsDirAppendLock`]**. The in-process lock is the load-bearing fix for
/// op-log corruption within one transport (two concurrent
/// `delta_sync`/`serve` runs gluing ops — `…}}}{"ts":…`); the flock extends
/// the same guarantee across processes (GUI + MCP + CLI transports on one
/// device). The blocking flock + file I/O run on `spawn_blocking` so a wait
/// on another process never stalls the tokio workers.
/// Cap on the number of distinct touched block ids [`ingest_received_ops`]
/// reports back for the progress feed. A live incremental sync touches a
/// handful of blocks (name the pages); a bulk pair touches tens of thousands
/// (naming them is meaningless), so we stop collecting past this and let the UI
/// show the count instead of an endless page list.
const NODE_REPORT_CAP: usize = 64;

pub(crate) async fn ingest_received_ops(
    ops_dir: &std::path::Path,
    local_actor: ActorId,
    received: &[LogOp],
    peer_ready_tx: &std::sync::mpsc::Sender<()>,
    append_lock: &AppendLock,
) -> Result<(usize, Vec<outl_core::id::NodeId>)> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // HLC sanity gate (pure, no I/O): skip ops more than 24h in the future.
    let mut candidates: Vec<LogOp> = Vec::with_capacity(received.len());
    for op in received {
        let op_ms = op.ts.physical_ms;
        if op_ms > now_ms + 86_400_000 {
            // Log the op's HLC + actor (its identity) so a dropped op is
            // traceable, not just "something 25h ahead vanished".
            warn!(
                ts = ?op.ts,
                actor = ?op.actor,
                "skipping op with future HLC ({}ms ahead)",
                op_ms - now_ms
            );
            continue;
        }
        candidates.push(op.clone());
    }

    if candidates.is_empty() {
        return Ok((0, Vec::new()));
    }

    // Collect the distinct block ids this batch touches, capped, BEFORE moving
    // `candidates` into the blocking write. This is best-effort for the UI feed
    // ("page X synced") — it names the candidate ops, not just the ones that
    // survived dedup, which for a live incremental sync is the same handful.
    let mut seen = std::collections::HashSet::new();
    let mut nodes = Vec::new();
    for op in &candidates {
        if nodes.len() >= NODE_REPORT_CAP {
            break;
        }
        if let Some(node) = outl_core::op::op_node(&op.op) {
            if seen.insert(node) {
                nodes.push(node);
            }
        }
    }

    // Serialize the whole batch against every other transport append. The
    // in-process guard is held across the blocking dedup+write; the
    // cross-process flock is taken inside, on the blocking thread.
    let _guard = append_lock.lock().await;
    let ops_dir_buf = ops_dir.to_path_buf();
    let applied = tokio::task::spawn_blocking(move || {
        write_deduped_batch(&ops_dir_buf, local_actor, &candidates)
    })
    .await
    .context("join ingest append task")??;

    if applied > 0 {
        peer_ready_tx.send(()).ok();
    }

    Ok((applied, nodes))
}
