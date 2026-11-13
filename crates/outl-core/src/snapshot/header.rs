//! Reading a snapshot's leading fields without its body.
//!
//! Choosing which snapshot to adopt never needed the snapshots. The
//! boot selector ranks on `cutoff` — how many actors a body already
//! covers, and how recent it is — which is the third field of
//! [`super::SnapshotBody`](super::SnapshotBody), a few hundred bytes ahead of
//! the `block_text` map that is almost the entire file.
//!
//! Decoding every candidate to pick one cost 124 MB of reads and
//! allocation on a real four-snapshot directory, on the path a user
//! waits on at launch. This module is the cheap half of that question;
//! [`super::SnapshotBody::decode`] still runs, once, on whatever won.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::Path;

use serde::Deserialize;

use super::SnapshotError;
use crate::hlc::Hlc;
use crate::id::ActorId;

/// The leading fields of a [`super::SnapshotBody`], decodable without the body.
///
/// Everything the boot selector ranks on — how many actors the cutoff
/// covers, and its newest HLC — lives in the first three fields, ahead
/// of `nodes`, `properties` and the `block_text` map that is almost all
/// of a snapshot's bytes. postcard decodes in field order, so those
/// three come back from a prefix of the file: about 1.4 KB for a
/// 24-actor cutoff, against 48 MB for the body behind it.
///
/// **This mirrors `SnapshotBody`'s first three fields and their order.**
/// A field inserted above `cutoff`, or reordered among these three,
/// silently changes what this reads. `the_header_matches_the_body` is
/// what catches that.
///
/// It carries no integrity check, deliberately: a header is only ever
/// used to *rank* candidates, and the winner is decoded in full through
/// [`super::SnapshotBody::decode`], which verifies. Ranking on an unverified
/// header can only pick a worse starting point, and a snapshot that
/// fails to decode falls back to a full op-log replay — the cost is a
/// slower boot, never a wrong tree.
#[derive(Clone, Debug, Deserialize)]
pub struct SnapshotHeader {
    /// Mirrors [`super::SnapshotBody::schema_version`].
    pub schema_version: u32,
    /// Mirrors [`super::SnapshotBody::actor`].
    pub actor: ActorId,
    /// Mirrors [`super::SnapshotBody::cutoff`].
    pub cutoff: BTreeMap<ActorId, Hlc>,
}

/// How much of a snapshot file to read looking for its header. A cutoff
/// of 24 actors encodes in ~1.4 KB; this leaves room for an order of
/// magnitude more before the fallback below has to grow it.
const HEADER_PROBE_BYTES: usize = 32 * 1024;

/// Read just the [`SnapshotHeader`] of `path`.
///
/// Reads a bounded prefix and grows to the whole file only if the cutoff
/// turned out not to fit in it — a workspace with thousands of actors,
/// where reading the file is what we were doing anyway.
///
/// `Ok(None)` for a file that is not there. A file that *is* there and
/// whose prefix will not decode is an `Err`, so a caller can tell "no
/// snapshot" from "a snapshot I cannot read".
pub fn read_header_from_disk(path: &Path) -> Result<Option<SnapshotHeader>, SnapshotError> {
    use std::io::Read;

    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SnapshotError::Io(format!("open {}: {e}", path.display()))),
    };
    let io = |e: std::io::Error| SnapshotError::Io(format!("read {}: {e}", path.display()));
    let mut buf = Vec::with_capacity(HEADER_PROBE_BYTES);
    (&mut file)
        .take(HEADER_PROBE_BYTES as u64)
        .read_to_end(&mut buf)
        .map_err(io)?;

    match postcard::take_from_bytes::<SnapshotHeader>(&buf) {
        Ok((header, _)) => Ok(Some(header)),
        // Short prefix, not a bad file: the cutoff ran past the probe, so
        // a full-length read means there is more to get.
        Err(_) if buf.len() == HEADER_PROBE_BYTES => {
            file.read_to_end(&mut buf).map_err(io)?;
            postcard::take_from_bytes::<SnapshotHeader>(&buf)
                .map(|(header, _)| Some(header))
                .map_err(|e| SnapshotError::Decode(e.to_string()))
        }
        Err(e) => Err(SnapshotError::Decode(e.to_string())),
    }
}

#[cfg(test)]
mod tests;
