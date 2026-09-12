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

pub mod gc;

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
        let body: Self =
            postcard::from_bytes(bytes).map_err(|e| SnapshotError::Decode(e.to_string()))?;
        if body.schema_version != SCHEMA_VERSION {
            return Err(SnapshotError::SchemaMismatch {
                expected: SCHEMA_VERSION,
                found: body.schema_version,
            });
        }
        let recomputed = compute_hash(&body)?;
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

/// Write `body` to `snapshots_dir/snap-<actor>.bin` atomically.
///
/// Encodes the body to postcard, writes to a **unique** sibling scratch
/// file ([`scratch_path`]), `fsync`s, and renames into place. A crash at
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
    name.push_str(&format!(".tmp.{}", ulid::Ulid::new()));
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
    // This device's own snapshot has the tightest cutoff for its local
    // ops, so prefer it and skip the directory scan entirely.
    match read_from_disk(snapshots_dir, prefer_actor) {
        Ok(Some(body)) => return Ok(Some(body)),
        Ok(None) => {}
        Err(e) => {
            // Read end to end and refused by the decoder: a cache entry
            // that can never be read again, sitting in the one slot this
            // selector consults on *every* boot. Drop it, then report the
            // failure exactly as before — this boot still full-replays,
            // the next one does not have to.
            gc::drop_own_if_unusable(snapshots_dir, prefer_actor, &e);
            return Err(e);
        }
    }

    // Adopt the most up-to-date peer snapshot; `survey` ranks on (max
    // cutoff HLC, filename) so two boots pick the same one, and a
    // snapshot that fails to decode is skipped, never fatal. No prune
    // here: see the doc comment.
    Ok(gc::survey(snapshots_dir, prefer_actor)?.into_best())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::NodeId;
    use tempfile::TempDir;

    /// See `crates/outl-core/fixtures/README.md`.
    const LEGACY_BINCODE_SCHEMA_3: &[u8] =
        include_bytes!("../fixtures/legacy-snapshot-schema3.bin");

    fn empty_body() -> SnapshotBody {
        SnapshotBody::from_parts(
            ActorId::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeSet::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .expect("empty body encodes")
    }

    #[test]
    fn roundtrips_empty_body() {
        let body = empty_body();
        let bytes = body.encode().expect("encode");
        let decoded = SnapshotBody::decode(&bytes).expect("decode");
        assert_eq!(decoded.schema_version, SCHEMA_VERSION);
        assert_eq!(decoded.nodes, body.nodes);
        assert_eq!(decoded.content_hash, body.content_hash);
    }

    #[test]
    fn detects_tampered_hash() {
        let mut bytes = empty_body().encode().expect("encode");
        // Flip one byte near the end (hash field is last in serialization
        // order). Mismatch between stored and recomputed hash must trip
        // HashMismatch, not silently accept.
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        let err = SnapshotBody::decode(&bytes).unwrap_err();
        assert!(matches!(err, SnapshotError::HashMismatch), "got {err:?}");
    }

    /// A snapshot written by a pre-#207 build (schema 3, bincode) must be
    /// rejected, never half-parsed. Either outcome the decoder can reach —
    /// a parse failure or a `content_hash` mismatch — routes the caller to
    /// the full op-log replay it already had for corrupt snapshots, so the
    /// upgrade costs one slower boot and nothing else.
    #[test]
    fn rejects_legacy_bincode_snapshot() {
        let err =
            SnapshotBody::decode(LEGACY_BINCODE_SCHEMA_3).expect_err("legacy must not decode");
        assert!(
            matches!(err, SnapshotError::Decode(_) | SnapshotError::HashMismatch),
            "got {err:?}"
        );
    }

    #[test]
    fn rejects_future_schema_version() {
        let mut body = empty_body();
        body.schema_version = SCHEMA_VERSION + 1;
        // Re-stamp the hash so the only thing wrong is the schema.
        body.content_hash = compute_hash(&body).expect("test body encodes");
        let bytes = body.encode().expect("encode");
        let err = SnapshotBody::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, SnapshotError::SchemaMismatch { found, .. } if found == SCHEMA_VERSION + 1),
            "got {err:?}"
        );
    }

    /// The `<=` this check used to be would have accepted this body — a
    /// well-formed, correctly-hashed buffer claiming an *older* schema.
    /// Harmless today (a real schema-3 file is bincode and dies in the
    /// parser), but the moment a schema bump keeps postcard and merely
    /// appends a field, that is a partial tree read as if it were whole.
    #[test]
    fn rejects_past_schema_version() {
        let mut body = empty_body();
        body.schema_version = SCHEMA_VERSION - 1;
        body.content_hash = compute_hash(&body).expect("test body encodes");
        let bytes = body.encode().expect("encode");
        let err = SnapshotBody::decode(&bytes).unwrap_err();
        assert!(
            matches!(err, SnapshotError::SchemaMismatch { found, .. } if found == SCHEMA_VERSION - 1),
            "got {err:?}"
        );
    }

    /// A multi-MB body, so a writer's `create` → `write` → `fsync`
    /// window is wide enough for a second writer to land inside it. A
    /// real snapshot of the maintainer's workspace is ~13 MB; this is
    /// the same shape, two orders of magnitude smaller.
    fn big_body(actor: ActorId, tag: usize, round: usize) -> SnapshotBody {
        let mut cutoff = BTreeMap::new();
        cutoff.insert(actor, Hlc::new(1_000 + tag as u64, round as u32, actor));
        let mut text = BTreeMap::new();
        for i in 0..3_000 {
            text.insert(
                NodeId::new(),
                format!("{tag}-{round}-{i}-{}", "x".repeat(1_000)),
            );
        }
        SnapshotBody::from_parts(
            actor,
            cutoff,
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeSet::new(),
            BTreeMap::new(),
            text,
        )
        .expect("test body encodes")
    }

    /// Every writer for one actor used to compose in the same
    /// `snap-<actor>.bin.tmp`. Two of them overlap and the loser is
    /// still writing into the inode the winner already renamed into
    /// place, so the *published* snapshot is torn — and the loser's own
    /// `rename` finds nothing left to publish.
    ///
    /// The cost is a slow boot, never lost notes: the op log is the
    /// source of truth and a snapshot that fails to decode falls back to
    /// a full replay. That is exactly why it has to be pinned — nothing
    /// else in the system complains.
    #[test]
    fn concurrent_writers_for_one_actor_never_publish_a_torn_snapshot() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join(".outl").join("snapshots");
        let actor = ActorId::new();
        let published = dir.join(format!("snap-{actor}.bin"));

        let handles: Vec<_> = (0..4)
            .map(|tag| {
                let dir = dir.clone();
                let published = published.clone();
                std::thread::spawn(move || {
                    for round in 0..4 {
                        write_to_disk(&dir, &big_body(actor, tag, round))
                            .expect("a concurrent publish must not fail");
                        let bytes = std::fs::read(&published).expect("published snapshot readable");
                        SnapshotBody::decode(&bytes)
                            .expect("a published snapshot is always a whole body");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("writer thread");
        }
    }
}
