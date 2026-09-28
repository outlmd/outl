//! The body of the delta-sync exchange carried on [`SYNC_ALPN`](super::SYNC_ALPN).
//!
//! One bi stream carries, in order: the initiator's [`SyncRequest`], the
//! responder's [`SyncResponse`], an ops blob each way, and the responder's
//! [`ACK_DURABLE`]. Everything here is one side of that conversation — the
//! vector clock the two sides compare, the op batch that closes the gap, and
//! the byte that says the batch is on disk.
//!
//! The framing underneath is in [`super::frame`]; this module owns what the
//! bytes mean, not where the body stops.

use anyhow::Result;
use outl_core::hlc::Hlc;
use outl_core::id::ActorId;
use outl_core::LogOp;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::warn;

use super::frame::ops_blob_len;

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
pub(crate) struct ActorClock {
    /// Highest HLC held for this actor.
    pub(crate) max: Hlc,
    /// Number of distinct ops (by HLC) held for this actor.
    pub(crate) count: u64,
}

/// The body of a sync request.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SyncRequest {
    /// Workspace slug identifier.
    pub(crate) workspace_id: String,
    /// Per-actor max-HLC + distinct-op count. Missing actors imply "never
    /// seen" (HLC zero, zero ops).
    pub(crate) vector_clock: HashMap<ActorId, ActorClock>,
}

/// Serialize a `SyncRequest` with a 4-byte big-endian length prefix.
pub(crate) fn encode_request(req: &SyncRequest) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(req)?;
    let len = u32::try_from(json.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Deserialize a `SyncRequest` from a 4-byte length-prefixed buffer.
pub(crate) fn decode_request(buf: &[u8]) -> Result<SyncRequest> {
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
pub(crate) struct SyncResponse {
    /// Per-actor max-HLC + distinct-op count the responder holds. Missing
    /// actors imply "never seen" (HLC zero, zero ops).
    pub(crate) vector_clock: HashMap<ActorId, ActorClock>,
}

/// Serialize a `SyncResponse` with a 4-byte big-endian length prefix.
pub(crate) fn encode_response(resp: &SyncResponse) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(resp)?;
    let len = u32::try_from(json.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Deserialize a `SyncResponse` from a 4-byte length-prefixed buffer.
pub(crate) fn decode_response(buf: &[u8]) -> Result<SyncResponse> {
    anyhow::ensure!(buf.len() >= 4, "buffer too short for length prefix");
    let len = u32::from_be_bytes(buf[..4].try_into()?) as usize;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    Ok(serde_json::from_slice(&buf[4..4 + len])?)
}

/// Serialize a single `LogOp` as a JSONL line (no trailing newline).
pub(crate) fn encode_op(op: &LogOp) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(op)?)
}

/// Serialize a batch of `LogOp`s into a length-prefixed JSONL blob.
///
/// Layout: `[4-byte big-endian length][JSONL body]`, where the body is
/// newline-separated `LogOp` JSON lines (with a trailing newline per line).
/// An empty slice yields a zero-length body, so "no ops to send" is still a
/// valid, unambiguous frame.
pub(crate) fn encode_ops_blob(ops: &[LogOp]) -> Result<Vec<u8>> {
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

/// Decode a length-prefixed ops blob into `LogOp`s.
///
/// A line that fails to decode is skipped **and named in the log**; the
/// function only errors on a malformed length prefix.
///
/// The doc used to say "the caller logs", and the caller cannot: it is
/// handed the survivors and has nothing left to name. A peer's op
/// vanishing in silence is the read path's "never quietly" rule
/// (`outl-core/CLAUDE.md`, invariant #5) broken one layer earlier, and
/// the *disk* path already gets this right — `read_ops_file_into` warns
/// with the file and line. Made reachable by new input rather than new
/// code: `Fractional` now validates its alphabet on deserialize, so a
/// peer op carrying `position: "!"` is rejected here instead of aborting
/// this process inside `Fractional::between` three layers down (issue
/// #282).
///
/// The log line carries the record's index, its byte length and the
/// decoder's complaint, never the record itself — a `LogOp` body holds
/// the user's block text.
pub(crate) fn decode_ops_blob(buf: &[u8]) -> Result<Vec<LogOp>> {
    let len = ops_blob_len(buf)?;
    anyhow::ensure!(buf.len() >= 4 + len, "buffer shorter than declared length");
    let body = &buf[4..4 + len];
    let mut ops = Vec::new();
    for (i, line) in body.split(|&b| b == b'\n').enumerate() {
        if line.is_empty() {
            continue;
        }
        match serde_json::from_slice::<LogOp>(line) {
            Ok(op) => ops.push(op),
            Err(e) => warn!(
                "skipping undecodable op at record {i} of this blob ({} bytes): {e}",
                line.len()
            ),
        }
    }
    Ok(ops)
}

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
pub(crate) const ACK_DURABLE: u8 = 1;

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A peer op the decoder refuses costs that op and nothing else, and
    /// the blob's healthy records still land.
    ///
    /// `position: "!"` is the shape that matters: `Fractional` validates
    /// its alphabet on deserialize now, so this is refused at the wire
    /// instead of reaching `Fractional::between` and aborting the
    /// process (issue #282). The skip is warned about in
    /// `decode_ops_blob` rather than left to a caller that never sees it.
    #[test]
    fn decode_ops_blob_drops_only_the_undecodable_record() {
        let actor = ActorId::new();
        let good = encode_ops_blob(&[sample_op(actor, 1), sample_op(actor, 2)]).unwrap();
        let body = String::from_utf8(good[4..].to_vec()).unwrap();
        let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
        let poisoned = lines[0].replace("\"position\":\"a\"", "\"position\":\"!\"");
        assert_ne!(
            poisoned, lines[0],
            "fixture must actually poison the position"
        );
        lines[0] = poisoned;

        let rebuilt = format!("{}\n", lines.join("\n"));
        let mut blob = (rebuilt.len() as u32).to_be_bytes().to_vec();
        blob.extend_from_slice(rebuilt.as_bytes());

        let decoded = decode_ops_blob(&blob).unwrap();
        assert_eq!(decoded.len(), 1, "exactly the poisoned record is lost");
        assert_eq!(decoded[0].ts.physical_ms, 2);
    }
}
