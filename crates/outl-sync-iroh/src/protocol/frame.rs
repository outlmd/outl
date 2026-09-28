//! The 4-byte big-endian length prefix, and nothing that knows what is inside
//! it.
//!
//! Every frame on every ALPN in this crate is `[u32 BE length][body]`. That is
//! what lets one bi stream carry two independent op batches, or a manifest
//! followed by N blobs, without EOF framing ambiguity — the reader always knows
//! where the current body stops, so a zero-length body ("no ops", "no
//! snapshot", "no assets") is a valid frame rather than a hang.
//!
//! `ops_blob_len` is named for its first caller; the prefix it reads is the
//! same one every other frame carries.

use anyhow::Result;

/// Frame an arbitrary byte blob with a 4-byte big-endian length prefix.
///
/// Same framing as [`encode_ops_blob`], but over raw bytes rather than encoded
/// ops — used by [`crate::engine_snapshot`] to ship a materialized snapshot
/// (`snap-<actor>.bin`) as one frame on a bi stream. An empty slice yields a
/// valid zero-length body, so "no snapshot to send" is still an unambiguous
/// frame the reader can skip.
///
/// [`encode_ops_blob`]: super::encode_ops_blob
pub(crate) fn encode_blob_frame(body: &[u8]) -> Result<Vec<u8>> {
    let len = u32::try_from(body.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + body.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(body);
    Ok(buf)
}

/// Read the declared length of a length-prefixed ops blob from its first
/// 4 bytes. The full frame is `4 + returned_len` bytes.
pub(super) fn ops_blob_len(prefix: &[u8]) -> Result<usize> {
    anyhow::ensure!(prefix.len() >= 4, "buffer too short for length prefix");
    Ok(u32::from_be_bytes(prefix[..4].try_into()?) as usize)
}
