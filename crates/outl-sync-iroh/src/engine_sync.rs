//! Delta-sync wire protocol — the on-the-wire half of the iroh transport.
//!
//! Extracted from `engine.rs` so that module stays focused on boot
//! orchestration (`run_iroh`, the `IrohSyncTransport` struct, channel wiring).
//! This module owns the four-message vector-clock exchange both sync directions
//! run over a single bi stream:
//!
//! - [`delta_sync`] — the **initiator** side (boot connect, catch-up, gossip,
//!   pairing, and the `sync_now` force-trigger all dial through it).
//! - The framing helpers (`read_frame` + the typed `read_*` wrappers).
//!
//! The **responder** half is [`crate::engine_serve`], split out when this file
//! crossed the size ratchet: the two sides share the wire format and nothing
//! else, and only the responder reads `peers.json`, the authz verdict and the
//! peer-address self-heal.
//!
//! Disk access: [`crate::oplog_census`] reads, [`crate::oplog`] writes.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use outl_actions::SyncProgress;
use outl_core::id::ActorId;
use outl_core::{LogOp, WorkspaceId};
use tracing::{debug, info};

use crate::engine::AppendLock;
use crate::oplog::ingest_received_ops;
use crate::oplog_census::{local_vector_clock, Census};
use crate::oplog_select::ops_missing_for;
use crate::peer_label::peer_log_label;
use crate::peers::workspace_peers_path;
use crate::protocol::{
    classify_close, close_refusal_reason, decode_ops_blob, decode_request, decode_response,
    encode_ops_blob, encode_request, read_failure_context, read_frame, CloseVerdict, SyncRequest,
    SyncResponse, ACK_DURABLE, FRAME_PREFIX_LEN, SYNC_ALPN,
};

/// Read a length-prefixed `SyncRequest` (vector clock) from a recv stream.
///
/// The initiator does not `finish()` after the request (it streams its push
/// later on the same bi stream), so the responder reads an explicit frame
/// rather than `read_to_end`.
pub(crate) async fn read_request(recv: &mut iroh::endpoint::RecvStream) -> Result<SyncRequest> {
    decode_request(&read_frame(recv).await?)
}

/// Read a length-prefixed `SyncResponse` (vector clock) from a recv stream.
async fn read_response(recv: &mut iroh::endpoint::RecvStream) -> Result<SyncResponse> {
    decode_response(&read_frame(recv).await?)
}

/// Read a length-prefixed ops blob from a recv stream and decode it.
pub(crate) async fn read_ops_blob(recv: &mut iroh::endpoint::RecvStream) -> Result<Vec<LogOp>> {
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
    let started = std::time::Instant::now();
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
    // Only the rtt is ours to add. Whether the peer was *reached* was already
    // decided by the connection pool, and a failure here writes nothing: an
    // exchange that opened and then broke (a half-sent push, a peer that
    // suspended) leaves a device we are plainly in contact with. Recording
    // that as offline is what made the dot flicker green/orange while a
    // 500k-op resend was still running.
    if result.is_ok() {
        conns.health().record_success(node_id, started);
    }
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
    let vector_clock = local_vector_clock(&ops_dir)?;

    // The workspace id is the STABLE, SHARED workspace identity (see
    // `outl_core::WorkspaceId`), never the local path's file name — two paired
    // devices live at different paths but share one id, so the responder can
    // validate it without rejecting a legit peer.
    let request = SyncRequest {
        workspace_id: workspace_id.as_str().to_string(),
        vector_clock,
        origin: Some(actor),
    };
    let encoded = encode_request(&request)?;

    let peer_addr: iroh::EndpointAddr = peer.into();
    let peer_node_id = peer_addr.id;
    let peer_short = peer_node_id.fmt_short().to_string();
    let peers_path = workspace_peers_path(workspace_root);
    let label = peer_log_label(&peers_path, peer_node_id);
    progress.emit(SyncProgress::Connecting {
        peer: peer_short.clone(),
    });
    // A hot connection turns a sync into one round trip. `get_or_connect`
    // dials only when there is nothing usable cached.
    //
    // It is also what decides reachability (`PeerConnections::health`), since
    // connecting is the only step that answers "can this device be reached".
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
            let refusal = close_refusal_reason(&conn);
            if let Some(reason) = refusal {
                progress.emit(SyncProgress::Failed {
                    peer: peer_short.clone(),
                    error: reason.to_string(),
                });
            }
            return Err(e).context(read_failure_context(refusal));
        }
    };
    let peer_clock = response.vector_clock;
    let peer_origin = response.origin;

    // 3. read the peer's ops blob (ops we lack) and persist.
    let received = read_ops_blob(&mut recv).await.context("read peer ops")?;
    let (received_count, touched_nodes) =
        ingest_received_ops(&ops_dir, actor, &received, &peer_ready_tx, append_lock).await?;
    if received_count > 0 {
        info!("delta sync: received {received_count} ops from {label}");
        progress.emit(SyncProgress::ReceivedOps {
            peer: peer_short.clone(),
            count: received_count as u64,
            nodes: touched_nodes.iter().map(|n| n.to_string()).collect(),
        });
    }

    // 4. push the ops the peer is missing under its own vector clock.
    //
    // Read after step 3 on purpose, and NOT shared with the clock sent in
    // step 1: the ingest in between appended the peer's ops to our log, so
    // the step-1 census is a snapshot of a log that no longer exists. The
    // selection must range over what we hold *now*, or an op that arrived
    // from a third peer via this one would wait for the next exchange.
    let census = Census::read(&ops_dir)?;
    let to_push = ops_missing_for(&ops_dir, &census, &peer_clock, peer_origin)?;
    let blob = encode_ops_blob(&to_push)?;
    send.write_all(&blob).await.context("send our ops blob")?;
    send.finish().context("finish send")?;
    if !to_push.is_empty() {
        info!("delta sync: pushed {} ops to {label}", to_push.len());
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
            crate::pending::pushed(workspace_root, peer_node_id, &peer_clock, &request);
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
