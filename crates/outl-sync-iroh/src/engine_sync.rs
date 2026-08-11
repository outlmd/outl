//! Delta-sync wire protocol — the on-the-wire half of the iroh transport.
//!
//! Extracted from `engine.rs` so that module stays focused on boot
//! orchestration (`run_iroh`, the `IrohSyncTransport` struct, channel wiring).
//! This module owns the four-message vector-clock exchange both sync directions
//! run over a single bi stream:
//!
//! - [`delta_sync`] — the **initiator** side (boot connect, catch-up, gossip,
//!   pairing, and the `sync_now` force-trigger all dial through it).
//! - [`SyncProtocolHandler`] — the **responder** side, mounted on the router.
//! - The framing helpers (`read_frame` + the typed `read_*` wrappers) and the
//!   op-log read/write helpers (`local_vector_clock`, `ops_missing_for`,
//!   `ingest_received_ops`).
//!
//! The [`AppendLock`](crate::engine::AppendLock) invariant lives here:
//! `ingest_received_ops` is the single op-log writer and holds the process-wide
//! lock across open+write+flush+sync. See `outl-sync-iroh/CLAUDE.md`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use outl_actions::SyncProgress;
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::{LogOp, WorkspaceId};
use tracing::{debug, info, warn};

use crate::engine::{AppendLock, SharedWorkspaceId};
use crate::protocol::{
    classify_close, close_refusal_reason, decode_ops_blob, decode_request, decode_response,
    encode_ops_blob, encode_request, encode_response, ActorClock, CloseVerdict, SyncRequest,
    SyncResponse, CLOSE_DONE, CLOSE_UNKNOWN_PEER, CLOSE_WORKSPACE_MISMATCH, SYNC_ALPN,
};

/// Per-actor census of the local op log: the set of DISTINCT HLCs held for
/// each actor. Distinct because historic unsynchronized concurrent pulls left
/// duplicated append lines on real disks — a raw line count would inflate the
/// `count` and mask a genuine gap on the advertising side.
fn actor_census(
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
fn local_vector_clock(
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
fn ops_missing_for(
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

async fn ingest_received_ops(
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

/// Hard ceiling on a single sync frame's body, enforced before allocating.
///
/// The 4-byte length prefix is attacker-controlled — a *paired* peer could be
/// compromised or buggy (QUIC authenticates the stream, so this isn't an
/// on-path attacker, but a bad peer is enough). Trusting it to size a `Vec` up
/// front let a 4-byte `0xFFFFFFFF` force a ~4 GiB allocation — an instant OOM /
/// iOS jetsam kill (issue #155). A full-actor-log resend (the gap-recovery
/// fallback in `ops_missing_for`) is the largest legitimate frame, so the cap
/// is generous; the incremental read in [`read_frame`] means we still only
/// allocate as bytes actually arrive.
const MAX_FRAME_BODY: usize = 256 * 1024 * 1024;

/// Validate a frame's declared body length against [`MAX_FRAME_BODY`] before we
/// act on it. Split out from [`read_frame`] so the ceiling is unit-testable
/// without standing up a QUIC stream.
fn checked_frame_body_len(prefix: [u8; 4]) -> Result<usize> {
    let body_len = u32::from_be_bytes(prefix) as usize;
    if body_len > MAX_FRAME_BODY {
        anyhow::bail!("sync frame body too large: {body_len} bytes (max {MAX_FRAME_BODY})");
    }
    Ok(body_len)
}

/// Read one length-prefixed frame from a recv stream.
///
/// Reads the 4-byte big-endian length prefix, then exactly that many body
/// bytes, and returns the full `[prefix || body]` buffer so the existing
/// `decode_*` helpers (which expect the prefix) consume it directly. Letting
/// several independent frames share a single bi stream without EOF ambiguity
/// is the whole point — `read_to_end` would swallow the next frame too.
pub(crate) async fn read_frame(recv: &mut iroh::endpoint::RecvStream) -> Result<Vec<u8>> {
    let mut prefix = [0u8; 4];
    recv.read_exact(&mut prefix)
        .await
        .context("read frame length prefix")?;
    let body_len = checked_frame_body_len(prefix)?;
    // Grow the buffer as bytes actually arrive rather than pre-sizing it to the
    // (untrusted) declared length: a peer that lies about the length — or dies
    // right after sending the prefix — then costs us only what it actually
    // transmits, not a speculative multi-hundred-MiB allocation. Read straight
    // into the freshly-extended tail so there is no extra per-chunk copy.
    let mut frame = Vec::with_capacity(4 + body_len.min(64 * 1024));
    frame.extend_from_slice(&prefix);
    let mut remaining = body_len;
    while remaining > 0 {
        let want = remaining.min(64 * 1024);
        let start = frame.len();
        frame.resize(start + want, 0);
        recv.read_exact(&mut frame[start..])
            .await
            .context("read frame body")?;
        remaining -= want;
    }
    Ok(frame)
}

/// Like [`read_frame`] but calls `on_progress(received, total)` as body bytes
/// arrive, for the snapshot pull's percentage feed. `total` is the declared
/// body length — known from the 4-byte prefix before any body byte — so the UI
/// gets a real bar, not a spinner. Kept separate from [`read_frame`] so the hot
/// path stays callback-free; the small body-read loop is duplicated on purpose.
pub(crate) async fn read_frame_reporting(
    recv: &mut iroh::endpoint::RecvStream,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<Vec<u8>> {
    let mut prefix = [0u8; 4];
    recv.read_exact(&mut prefix)
        .await
        .context("read frame length prefix")?;
    let body_len = checked_frame_body_len(prefix)?;
    on_progress(0, body_len as u64);
    let mut frame = Vec::with_capacity(4 + body_len.min(64 * 1024));
    frame.extend_from_slice(&prefix);
    let mut remaining = body_len;
    while remaining > 0 {
        let want = remaining.min(64 * 1024);
        let start = frame.len();
        frame.resize(start + want, 0);
        recv.read_exact(&mut frame[start..])
            .await
            .context("read frame body")?;
        remaining -= want;
        on_progress((body_len - remaining) as u64, body_len as u64);
    }
    Ok(frame)
}

/// Read a length-prefixed `SyncRequest` (vector clock) from a recv stream.
///
/// The initiator does not `finish()` after the request (it streams its push
/// later on the same bi stream), so the responder reads an explicit frame
/// rather than `read_to_end`.
async fn read_request(recv: &mut iroh::endpoint::RecvStream) -> Result<SyncRequest> {
    decode_request(&read_frame(recv).await?)
}

/// Read a length-prefixed `SyncResponse` (vector clock) from a recv stream.
async fn read_response(recv: &mut iroh::endpoint::RecvStream) -> Result<SyncResponse> {
    decode_response(&read_frame(recv).await?)
}

/// Read a length-prefixed ops blob from a recv stream and decode it.
async fn read_ops_blob(recv: &mut iroh::endpoint::RecvStream) -> Result<Vec<LogOp>> {
    decode_ops_blob(&read_frame(recv).await?)
}

/// How long to wait for a single connect attempt before giving up.
///
/// iroh 1.0.0's QUIC multipath opens paths to every candidate address at
/// once and stalls ~30s on a dead one (`MultipathNotNegotiated`) before
/// the relay path can carry the connection. Bounding each attempt caps
/// that stall so a stale direct address can't wedge every catch-up tick,
/// and lets the bare-id (relay/discovery) fallback take over.
///
/// The **direct** attempt is short: a live on-LAN peer connects sub-second, so
/// a longer wait only ever pays off for a DEAD direct addr — where waiting is
/// exactly wrong (it wedges the peer's `InFlightGuard`, so the pull-to-refresh /
/// boot-kick `sync_now` calls all skip as "already in flight" while a stale LAN
/// IP times out). 5s fails the dead path fast and hands off to the relay — the
/// path that actually carries the iOS peer's inbound. The **relay** fallback
/// keeps the full 10s (discovery + relay handshake legitimately takes longer).
const DIRECT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RELAY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Connect to `peer` for a sync, resilient to a stale direct address.
///
/// Tries the full `EndpointAddr` (relay + on-LAN direct) first — fast on
/// the same LAN — bounded by [`CONNECT_TIMEOUT`]. If that stalls or fails
/// **and** the address carried direct addrs (which may be a moved peer's
/// dead LAN IP that still sits in `peers.json`), it retries by **bare
/// node id**, so iroh's relay + discovery learns the peer's CURRENT
/// address instead of wedging on the dead path. Same route the
/// gossip-triggered dial uses; here it's the self-heal for a moved peer.
async fn connect_with_fallback(
    endpoint: &iroh::Endpoint,
    peer_addr: iroh::EndpointAddr,
) -> Result<Connection> {
    let node_id = peer_addr.id;
    let had_direct = peer_addr.ip_addrs().next().is_some();

    match tokio::time::timeout(
        DIRECT_CONNECT_TIMEOUT,
        endpoint.connect(peer_addr, SYNC_ALPN),
    )
    .await
    {
        Ok(Ok(conn)) => return Ok(conn),
        Ok(Err(e)) if !had_direct => return Err(e).context("connect for delta sync"),
        Err(_) if !had_direct => return Err(anyhow::anyhow!("connect for delta sync timed out")),
        Ok(Err(e)) => debug!(
            "direct connect to {} failed ({e}); retrying via relay/discovery",
            node_id.fmt_short()
        ),
        Err(_) => debug!(
            "direct connect to {} timed out; retrying via relay/discovery",
            node_id.fmt_short()
        ),
    }

    // Fallback: bare node id → relay + discovery resolves the current addr.
    tokio::time::timeout(RELAY_CONNECT_TIMEOUT, endpoint.connect(node_id, SYNC_ALPN))
        .await
        .context("relay/discovery connect timed out")?
        .context("connect for delta sync (relay/discovery)")
}

/// Bidirectional delta sync (initiator side).
///
/// One bi stream, four framed messages:
/// 1. send our [`SyncRequest`] (vector clock A).
/// 2. read the peer's [`SyncResponse`] (vector clock B).
/// 3. read the peer's ops blob (ops we lack) and write them to disk.
/// 4. send our ops blob (ops the peer lacks under B) and `finish()`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn delta_sync(
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &std::path::Path,
    workspace_id: &WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    append_lock: &AppendLock,
    progress: &crate::progress::ProgressSink,
) -> Result<()> {
    let ops_dir = workspace_root.join("ops");
    let vector_clock = local_vector_clock(&ops_dir, actor)?;

    // The workspace id is the STABLE, SHARED workspace identity (see
    // `outl_core::WorkspaceId`), never the local path's file name — two paired
    // devices live at different paths but share one id, so the responder can
    // validate it without rejecting a legit peer.
    let request = SyncRequest {
        workspace_id: workspace_id.as_str().to_string(),
        vector_clock,
    };
    let encoded = encode_request(&request)?;

    let peer_addr: iroh::EndpointAddr = peer.into();
    let peer_node_id = peer_addr.id;
    let peer_short = peer_node_id.fmt_short().to_string();
    progress.emit(SyncProgress::Connecting {
        peer: peer_short.clone(),
    });
    let conn = connect_with_fallback(endpoint, peer_addr).await?;

    let (mut send, mut recv) = conn.open_bi().await.context("open bi stream")?;

    // 1. send our vector clock.
    send.write_all(&encoded)
        .await
        .context("send sync request")?;

    // 2. read the peer's vector clock so we can compute the reverse delta.
    //
    // A REFUSAL lands here, not on the `conn.closed()` match at the end: the
    // responder validates the workspace id and the peer allow-list *before*
    // writing anything, so a rejected dial dies in this read. Left as a bare
    // `?` it was the one failure that reached the user as nothing at all — a
    // device revoked on the other end just showed "offline", panel silent,
    // while a phone locking its screen painted red. Exactly backwards.
    let response = match read_response(&mut recv).await {
        Ok(response) => response,
        Err(e) => {
            if let Some(reason) = close_refusal_reason(&conn) {
                progress.emit(SyncProgress::Failed {
                    peer: peer_short.clone(),
                    error: reason.to_string(),
                });
            }
            return Err(e).context("read sync response");
        }
    };
    let peer_clock = response.vector_clock;

    // 3. read the peer's ops blob (ops we lack) and persist.
    let received = read_ops_blob(&mut recv).await.context("read peer ops")?;
    let (received_count, touched_nodes) =
        ingest_received_ops(&ops_dir, actor, &received, &peer_ready_tx, append_lock).await?;
    if received_count > 0 {
        info!(
            "delta sync: received {} ops from {}",
            received_count,
            peer_node_id.fmt_short()
        );
        progress.emit(SyncProgress::ReceivedOps {
            peer: peer_short.clone(),
            count: received_count as u64,
            nodes: touched_nodes.iter().map(|n| n.to_string()).collect(),
        });
    }

    // 4. push the ops the peer is missing under its own vector clock.
    let to_push = ops_missing_for(&ops_dir, actor, &peer_clock)?;
    let blob = encode_ops_blob(&to_push)?;
    send.write_all(&blob).await.context("send our ops blob")?;
    send.finish().context("finish send")?;
    if !to_push.is_empty() {
        info!(
            "delta sync: pushed {} ops to {}",
            to_push.len(),
            peer_node_id.fmt_short()
        );
        progress.emit(SyncProgress::PushedOps {
            peer: peer_short.clone(),
            count: to_push.len() as u64,
        });
    }

    // Do NOT force-close here: the responder still has to read this final ops
    // blob off the stream. Slamming the connection shut with `conn.close()`
    // races that read and silently drops our push (the "B never receives A's
    // ops" bug). Instead, wait for the responder to finish ingesting and close
    // the connection itself; `closed()` returns once it does.
    //
    // The responder closes with code 0 ("done") ONLY after it has durably
    // ingested our pushed ops (`serve` step 4). Any OTHER close — a
    // mid-exchange teardown on a suspended iPhone / carrier-NAT drop, a reset,
    // a timeout — means our push may never have landed. So require the "done"
    // close: without it, reporting `Ok` was a false success that logged
    // "catch-up: sync ok" while the peer stayed empty (the desktop→mobile
    // "synced ok but nothing arrived" bug). A lost close frame on an otherwise-
    // successful ingest only costs a redundant re-push next tick, which the
    // receiver's ingest dedup absorbs — far cheaper than silently losing ops.
    //
    // The `Err` is identical in both unconfirmed arms below — only what the
    // user is told differs. A peer that went away mid-exchange is a phone that
    // locked its screen, and telling someone their sync is broken every time
    // they pocket their phone is how a correct retry reads as a defect.
    let closed = conn.closed().await;
    match classify_close(&closed) {
        CloseVerdict::Confirmed => {
            progress.emit(SyncProgress::Synced { peer: peer_short });
            Ok(())
        }
        CloseVerdict::Interrupted => {
            progress.emit(SyncProgress::Interrupted {
                peer: peer_short,
                reason: closed.to_string(),
            });
            Err(anyhow::anyhow!(
                "peer did not confirm durable ingest (interrupted: {closed})"
            ))
        }
        CloseVerdict::Failed => {
            progress.emit(SyncProgress::Failed {
                peer: peer_short,
                error: format!("peer did not confirm durable ingest (closed: {closed})"),
            });
            Err(anyhow::anyhow!(
                "peer did not confirm durable ingest (closed: {closed})"
            ))
        }
    }
}

// ── Sync protocol handler ────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct SyncProtocolHandler {
    pub(crate) workspace_root: PathBuf,
    /// Live, shared workspace identity. The serve side reads it per-connection to
    /// validate the initiator's `SyncRequest.workspace_id` — a mismatch means the
    /// peer is on a DIFFERENT workspace, so we reject instead of cross-merging two
    /// distinct workspaces' op logs. Because pairing adoption updates this handle,
    /// a freshly paired peer (now sharing the host's id) passes validation.
    pub(crate) workspace_id: SharedWorkspaceId,
    pub(crate) actor: ActorId,
    pub(crate) peer_ready_tx: std::sync::mpsc::Sender<()>,
    /// Same process-wide append guard the initiator side holds — the serve side
    /// writes received ops too, so it must serialize against `delta_sync`.
    pub(crate) append_lock: AppendLock,
    /// Bumped for the duration of every accepted exchange (RAII, see
    /// [`crate::coordination::begin_inbound_serve`]) so
    /// `IrohSyncTransport::peers_in_flight` counts responder-side work too.
    /// The mobile background flush waits on that count before releasing its
    /// OS runtime assertion, and an inbound push still short of its
    /// close-code-0 confirmation is precisely the exchange it holds the
    /// window open to finish.
    pub(crate) inbound_serves: crate::coordination::InboundServes,
}

impl std::fmt::Debug for SyncProtocolHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncProtocolHandler")
            .field("workspace_root", &self.workspace_root)
            .field("actor", &self.actor)
            .finish_non_exhaustive()
    }
}

impl ProtocolHandler for SyncProtocolHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = self.serve(conn).await {
            warn!("sync serve failed: {e:#}");
            return Err(AcceptError::from_boxed(e.into()));
        }
        Ok(())
    }
}

impl SyncProtocolHandler {
    /// Bidirectional delta sync (responder side).
    ///
    /// Mirrors [`delta_sync`] on the same bi stream, four framed messages:
    /// 1. read the initiator's [`SyncRequest`] (vector clock A).
    /// 2. send our [`SyncResponse`] (vector clock B).
    /// 3. send our ops blob — ops the initiator lacks under A.
    /// 4. read the initiator's ops blob (ops we lack) and persist, firing
    ///    `peer_ready_tx`.
    async fn serve(&self, conn: Connection) -> Result<()> {
        // Held for the WHOLE exchange (RAII, so every early return, error and
        // cancellation clears it): from here until after the durable-ingest
        // confirmation close, this serve must be visible to
        // `peers_in_flight()` — a background flush reading zero while a peer
        // is mid-push would release the OS window and suspend the exact
        // exchange it exists to finish.
        let _serving = crate::coordination::begin_inbound_serve(&self.inbound_serves);
        let (mut send, mut recv) = conn.accept_bi().await.context("accept bi stream")?;

        // 1. read the initiator's vector clock (length-prefixed: the initiator
        //    does NOT finish the stream here, so we can't read_to_end).
        let request = read_request(&mut recv).await.context("read sync request")?;

        // Reject a peer on a DIFFERENT workspace. The id is the stable, shared
        // workspace identity, so two legit devices for the same workspace match;
        // a genuinely different workspace (different id) is correctly refused
        // before any op crosses. Read the live value — pairing adoption may have
        // updated it since boot.
        let local_id = self
            .workspace_id
            .read()
            .expect("workspace id rwlock poisoned")
            .as_str()
            .to_string();
        if request.workspace_id != local_id {
            warn!(
                local = %local_id,
                remote = %request.workspace_id,
                "rejecting sync from peer on a different workspace"
            );
            conn.close(CLOSE_WORKSPACE_MISMATCH.into(), b"workspace-mismatch");
            return Ok(());
        }

        // Reject a peer that is NOT (or is no longer) an approved device for this
        // workspace. The workspace_id check above only proves the peer THINKS it
        // belongs here — a device removed from `peers.json` still knows the id, so
        // without this it would keep pulling history and pushing edits (issue #158
        // "removing a paired device doesn't revoke its access"). Read `peers.json`
        // fresh on every inbound connection (not cached at boot): it can change
        // while the transport runs, so a `peer remove` takes effect on the very
        // next connection instead of after a restart.
        let remote_id = conn.remote_id().to_string();
        let peers_path = crate::peers::workspace_peers_path(&self.workspace_root);
        let store = match tokio::task::spawn_blocking(move || {
            crate::peers::PeersStore::load_or_default(&peers_path)
        })
        .await
        {
            Ok(Ok(store)) => store,
            Ok(Err(e)) => {
                // Fail CLOSED: if the peer list can't be read we can't prove the
                // peer is approved, so reject rather than fall back to open access.
                warn!(
                    peer = %conn.remote_id().fmt_short(),
                    "rejecting sync: peers.json unreadable ({e:#})"
                );
                conn.close(CLOSE_UNKNOWN_PEER.into(), b"unknown-peer");
                return Ok(());
            }
            Err(e) => {
                warn!(
                    peer = %conn.remote_id().fmt_short(),
                    "rejecting sync: peers.json load task failed ({e})"
                );
                conn.close(CLOSE_UNKNOWN_PEER.into(), b"unknown-peer");
                return Ok(());
            }
        };
        let authorized = store.list().iter().any(|p| p.node_id == remote_id);
        if !authorized {
            warn!(
                peer = %conn.remote_id().fmt_short(),
                "rejecting sync from an unknown / revoked peer (not in peers.json)"
            );
            conn.close(CLOSE_UNKNOWN_PEER.into(), b"unknown-peer");
            return Ok(());
        }

        let ops_dir = self.workspace_root.join("ops");

        // 2. send our own vector clock so the initiator can compute its push.
        let our_clock = local_vector_clock(&ops_dir, self.actor)?;
        let response = SyncResponse {
            vector_clock: our_clock,
        };
        send.write_all(&encode_response(&response)?)
            .await
            .context("send sync response")?;

        // 3. send the ops the initiator is missing under A.
        let to_push = ops_missing_for(&ops_dir, self.actor, &request.vector_clock)?;
        let blob = encode_ops_blob(&to_push)?;
        send.write_all(&blob).await.context("send our ops blob")?;
        send.finish().context("finish sending ops")?;

        // 4. read the initiator's ops blob (ops we lack) and persist.
        let received = read_ops_blob(&mut recv)
            .await
            .context("read initiator ops")?;
        let (received_count, _touched) = ingest_received_ops(
            &ops_dir,
            self.actor,
            &received,
            &self.peer_ready_tx,
            &self.append_lock,
        )
        .await?;
        if received_count > 0 {
            info!("delta sync: received {} ops (serve side)", received_count);
        }

        // Snapshot the peer's current direct socket, but do NOT write it yet —
        // see the close below.
        let direct_sock = conn.paths().iter().find_map(|p| match p.remote_addr() {
            iroh::TransportAddr::Ip(sock) => Some(*sock),
            _ => None,
        });

        // We've now drained the initiator's pushed ops AND fsynced them, so
        // it's safe to close. The initiator is parked on `conn.closed()`
        // waiting for exactly this — code 0 is the signal that the responder
        // durably has the push.
        //
        // Nothing may run between the fsync above and this line. Every
        // instruction here is time in which a peer whose OS is suspending it
        // (a locked iPhone, a dozing Android) dies *after* durably ingesting
        // but *before* confirming — a false negative that costs the initiator
        // a redundant re-push and paints the user's Sync panel red. The
        // `peers.json` refresh below used to sit here, and it is blocking file
        // I/O on the async task.
        conn.close(CLOSE_DONE.into(), b"done");

        // Self-heal a moved peer's stale stored address: this inbound dial
        // arrived over the peer's CURRENT direct socket, so refresh
        // `peers.json` with it (dropping any dead direct addr) — the next
        // outbound dial then reaches the peer directly instead of stalling on
        // the old IP. Only when there IS a direct (IP) path; a purely relayed
        // connection carries no usable peer socket. Best-effort — never fail
        // the sync over it, and now never delay the confirmation either.
        if let Some(sock) = direct_sock {
            let workspace_root = self.workspace_root.clone();
            let remote = conn.remote_id();
            let refreshed = tokio::task::spawn_blocking(move || {
                crate::peers::refresh_peer_direct_addr(&workspace_root, remote, sock)
            })
            .await;
            match refreshed {
                Ok(Ok(true)) => info!("refreshed direct addr for {} → {sock}", remote.fmt_short()),
                Ok(Ok(false)) => {}
                Ok(Err(e)) => debug!("peer addr refresh failed: {e}"),
                Err(e) => debug!("peer addr refresh task failed: {e}"),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #155 — a frame's declared length is attacker-controlled, so an oversized
    /// claim must be rejected *before* it can size an allocation. Guards the
    /// `0xFFFFFFFF` → ~4 GiB OOM as well as the exact cap boundary.
    #[test]
    fn frame_body_length_is_capped() {
        // The 4-byte 0xFFFFFFFF prefix (the ~4 GiB OOM claim) is refused.
        assert!(checked_frame_body_len([0xff, 0xff, 0xff, 0xff]).is_err());
        // One byte over the ceiling is refused.
        let over = (MAX_FRAME_BODY as u32) + 1;
        assert!(checked_frame_body_len(over.to_be_bytes()).is_err());
        // The ceiling itself, and anything under it, is accepted verbatim.
        assert_eq!(
            checked_frame_body_len((MAX_FRAME_BODY as u32).to_be_bytes()).expect("cap is allowed"),
            MAX_FRAME_BODY
        );
        assert_eq!(
            checked_frame_body_len(0u32.to_be_bytes()).expect("empty body is allowed"),
            0
        );
        assert_eq!(
            checked_frame_body_len(1234u32.to_be_bytes()).expect("small body is allowed"),
            1234
        );
    }
}
