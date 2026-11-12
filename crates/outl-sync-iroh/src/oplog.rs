//! Writing the op log on behalf of the sync wire.
//!
//! `engine_sync` owns what crosses the wire; this module owns what the wire
//! writes to disk. The split matters because the durability rules here are
//! not protocol rules: they hold no matter which version of the wire calls
//! them, and they are the reason a sync can be interrupted at any point
//! without corrupting anything.
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
//!
//! The read half — what this device holds, and what a peer therefore lacks —
//! lives in [`oplog_census`](crate::oplog_census). The ingest's dedup set is
//! the one thing that crosses the line, and it asks that module rather than
//! reading op bodies of its own.

use std::collections::{HashMap, HashSet};
use std::io::Write as _;

use anyhow::{Context, Result};
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::LogOp;
use tracing::warn;

use crate::engine::AppendLock;
use crate::oplog_census::actor_census;

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

/// Does `file` end in an un-terminated (torn) line, so the next append must
/// write a separating newline first?
///
/// `false` for an empty / absent file (nothing to tear) and for one already
/// ending in `\n`. Leaves the write position alone: the caller's handle is
/// `O_APPEND`, so the seek here only moves the READ cursor.
fn needs_tail_separator(file: &mut std::fs::File) -> std::io::Result<bool> {
    use std::io::{Read as _, Seek as _, SeekFrom};

    if file.seek(SeekFrom::End(0))? == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

/// Blocking half of the ingest: dedup against the on-disk log and append.
///
/// Runs under BOTH locks — the caller holds the in-process [`AppendLock`],
/// and this function takes the cross-process [`OpsDirAppendLock`] — so the
/// read-present/append pair is atomic against every other transport writer
/// on this device. Loading the present HLC set inside the flock is what
/// makes the dedup race-free: nothing can append between the read and the
/// write.
///
/// The present set is **the census**, not `all_ops`. "Which ops do I already
/// hold" is a question about identity, and an op's identity is its HLC —
/// globally unique, since the producing actor is a field of it. Reading the
/// bodies to answer it meant every ingest batch loaded the whole workspace
/// while holding the device-wide append flock, so every other sync write on
/// the machine queued behind it.
///
/// Keying on the HLC alone is also the stricter rule. The old `(actor, ts)`
/// pair let a forged op re-use a known HLC under a different actor label and
/// land as a second line claiming one op's identity; two distinct honest ops
/// can never collide, because the generator varies `(physical, logical)`
/// within an actor and the actor itself across them.
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

    let mut present: HashSet<Hlc> = actor_census(ops_dir)
        .context("load present ops for dedup")?
        .into_values()
        .flatten()
        .collect();

    let mut per_actor: HashMap<ActorId, Vec<u8>> = HashMap::new();
    let mut applied = 0usize;
    let mut foreign_claims = 0usize;
    // HLCs claimed under our actor that the census does not list. Normally a
    // spoof; also what a holed own sidecar looks like (see `heal_own_index`).
    let mut unlisted_own: Vec<Hlc> = Vec::new();
    for op in candidates {
        // A peer may not write OUR actor's file. This mirrors
        // `outl_core::storage::jsonl::append`'s `append_ops_inner`, which
        // rejects a whole batch carrying a foreign-actor op; that guard is
        // `pub(super)` and reachable only through `Storage::append_ops`, which
        // writes solely to `own_ops_path()` — so ingest, which legitimately
        // writes OTHER actors' files, cannot call it and has to state the
        // mirrored rule itself. Same invariant, opposite side of the fence:
        // there, "only my own actor"; here, "anything but my own actor".
        //
        // Two things break without it, and only one is an attack:
        //
        // - **Corruption.** `ops-<local>.jsonl` has a single writer by design,
        //   serialized by `outl-core`'s `ops/.lock-<actor>` — a DIFFERENT lock
        //   from the `ops/.append.lock` this function holds. An ingest writing
        //   that file races the local appender with nothing in common to
        //   serialize them.
        // - **Silent loss.** The dedup below is by HLC and content-blind, so
        //   an op forged under our actor id pre-claims an HLC slot; our real
        //   op is then dropped as a duplicate when it is written. The forged
        //   content wins and the genuine edit disappears with no error anywhere.
        //
        // A device that genuinely lost its own `ops-<actor>.jsonl` does not
        // recover through this path (`outl recover` / the snapshot pull is the
        // route); silently re-accepting our own actor's history from a peer is
        // how the loss above gets a legitimate-looking cover story.
        if op.actor == local_actor {
            foreign_claims += 1;
            if !present.contains(&op.ts) {
                unlisted_own.push(op.ts);
            }
            continue;
        }
        // Already on disk, or a duplicate earlier in this same batch.
        if !present.insert(op.ts) {
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

    if !unlisted_own.is_empty() {
        heal_own_index(ops_dir, local_actor, &unlisted_own);
    }

    if foreign_claims > 0 {
        // Loud, not silent: either a peer is misbehaving or a sync path has a
        // bug, and both are things an operator needs to see. The refusal is
        // per-op (the rest of the batch still lands) because a peer that
        // mislabels one op must not be able to stall convergence for everyone.
        warn!(
            refused = foreign_claims,
            actor = %local_actor,
            "refused {foreign_claims} received op(s) claiming THIS device's actor id — a peer may not write our own op log"
        );
    }

    for (actor_id, lines) in per_actor {
        let path = ops_dir.join(format!("ops-{actor_id}.jsonl"));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open ops file {}", path.display()))?;
        // Torn-tail self-heal, mirroring `outl_core`'s appender. A previous
        // append cut off mid-line (crash, power loss, iOS jetsam) leaves a
        // fragment with no terminator; appending straight after glues our
        // first op onto it (`{"ts":…partial{"ts":…ours}`) and the reader's
        // glued-line recovery cannot split a TORN prefix from a good op, so
        // BOTH are lost. One newline turns that into one unparseable line
        // (the fragment, already lost) plus our op, parseable.
        //
        // `O_APPEND` sends every write to EOF regardless of the read cursor,
        // so seeking back to probe the last byte cannot disturb where the ops
        // land. Both append locks are held, so nothing writes between the
        // probe and the write.
        if needs_tail_separator(&mut file)
            .with_context(|| format!("probe tail of {}", path.display()))?
        {
            file.write_all(b"\n")
                .with_context(|| format!("heal torn tail of {}", path.display()))?;
        }
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

/// Minimum time between two verification passes over the same `ops/` dir.
const OWN_INDEX_CHECK_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(300);

/// Heal a holed offset sidecar for OUR OWN actor, when a peer's resend proves
/// one.
///
/// The census is read from the offset sidecar, and a sidecar can be missing
/// entries mid-file while still passing the freshness ladder (which only
/// checks the max-offset record). Our advertised count is then short, the
/// peer holds more than we say, concludes we have a gap and resends our whole
/// actor log every sync — which the guard above refuses every time, so the
/// clock never moves. For a foreign actor a short count is the safe
/// direction; for our own it is a loop.
///
/// `unlisted` are the HLCs peers claimed under our actor that the census did
/// not list. Stream our own `.jsonl` (not the sidecar): if it holds any of
/// them the sidecar is holed, and the rebuild persists a complete pair so the
/// next advertised clock is right. Nothing is ever appended; an HLC absent
/// from the log itself stays refused, so spoof protection is unchanged.
///
/// The stream costs a pass over the whole log, so it runs at most once per
/// [`OWN_INDEX_CHECK_COOLDOWN`] per directory. A successful rebuild ends the
/// condition; only a peer that keeps forging our actor can re-trigger it, and
/// the cooldown bounds what that costs.
fn heal_own_index(ops_dir: &std::path::Path, local_actor: ActorId, unlisted: &[Hlc]) {
    use outl_core::storage::JsonlStorage;
    use std::sync::{Mutex, OnceLock};
    use std::time::Instant;

    static LAST_CHECK: OnceLock<Mutex<HashMap<std::path::PathBuf, Instant>>> = OnceLock::new();
    {
        let mut last = LAST_CHECK
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        match last.get(ops_dir) {
            Some(at) if now.duration_since(*at) < OWN_INDEX_CHECK_COOLDOWN => return,
            _ => {
                last.insert(ops_dir.to_path_buf(), now);
            }
        }
    }

    let rebuilt = match JsonlStorage::rebuild_own_indexes(ops_dir, local_actor) {
        Ok(index) => index,
        Err(e) => {
            warn!(
                actor = %local_actor,
                "could not verify our own op log against a peer's resend: {e}"
            );
            return;
        }
    };
    let in_log = unlisted.iter().filter(|ts| rebuilt.get(ts).is_some()).count();
    if in_log > 0 {
        warn!(
            actor = %local_actor,
            missing = in_log,
            "our own offset index was missing {in_log} op(s) the log holds; rebuilt it so the next advertised clock is complete"
        );
    }
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
    // A local clock before UNIX_EPOCH cannot anchor "the future". Treating it
    // as 0 (the old `unwrap_or_default()`) made the gate below classify every
    // incoming op as >24h ahead and drop it — a sync black hole instead of a
    // safety gate. Skip the gate for the batch instead: applying an op the
    // gate cannot judge is recoverable (HLC ordering absorbs it), silently
    // refusing all sync is not.
    let now_ms = outl_core::hlc::wall_clock_ms_checked();
    if now_ms.is_none() {
        warn!("local clock is before UNIX_EPOCH; skipping the future-HLC gate");
    }

    // HLC sanity gate (pure, no I/O): skip ops too far in the future.
    // The verdict has one owner — `outl_core::hlc::skew_ahead_ms` — because
    // `Workspace::seed_clock` clamps the boot seed to the same window, and
    // `outl-plugins`' `PluginHost::sync_pull` is a second ingestor of foreign
    // ops. Three copies of "how far ahead is too far" means they can disagree,
    // and the seed is the side that persists.
    let mut candidates: Vec<LogOp> = Vec::with_capacity(received.len());
    for op in received {
        if let Some(ahead) = outl_core::hlc::skew_ahead_ms(op.ts, now_ms) {
            // Log the op's HLC + actor (its identity) so a dropped op is
            // traceable, not just "something 25h ahead vanished".
            warn!(
                ts = ?op.ts,
                actor = ?op.actor,
                "skipping op with future HLC ({ahead}ms ahead)"
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
