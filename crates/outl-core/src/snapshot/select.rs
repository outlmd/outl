//! Which snapshot a boot adopts.
//!
//! Split from [`gc`](super::gc) because the two questions are not the
//! same one: this module decides what to *read*, the collector decides
//! what to *delete*, and only the second needs to have proven a body
//! decodable. They share `selection_key` so they cannot develop
//! separate opinions about which file matters — that shared owner is
//! what lets the boot side be a much cheaper read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::SnapshotError;
use crate::hlc::Hlc;
use crate::id::ActorId;

/// The boot selector's ranking key for a body: highest cutoff HLC, then
/// filename. A body with an empty cutoff has no key — the selector skips
/// it outright, since adopting it would save no replay.
/// `(actors covered, newest cutoff, filename)` — see `selection_key`.
pub(super) type SelectionKey = (usize, Hlc, String);

/// Rank a snapshot for the adopt slot: **how many actors its cutoff
/// covers**, then how recent it is, then the filename so two boots agree.
///
/// Coverage leads, and the order is load-bearing. An actor missing from
/// the cutoff reads as "seen none of your ops", so `ops_since_per_actor`
/// yields that actor's entire history into the delta. A body naming one
/// actor therefore costs more to adopt than one naming twenty, however
/// recent its single mark is — and "recent" is exactly what a fresh
/// device's own snapshot looks like, since the only ops it has ever
/// applied are today's.
pub(super) fn selection_key(cutoff: &BTreeMap<ActorId, Hlc>, name: &str) -> Option<SelectionKey> {
    cutoff
        .values()
        .copied()
        .max()
        .map(|hlc| (cutoff.len(), hlc, name.to_string()))
}

/// Every `snap-*.bin` in `dir` that is worth adopting, best first,
/// judged from each file's **header** alone.
///
/// This is the boot selector's half of [`super::gc::survey`], and it exists
/// because ranking never needed the bodies. `selection_key` reads
/// `cutoff`, which is the third field of a snapshot and a few hundred
/// bytes; the `block_text` map behind it is essentially the whole file.
/// Decoding every candidate to choose one cost 124 MB of reads and
/// allocation on a real four-snapshot directory, on the path the user
/// waits on at launch, to pick 48 MB of it.
///
/// Both halves still rank through `selection_key`, so the selector and
/// the GC cannot develop separate opinions about which file matters —
/// that shared owner is why this can be a cheaper *read* rather than a
/// second policy. They do differ in what they rank *over*, and
/// deliberately: the GC judges only bodies that decode, because its
/// output is a delete. This one is followed by a decode of whatever it
/// picked, so a file that lies in its header costs one retry.
///
/// Candidates come back in two tiers. First the files whose header read
/// and named a cutoff, best key first. Then, name-sorted, the files
/// whose header would **not** read — ranked last, not dropped. A header
/// that fails to parse says the file is probably rubble, and rubble is
/// exactly what the caller has to decode in order to be allowed to
/// delete it: `drop_own_if_unusable` needs a decode error, and a device
/// whose own snapshot is garbage pays for it on every boot until
/// something proves it dead. Filtering those out here is the one way
/// this optimization could have made a boot permanently slower.
///
/// A header that reads and carries an **empty** cutoff is skipped
/// outright, which is not the same case: that is a definite answer, and
/// the answer is that adopting the body would save no replay.
pub fn adopt_order(dir: &Path) -> Result<Vec<PathBuf>, SnapshotError> {
    let read = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(SnapshotError::Io(format!(
                "read dir {}: {e}",
                dir.display()
            )))
        }
    };

    let mut ranked: Vec<(SelectionKey, PathBuf)> = Vec::new();
    let mut unreadable: Vec<PathBuf> = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        if !name.starts_with("snap-") || !name.ends_with(".bin") {
            continue;
        }
        match crate::snapshot::read_header_from_disk(&path) {
            Ok(Some(header)) => {
                if let Some(key) = selection_key(&header.cutoff, &name) {
                    ranked.push((key, path));
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!("snapshot header unreadable for {}: {e}", path.display());
                unreadable.push(path);
            }
        }
    }

    ranked.sort_by(|a, b| b.0.cmp(&a.0));
    unreadable.sort();
    Ok(ranked
        .into_iter()
        .map(|(_, path)| path)
        .chain(unreadable)
        .collect())
}
