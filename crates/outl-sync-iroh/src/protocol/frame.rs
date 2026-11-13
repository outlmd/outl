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

use anyhow::{Context, Result};

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

/// Hard ceiling on a single sync frame's body, enforced before allocating.
///
/// The 4-byte length prefix is attacker-controlled — a *paired* peer could be
/// compromised or buggy (QUIC authenticates the stream, so this isn't an
/// on-path attacker, but a bad peer is enough). Trusting it to size a `Vec` up
/// front let a 4-byte `0xFFFFFFFF` force a ~4 GiB allocation — an instant OOM /
/// iOS jetsam kill (issue #155). The incremental read in [`read_frame`] means
/// we still only allocate as bytes actually arrive.
///
/// It was 256 MB, sized so a gap-recovery full-actor-log resend always fit. A
/// real 253 MB op log then cleared that ceiling by 1.2% and killed an Android
/// phone instead of being refused — a limit above what the smallest client can
/// hold protects nobody. 64 MB is ~9x the delta a budgeted exchange produces
/// (`crate::oplog_census::MAX_OPS_PER_EXCHANGE`) and a refusal a phone
/// survives. The gap resend is the one selection no budget may cut, so it is
/// also the one frame this can still refuse — loudly, which is the point, and
/// which is what it was doing silently by crashing before.
pub(crate) const MAX_FRAME_BODY: usize = 64 * 1024 * 1024;

/// Length of the big-endian length prefix [`read_frame`] leaves on the front
/// of the buffer it returns. Every consumer that reaches past the prefix
/// (rather than handing the whole `[prefix || body]` buffer to a `decode_*`
/// helper) must offset by this, never by a bare literal, so a change to the
/// framing shape has one name to chase.
pub(crate) const FRAME_PREFIX_LEN: usize = 4;

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
