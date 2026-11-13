//! Encoding, integrity and the adopt-slot selector.

use super::*;
use crate::id::NodeId;
use tempfile::TempDir;

/// See `crates/outl-core/fixtures/README.md`.
const LEGACY_BINCODE_SCHEMA_3: &[u8] = include_bytes!("../../fixtures/legacy-snapshot-schema3.bin");

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

/// A body with real content in every map, so a test about the
/// encoding is not a test about the empty case.
fn full_body() -> SnapshotBody {
    let actor = ActorId::new();
    let mut cutoff = BTreeMap::new();
    for _ in 0..24 {
        let a = ActorId::new();
        cutoff.insert(a, Hlc::new(7, 3, a));
    }
    let mut nodes = BTreeMap::new();
    let mut block_text = BTreeMap::new();
    for i in 0..64 {
        let n = NodeId::new();
        nodes.insert(n, (NodeId::new(), crate::fractional::Fractional::first()));
        block_text.insert(n, format!("bloco {i}"));
    }
    SnapshotBody::from_parts(
        actor,
        cutoff,
        nodes,
        BTreeMap::new(),
        BTreeSet::new(),
        BTreeMap::new(),
        block_text,
    )
    .expect("body encodes")
}

/// `decode` verifies the digest against the buffer instead of
/// re-encoding a zeroed clone of the body. The two are the same
/// number only because `content_hash` is the last field and a
/// fixed-size `[u8; 32]`, so the encodings share every byte before
/// it.
///
/// That is a property of this field order and of postcard's array
/// encoding, neither of which is ours to promise. If a schema bump
/// moves the field, or postcard starts length-prefixing fixed
/// arrays, this is what says so — rather than every snapshot
/// quietly failing to verify and every boot falling back to a full
/// replay, which is invisible except as "the app got slow".
#[test]
fn the_buffer_hash_equals_the_body_hash() {
    for body in [empty_body(), full_body()] {
        let bytes = body.encode().expect("encode");
        assert_eq!(
            bytes.len().checked_sub(32).map(|n| &bytes[n..]),
            Some(&body.content_hash[..]),
            "content_hash must be the final 32 bytes of the encoding"
        );
        assert_eq!(
            hash_of_encoded(&bytes),
            Some(compute_hash(&body).expect("hash")),
            "hashing the buffer must equal hashing the zeroed body"
        );
    }
}

/// Hashing the buffer must not read out of bounds on a file too
/// short to carry the field, and the caller must read that as the
/// mismatch it is.
#[test]
fn a_buffer_too_short_to_hold_the_hash_is_a_mismatch() {
    assert_eq!(hash_of_encoded(&[0u8; 31]), None);
    let err = SnapshotBody::decode(&[0u8; 4]).unwrap_err();
    assert!(matches!(err, SnapshotError::HashMismatch), "got {err:?}");
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
    let err = SnapshotBody::decode(LEGACY_BINCODE_SCHEMA_3).expect_err("legacy must not decode");
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
