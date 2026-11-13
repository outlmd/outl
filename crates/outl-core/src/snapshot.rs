//! Materialized-state snapshot.
//!
//! A snapshot is a projection of the workspace tree + block text at a
//! specific HLC cutoff. It is **not** source of truth — the op log is.
//! Its only job is to short-circuit the O(total history) replay on boot
//! (issue #109) by giving `Workspace::open_with_storage` a starting
//! point that costs O(current state) to load and O(delta) to bring
//! up-to-date via [`crate::storage::Storage::ops_since`].
//!
//! ## Layout
//!
//! [`SnapshotBody`] is [`postcard`]-serialized and written straight to
//! `<root>/.outl/snapshots/snap-<actor>.bin` by [`write_to_disk`], and
//! read back by [`read_from_disk`]. `Workspace` owns both the format and
//! the on-disk location — the snapshot is a local boot cache, never
//! routed through the storage backend (the op log). A single
//! `schema_version` lets us migrate later without guessing.
//!
//! The encoder is postcard as of schema 4 (bincode through schema 3) —
//! a dependency-policy decision, since this crate is published for
//! embedding. Rationale in `docs/storage.md` → Wire format.
//!
//! A format change here needs no converter: an unreadable snapshot falls
//! back to full op-log replay, so the worst case is one slower boot.
//!
//! ## Integrity
//!
//! `content_hash` is `sha256(body)` computed with the hash field zeroed.
//! `decode` recomputes and compares; a mismatch falls back to full
//! replay (see `Workspace::open_with_storage`). Snapshot is a cache —
//! never a source of truth — so a stale snapshot is silently ignored.
//!
//! ## Scope (Phase 1)
//!
//! This module handles **local boot** only. Sharing snapshots between
//! peers (Phase 2, via iroh) and compacting the op log (Phase 3, with
//! undo horizon) live elsewhere.
//!
//! [`Workspace`]: crate::workspace::Workspace

use crate::fractional::Fractional;
use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::property::PropValue;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use tracing::warn;

pub mod gc;
mod header;
pub mod select;

pub use header::{read_header_from_disk, SnapshotHeader};

/// Current snapshot wire format. Bump it on any breaking change to
/// [`SnapshotBody`] **or to the encoder**; `decode` rejects every other
/// version instead of guessing at backward compatibility.
pub const SCHEMA_VERSION: u32 = 4;

/// Errors that can occur while encoding or decoding a snapshot.
///
/// None of these are fatal for the caller — the boot path treats every
/// variant as "snapshot unusable, fall back to full replay" — but they
/// are surfaced so the caller can log a targeted warning instead of
/// silently eating the I/O cost.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// Snapshot was written by a different schema version, newer or
    /// older — see [`SnapshotBody::decode`] on why older is rejected too.
    #[error("snapshot schema version mismatch: expected {expected}, got {found}")]
    SchemaMismatch {
        /// The only schema version this binary reads.
        expected: u32,
        /// Schema version found in the snapshot buffer.
        found: u32,
    },
    /// `content_hash` didn't match the body — file is corrupt or was
    /// partially rewritten (e.g. `kill -9` mid-save).
    #[error("snapshot content hash mismatch — corrupt or stale")]
    HashMismatch,
    /// Failed to serialize the snapshot body via postcard.
    #[error("snapshot encode error: {0}")]
    Encode(String),
    /// Failed to deserialize the snapshot body via postcard. Also where
    /// a snapshot from an older encoder lands — a foreign format dies in
    /// the parser, before the schema check ever runs.
    #[error("snapshot decode error: {0}")]
    Decode(String),
    /// Filesystem error while writing or reading the snapshot file.
    /// Surfaced separately from encode/decode so a caller can
    /// distinguish "format broken" from "disk full".
    #[error("snapshot I/O error: {0}")]
    Io(String),
}

/// Typed view over a snapshot's `bytes`.
///
/// Built from a `Workspace` via [`SnapshotBody::from_parts`], serialized
/// with postcard, and persisted by [`write_to_disk`]. On boot,
/// `Workspace` calls [`SnapshotBody::decode`] on the bytes read by
/// [`read_from_disk`]; a [`SnapshotError`] triggers the full replay
/// fallback.
///
/// Not a `Storage` responsibility, and deliberately so: `Storage` owns
/// the op log, `Workspace` owns this cache under `<root>/.outl/snapshots`.
/// Two owners of the snapshot path is exactly what made snapshot boot
/// silently inert in production (#156).
///
/// All maps are `BTreeMap`s (not `HashMap`s) on purpose: the
/// `content_hash` is computed over the serialized body, and `BTreeMap`'s
/// iteration order is determined by key order — not by per-instance
/// hash-table state — so two bodies with the same content produce the
/// same hash. (Rust's `HashMap` randomizes layout per process and even
/// two `HashMap`s with identical contents can iterate in different
/// orders after different insertion histories.)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotBody {
    /// Bumped on any breaking change to this struct.
    pub schema_version: u32,
    /// Actor that produced this snapshot. Informational; any actor may
    /// load any actor's snapshot (the materialized tree is the union of
    /// every actor's ops up to the per-actor `cutoff`).
    pub actor: ActorId,
    /// Per-actor replay cutoff: the high-water-mark HLC of each actor
    /// whose ops the materialized state below already includes.
    ///
    /// On boot `Workspace` replays, for each actor `A`, only the ops with
    /// `hlc > cutoff[A]` — and **every** op of an actor absent from this
    /// map (that actor was entirely unseen when the snapshot was taken).
    ///
    /// This must be a per-actor vector clock, not a single global HLC: a
    /// single cutoff tracks only the high-water mark of the snapshotting
    /// actor, so a legitimately-low-HLC op from a *different* actor
    /// delivered after the snapshot (offline device, lagging clock) would
    /// fall below it and be silently dropped from the tree even though
    /// it's durably in storage (#156).
    pub cutoff: BTreeMap<ActorId, Hlc>,
    /// Tree nodes: `(node_id -> (parent, position))`. `ROOT` and
    /// `TRASH_ROOT` are implicit, never present as keys.
    pub nodes: BTreeMap<NodeId, (NodeId, Fractional)>,
    /// Property triples: `(node_id, key) -> value`.
    pub properties: BTreeMap<(NodeId, String), PropValue>,
    /// Nodes currently flagged collapsed. Absence = expanded (default).
    pub collapsed: BTreeSet<NodeId>,
    /// Snoozed reminders: `node -> resume-at`, Unix-epoch milliseconds
    /// (`Op::SnoozeRemind`). Absence = not snoozed (default).
    pub snoozed: BTreeMap<NodeId, u64>,
    /// Materialized text of every block that has text. Blocks without
    /// text are simply absent; `ContentStore` treats missing keys as
    /// empty.
    pub block_text: BTreeMap<NodeId, String>,
    /// `sha256` over the body with this field zeroed. Computed in
    /// [`SnapshotBody::from_parts`] and verified in [`SnapshotBody::decode`].
    pub content_hash: [u8; 32],
}

impl SnapshotBody {
    /// Assemble a snapshot from the materialized pieces of a workspace.
    ///
    /// The caller (always `Workspace` today) is responsible for picking
    /// the per-actor `cutoff` — the high-water-mark HLC of each actor the
    /// materialized state already reflects. We compute the `content_hash`
    /// here so what's returned is ready to [`encode`](Self::encode) and
    /// persist.
    ///
    /// Fails only if the body can't be encoded, which is also the point
    /// at which [`encode`](Self::encode) would fail — no snapshot reaches
    /// the disk either way, and the workspace keeps its op log.
    pub fn from_parts(
        actor: ActorId,
        cutoff: BTreeMap<ActorId, Hlc>,
        nodes: BTreeMap<NodeId, (NodeId, Fractional)>,
        properties: BTreeMap<(NodeId, String), PropValue>,
        collapsed: BTreeSet<NodeId>,
        snoozed: BTreeMap<NodeId, u64>,
        block_text: BTreeMap<NodeId, String>,
    ) -> Result<Self, SnapshotError> {
        let mut body = Self {
            schema_version: SCHEMA_VERSION,
            actor,
            cutoff,
            nodes,
            properties,
            collapsed,
            snoozed,
            block_text,
            content_hash: [0u8; 32],
        };
        body.content_hash = compute_hash(&body)?;
        Ok(body)
    }

    /// Serialize via postcard for persistence by [`write_to_disk`].
    pub fn encode(&self) -> Result<Vec<u8>, SnapshotError> {
        postcard::to_stdvec(self).map_err(|e| SnapshotError::Encode(e.to_string()))
    }

    /// Deserialize and validate a snapshot buffer.
    ///
    /// Returns [`SnapshotError::HashMismatch`] on tampering or
    /// truncation, and [`SnapshotError::SchemaMismatch`] if the buffer
    /// came from any version but the current one. Both are recoverable:
    /// the caller falls back to a full op-log replay.
    ///
    /// The version check is `!=`, not `<=`, because there is no readable
    /// older snapshot: encoder and `SCHEMA_VERSION` are bumped together.
    /// Accepting an older number would let a future schema that keeps
    /// postcard and merely appends a field half-parse into a partial tree.
    pub fn decode(bytes: &[u8]) -> Result<Self, SnapshotError> {
        // Hash the buffer we were handed, rather than re-deriving one
        // from the decoded body. Same number, and the body form cost a
        // full `clone()` plus a second `to_stdvec` on every decode — on
        // a 48 MB snapshot that is ~150 MB of cloned `String`s and
        // another 48 MB of re-encoded bytes, paid on the boot path, to
        // check a digest of bytes already in memory. See
        // `hash_of_encoded` for why the two agree.
        let recomputed = hash_of_encoded(bytes).ok_or(SnapshotError::HashMismatch)?;
        let body: Self =
            postcard::from_bytes(bytes).map_err(|e| SnapshotError::Decode(e.to_string()))?;
        if body.schema_version != SCHEMA_VERSION {
            return Err(SnapshotError::SchemaMismatch {
                expected: SCHEMA_VERSION,
                found: body.schema_version,
            });
        }
        if recomputed != body.content_hash {
            return Err(SnapshotError::HashMismatch);
        }
        Ok(body)
    }
}

/// Hash the body with the `content_hash` field zeroed. Used both at
/// build time (to stamp the hash) and at load time (to verify it).
///
/// The encode error propagates rather than degrading to a default. A
/// swallowed failure would hash the empty vector on **both** sides, and
/// two `sha256([])` values compare equal — the integrity check would go
/// on passing while checking nothing.
fn compute_hash(body: &SnapshotBody) -> Result<[u8; 32], SnapshotError> {
    let mut clone = body.clone();
    clone.content_hash = [0u8; 32];
    // Two bodies with identical content must hash identical regardless
    // of HashMap iteration order — serialize the canonical-form clone.
    // postcard's encoding is order-deterministic given a fixed in-memory
    // layout, so this is sufficient for the integrity check. Cross-actor
    // canonical comparison is not a goal of the hash.
    let bytes = postcard::to_stdvec(&clone).map_err(|e| SnapshotError::Encode(e.to_string()))?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let out = hasher.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(out.as_slice());
    Ok(arr)
}

/// [`compute_hash`]'s answer, read straight off an encoded buffer.
///
/// `content_hash` is the **last** field of [`SnapshotBody`] and a
/// `[u8; 32]`, which postcard writes as 32 raw bytes with no length
/// prefix. The format is sequential, so the encoding of a body is the
/// encoding of that same body with the hash zeroed, with the final 32
/// bytes swapped — identical prefix, different tail. Hashing
/// `bytes[..n-32] ++ [0; 32]` therefore produces exactly what
/// `compute_hash` produces, without materializing a second body.
///
/// That equality is a property of the current encoder and of the field
/// order above, not a guarantee postcard makes us. It is pinned by
/// `the_buffer_hash_equals_the_body_hash`, which fails loudly if either
/// moves — a bump of `SCHEMA_VERSION` that reorders fields, or a
/// postcard release that starts length-prefixing fixed arrays.
///
/// `None` for a buffer too short to carry the field at all: nothing that
/// small decodes into a body either, and the caller reads it as the
/// mismatch it is.
fn hash_of_encoded(bytes: &[u8]) -> Option<[u8; 32]> {
    let body = bytes.len().checked_sub(32)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes[..body]);
    hasher.update([0u8; 32]);
    Some(hasher.finalize().into())
}

/// Write `body` to `snapshots_dir/snap-<actor>.bin` atomically.
///
/// Encodes the body to postcard, writes to a **unique** sibling scratch
/// file (`scratch_path`), `fsync`s, and renames into place. A crash at
/// any point leaves either nothing (scratch never created) or an
/// abandoned scratch (rename didn't happen) — never a half-written
/// `snap-*.bin` that `load` could mistake for a valid snapshot.
///
/// This is a standalone function (not on `Storage`) on purpose: the
/// background-snapshot path in `Workspace::apply` calls it from a
/// worker thread that owns the body outright, with no borrow of the
/// storage backend. `Workspace::save_snapshot` delegates here for its
/// synchronous shutdown path, and `Workspace::spawn_background_snapshot`
/// for the in-band worker-thread path.
pub fn write_to_disk(snapshots_dir: &Path, body: &SnapshotBody) -> Result<(), SnapshotError> {
    let actor = body.actor;
    let bytes = body.encode()?;
    let final_path = snapshots_dir.join(format!("snap-{actor}.bin"));
    let tmp_path = scratch_path(&final_path);

    std::fs::create_dir_all(snapshots_dir)
        .map_err(|e| SnapshotError::Io(format!("create {}: {e}", snapshots_dir.display())))?;
    if let Err(e) = compose_and_publish(&tmp_path, &final_path, &bytes) {
        // The scratch name is ours alone, so nothing else will ever
        // reuse it — an in-process failure that left it behind would
        // leave it until `gc::stale_tmp`'s 24h sweep. Unlink it here, so
        // what reaches that sweep is only a process killed between the
        // `create` and the `rename`.
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    tracing::debug!(
        "snapshot written to {} ({} bytes)",
        final_path.display(),
        bytes.len()
    );
    Ok(())
}

/// Compose `bytes` in `tmp_path` and publish them at `final_path`.
///
/// Split out so [`write_to_disk`] has one place to unlink the scratch
/// file on any failure between the `create` and the `rename`.
fn compose_and_publish(
    tmp_path: &Path,
    final_path: &Path,
    bytes: &[u8],
) -> Result<(), SnapshotError> {
    let mut file = File::create(tmp_path)
        .map_err(|e| SnapshotError::Io(format!("create {}: {e}", tmp_path.display())))?;
    file.write_all(bytes)
        .map_err(|e| SnapshotError::Io(format!("write {}: {e}", tmp_path.display())))?;
    file.sync_all()
        .map_err(|e| SnapshotError::Io(format!("fsync {}: {e}", tmp_path.display())))?;
    drop(file);
    std::fs::rename(tmp_path, final_path).map_err(|e| {
        SnapshotError::Io(format!(
            "rename {} -> {}: {e}",
            tmp_path.display(),
            final_path.display()
        ))
    })
}

/// Scratch path for one in-flight write of `final_path`: the published
/// name plus `.tmp.<ulid>`.
///
/// **The ULID is the whole point.** Two writers for one actor are
/// routine — [`crate::workspace::Workspace`]'s threshold worker can be
/// spawned again while the previous one is still fsyncing a ~13 MB body,
/// and a reload builds a second `Workspace` for the same actor whose
/// `save_snapshot` runs alongside it. With one shared scratch name they
/// share an *inode*: the loser of the `rename` goes on writing through a
/// path that now points at the published snapshot, so a boot reads a
/// torn body, and the loser's own `rename` fails `ENOENT`.
///
/// That costs a slow boot and nothing else — the op log is the source of
/// truth and an undecodable snapshot falls back to a full replay — but
/// nothing reports it, so it is silent replay forever on the affected
/// device. A per-write name removes the sharing outright: each writer
/// composes in private and publishes with one atomic rename, so the
/// published slot only ever holds a whole body and the newest rename
/// wins (a stale-but-whole body is just a slightly larger boot delta).
///
/// Same fix, same reason, as `storage::sidecar`'s `tmp_path_for`, which
/// took the identical `rename …idx.tmp -> …idx: No such file or
/// directory` in production.
///
/// The name keeps the `snap-` prefix and the `.bin.tmp` marker so
/// [`gc::stale_tmp`] still recognises what a killed writer abandoned —
/// and it has more to recognise now, because a unique name is never
/// recycled by the next write the way the shared one was.
pub(crate) fn scratch_path(final_path: &Path) -> PathBuf {
    let mut name = final_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(&format!(".tmp.{}", ulid::Ulid::generate()));
    final_path.with_file_name(name)
}

/// Read and decode `snapshots_dir/snap-<actor>.bin`, if present.
///
/// Returns `Ok(None)` when no snapshot exists yet — first boot, or the
/// file was never written. A decode / hash / schema failure is surfaced
/// as `Err` so the caller can log a targeted warning; the boot path
/// treats *every* outcome other than `Ok(Some(_))` as "snapshot
/// unusable, fall back to full replay". Only the exact `snap-<actor>.bin`
/// is read, so a leftover `.tmp` from a crashed [`write_to_disk`] is
/// never mistaken for a valid snapshot.
///
/// Standalone (not on `Storage`) for the same reason as [`write_to_disk`]:
/// the snapshot is a local boot cache owned by `Workspace`, not part of
/// the source-of-truth op log, so it never routes through the storage
/// backend.
pub fn read_from_disk(
    snapshots_dir: &Path,
    actor: ActorId,
) -> Result<Option<SnapshotBody>, SnapshotError> {
    let path = snapshots_dir.join(format!("snap-{actor}.bin"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SnapshotError::Io(format!("read {}: {e}", path.display()))),
    };
    SnapshotBody::decode(&bytes).map(Some)
}

/// Read the best snapshot available in `snapshots_dir`, preferring this
/// device's own.
///
/// **Phase 2 — peer snapshot adoption.** A freshly-paired device receives
/// a huge op log (200k+ ops / tens of MB) that is slow to transfer AND slow
/// to replay. A peer's snapshot is ~5× smaller (settled state, no `Edit`
/// history) and lets this device skip the full replay. So when this
/// device has no snapshot of its own yet, adopt a peer's `snap-*.bin`.
///
/// **No local input is ever lost by adopting a peer snapshot.** The
/// snapshot only pre-loads the materialized tree; `boot_from_snapshot`
/// then replays `ops_since_per_actor_combined(body.cutoff)` — every op of
/// every actor above the snapshot's per-actor cutoff, INCLUDING this
/// device's own ops that the peer hadn't seen when it wrote the snapshot.
/// Idempotency dedups the ops the snapshot already covers. A corrupt /
/// incompatible peer snapshot is skipped (never fatal — full replay).
///
/// Selection is deterministic: own snapshot first; otherwise the peer
/// snapshot whose per-actor cutoff reaches the highest HLC (the most
/// up-to-date, smallest delta), with the actor id as a tie-break.
///
/// The directory scan is [`gc::survey`], so the ranking the selector
/// applies and the verdict the GC records come from one decode pass and
/// one comparison. It **reads only**: opening a workspace is something
/// `outl doctor` does in its documented read-only mode, so the bulk
/// sweep belongs to [`gc::sweep`] on the writer's worker thread, not
/// here. The single deletion on this path is
/// `gc::drop_own_if_unusable` — one file, read end to end and refused
/// by the decoder, in the slot this selector consults on every boot.
pub fn read_best_from_disk(
    snapshots_dir: &Path,
    prefer_actor: ActorId,
) -> Result<Option<SnapshotBody>, SnapshotError> {
    // Read this device's own first — but only to catch an unusable one
    // and drop it. It no longer short-circuits the choice: `survey` ranks
    // it against the peers like any other candidate. Returning it
    // unconditionally is what stranded a freshly paired device, whose own
    // first-launch snapshot names a single actor and shadowed a peer's
    // covering every actor in the workspace.
    // Rank on headers, then decode only what won. `gc::adopt_order`
    // reads a few hundred bytes per candidate instead of the whole body,
    // which is the difference between 124 MB and 16 KB on a real
    // four-snapshot directory — paid at launch, every launch.
    let own_name = format!("snap-{prefer_actor}.bin");
    let mut last_err: Option<SnapshotError> = None;

    for path in select::adopt_order(snapshots_dir)? {
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                warn!("snapshot {} unreadable: {e}", path.display());
                last_err = Some(SnapshotError::Io(format!("read {}: {e}", path.display())));
                continue;
            }
        };
        match SnapshotBody::decode(&bytes) {
            Ok(body) => return Ok(Some(body)),
            Err(e) => {
                // A cache entry that can never be read again. Drop it
                // when it is ours — nobody else will, and the selector
                // would keep paying for it. A peer's stays: the GC
                // sweep judges those, and it runs where it can afford
                // the full decode.
                if path.file_name().and_then(|n| n.to_str()) == Some(own_name.as_str()) {
                    gc::drop_own_if_unusable(snapshots_dir, prefer_actor, &e);
                }
                // **Skip the entry, never the cache.** Returning here is
                // what let one unreadable own snapshot send the boot to
                // a full replay of 722,943 ops with a healthy peer body
                // in the same directory.
                warn!("snapshot {} unusable, trying the next: {e}", path.display());
                last_err = Some(e);
            }
        }
    }

    // Nothing adoptable. An empty directory and a directory of rubble
    // both end in a full replay, but only the second is worth reporting.
    match last_err {
        Some(e) => Err(e),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests;
