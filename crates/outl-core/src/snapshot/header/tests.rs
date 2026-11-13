//! What a header read promises: the same three fields the body would
//! have given, and an honest answer when the file is absent or broken.

use super::*;
use crate::fractional::Fractional;
use crate::id::NodeId;
use crate::snapshot::SnapshotBody;
use std::collections::BTreeSet;
use tempfile::TempDir;

/// A body with real content in every map, so a test about the encoding
/// is not a test about the empty case.
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
        nodes.insert(n, (NodeId::new(), Fractional::first()));
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

/// The header mirrors the body's first three fields. A field
/// inserted above `cutoff` would change what a prefix read returns
/// while every existing test kept passing, and the boot selector
/// would start ranking on whatever landed there.
#[test]
fn the_header_matches_the_body() {
    let tmp = TempDir::new().unwrap();
    let body = full_body();
    let path = tmp.path().join("snap.bin");
    std::fs::write(&path, body.encode().expect("encode")).unwrap();

    let header = read_header_from_disk(&path)
        .expect("header reads")
        .expect("file exists");
    assert_eq!(header.schema_version, body.schema_version);
    assert_eq!(header.actor, body.actor);
    assert_eq!(header.cutoff, body.cutoff);
}

/// A cutoff larger than the probe window still reads — the reader
/// grows to the file rather than reporting the snapshot unreadable
/// and costing a full replay.
#[test]
fn a_header_larger_than_the_probe_still_reads() {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let mut cutoff = BTreeMap::new();
    // ~58 bytes per entry encoded; well past HEADER_PROBE_BYTES.
    for _ in 0..2_000 {
        let a = ActorId::new();
        cutoff.insert(a, Hlc::new(1, 0, a));
    }
    let body = SnapshotBody::from_parts(
        actor,
        cutoff.clone(),
        BTreeMap::new(),
        BTreeMap::new(),
        BTreeSet::new(),
        BTreeMap::new(),
        BTreeMap::new(),
    )
    .expect("encodes");
    let path = tmp.path().join("snap.bin");
    let bytes = body.encode().expect("encode");
    assert!(
        bytes.len() > HEADER_PROBE_BYTES,
        "the probe must come up short"
    );
    std::fs::write(&path, bytes).unwrap();

    let header = read_header_from_disk(&path)
        .expect("header reads")
        .expect("file exists");
    assert_eq!(header.cutoff, cutoff);
}

/// A missing file is not an error; an unreadable one is. A caller
/// that cannot tell those apart either treats a fresh device as
/// broken or treats a corrupt snapshot as absent.
#[test]
fn an_absent_header_is_none_and_a_bad_one_is_an_error() {
    let tmp = TempDir::new().unwrap();
    assert!(read_header_from_disk(&tmp.path().join("nope.bin"))
        .expect("absent is not an error")
        .is_none());

    let bad = tmp.path().join("bad.bin");
    std::fs::write(&bad, b"not a snapshot at all").unwrap();
    assert!(read_header_from_disk(&bad).is_err());
}
