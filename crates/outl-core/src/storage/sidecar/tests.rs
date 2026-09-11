//! Naming and write-path behaviour of the shared sidecar layer.

use super::*;
use crate::id::ActorId;
use std::io::Write as _;
use tempfile::TempDir;

/// Every sidecar name is dot-prefixed, in every layout. This is what
/// keeps a purely local boot cache off the file-sync surface — `ops/`
/// itself is deliberately not a dotfile, so an undotted sidecar in there
/// rides iCloud / Syncthing to every other device.
#[test]
fn every_sidecar_name_is_dot_prefixed() {
    let a = ActorId::new();
    for scope in [PageScope::Global, PageScope::PerPage("home".into())] {
        for kind in SidecarKind::ALL {
            let name = file_name(a, &scope, kind);
            assert!(name.starts_with('.'), "{name} must be dot-prefixed");
        }
    }
}

/// The two kinds never resolve to the same file, or one would silently
/// overwrite the other's payload.
#[test]
fn the_two_kinds_never_collide() {
    let a = ActorId::new();
    assert_ne!(
        file_name(a, &PageScope::Global, SidecarKind::Offset),
        file_name(a, &PageScope::Global, SidecarKind::Node)
    );
}

/// Under `PerPage`, one actor owns many `.jsonl` files in one directory.
/// An actor-derived sidecar name makes every page of that actor share a
/// single offset index — offsets into `a.jsonl` and `b.jsonl` interleaved
/// in one file, attributed to whichever shard loads it next. What kept
/// that from being a wrong read was the boot freshness check rejecting
/// the mixture and rebuilding: a guard, not a design.
#[test]
fn per_page_shards_of_one_actor_get_distinct_sidecars() {
    let a = ActorId::new();
    let home = PageScope::PerPage("home".into());
    let work = PageScope::PerPage("work".into());
    for kind in SidecarKind::ALL {
        assert_ne!(
            file_name(a, &home, kind),
            file_name(a, &work, kind),
            "two pages of one actor must not share a sidecar"
        );
    }
}

/// The complete set for an actor, asked for rather than listed from
/// memory. Compaction renumbers every byte offset, so a sidecar it does
/// not know to delete is a wrong offset on the next cold read.
#[test]
fn paths_for_covers_every_kind() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    let paths = paths_for(tmp.path(), a, &PageScope::Global);
    assert_eq!(paths.len(), SidecarKind::ALL.len());
    for kind in SidecarKind::ALL {
        assert!(paths.contains(&path_for(tmp.path(), a, &PageScope::Global, kind)));
    }
}

/// `write_atomic` used to remove its scratch file only when the *rename*
/// failed, so every other failure path leaked one permanently. A real
/// workspace accumulated 84 MB of them.
#[test]
fn a_failed_write_leaves_no_temp_behind() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(".ops-x.idx");

    let err = write_atomic(&target, |_, _| {
        Err(StorageError::Backend("body failed".into()))
    });
    assert!(err.is_err());

    let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        leftovers.is_empty(),
        "a failed write must leave nothing behind, found {leftovers:?}"
    );
}

/// The same guarantee when the body panics — a `?` return is not the only
/// way out of that closure.
#[test]
fn a_panicking_write_leaves_no_temp_behind() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(".ops-x.idx");

    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = write_atomic(&target, |_, _| panic!("body exploded"));
    }));
    assert!(caught.is_err());

    let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        leftovers.is_empty(),
        "a panicking write must leave nothing behind, found {leftovers:?}"
    );
}

/// The scratch name appends to the published name rather than replacing
/// its extension, so `.nodes` survives — and it keeps the leading dot, so
/// even a temp a killed process abandons stays off the sync surface.
#[test]
fn the_scratch_name_extends_the_published_one() {
    let a = ActorId::new();
    let published = format!(".ops-{a}.nodes.idx");
    let tmp = tmp_path_for(std::path::Path::new(&published));
    let name = tmp.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with(&format!("{published}.tmp.")), "{name}");
}

/// A successful write publishes the content and keeps no scratch.
#[test]
fn a_successful_write_publishes_and_cleans_up() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(".ops-x.idx");
    write_atomic(&target, |f, _| {
        writeln!(f, "hello").map_err(|e| StorageError::Backend(e.to_string()))
    })
    .unwrap();

    assert_eq!(std::fs::read_to_string(&target).unwrap(), "hello\n");
    let names: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec![".ops-x.idx".to_string()]);
}

/// A counting sink: how many `write` calls actually reach the layer below.
#[derive(Default)]
struct CountingSink {
    writes: usize,
    bytes: usize,
}

impl std::io::Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.writes += 1;
        self.bytes += buf.len();
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The bulk sidecar write emits one line per index entry, and a real
/// workspace rebuilds 435,622 of them across 40 files on a cold boot.
/// Unbuffered that is one `write(2)` each: **71.5 s of a 72.0 s cold
/// boot**, ~95% of it blocked in the kernel. Buffered, the same bytes
/// take ~0.5 s.
///
/// So the buffer is not a tuning detail — it is the difference between a
/// boot and a hang, and `outl compact --apply` deletes these sidecars by
/// design, so every compaction arms the next cold boot.
#[test]
fn the_bulk_writer_batches_lines_into_few_syscalls() {
    const LINES: usize = 50_000;
    let mut sink = CountingSink::default();
    {
        let mut out = buffered(&mut sink);
        for i in 0..LINES {
            writeln!(out, "{{\"ts\":{i},\"offset\":{}}}", i * 64).expect("write");
        }
        out.flush().expect("flush");
    }
    assert!(
        sink.writes < LINES / 100,
        "the bulk writer must batch: {} write(2) calls for {LINES} lines ({} bytes) — \
         unbuffered this is one syscall per index entry, which is what made a cold boot \
         on a 217k-op workspace take 72 seconds",
        sink.writes,
        sink.bytes,
    );
}

/// Buffering must not truncate the tail. `BufWriter`'s `Drop` flushes and
/// **discards** the error, so a write path whose whole job is durability
/// has to flush explicitly and check — and the bytes have to survive.
#[test]
fn a_bulk_save_keeps_every_entry_across_buffer_boundaries() {
    use crate::hlc::HlcGenerator;
    use crate::storage::index::OffsetIndex;

    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join(".ops-big.idx");
    let g = HlcGenerator::new(ActorId::new());
    let mut index = OffsetIndex::new();
    // Far more than one buffer's worth of lines.
    for offset in 0..20_000u64 {
        index.insert(g.next(), offset * 96);
    }
    index.save(&path).expect("save");

    let loaded = OffsetIndex::load(&path).expect("load").expect("present");
    assert_eq!(
        loaded.len(),
        index.len(),
        "a buffered write must not lose the last partial buffer"
    );
    assert_eq!(loaded.max_offset(), index.max_offset());
}
