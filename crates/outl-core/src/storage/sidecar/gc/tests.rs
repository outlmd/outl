//! What the index-sidecar GC collects, and — the half that matters —
//! what it refuses.
//!
//! An index sidecar is pure cache, so a wrong delete costs one slower
//! boot and never an op. The refusals below are still tested harder than
//! the deletions, because every one of them is a path by which this
//! module could reach something that is *not* a cache.

use std::time::{Duration, SystemTime};

use super::*;
use crate::id::ActorId;
use crate::storage::sidecar::{file_name, SidecarKind};
use crate::storage::PageScope;
use tempfile::TempDir;

fn touch(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write fixture");
    p
}

/// Backdate a file so the TTL sees it as abandoned.
fn age(path: &std::path::Path, by: Duration) {
    let when = SystemTime::now() - by;
    filetime::set_file_mtime(path, filetime::FileTime::from_system_time(when)).expect("set mtime");
}

fn verdict_of(survey: &Survey, name: &str) -> Option<SidecarVerdict> {
    survey
        .entries
        .iter()
        .find(|e| e.path.file_name().and_then(|n| n.to_str()) == Some(name))
        .map(|e| e.verdict)
}

// ---------------------------------------------------------------- deletions

#[test]
fn the_undotted_generation_is_prunable() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    touch(tmp.path(), &format!("ops-{a}.idx"), "x");
    touch(tmp.path(), &format!("ops-{a}.nodes.idx"), "x");

    let survey = survey(tmp.path()).unwrap();
    assert_eq!(
        verdict_of(&survey, &format!("ops-{a}.idx")),
        Some(SidecarVerdict::Legacy)
    );
    assert_eq!(
        verdict_of(&survey, &format!("ops-{a}.nodes.idx")),
        Some(SidecarVerdict::Legacy)
    );
    assert_eq!(survey.prunable().count(), 2);
}

#[test]
fn an_old_write_temp_is_prunable() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    let name = format!(".ops-{a}.idx.tmp.{}", ulid::Ulid::new());
    let p = touch(tmp.path(), &name, "half written");
    age(&p, Duration::from_secs(60 * 60 * 48));

    let survey = survey(tmp.path()).unwrap();
    assert_eq!(
        verdict_of(&survey, &name),
        Some(SidecarVerdict::AbandonedScratch)
    );
}

#[test]
fn pruning_removes_the_file_and_reports_it() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    let p = touch(tmp.path(), &format!("ops-{a}.idx"), "dead");

    let survey = survey(tmp.path()).unwrap();
    let entry = survey.prunable().next().expect("one prunable entry");
    assert!(prune(entry).unwrap());
    assert!(!p.exists());
}

// ---------------------------------------------------------------- refusals

/// The op log itself is never a candidate — not for an actor we know,
/// not for a peer whose log arrived over the sync transport, not for a
/// conflict copy some file-sync tool left behind.
#[test]
fn no_op_log_is_ever_a_candidate() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    for name in [
        format!("ops-{a}.jsonl"),
        format!("ops-{a} 2.jsonl"),
        format!("ops-{a}.jsonl.sync-conflict-20260101-120000"),
    ] {
        touch(tmp.path(), &name, "{}\n");
    }

    let survey = survey(tmp.path()).unwrap();
    assert_eq!(survey.entries.len(), 0, "op logs must not even be surveyed");
}

/// A live sidecar belongs to whoever wrote it, and `ops/` carries files
/// from other devices. An actor with no `.jsonl` on this disk is a peer
/// whose log has not been pulled yet — not evidence of anything.
#[test]
fn a_live_sidecar_for_an_unknown_actor_is_kept() {
    let tmp = TempDir::new().unwrap();
    let peer = ActorId::new();
    let name = file_name(peer, &PageScope::Global, SidecarKind::Offset);
    touch(tmp.path(), &name, "x");

    let survey = survey(tmp.path()).unwrap();
    assert_eq!(verdict_of(&survey, &name), Some(SidecarVerdict::Live));
    assert_eq!(survey.prunable().count(), 0);
}

/// A temp whose companion write may still be in flight. Same TTL
/// reasoning as `snapshot::gc::stale_tmp`: a real write lives for as long
/// as one fsync, so a day is orders of magnitude of headroom.
#[test]
fn a_fresh_write_temp_is_refused() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    let name = format!(".ops-{a}.idx.tmp.{}", ulid::Ulid::new());
    touch(tmp.path(), &name, "in flight");

    let survey = survey(tmp.path()).unwrap();
    assert_eq!(
        verdict_of(&survey, &name),
        Some(SidecarVerdict::Inconclusive),
        "a temp younger than the TTL may still be mid-write"
    );
    assert_eq!(survey.prunable().count(), 0);
}

/// A name we cannot attribute to an actor is not a file we produced, and
/// a file we did not produce is not a file we proved dead.
#[test]
fn an_unattributable_name_is_never_surveyed() {
    let tmp = TempDir::new().unwrap();
    for name in [
        "ops-not-a-ulid.idx",
        "notes.idx",
        "ops-.idx",
        "README.md",
        ".DS_Store",
    ] {
        touch(tmp.path(), name, "x");
    }

    let survey = survey(tmp.path()).unwrap();
    assert_eq!(survey.entries.len(), 0);
}

/// Arbitration state, not cache. Deleting a `.lock-<actor>` while a
/// process holds the flock lets a second process create a fresh inode
/// and believe it owns the same actor — two writers on one
/// `ops-<actor>.jsonl`. The GC does not go near them.
#[test]
fn lock_files_are_never_surveyed() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    let p = touch(tmp.path(), &format!(".lock-{a}"), "");
    age(&p, Duration::from_secs(60 * 60 * 24 * 365));
    touch(tmp.path(), ".append.lock", "");

    let survey = survey(tmp.path()).unwrap();
    assert_eq!(survey.entries.len(), 0);
    assert!(p.exists());
}

/// The verdict was computed from bytes; if those bytes moved, the verdict
/// is no longer evidence. Mirrors `snapshot::gc::prune`.
#[test]
fn a_file_that_changed_since_the_survey_is_refused() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    let p = touch(tmp.path(), &format!("ops-{a}.idx"), "dead");

    let survey = survey(tmp.path()).unwrap();
    let entry = survey.prunable().next().unwrap();

    std::fs::write(&p, "somebody rewrote this").unwrap();
    age(&p, Duration::from_secs(0));

    assert!(!prune(entry).unwrap(), "changed bytes revoke the verdict");
    assert!(p.exists());
}

/// `Path::starts_with` compares components without normalising, so a
/// crafted path could climb out of the surveyed directory. Every real
/// sidecar sits directly in the directory it was surveyed in.
#[test]
fn a_path_outside_the_surveyed_directory_is_refused() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    let p = touch(tmp.path(), &format!("ops-{a}.idx"), "dead");

    let survey = survey(tmp.path()).unwrap();
    let mut entry = survey.entries.into_iter().next().unwrap();
    entry.dir = tmp.path().join("elsewhere");

    assert!(!prune(&entry).unwrap());
    assert!(p.exists());
}

/// The entire basis for calling `ops-<actor>.idx` dead is that
/// [`file_name`] composes a dot-prefixed name, so nothing reads the
/// undotted one. If that ever stops being true, the undotted file is the
/// **live** cache and this GC becomes a data-losing bug. So the rule
/// disarms itself rather than trusting a comment to stay accurate.
#[test]
fn the_dead_name_is_derived_from_the_live_one() {
    let a = ActorId::new();
    for kind in SidecarKind::ALL {
        let live = file_name(a, &PageScope::Global, kind);
        assert!(
            live.starts_with('.'),
            "if the live sidecar name loses its dot, `legacy_name` stops naming a dead \
             file and starts naming the live one — see `classify`"
        );
        assert_eq!(
            legacy_name(a, kind),
            live.trim_start_matches('.'),
            "the legacy spelling must be derived from the live one, not typed out again"
        );
    }
}

/// A directory that does not exist is an empty survey, not an error — a
/// workspace that has never written an op is a normal state.
#[test]
fn a_missing_directory_surveys_empty() {
    let tmp = TempDir::new().unwrap();
    let survey = survey(&tmp.path().join("nope")).unwrap();
    assert!(survey.entries.is_empty());
}

/// Per-page shard directories are deliberately out of scope: deciding
/// which `.<slug>.idx` is live there needs the directory's `.jsonl` set,
/// and a wrong call deletes a live cache for a few KB. See the module doc.
#[test]
fn per_page_shard_directories_are_not_descended_into() {
    let tmp = TempDir::new().unwrap();
    let a = ActorId::new();
    let shard = tmp.path().join(a.to_string());
    std::fs::create_dir_all(&shard).unwrap();
    touch(&shard, &format!("ops-{a}.idx"), "x");

    let survey = survey(tmp.path()).unwrap();
    assert!(survey.entries.is_empty());
}
