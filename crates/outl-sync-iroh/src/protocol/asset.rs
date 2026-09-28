//! The asset manifest carried on [`ASSET_ALPN`](super::ASSET_ALPN).
//!
//! Unlike a snapshot (one blob), assets are N files, so the responder first
//! ships the list of `<hash>.<ext>` basenames it holds and the initiator pulls
//! only the ones it lacks. The names ARE the content hashes, so a name match
//! means the bytes match and no per-file comparison is needed.
//!
//! The bytes themselves are plain [`super::frame`] blobs, which is why only the
//! manifest needs a codec of its own.

use anyhow::Result;

/// Serialize an asset manifest (a peer's `assets/` basenames) into a
/// length-prefixed, newline-separated frame.
///
/// Same framing as [`encode_ops_blob`] but over plain filename strings: the
/// responder ships the list of `<hash>.<ext>` names it holds so the initiator
/// can diff against its own `assets/` and pull only what it lacks (the names
/// ARE the content hashes, so a name match means the bytes match). An empty
/// slice yields a valid zero-length body, so "I have no assets" is still an
/// unambiguous frame.
///
/// [`encode_ops_blob`]: super::encode_ops_blob
pub(crate) fn encode_asset_manifest(names: &[String]) -> Result<Vec<u8>> {
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
pub(crate) fn decode_asset_manifest(buf: &[u8]) -> Result<Vec<String>> {
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
