//! One owner for the `ops/` index sidecars: where they live, how they are
//! written, and — in [`gc`] — when they may be collected.
//!
//! ## Why this module exists
//!
//! `ops/` holds two caches per actor: the HLC → offset map
//! ([`super::index::OffsetIndex`]) and the node → offsets map
//! ([`super::node_index::NodeIndex`]). Their *payloads* differ and stay
//! separate. Everything around them was duplicated: the filename, the
//! temp-and-rename write, the "load, or give up and rebuild" loop, the
//! append, and the deletion compaction performs.
//!
//! Two copies of a filename is how a whole generation of sidecars
//! survived unnoticed. The indexes were moved to dot-prefixed names
//! precisely so they would stop riding the file-sync transport (`ops/`
//! is deliberately **not** a dotfile — see `docs/storage.md` → "Why the
//! directory is named `ops/`, not `.ops/`"), and nothing ever removed
//! what the rename left behind. A real workspace carried 50 MB of
//! undotted sidecars and 84 MB of abandoned temps — 134 MB that no code
//! path reads — for months.
//!
//! So: [`path_for`] is the **only** thing that composes a sidecar name,
//! [`paths_for`] is the only thing that enumerates an actor's complete
//! set, and [`gc`] decides what may go. A second opinion about any of
//! those three is the bug this module exists to make impossible.
//!
//! ## Naming
//!
//! | Layout | `.jsonl` | sidecars |
//! |---|---|---|
//! | [`PageScope::Global`] | `ops/ops-<actor>.jsonl` | `ops/.ops-<actor>.idx`, `ops/.ops-<actor>.nodes.idx` |
//! | [`PageScope::PerPage`] | `ops/<actor>/<slug>.jsonl` | `ops/<actor>/.<slug>.idx`, `ops/<actor>/.<slug>.nodes.idx` |
//!
//! The per-page names carry the **slug**, not the actor. Before this
//! module they carried the actor, so every page shard of one actor wrote
//! into the same `.ops-<actor>.idx` — offsets into `a.jsonl` and
//! `b.jsonl` interleaved in one file. What kept that from being a wrong
//! read was the boot freshness check rejecting the mixture and rebuilding,
//! i.e. a guard, not a design. See
//! [RFC 0265](../../../../docs/rfcs/0265-index-sidecar-lifecycle.md).
//!
//! ## Dot-prefixed on purpose
//!
//! An index is a purely *local* boot cache — every device rebuilds it
//! from its own `.jsonl` — so it must not ride the file-sync surface:
//! iCloud Documents drops `.`-prefixed paths across devices, and iroh
//! never ships sidecars. A synced index could arrive torn-in-the-middle
//! with an intact tail, pass the freshness check, and feed a wrong middle
//! offset into `read_op_at` — silent op loss. Keeping it off the sync
//! surface removes that vector entirely.

pub mod gc;

#[cfg(test)]
mod tests;

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::warn;

use crate::id::ActorId;
use crate::storage::{PageScope, StorageError};

/// Which of the two index caches a sidecar file holds.
///
/// The payloads are genuinely different — one maps HLCs to offsets, the
/// other nodes to `(HLC, offset)` lists — so this unifies their
/// *lifecycle*, not their contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SidecarKind {
    /// `OffsetIndex`: HLC → byte offset.
    Offset,
    /// `NodeIndex`: `NodeId` → `[(HLC, byte offset)]`.
    Node,
}

impl SidecarKind {
    /// Every kind. An exhaustive list, so "the complete set of sidecars
    /// for an actor" has one definition rather than one per caller.
    pub const ALL: [SidecarKind; 2] = [SidecarKind::Offset, SidecarKind::Node];

    /// The filename tail this kind contributes, extension included.
    pub fn suffix(self) -> &'static str {
        match self {
            SidecarKind::Offset => ".idx",
            SidecarKind::Node => ".nodes.idx",
        }
    }
}

/// The stem a sidecar name is built on: the actor under
/// [`PageScope::Global`], the page slug under [`PageScope::PerPage`].
///
/// The distinction is the whole point — under `PerPage` every shard of
/// one actor lives in `ops/<actor>/`, so an actor-derived name collides
/// across pages.
fn stem(actor: ActorId, scope: &PageScope) -> String {
    match scope {
        PageScope::Global => format!("ops-{actor}"),
        PageScope::PerPage(slug) => slug.clone(),
    }
}

/// The file name the reader composes for `(actor, scope, kind)`.
///
/// **The single owner.** Anything that reads, writes, deletes or judges a
/// sidecar asks this — including [`gc`], whose entire basis for calling
/// the undotted generation dead is that this function emits a dot.
pub fn file_name(actor: ActorId, scope: &PageScope, kind: SidecarKind) -> String {
    format!(".{}{}", stem(actor, scope), kind.suffix())
}

/// Path of one sidecar inside `dir`, where `dir` is the directory holding
/// the `.jsonl` it indexes (`ops/` for `Global`, `ops/<actor>/` for
/// `PerPage`).
pub fn path_for(dir: &Path, actor: ActorId, scope: &PageScope, kind: SidecarKind) -> PathBuf {
    dir.join(file_name(actor, scope, kind))
}

/// Every sidecar path for one `(actor, scope)` — the **complete** set.
///
/// Compaction renumbers every byte offset in a `.jsonl`, so the set it
/// invalidates has to be complete or a survivor silently feeds a wrong
/// offset into `read_op_at`. Callers ask for the set rather than listing
/// the names they happen to remember.
pub fn paths_for(dir: &Path, actor: ActorId, scope: &PageScope) -> Vec<PathBuf> {
    SidecarKind::ALL
        .iter()
        .map(|kind| path_for(dir, actor, scope, *kind))
        .collect()
}

/// Delete every sidecar for `(actor, scope)`, plus any generation of them
/// [`gc`] can prove dead in the same directory.
///
/// Used by compaction after a rewrite. A missing file is not an error —
/// that is the state we are driving towards.
pub fn remove_all(
    dir: &Path,
    actor: ActorId,
    scope: &PageScope,
) -> Result<Vec<PathBuf>, StorageError> {
    let mut removed = Vec::new();
    for path in paths_for(dir, actor, scope) {
        match std::fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(StorageError::Backend(format!(
                    "remove {}: {e}",
                    path.display()
                )))
            }
        }
    }
    // Dead generations of the same actor's caches: the undotted
    // pre-dotfile spelling and this actor's abandoned write temps. They
    // hold offsets into the file just renumbered too, and leaving them is
    // what accumulated 134 MB in the first place.
    for entry in gc::survey(dir)?.prunable() {
        if entry.actor() != Some(actor) {
            continue;
        }
        if gc::prune(entry).unwrap_or(false) {
            removed.push(entry.path.clone());
        }
    }
    removed.sort();
    Ok(removed)
}

/// A temp file that deletes itself unless [`TempFile::keep`] is called.
///
/// [`write_atomic`] used to remove its scratch file on a failed `rename`
/// and nowhere else, so every error return and every panic in between
/// leaked one permanently. This closes each of those paths. What it
/// cannot close is a process that never runs code again — `SIGKILL`,
/// iOS jetsam, power loss — which is why [`gc`] exists with a TTL.
struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl TempFile {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    /// The rename succeeded; there is nothing left at this path.
    fn keep(mut self) {
        self.armed = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// How much of a bulk sidecar write is batched before it reaches the
/// kernel.
///
/// **Not a tuning knob.** [`save_entries`] emits one line per index
/// entry, and a 217,811-op workspace rebuilds 435,622 of them across 40
/// files on a cold boot. Unbuffered that is one `write(2)` per entry:
/// measured at **71.5 s of a 72.0 s cold boot**, 0.62 s user + 3.07 s sys
/// against 72.00 s wall — ~95% of the boot blocked in the kernel. The
/// same 47.5 MB through a 256 KiB buffer takes 0.54 s.
///
/// `outl compact --apply` deletes every sidecar it invalidates, by
/// design, so without this each compaction armed a 70-second freeze on
/// the next open.
pub(crate) const WRITE_BUF_BYTES: usize = 256 * 1024;

/// Wrap a sink in the bulk-write buffer.
///
/// One function so the buffering is a property of this module rather than
/// of each call site remembering to add it — which is how it went missing
/// in the first place.
pub(crate) fn buffered<W: Write>(inner: W) -> BufWriter<W> {
    BufWriter::with_capacity(WRITE_BUF_BYTES, inner)
}

/// Atomically replace `path`'s contents.
///
/// Creates a unique sibling temp — the per-write ULID suffix avoids the
/// ENOENT race when two reindex passes for the same actor write
/// concurrently (both write the same content, last rename wins) — lets
/// `write_body` stream the lines into a [`buffered`] writer, flushes,
/// fsyncs, then renames over `path`. The temp is removed on **every**
/// in-process exit path, not just a failed rename (see [`TempFile`]).
///
/// The temp inherits `path`'s leading dot, so a temp a killed process
/// abandons is still off the file-sync surface.
pub(crate) fn write_atomic(
    path: &Path,
    write_body: impl FnOnce(&mut dyn Write, &Path) -> Result<(), StorageError>,
) -> Result<(), StorageError> {
    let tmp_path = tmp_path_for(path);
    let guard = TempFile::new(tmp_path);
    let tmp = guard.path.as_path();
    let mut file = std::fs::File::create(tmp)
        .map_err(|e| StorageError::Backend(format!("create {}: {e}", tmp.display())))?;
    {
        let mut out = buffered(&mut file);
        write_body(&mut out, tmp)?;
        // `into_inner` flushes and **returns** the error. `Drop` also
        // flushes, and throws the result away — on a path whose entire
        // job is durability, an ENOSPC on the last buffer would then look
        // exactly like a successful write.
        out.into_inner().map_err(|e| {
            StorageError::Backend(format!("flush {}: {}", tmp.display(), e.error()))
        })?;
    }
    file.sync_all()
        .map_err(|e| StorageError::Backend(format!("fsync {}: {e}", tmp.display())))?;
    drop(file);
    std::fs::rename(tmp, path).map_err(|e| {
        StorageError::Backend(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })?;
    guard.keep();
    Ok(())
}

/// Scratch path for an in-flight rewrite of `path`.
///
/// Appends rather than replacing the extension: `.ops-<a>.nodes.idx` must
/// not lose its `.nodes` half, and [`gc`] recovers the published name by
/// stripping exactly this suffix.
fn tmp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.push_str(&format!(".tmp.{}", ulid::Ulid::new()));
    path.with_file_name(name)
}

/// Read a JSONL sidecar, handing every recovered entry to `sink`.
///
/// Returns the number of entries recovered, or `Ok(None)` meaning
/// **rebuild from the `.jsonl`** — the file was missing, empty, or did
/// not parse. A sidecar is a cache, so a parse failure is never a hard
/// error; `Err` is reserved for an I/O failure opening it.
///
/// Lines are streamed with [`serde_json::Deserializer`] rather than
/// parsed one-per-line so glued entries — two appends interleaving with
/// no separating newline — recover both sides instead of losing the line.
pub(crate) fn load_entries<T, F>(
    path: &Path,
    label: &str,
    mut sink: F,
) -> Result<Option<usize>, StorageError>
where
    T: serde::de::DeserializeOwned,
    F: FnMut(T),
{
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(StorageError::Backend(format!(
                "open {label} {}: {e}",
                path.display()
            )))
        }
    };
    let mut recovered = 0usize;
    for (lineno, line) in BufReader::new(file).lines().enumerate() {
        let raw = match line {
            Ok(l) if !l.is_empty() => l,
            Ok(_) => continue,
            Err(e) => {
                warn!("{label} io error {}:{}: {e}", path.display(), lineno + 1);
                return Ok(None);
            }
        };
        let mut saw_any = false;
        for item in serde_json::Deserializer::from_str(&raw).into_iter::<T>() {
            match item {
                Ok(entry) => {
                    sink(entry);
                    recovered += 1;
                    saw_any = true;
                }
                Err(e) => {
                    warn!(
                        "{label} parse {}:{}: {e} — rebuilding from .jsonl",
                        path.display(),
                        lineno + 1
                    );
                    return Ok(None);
                }
            }
        }
        if !saw_any {
            warn!(
                "{label} empty line {}:{} — rebuilding",
                path.display(),
                lineno + 1
            );
            return Ok(None);
        }
    }
    if recovered == 0 {
        return Ok(None);
    }
    Ok(Some(recovered))
}

/// Persist a whole sidecar atomically, one JSON value per line.
///
/// The `writeln!` below lands in [`write_atomic`]'s buffer, not in a
/// syscall — see [`WRITE_BUF_BYTES`] for what that is worth.
pub(crate) fn save_entries<T: Serialize>(
    path: &Path,
    entries: impl Iterator<Item = T>,
) -> Result<(), StorageError> {
    write_atomic(path, |file, tmp| {
        for entry in entries {
            let line = serde_json::to_string(&entry)
                .map_err(|e| StorageError::Serialize(e.to_string()))?;
            writeln!(file, "{line}")
                .map_err(|e| StorageError::Backend(format!("write {}: {e}", tmp.display())))?;
        }
        Ok(())
    })
}

/// Append one entry to a sidecar. The hot path from `append_ops_inner`.
///
/// Deliberately **not** buffered: this is one line per call into a file
/// it opens and closes, so a buffer would have nothing to batch and
/// would only add a flush that can fail. The buffering in
/// [`write_atomic`] exists for the bulk rebuild, which is a different
/// shape — hundreds of thousands of lines in one pass.
///
/// No fsync: the caller has just fsynced the `.jsonl`, and the index is a
/// cache — a lost tail entry costs the next boot a rebuild, never an op.
pub(crate) fn append_entry<T: Serialize>(path: &Path, entry: &T) -> Result<(), StorageError> {
    let line = serde_json::to_string(entry).map_err(|e| StorageError::Serialize(e.to_string()))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| StorageError::Backend(format!("open {}: {e}", path.display())))?;
    writeln!(file, "{line}")
        .map_err(|e| StorageError::Backend(format!("write {}: {e}", path.display())))?;
    Ok(())
}
