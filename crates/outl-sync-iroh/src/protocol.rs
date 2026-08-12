//! Wire protocol for the outl sync ALPN.
//!
//! ALPN: `b"outl-sync/3"`
//!
//! ## Sync request (JSON, 4-byte length prefix)
//!
//! Sent by the side that wants to pull:
//! ```json
//! {
//!   "workspace_id": "my-workspace",
//!   "vector_clock": {
//!     "<actor-ulid>": {
//!       "max": { "physical_ms": 1234567890123, "logical": 5, "actor": "<ulid>" },
//!       "count": 347
//!     }
//!   }
//! }
//! ```
//!
//! ## Response (JSON, 4-byte length prefix)
//!
//! Sent by the responder right after it decodes the request, carrying the
//! responder's own vector clock so the initiator can compute the reverse
//! delta. Same `{ actor → ActorClock }` shape as the request's `vector_clock`.
//!
//! ## Ops blob (JSONL, 4-byte length prefix)
//!
//! A length-prefixed batch of newline-separated `LogOp` JSON lines. Used in
//! both directions so a single bi stream can carry two independent op batches
//! without EOF framing ambiguity.
//!
//! ## Bidirectional exchange (single bi stream)
//!
//! 1. initiator → responder: [`SyncRequest`] (vector clock A).
//! 2. responder → initiator: [`SyncResponse`] (vector clock B).
//! 3. responder → initiator: ops blob — ops missing under clock A (per-actor:
//!    everything above `A[actor].max`, or the actor's FULL log when a gap
//!    below `A[actor].max` is detected — see `engine_sync::ops_missing_for`).
//! 4. initiator → responder: ops blob — same rule under clock B, then
//!    `finish()`.
//! 5. responder → initiator: [`ACK_DURABLE`], written only after the batch is
//!    fsynced, then `finish()`.
//!
//! Every step is length-prefixed, so both directions fully reconcile on one
//! stream — and step 5 is why the CONNECTION survives it. Confirming by
//! closing (v2) meant a fresh QUIC connect per sync; see [`crate::peer_conn`].

use anyhow::Result;
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::LogOp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// ALPN for the op-sync protocol.
///
/// v2 bumped the vector clock from a bare max-HLC per actor to
/// `ActorClock` (max + count) so the sender can detect gaps below the
/// receiver's watermark. v1 and v2 clocks are wire-incompatible; the ALPN
/// bump makes an old↔new dial fail cleanly at connect instead of
/// half-conversing.
/// v3 moves the durable-ingest confirmation from the connection close code
/// onto the stream (`ACK_DURABLE`), so a connection outlives the exchange
/// and can be pooled. A v2 peer confirms by closing and a v3 peer waits for a
/// frame that never arrives, so the two must not talk: the ALPN bump makes
/// that a clean connect failure instead of a 30s hang on every sync.
pub const SYNC_ALPN: &[u8] = b"outl-sync/3";

/// Close code for "I am ending this connection normally" — a shutdown, a
/// finished snapshot or asset transfer, a completed pairing, a status probe.
///
/// It used to mean "durably ingested your push", which is why it was named
/// `CLOSE_DONE`. That moved onto the stream as [`ACK_DURABLE`] so the
/// connection could survive the exchange, and a close code that no longer
/// confirms anything should not keep a name that says it does.
pub const CLOSE_NORMAL: u32 = 0;

/// Close code: the dialer belongs to a different workspace (its
/// `SyncRequest.workspace_id` does not match ours). Sent before any payload.
pub const CLOSE_WORKSPACE_MISMATCH: u32 = 3;

/// Close code: the dialer is not in our `peers.json` — unpaired, or revoked
/// on this side. Sent before any payload.
pub const CLOSE_UNKNOWN_PEER: u32 = 4;

/// Durable-ingest confirmation, sent by the responder **on the stream** after
/// its `sync_data()` returns.
///
/// This used to be a connection close code, and that choice cost more than it
/// looked. Confirming by closing means the connection cannot survive the
/// exchange, so every sync pays a fresh QUIC connect: ~5s burned on a stale
/// direct address before the relay fallback, then the relay handshake. Two ops
/// between two devices on one LAN measured 23 seconds, ~20 of them connection
/// overhead. A frame costs one byte and leaves the connection hot, which is
/// what makes [`crate::peer_conn`] pooling possible at all.
///
/// The guarantee is unchanged and the ordering is the point: this is written
/// only after the batch is on disk and fsynced, so reading it still means "the
/// peer durably has your push". Writing it any earlier turns a confirmation
/// into a guess.
pub const ACK_DURABLE: u8 = 1;

/// What a peer's close means for the pass that just ran.
///
/// The distinction is the difference between "your sync is broken" and "your
/// phone locked", and only one of those is worth a red row in the UI. There is
/// no success variant: success is reading [`ACK_DURABLE`] off the stream, and
/// by the time anyone asks this question that read has already failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseVerdict {
    /// The peer stopped answering rather than refusing — OS suspension, sleep,
    /// a dropped carrier-NAT flow, or a clean shutdown on its side. Expected,
    /// transient, retried next tick.
    Interrupted,
    /// The peer answered and said no, or the transport itself failed. A real
    /// problem the user may have to act on.
    Failed,
}

/// Classify how a connection ended.
///
/// Split out of `delta_sync` so the table is a value one test can enumerate
/// rather than a `match` reachable only over real QUIC. Getting a variant into
/// the wrong bucket is silent by construction: both verdicts return the same
/// error and re-push, so nothing fails, the user just sees the wrong colour.
pub fn classify_close(err: &iroh::endpoint::ConnectionError) -> CloseVerdict {
    use iroh::endpoint::ConnectionError;
    match err {
        ConnectionError::TimedOut
        | ConnectionError::Reset
        | ConnectionError::LocallyClosed
        | ConnectionError::ConnectionClosed(_) => CloseVerdict::Interrupted,
        // A peer shutting down cleanly is going away, not refusing us.
        ConnectionError::ApplicationClosed(ac) if ac.error_code == CLOSE_NORMAL.into() => {
            CloseVerdict::Interrupted
        }
        _ => CloseVerdict::Failed,
    }
}

/// Human-readable reason a peer refused this connection, or `None` when it
/// ended some other way — still open, timed out, reset, or closed with a code
/// that is not a refusal.
///
/// A refusal reaches the initiator as a failed *read*, because the responder
/// closes before writing a byte. Without translating the code, the one failure
/// a user genuinely has to act on — this device is no longer paired — arrives
/// as an unexplained dead peer.
pub fn close_refusal_reason(conn: &iroh::endpoint::Connection) -> Option<&'static str> {
    let iroh::endpoint::ConnectionError::ApplicationClosed(ac) = conn.close_reason()? else {
        return None;
    };
    match u32::try_from(u64::from(ac.error_code)).ok()? {
        CLOSE_WORKSPACE_MISMATCH => Some("peer refused: different workspace"),
        CLOSE_UNKNOWN_PEER => Some("peer refused: this device is not paired with it"),
        _ => None,
    }
}

/// ALPN for device pairing.
pub const PAIRING_ALPN: &[u8] = b"outl-sync/pair/1";

/// ALPN for peer snapshot transfer (Phase 2 snapshot sync).
///
/// A freshly-paired device pulls a peer's materialized snapshot
/// (`snap-<actor>.bin`) over this ALPN so it can boot from settled state
/// instead of receiving + replaying the full op log. Carried on the SAME sync
/// endpoint's router (one endpoint per identity). See `crate::engine_snapshot`.
pub const SNAPSHOT_ALPN: &[u8] = b"outl-snapshot/1";

/// ALPN for peer binary-asset transfer (uploaded files: PDFs, images).
///
/// Asset bytes are content-addressed blobs stored at `<root>/assets/<hash>.<ext>`
/// and NEVER enter the op log (a multi-MB PDF replayed through the CRDT would
/// bloat every device's log). The `file` transport (iCloud / Syncthing) carries
/// them for free; over iroh they must be transferred explicitly. Unlike a
/// snapshot (one blob), assets are N files, so this ALPN negotiates a manifest
/// first (the peer's `assets/` basenames), then the initiator pulls only the
/// files it lacks. Carried on the SAME sync endpoint's router (one endpoint per
/// identity). See `crate::engine_assets`.
pub const ASSET_ALPN: &[u8] = b"outl-asset/1";

/// What one side knows about one actor's ops: the highest HLC it holds and
/// how many DISTINCT ops (by HLC) it holds for that actor — all `<= max` by
/// definition.
///
/// The `count` is what turns the max-HLC watermark into a gap detector: a
/// bare max assumes in-order, gapless delivery, so an op landing AHEAD of a
/// pending backlog permanently hid everything below the watermark (the
/// sender assumed the receiver had it). With the count, the sender can tell
/// "receiver holds fewer ops below its own max than I do" and fall back to a
/// full-log resend for that actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorClock {
    /// Highest HLC held for this actor.
    pub max: Hlc,
    /// Number of distinct ops (by HLC) held for this actor.
    pub count: u64,
}

/// The body of a sync request.
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncRequest {
    /// Workspace slug identifier.
    pub workspace_id: String,
    /// Per-actor max-HLC + distinct-op count. Missing actors imply "never
    /// seen" (HLC zero, zero ops).
    pub vector_clock: HashMap<ActorId, ActorClock>,
}

/// Serialize a `SyncRequest` with a 4-byte big-endian length prefix.
pub fn encode_request(req: &SyncRequest) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(req)?;
    let len = u32::try_from(json.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Deserialize a `SyncRequest` from a 4-byte length-prefixed buffer.
pub fn decode_request(buf: &[u8]) -> Result<SyncRequest> {
    anyhow::ensure!(buf.len() >= 4, "buffer too short for length prefix");
    let len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    Ok(serde_json::from_slice(&buf[4..4 + len])?)
}

/// The body of a sync response — the responder's own vector clock.
///
/// Sent right after the responder decodes the request, so the initiator can
/// compute the reverse delta (the ops the responder is missing).
#[derive(Debug, Serialize, Deserialize)]
pub struct SyncResponse {
    /// Per-actor max-HLC + distinct-op count the responder holds. Missing
    /// actors imply "never seen" (HLC zero, zero ops).
    pub vector_clock: HashMap<ActorId, ActorClock>,
}

/// Serialize a `SyncResponse` with a 4-byte big-endian length prefix.
pub fn encode_response(resp: &SyncResponse) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(resp)?;
    let len = u32::try_from(json.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Deserialize a `SyncResponse` from a 4-byte length-prefixed buffer.
pub fn decode_response(buf: &[u8]) -> Result<SyncResponse> {
    anyhow::ensure!(buf.len() >= 4, "buffer too short for length prefix");
    let len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    Ok(serde_json::from_slice(&buf[4..4 + len])?)
}

/// Serialize a single `LogOp` as a JSONL line (no trailing newline).
pub fn encode_op(op: &LogOp) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(op)?)
}

/// Deserialize a JSONL line into a `LogOp`.
pub fn decode_op(line: &[u8]) -> Result<LogOp> {
    Ok(serde_json::from_slice(line)?)
}

/// Serialize a batch of `LogOp`s into a length-prefixed JSONL blob.
///
/// Layout: `[4-byte big-endian length][JSONL body]`, where the body is
/// newline-separated `LogOp` JSON lines (with a trailing newline per line).
/// An empty slice yields a zero-length body, so "no ops to send" is still a
/// valid, unambiguous frame.
pub fn encode_ops_blob(ops: &[LogOp]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for op in ops {
        body.extend_from_slice(&encode_op(op)?);
        body.push(b'\n');
    }
    let len = u32::try_from(body.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&body);
    Ok(buf)
}

/// Frame an arbitrary byte blob with a 4-byte big-endian length prefix.
///
/// Same framing as [`encode_ops_blob`], but over raw bytes rather than encoded
/// ops — used by [`crate::engine_snapshot`] to ship a materialized snapshot
/// (`snap-<actor>.bin`) as one frame on a bi stream. An empty slice yields a
/// valid zero-length body, so "no snapshot to send" is still an unambiguous
/// frame the reader can skip.
pub fn encode_blob_frame(body: &[u8]) -> Result<Vec<u8>> {
    let len = u32::try_from(body.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(body);
    Ok(buf)
}

/// Read the declared length of a length-prefixed ops blob from its first
/// 4 bytes. The full frame is `4 + returned_len` bytes.
pub fn ops_blob_len(prefix: &[u8]) -> Result<usize> {
    anyhow::ensure!(prefix.len() >= 4, "buffer too short for length prefix");
    Ok(u32::from_be_bytes(prefix[..4].try_into()?) as usize)
}

/// Decode a length-prefixed ops blob into `LogOp`s.
///
/// Lines that fail to decode are skipped (the caller logs); the function only
/// errors on a malformed length prefix.
pub fn decode_ops_blob(buf: &[u8]) -> Result<Vec<LogOp>> {
    let len = ops_blob_len(buf)?;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    let body = &buf[4..4 + len];
    let mut ops = Vec::new();
    for line in body.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Ok(op) = decode_op(line) {
            ops.push(op);
        }
    }
    Ok(ops)
}

/// Serialize an asset manifest (a peer's `assets/` basenames) into a
/// length-prefixed, newline-separated frame.
///
/// Same framing as [`encode_ops_blob`] but over plain filename strings: the
/// responder ships the list of `<hash>.<ext>` names it holds so the initiator
/// can diff against its own `assets/` and pull only what it lacks (the names
/// ARE the content hashes, so a name match means the bytes match). An empty
/// slice yields a valid zero-length body, so "I have no assets" is still an
/// unambiguous frame.
pub fn encode_asset_manifest(names: &[String]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for name in names {
        // Names are validated filenames (no newline); the split on decode keys
        // on `\n`, so a stray one would corrupt the list — skip defensively.
        if name.contains('\n') {
            continue;
        }
        body.extend_from_slice(name.as_bytes());
        body.push(b'\n');
    }
    let len = u32::try_from(body.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&body);
    Ok(buf)
}

/// Decode a length-prefixed, newline-separated asset manifest into basenames.
///
/// Blank lines are skipped; non-UTF-8 lines are dropped. Names are NOT validated
/// here (that is the receiver's anti-traversal job in
/// [`crate::engine_assets`]) — this only reverses the framing.
pub fn decode_asset_manifest(buf: &[u8]) -> Result<Vec<String>> {
    anyhow::ensure!(buf.len() >= 4, "buffer too short for length prefix");
    let len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    let body = &buf[4..4 + len];
    Ok(body
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
        .filter_map(|line| std::str::from_utf8(line).ok().map(str::to_string))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::endpoint::ConnectionError;

    fn app_close(code: u32) -> ConnectionError {
        ConnectionError::ApplicationClosed(iroh::endpoint::ApplicationClose {
            error_code: code.into(),
            reason: Vec::new().into(),
        })
    }

    /// The whole decision table, enumerated.
    ///
    /// Every arm here is silent when it is wrong: a misclassified close still
    /// returns the same error and still re-pushes, so nothing fails and no
    /// test over real QUIC would notice. The only symptom is the user being
    /// told the wrong thing — which is exactly the bug this table was added to
    /// fix, so the table is the thing worth pinning.
    #[test]
    fn close_classification_covers_every_variant() {
        // Code 0 is a normal close, NOT a confirmation: v3 moved durable
        // ingest onto the stream as `ACK_DURABLE`, so a peer closing with 0
        // is just going away cleanly — amber, retried, never a red row.
        assert_eq!(
            classify_close(&app_close(CLOSE_NORMAL)),
            CloseVerdict::Interrupted
        );

        // The peer went away mid-exchange. A locked phone, a sleeping laptop,
        // a dropped carrier-NAT flow. Amber, retried, not the user's problem.
        for err in [
            ConnectionError::TimedOut,
            ConnectionError::Reset,
            ConnectionError::LocallyClosed,
        ] {
            assert_eq!(
                classify_close(&err),
                CloseVerdict::Interrupted,
                "{err:?} is a peer going away, not a peer refusing"
            );
        }

        // The peer answered and said no, or the two builds cannot talk. Red.
        assert_eq!(
            classify_close(&app_close(CLOSE_WORKSPACE_MISMATCH)),
            CloseVerdict::Failed
        );
        assert_eq!(
            classify_close(&app_close(CLOSE_UNKNOWN_PEER)),
            CloseVerdict::Failed
        );
        assert_eq!(
            classify_close(&ConnectionError::VersionMismatch),
            CloseVerdict::Failed
        );

        // An application code we do not recognise is Failed. The peer answered
        // with something this version has no meaning for, and guessing is how
        // the desktop→mobile "synced ok but nothing arrived" bug happened.
        assert_eq!(classify_close(&app_close(9)), CloseVerdict::Failed);
    }

    #[test]
    fn request_roundtrips_through_length_prefix() {
        let mut vc = HashMap::new();
        let actor = ActorId::new();
        vc.insert(
            actor,
            ActorClock {
                max: Hlc::new(42, 7, actor),
                count: 12,
            },
        );
        let req = SyncRequest {
            workspace_id: "demo".into(),
            vector_clock: vc,
        };
        let encoded = encode_request(&req).unwrap();
        let decoded = decode_request(&encoded).unwrap();
        assert_eq!(decoded.workspace_id, "demo");
        let clock = decoded.vector_clock.get(&actor).unwrap();
        assert_eq!(clock.max.physical_ms, 42);
        assert_eq!(clock.count, 12);
    }

    #[test]
    fn decode_request_rejects_short_buffer() {
        assert!(decode_request(&[0, 0]).is_err());
    }

    #[test]
    fn response_roundtrips_through_length_prefix() {
        let mut vc = HashMap::new();
        let actor = ActorId::new();
        vc.insert(
            actor,
            ActorClock {
                max: Hlc::new(99, 3, actor),
                count: 2000,
            },
        );
        let resp = SyncResponse { vector_clock: vc };
        let encoded = encode_response(&resp).unwrap();
        let decoded = decode_response(&encoded).unwrap();
        let clock = decoded.vector_clock.get(&actor).unwrap();
        assert_eq!(clock.max.physical_ms, 99);
        assert_eq!(clock.max.logical, 3);
        assert_eq!(clock.count, 2000);
    }

    #[test]
    fn decode_response_rejects_short_buffer() {
        assert!(decode_response(&[0, 0]).is_err());
    }

    fn sample_op(actor: ActorId, physical_ms: u64) -> LogOp {
        use outl_core::fractional::Fractional;
        use outl_core::id::NodeId;
        use outl_core::op::Op;
        LogOp {
            ts: Hlc::new(physical_ms, 0, actor),
            actor,
            op: Op::Create {
                node: NodeId::new(),
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        }
    }

    #[test]
    fn ops_blob_roundtrips() {
        let actor = ActorId::new();
        let ops = vec![
            sample_op(actor, 1),
            sample_op(actor, 2),
            sample_op(actor, 3),
        ];
        let blob = encode_ops_blob(&ops).unwrap();
        let decoded = decode_ops_blob(&blob).unwrap();
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].ts.physical_ms, 1);
        assert_eq!(decoded[2].ts.physical_ms, 3);
    }

    #[test]
    fn empty_ops_blob_is_valid_zero_length_frame() {
        let blob = encode_ops_blob(&[]).unwrap();
        assert_eq!(blob.len(), 4, "empty blob is just the length prefix");
        assert_eq!(ops_blob_len(&blob).unwrap(), 0);
        assert!(decode_ops_blob(&blob).unwrap().is_empty());
    }

    #[test]
    fn ops_blob_len_reads_declared_length() {
        let actor = ActorId::new();
        let ops = vec![sample_op(actor, 7)];
        let blob = encode_ops_blob(&ops).unwrap();
        let declared = ops_blob_len(&blob[..4]).unwrap();
        assert_eq!(declared, blob.len() - 4);
    }

    #[test]
    fn decode_ops_blob_rejects_short_buffer() {
        assert!(decode_ops_blob(&[0, 0]).is_err());
    }

    #[test]
    fn asset_manifest_roundtrips() {
        let names = vec![
            "abc123.pdf".to_string(),
            "deadbeef.png".to_string(),
            "0f0f0f".to_string(),
        ];
        let blob = encode_asset_manifest(&names).unwrap();
        let decoded = decode_asset_manifest(&blob).unwrap();
        assert_eq!(decoded, names);
    }

    #[test]
    fn empty_asset_manifest_is_valid_zero_length_frame() {
        let blob = encode_asset_manifest(&[]).unwrap();
        assert_eq!(blob.len(), 4, "empty manifest is just the length prefix");
        assert!(decode_asset_manifest(&blob).unwrap().is_empty());
    }

    #[test]
    fn decode_asset_manifest_rejects_short_buffer() {
        assert!(decode_asset_manifest(&[0, 0]).is_err());
    }

    #[test]
    fn asset_manifest_drops_names_with_embedded_newline() {
        // A name carrying a newline would split into two bogus entries on
        // decode; `encode_asset_manifest` skips it defensively.
        let names = vec!["good.pdf".to_string(), "bad\nname.pdf".to_string()];
        let decoded = decode_asset_manifest(&encode_asset_manifest(&names).unwrap()).unwrap();
        assert_eq!(decoded, vec!["good.pdf".to_string()]);
    }
}
