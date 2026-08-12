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
//! - The framing helpers (`read_frame` + the typed `read_*` wrappers).
//!
//! What the wire reads from and writes to DISK lives in [`crate::oplog`]
//! (`local_vector_clock`, `ops_missing_for`, `ingest_received_ops` and the
//! append-serialization invariant). Those rules hold regardless of which wire
//! version calls them, which is why they are not in here.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use outl_actions::SyncProgress;
use outl_core::id::ActorId;
use outl_core::{LogOp, WorkspaceId};
use tracing::{debug, info, warn};

use crate::engine::{AppendLock, SharedWorkspaceId};
use crate::oplog::{ingest_received_ops, local_vector_clock, ops_missing_for};
use crate::protocol::{
    classify_close, close_refusal_reason, decode_ops_blob, decode_request, decode_response,
    encode_blob_frame, encode_ops_blob, encode_request, encode_response, CloseVerdict, SyncRequest,
    SyncResponse, ACK_DURABLE, CLOSE_NORMAL, CLOSE_UNKNOWN_PEER, CLOSE_WORKSPACE_MISMATCH,
    SYNC_ALPN,
};

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

/// Length of the big-endian length prefix [`read_frame`] leaves on the front
/// of the buffer it returns. Every consumer that reaches past the prefix
/// (rather than handing the whole `[prefix || body]` buffer to a `decode_*`
/// helper) must offset by this, never by a bare literal, so a change to the
/// framing shape has one name to chase.
const FRAME_PREFIX_LEN: usize = 4;

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

/// Why [`read_ack`] failed, split by who is at fault.
///
/// The split is load-bearing for the verdict: a broken stream is the peer
/// going away, so the connection's close reason decides amber vs red. A
/// well-formed frame that is not [`ACK_DURABLE`] is a live peer speaking a
/// protocol this version does not understand — the connection is still open,
/// `close_reason()` is `None`, and defaulting that to "interrupted" would
/// dress a protocol failure up as a transient suspend and retry it forever.
enum AckError {
    /// The stream ended before a full ack frame arrived.
    Stream(anyhow::Error),
    /// A frame arrived, but it is not the durable-ingest confirmation.
    Protocol(anyhow::Error),
}

impl AckError {
    fn into_inner(self) -> anyhow::Error {
        match self {
            AckError::Stream(e) | AckError::Protocol(e) => e,
        }
    }
}

/// Read the responder's durable-ingest confirmation.
///
/// One byte, framed like everything else on this stream. `Ok(())` means the
/// peer has our push on disk and fsynced; any error means we cannot claim it
/// landed and must re-push, which the receiver's dedup makes free.
///
/// A byte that is not [`ACK_DURABLE`] is an error rather than a shrug. It
/// means the peer answered something this version does not understand, and
/// treating an unknown answer as a yes is how a confirmation protocol stops
/// being one.
async fn read_ack(recv: &mut iroh::endpoint::RecvStream) -> Result<(), AckError> {
    let frame = read_frame(recv)
        .await
        .context("read durable-ingest ack")
        .map_err(AckError::Stream)?;
    // `read_frame` returns `[prefix || body]`; the ack byte is the body's
    // first byte, one prefix-length past the front.
    match frame.get(FRAME_PREFIX_LEN) {
        Some(&ACK_DURABLE) => Ok(()),
        Some(other) => Err(AckError::Protocol(anyhow::anyhow!(
            "peer sent an unknown ack byte: {other}"
        ))),
        None => Err(AckError::Protocol(anyhow::anyhow!(
            "peer sent an empty ack frame"
        ))),
    }
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
pub(crate) async fn connect_with_fallback(
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
///
/// Any failure drops this peer's pooled connection before returning. A sync
/// can fail with the connection half-open — a stream reset, a peer whose OS
/// froze it mid-exchange — and reusing that connection fails the next sync
/// identically, forever, with no way out but a restart. Re-dialling costs one
/// connect; keeping a dead entry costs every future sync.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn delta_sync(
    conns: &crate::peer_conn::PeerConnections,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &std::path::Path,
    workspace_id: &WorkspaceId,
    actor: ActorId,
    peer_ready_tx: std::sync::mpsc::Sender<()>,
    append_lock: &AppendLock,
    progress: &crate::progress::ProgressSink,
) -> Result<()> {
    let peer_addr: iroh::EndpointAddr = peer.into();
    let node_id = peer_addr.id;
    let result = delta_sync_inner(
        conns,
        peer_addr,
        workspace_root,
        workspace_id,
        actor,
        peer_ready_tx,
        append_lock,
        progress,
    )
    .await;
    if result.is_err() {
        conns.invalidate(node_id);
    }
    result
}

#[allow(clippy::too_many_arguments)]
async fn delta_sync_inner(
    conns: &crate::peer_conn::PeerConnections,
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
    // A hot connection turns a sync into one round trip. `get_or_connect`
    // dials only when there is nothing usable cached.
    let conn = conns.get_or_connect(peer_addr).await?;

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

    // Wait for the responder's durable-ingest confirmation, which arrives as a
    // frame on this stream once its `sync_data()` returns.
    //
    // Requiring it is load-bearing and predates the frame: without it,
    // `delta_sync` returned `Ok` on any clean teardown, so a peer that
    // completed the exchange and never persisted anything logged
    // "catch-up: sync ok" while staying empty — the desktop→mobile "synced ok
    // but nothing arrived" bug. Not receiving it costs a redundant re-push
    // next tick, which the receiver's ingest dedup absorbs. That is far
    // cheaper than silently losing ops, and it is why this must never be
    // relaxed into "the exchange finished, assume it landed".
    //
    // It travels on the STREAM rather than as a connection close code, which
    // it used to be. Confirming by closing meant the connection could not
    // outlive one exchange, so every sync paid a fresh QUIC connect: measured
    // at ~20s of the 23s it took to move two ops between two devices on one
    // LAN. Same guarantee, same ordering, and the connection stays hot for
    // `peer_conn` to reuse.
    //
    // A read error here is the honest analogue of the old non-"done" close:
    // the peer went away mid-exchange (a locked iPhone, a dropped carrier-NAT
    // flow) and we cannot claim the push landed.
    match read_ack(&mut recv).await {
        Ok(()) => {
            progress.emit(SyncProgress::Synced { peer: peer_short });
            Ok(())
        }
        Err(ack_err) => {
            let verdict = match &ack_err {
                // A live peer answered with a frame this version has no
                // meaning for. No close reason will ever explain it (the
                // connection is still up), and calling it an interruption
                // keeps the amber retry loop spinning against a protocol
                // mismatch that retrying cannot fix.
                AckError::Protocol(_) => CloseVerdict::Failed,
                // The stream broke, so the connection tells us WHY it ended,
                // which decides whether the user sees amber (peer suspended,
                // retried) or red (peer refused, act on it). `close_reason()`
                // is `None` while the connection is still up, which happens
                // when the stream broke on its own; that is an interruption
                // too.
                AckError::Stream(_) => conn
                    .close_reason()
                    .map(|err| classify_close(&err))
                    .unwrap_or(CloseVerdict::Interrupted),
            };
            let e = ack_err.into_inner();
            match verdict {
                CloseVerdict::Failed => {
                    progress.emit(SyncProgress::Failed {
                        peer: peer_short,
                        error: format!("peer did not confirm durable ingest ({e:#})"),
                    });
                }
                CloseVerdict::Interrupted => {
                    progress.emit(SyncProgress::Interrupted {
                        peer: peer_short,
                        reason: e.to_string(),
                    });
                }
            }
            Err(e).context("peer did not confirm durable ingest")
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
    /// `IrohSyncTransport::inbound_serves` is what reports this to a caller.
    /// The mobile background flush waits on that count before releasing its
    /// OS runtime assertion, and an inbound push still short of its
    /// [`ACK_DURABLE`] confirmation is precisely the exchange it holds the
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
    /// Serve every exchange this connection carries, not just the first.
    ///
    /// The router calls this once per CONNECTION and closes the connection
    /// when it returns. Serving one stream and returning is therefore what
    /// made a pooled connection useless: the initiator kept it, the peer had
    /// already closed it, and the next sync died on
    /// `connection lost: closed by peer`. The pool would then reconnect, so
    /// nothing failed and nothing got faster — the change would have been
    /// invisible in every test that only syncs once.
    ///
    /// Looping until `accept_bi` errors is what makes the connection worth
    /// keeping: one connect, then a stream per sync.
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        // One bad exchange does not condemn the connection (the next stream
        // may be fine, and tearing down puts us back to a connect per sync),
        // but an unbroken run of failures means the peer is wedged — a
        // protocol bug, a corrupt sender — and serving it forever keeps this
        // task alive and logging with no exchange ever landing. Cap the RUN,
        // not the total: any success proves the connection still works and
        // resets the count.
        const MAX_CONSECUTIVE_FAILURES: u32 = 3;
        let mut consecutive_failures = 0u32;
        loop {
            let (send, recv) = match conn.accept_bi().await {
                Ok(streams) => streams,
                // The peer closed, or the connection timed out. Both are the
                // normal end of a pooled connection's life, not a failure.
                Err(e) => {
                    debug!("sync connection closed: {e}");
                    return Ok(());
                }
            };
            if let Err(e) = self.serve_exchange(&conn, send, recv).await {
                consecutive_failures += 1;
                warn!(
                    "sync serve failed ({consecutive_failures}/{MAX_CONSECUTIVE_FAILURES} in a row): {e:#}"
                );
                if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
                    // CLOSE_NORMAL, so the initiator reads Interrupted (amber,
                    // reconnect-and-retry), not a red refusal: the remedy for
                    // a wedged pooled connection is a fresh one.
                    warn!("closing sync connection after {MAX_CONSECUTIVE_FAILURES} consecutive failed exchanges");
                    conn.close(CLOSE_NORMAL.into(), b"too-many-failed-exchanges");
                    return Ok(());
                }
                continue;
            }
            consecutive_failures = 0;
        }
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
    async fn serve_exchange(
        &self,
        conn: &Connection,
        mut send: iroh::endpoint::SendStream,
        mut recv: iroh::endpoint::RecvStream,
    ) -> Result<()> {
        // Held for the WHOLE exchange (RAII, so every early return, error and
        // cancellation clears it): from here until after the durable-ingest
        // confirmation, this serve must be visible to `inbound_serves()` — a
        // background flush reading zero while a peer is mid-push would release
        // the OS window and suspend the exact exchange it exists to finish.
        //
        // Scoped to the EXCHANGE, not the connection, now that a connection
        // carries many. Holding it for the connection's life would pin the
        // count above zero between syncs and make every background window run
        // to its cap.
        let _serving = crate::coordination::begin_inbound_serve(&self.inbound_serves);

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
        // Deliberately NOT finished here. The durable-ingest confirmation goes
        // out on this same stream once the initiator's push is fsynced, and
        // finishing now is what forced that confirmation onto the connection
        // close code — which in turn forced a fresh QUIC connect per sync.

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
        // see the confirmation below.
        let direct_sock = conn.paths().iter().find_map(|p| match p.remote_addr() {
            iroh::TransportAddr::Ip(sock) => Some(*sock),
            _ => None,
        });

        // We've drained the initiator's push AND fsynced it, so confirm.
        //
        // Nothing may run between the fsync above and this line. Every
        // instruction here is time in which a peer whose OS is suspending it
        // (a locked iPhone, a dozing Android) dies *after* durably ingesting
        // but *before* confirming — a false negative that costs the initiator
        // a redundant re-push and paints the user's Sync panel amber. The
        // `peers.json` refresh below used to sit here, and it is blocking file
        // I/O on the async task.
        //
        // The stream is finished right after, but the CONNECTION stays up: it
        // is the caller's to reuse (see `peer_conn`), and tearing it down here
        // is what used to make every sync pay a fresh QUIC connect.
        send.write_all(&encode_blob_frame(&[ACK_DURABLE])?)
            .await
            .context("send durable-ingest ack")?;
        send.finish().context("finish ack")?;

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
