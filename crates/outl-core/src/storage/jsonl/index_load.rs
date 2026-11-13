//! Loading every actor's persisted index sidecars at boot.
//!
//! Boot does not read the op log; it reads the `.ops-<actor>.idx` /
//! `.ops-<actor>.nodes.idx` pair beside each one (RFC #137 Front A).
//! That is the cheap path, and on a real workspace it is still the most
//! expensive phase of a launch: 23 actors, **169 MB** of sidecar
//! (72 MB of offset index, 97 MB of node index), 450 ms of the 977 ms
//! before today's journal could paint. The app's premise is that its
//! text input is live when it opens, so this is the phase that premise
//! is spent on.
//!
//! One actor's pair says nothing about another's, so the sequential
//! loop that read them bought only its own ordering, and the ordering
//! does not matter — the results land in a map keyed by actor.
//!
//! **The ceiling is one file, not the thread count.** On that same
//! workspace a single actor holds 114.7 MB of the 169 MB, so no
//! distribution of whole files across lanes beats ~1.45x, and that is
//! what this measures (450 ms → 310 ms). Going below it means reading
//! less, not reading wider: the node index is 97 MB of the total and a
//! snapshot boot needs it for a handful of nodes.
//!
//! `std::thread::scope` rather than a thread-pool crate, because this
//! crate is published for embedding and a direct dependency here is
//! public surface (`CLAUDE.md` → "This crate's dependency graph is
//! public surface").

use std::path::{Path, PathBuf};

use tracing::warn;

use crate::id::ActorId;
use crate::storage::sidecar::{self, SidecarKind};
use crate::storage::{NodeIndex, OffsetIndex, PageScope};

use super::JsonlStorage;

/// One `ops-<actor>.jsonl` to index: its file name, path, actor and size.
pub(super) type IndexJob = (String, PathBuf, ActorId, u64);

/// What one job produced, in the shape `reload_global` folds into its
/// per-actor maps.
pub(super) type LoadedIndex = (String, u64, ActorId, OffsetIndex, NodeIndex);

/// Load every job's sidecar pair, spreading the files across lanes.
///
/// Chunked rather than one thread per file, so a workspace with
/// hundreds of actors does not answer a boot by spawning hundreds of
/// threads.
pub(super) fn load_in_parallel(ops_dir: &Path, jobs: &[IndexJob]) -> Vec<LoadedIndex> {
    if jobs.is_empty() {
        return Vec::new();
    }
    let lanes = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .min(jobs.len());
    let chunk = jobs.len().div_ceil(lanes);

    let mut loaded: Vec<LoadedIndex> = Vec::with_capacity(jobs.len());
    std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(lanes);
        for slice in jobs.chunks(chunk) {
            handles.push(scope.spawn(move || {
                slice
                    .iter()
                    .map(|job| load_one(ops_dir, job))
                    .collect::<Vec<_>>()
            }));
        }
        for handle in handles {
            match handle.join() {
                Ok(batch) => loaded.extend(batch),
                // A panic here is a bug, not a damaged file: every read
                // path in the loader is fallible and falls back to a
                // rebuild. Losing the actor silently would boot a tree
                // missing its ops, so say so and let the rest of the
                // workspace still open.
                Err(_) => warn!("sidecar loader panicked; its actors are not indexed"),
            }
        }
    });
    loaded
}

/// Load the persisted sidecars for one actor when they exactly cover
/// the `.jsonl`; rebuild via parse-lite otherwise. The `.idx` bytes are
/// the ~4.3s reload win — they skip re-tokenizing every `text_op`.
fn load_one(ops_dir: &Path, (name, path, actor, size): &IndexJob) -> LoadedIndex {
    let offset = sidecar::path_for(ops_dir, *actor, &PageScope::Global, SidecarKind::Offset);
    let node = sidecar::path_for(ops_dir, *actor, &PageScope::Global, SidecarKind::Node);
    let (hlc_idx, node_idx) = JsonlStorage::load_actor_indexes(path, &offset, &node);
    (name.clone(), *size, *actor, hlc_idx, node_idx)
}
