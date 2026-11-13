//! Whether the op log on disk has moved since this process last read it.
//!
//! A reload exists to pick up ops somebody **else** wrote: a peer's,
//! delivered by sync ingest straight into `ops-<peer>.jsonl`, or another
//! local process's. It is O(whole log) — on a real workspace it decodes a
//! 48 MB snapshot, replays the delta and rehydrates 169,037 nodes, which
//! was measured at 271-317 ms.
//!
//! Mobile's poll runs `syncNow` then `reloadWorkspace` on a timer, with
//! no way to know whether the pass delivered anything, so on a device
//! whose peers are unreachable that full rebuild ran every tick, forever,
//! while the user typed. Measured on a launch against Avelino's
//! workspace: three rebuilds in the first seven seconds, `delta_ops`
//! identical across all three. Nothing had arrived. That is the "it keeps
//! loading" the journal shows.
//!
//! The honest fix is not in the caller. A reload with nothing to read
//! should be free for *every* caller, so this answers the one question
//! that decides it.
//!
//! ## What the fingerprint is, and why a length is enough
//!
//! One `(file name, length)` pair per `ops-*.jsonl`. An op log is
//! append-only, so a landed op always grows its actor's file; compaction
//! is the only rewrite and it removes lines, which also changes lengths.
//! A set of lengths that is identical across 23 files therefore means
//! nothing was written.
//!
//! **This device's own file is excluded.** Our own appends are already in
//! the resident log — replaying for them is pure waste, and worse, it
//! races: `publish_replayed` refuses to publish a tree that is missing a
//! local op and retries, so a user typing steadily could keep a reload
//! spinning against their own keystrokes.
//!
//! ## What a wrong answer costs
//!
//! Only latency, in one direction. A fingerprint read from disk cannot
//! say "unchanged" about a file that grew, so an arriving op is never
//! hidden; at worst a reload is skipped and the next tick (3 s on mobile)
//! takes it. A failure to read the directory answers "changed", so the
//! reload proceeds exactly as it does today.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::OnceLock;

use outl_core::id::ActorId;

/// `(file name, length)` for every actor log but our own.
type Fingerprint = Vec<(String, u64)>;

/// Last fingerprint a successful read or publish observed, per ops dir.
///
/// Keyed by path so switching workspaces cannot answer for the wrong one.
/// Bounded by the number of workspaces a process opens, which is one on
/// mobile and a handful on the desktop.
fn seen() -> &'static Mutex<HashMap<PathBuf, Fingerprint>> {
    static SEEN: OnceLock<Mutex<HashMap<PathBuf, Fingerprint>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn fingerprint(ops_dir: &Path, own: ActorId) -> Option<Fingerprint> {
    let own_file = format!("ops-{own}.jsonl");
    let mut out: Fingerprint = std::fs::read_dir(ops_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            if !name.starts_with("ops-") || !name.ends_with(".jsonl") || name == own_file {
                return None;
            }
            Some((name, entry.metadata().ok()?.len()))
        })
        .collect();
    out.sort();
    Some(out)
}

/// What the op log looked like when a read was authorised.
///
/// Opaque on purpose, and the only way to call [`note`]: the state it
/// carries has to be the one sampled **before** the read, never a fresh
/// sample taken after it. A reload takes hundreds of milliseconds, a
/// peer can append inside that window, and noting the later state would
/// record ops as seen that nothing replayed — the one way this guard
/// could hide an edit instead of merely delaying one.
#[must_use = "a watermark is only useful if `note` records it after the read succeeds"]
pub struct Watermark(Fingerprint);

/// Has anyone else written to this op log since the last [`note`]?
///
/// `Some(watermark)` means read it, and hand the watermark back to
/// [`note`] once the read has landed. `None` means nothing moved and the
/// caller can skip the work entirely.
///
/// Answers `Some` whenever the answer is not a confident no — an
/// unreadable directory, or a path this process has never noted — so a
/// guard that cannot tell degrades to the behaviour it replaced.
pub fn read_needed(ops_dir: &Path, own: ActorId) -> Option<Watermark> {
    let now = fingerprint(ops_dir, own)?;
    let unchanged = seen()
        .lock()
        .expect("ops watermark")
        .get(ops_dir)
        .is_some_and(|last| *last == now);
    (!unchanged).then_some(Watermark(now))
}

/// Record that the read authorised by `watermark` has landed.
pub fn note(ops_dir: &Path, watermark: Watermark) {
    seen()
        .lock()
        .expect("ops watermark")
        .insert(ops_dir.to_path_buf(), watermark.0);
}

/// Sample and record in one step, for a caller that has just read the
/// log as part of doing something else — a boot, which replays it in
/// full before anything can ask.
pub fn note_current(ops_dir: &Path, own: ActorId) {
    if let Some(w) = fingerprint(ops_dir, own) {
        seen()
            .lock()
            .expect("ops watermark")
            .insert(ops_dir.to_path_buf(), w);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &Path, name: &str, bytes: &str) {
        std::fs::write(dir.join(name), bytes).unwrap();
    }

    #[test]
    fn a_log_nobody_wrote_to_reads_as_unchanged() {
        let tmp = TempDir::new().unwrap();
        let own = ActorId::new();
        let peer = ActorId::new();
        write(tmp.path(), &format!("ops-{peer}.jsonl"), "{}\n");

        assert!(
            read_needed(tmp.path(), own).is_some(),
            "a path never noted has to be read, not assumed clean"
        );
        note_current(tmp.path(), own);
        assert!(read_needed(tmp.path(), own).is_none());
    }

    #[test]
    fn a_peer_append_reads_as_changed() {
        let tmp = TempDir::new().unwrap();
        let own = ActorId::new();
        let peer = ActorId::new();
        let file = format!("ops-{peer}.jsonl");
        write(tmp.path(), &file, "{}\n");
        note_current(tmp.path(), own);

        write(tmp.path(), &file, "{}\n{}\n");
        assert!(read_needed(tmp.path(), own).is_some());
    }

    #[test]
    fn a_brand_new_peer_file_reads_as_changed() {
        let tmp = TempDir::new().unwrap();
        let own = ActorId::new();
        note_current(tmp.path(), own);
        write(tmp.path(), &format!("ops-{}.jsonl", ActorId::new()), "{}\n");
        assert!(read_needed(tmp.path(), own).is_some());
    }

    /// Our own appends are already in the resident log. Replaying for
    /// them costs a full rebuild and races `publish_replayed`'s
    /// local-op guard, so a user typing steadily would keep a reload
    /// spinning against their own keystrokes.
    #[test]
    fn our_own_appends_do_not_ask_for_a_reload() {
        let tmp = TempDir::new().unwrap();
        let own = ActorId::new();
        let file = format!("ops-{own}.jsonl");
        write(tmp.path(), &file, "{}\n");
        note_current(tmp.path(), own);

        write(tmp.path(), &file, "{}\n{}\n{}\n");
        assert!(read_needed(tmp.path(), own).is_none());
    }

    /// Two workspaces must not answer for each other.
    #[test]
    fn a_different_ops_dir_is_never_answered_from_cache() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        let own = ActorId::new();
        note_current(a.path(), own);
        assert!(read_needed(b.path(), own).is_some());
    }
}
