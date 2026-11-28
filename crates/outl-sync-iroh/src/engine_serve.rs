//! The **responder** half of the delta-sync exchange: the protocol handler the
//! router mounts on [`crate::protocol::SYNC_ALPN`].
//!
//! Split from [`crate::engine_sync`], which owns the initiator and the framing
//! both sides share. What lives here is everything only the answering side
//! does: the workspace-id and `authz` verdicts, the inbound-serve count a
//! background flush waits on, the reachability record, and the stale-address
//! self-heal off the connection that just arrived.

use std::path::PathBuf;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use outl_core::id::ActorId;
use tracing::{debug, info, warn};

use crate::engine::{AppendLock, SharedWorkspaceId};
use crate::engine_sync::{read_ops_blob, read_request};
use crate::oplog::ingest_received_ops;
use crate::oplog_census::Census;
use crate::oplog_select::ops_missing_for;
use crate::peer_label::peer_log_label;
use crate::peers::{refresh_peer_direct_addr, workspace_peers_path};
use crate::protocol::{
    encode_blob_frame, encode_ops_blob, encode_response, SyncResponse, ACK_DURABLE, CLOSE_NORMAL,
    CLOSE_WORKSPACE_MISMATCH,
};

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
    /// The transport's one reachability map, shared with every outbound dial.
    /// The serve side writes to it because a peer that can only *dial* us (a
    /// phone behind carrier NAT) never answers one of ours — `crate::health`
    /// carries the reasoning.
    pub(crate) health: crate::health::PeerHealthMap,
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
        // "removing a paired device doesn't revoke its access").
        //
        // The verdict lives in `crate::authz`, NOT here. It used to be inline,
        // which is exactly why `SNAPSHOT_ALPN` and `ASSET_ALPN` — mounted on this
        // same endpoint, answering these same dialers — shipped the whole
        // materialized workspace and every uploaded file to anyone who asked.
        // One owner, three callers; see that module's header.
        if !crate::authz::authorize_or_close(conn, &self.workspace_root).await {
            return Ok(());
        }

        // An approved device reached us — for a peer we cannot dial back, the
        // only evidence the sync dot will ever get (#575). The position is
        // load-bearing both ways: before `authz` a refused dialer would mark
        // itself reachable, and after the ack a `send.finish()` that errors on
        // an initiator which already hung up skips it, though the peer reached
        // us all the same.
        self.health.record_reached(conn.remote_id());

        let ops_dir = self.workspace_root.join("ops");

        // 2. send our own vector clock so the initiator can compute its push.
        //
        // One census answers both this and step 3: nothing between them
        // touches the op log (the initiator's ops are ingested in step 4),
        // so a second read would re-derive the same answer at `O(total
        // ops)`. See `oplog_census::Census`.
        let census = Census::read(&ops_dir)?;
        let response = SyncResponse {
            vector_clock: census.vector_clock(),
            origin: Some(self.actor),
        };
        send.write_all(&encode_response(&response)?)
            .await
            .context("send sync response")?;

        // 3. send the ops the initiator is missing under A.
        let to_push = ops_missing_for(&ops_dir, &census, &request.vector_clock, request.origin)?;
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
        crate::pending::served(&self.workspace_root, conn.remote_id(), &request);

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
            let refreshed = tokio::task::spawn_blocking(move || -> Result<Option<String>> {
                let changed = refresh_peer_direct_addr(&workspace_root, remote, sock)?;
                // Resolved here, not at the `info!`: `peer_log_label` reads
                // `peers.json` in `Display::fmt`, which belongs off this task.
                let peers = workspace_peers_path(&workspace_root);
                Ok(changed.then(|| peer_log_label(&peers, remote).to_string()))
            })
            .await;
            match refreshed {
                Ok(Ok(Some(label))) => info!("refreshed direct addr for {label} → {sock}"),
                Ok(Ok(None)) => {}
                Ok(Err(e)) => debug!("peer addr refresh failed: {e}"),
                Err(e) => debug!("peer addr refresh task failed: {e}"),
            }
        }
        Ok(())
    }
}
