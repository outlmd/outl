//! The write half of compaction: read the log, take every lock, back up,
//! replace atomically, invalidate the offset sidecars.
//!
//! Order matters and each step is a refusal point:
//!
//! 1. **Exclusive** flock on `<root>/.outl/.lock`. Every live `outl`
//!    process holds that file *shared*, so an exclusive acquire is the
//!    one question worth asking: is anyone else in this workspace? A
//!    running client caches byte offsets into the files we are about to
//!    renumber, so racing one is silent corruption on its next cold read.
//! 2. [`ActorWriteLock`] on **every** actor in `ops/`, not just the ones
//!    being rewritten. A writer holds exactly one of these, and it may
//!    be an actor whose file this plan does not touch. The set is
//!    re-listed from disk here and unioned with the plan's, so a file
//!    created since the plan is locked too.
//! 3. The actor set on disk still matches the plan's, and each file's
//!    byte length still matches what the plan measured. The length check
//!    sees a file that *changed*; only the set check sees one that was
//!    *created*, whose ops never entered the merged log inertness was
//!    decided against.
//! 4. Copy every file to be rewritten into
//!    `<root>/.outl/compact-backup/<timestamp>-<ulid>/`, fsynced, *before*
//!    a byte of `ops/` changes. Restoring is a plain `cp` back. The
//!    directory is fresh per run — never reused — so a second `--apply`
//!    cannot overwrite the rollback point the first one left.
//! 5. Rewrite via sibling temp + `rename`, so a reader in another
//!    process (one that ignored step 1) sees either the whole old file
//!    or the whole new one, never a torn middle.
//! 6. Delete `.ops-<actor>.idx` and `.ops-<actor>.nodes.idx`. Every
//!    offset in them points into the file we just renumbered. They are
//!    pure local caches and a missing one always rebuilds, so deleting
//!    is strictly safer than recomputing: there is no version of
//!    "deleted" that silently feeds a wrong offset into `read_op_at`.
//!
//! The **snapshot** cache under `.outl/snapshots/` is deliberately left
//! alone. Compaction removes only ops whose application changes nothing,
//! so the materialized state a snapshot projects is unchanged, and its
//! per-actor cutoff is compared with `>` — it never needs the op at the
//! cutoff to still exist.
//!
//! Kept lines are copied **byte for byte**, never re-serialized, so an
//! op compaction keeps is bit-identical to the one it replaces.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::id::ActorId;
use crate::lock::{ActorWriteLock, LockError};
use crate::op::LogOp;
use crate::storage::jsonl::{parse_actor_from_ops_filename, parse_log_line};
use crate::storage::{sidecar, PageScope};

use super::plan::{line_is_droppable, ops_path, CompactPlan};
use super::{CompactError, CompactReport};

/// One physical record: its bytes, and the op(s) it decodes to.
pub(super) struct Line {
    /// Ops on this line. More than one means a glued record (an
    /// interleaved non-atomic append); such a line is never droppable.
    pub ops: Vec<LogOp>,
    /// Length of the record on disk, newline included.
    pub bytes: u64,
}

/// One `ops-<actor>.jsonl`, fully read.
pub(super) struct ActorFile {
    pub actor: ActorId,
    pub bytes: u64,
    pub lines: Vec<Line>,
}

/// Read every `ops-<actor>.jsonl` under `ops_dir`.
///
/// Refuses (rather than skipping) on two things, because both would make
/// the decision pass reason about an incomplete log:
///
/// - a per-actor subdirectory holding `.jsonl` files — the `PerPage`
///   layout, whose ops would be invisible here;
/// - a record that does not parse — a damaged log is reported, never
///   rewritten.
pub(super) fn read_actor_files(ops_dir: &Path) -> Result<Vec<ActorFile>, CompactError> {
    let entries = std::fs::read_dir(ops_dir).map_err(|e| CompactError::Io {
        path: ops_dir.to_path_buf(),
        source: e,
    })?;
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| CompactError::Io {
            path: ops_dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() {
            if holds_op_log(&path) {
                return Err(CompactError::PerPageLayout(path));
            }
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(actor) = parse_actor_from_ops_filename(name) else {
            continue;
        };
        files.push(read_one(actor, path)?);
    }
    files.sort_by_key(|f| f.actor.0);
    Ok(files)
}

/// Whether a subdirectory of `ops/` looks like a per-page shard dir.
///
/// A directory we cannot **read** answers `true`, not `false`. This is
/// the one question in compaction whose wrong answer is silent: the plan
/// decides inertness against the *merged* log, so proceeding past a
/// directory that turns out to hold shards can drop a `Move` those shards
/// made meaningful. [RFC 0211](../../../../docs/rfcs/0211-state-that-leaves-a-boundary.md)'s
/// rule applies unchanged — an unreadable thing counts as present,
/// because one spurious refusal costs a re-run and one wrong proceed
/// costs the op log.
fn holds_op_log(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    entries.flatten().any(|e| {
        e.path()
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("jsonl"))
    })
}

fn read_one(actor: ActorId, path: PathBuf) -> Result<ActorFile, CompactError> {
    let file = File::open(&path).map_err(|e| CompactError::Io {
        path: path.clone(),
        source: e,
    })?;
    let mut reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut bytes = 0u64;
    let mut buf = Vec::new();
    let mut lineno = 0usize;
    loop {
        buf.clear();
        let n = reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| CompactError::Io {
                path: path.clone(),
                source: e,
            })?;
        if n == 0 {
            break;
        }
        lineno += 1;
        bytes += n as u64;
        let text = std::str::from_utf8(&buf).map_err(|e| CompactError::DamagedLog {
            path: path.clone(),
            line: lineno,
            reason: e.to_string(),
        })?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            // A blank line carries no op, so dropping it in the rewrite
            // loses nothing. Recorded with its bytes so the totals stay
            // honest about the file's size.
            lines.push(Line {
                ops: Vec::new(),
                bytes: n as u64,
            });
            continue;
        }
        let ops = parse_log_line(trimmed).map_err(|e| CompactError::DamagedLog {
            path: path.clone(),
            line: lineno,
            reason: e.to_string(),
        })?;
        lines.push(Line {
            ops,
            bytes: n as u64,
        });
    }
    Ok(ActorFile {
        actor,
        bytes,
        lines,
    })
}

/// Execute `plan`, refusing any `ops-<actor>.jsonl` that is not `own`'s.
///
/// **This is the entry point a client should call.** `apply_compaction`
/// will rewrite every actor file the plan names, including the mirrors of
/// other devices, and that is only safe when the caller has established
/// that no file transport carries this workspace.
///
/// `docs/storage.md` states the premise the whole per-actor layout rests
/// on: *"Each device's file is append-only and owned by exactly one
/// writer."* iCloud Drive, Syncthing, Dropbox and any shared filesystem
/// reconcile **per path**, last-write-wins. A device that shortens a
/// peer's file publishes a competing, shorter version of that path; the
/// peer's longer copy loses, and every op it had not yet shipped is gone
/// with no error anywhere. The locks cannot help — `flock(2)` is
/// advisory and machine-local, which `docs/clients.md` says in as many
/// words.
///
/// Whether a given workspace is on such a transport is not knowable from
/// here (a `transport = "iroh"` workspace living in a Dropbox folder is
/// exactly the trap), so the condition this refuses on is the provable
/// one: *is this file mine?*
///
/// The escape from the refusal is normally to do less, not to force:
/// [`CompactPlan::restricted_to`] narrows a plan to this device, and each
/// device compacting its own history is the intended usage anyway
/// ([RFC 0256](../../../../docs/rfcs/0256-op-log-compaction.md) →
/// "Re-pairing a device does not undo it").
pub fn apply_compaction_as(
    root: &Path,
    plan: &CompactPlan,
    own: ActorId,
) -> Result<CompactReport, CompactError> {
    if let Some(actor) = plan.foreign_actors(own).first().copied() {
        return Err(CompactError::ForeignActorFile { actor, own });
    }
    apply_compaction(root, plan)
}

/// Execute `plan`, rewriting **every** actor file it names.
///
/// `ops/` is either untouched or replaced file-by-file atomically; there
/// is no partial state a reader can observe.
///
/// Prefer [`apply_compaction_as`]: this function will shorten another
/// device's `ops-<actor>.jsonl`, which is unsafe on any file-based sync
/// transport. It is the `--force` path, and the caller owns the claim
/// that no such transport carries this workspace.
pub fn apply_compaction(root: &Path, plan: &CompactPlan) -> Result<CompactReport, CompactError> {
    let mut report = plan.report().clone();
    if plan.is_empty() {
        return Ok(report);
    }
    let ops_dir = root.join("ops");

    let _workspace = exclusive_workspace_lock(root)?;

    // Re-list under the lock. `plan.sizes` is the actor set as it was at
    // plan time; an `ops-<actor>.jsonl` that appeared since is unlocked,
    // unmeasured, and — the part that matters — its ops never entered the
    // merged log inertness was decided against, so a `Move` it makes
    // meaningful can still be dropped. The length check below catches a
    // file that *changed*, never one that was *created*.
    let present = actor_files_present(&ops_dir)?;
    let planned: BTreeSet<ActorId> = plan.sizes.keys().copied().collect();
    let _actors = lock_every_actor(&ops_dir, planned.union(&present).copied())?;
    if present != planned {
        let moved = present
            .symmetric_difference(&planned)
            .next()
            .map(|actor| ops_path(&ops_dir, *actor))
            .unwrap_or_else(|| ops_dir.clone());
        return Err(CompactError::PlanStale(moved));
    }

    for (actor, expected) in &plan.sizes {
        let path = ops_path(&ops_dir, *actor);
        let len = std::fs::metadata(&path)
            .map_err(|e| CompactError::Io {
                path: path.clone(),
                source: e,
            })?
            .len();
        if len != *expected {
            return Err(CompactError::PlanStale(path));
        }
    }

    // Re-read EVERY file this plan will rewrite before rewriting any of
    // them. `rewrite_one` used to read its own file, so a record that
    // stopped parsing between the plan and now was discovered in the
    // middle of the loop — with the actors sorted ahead of it already
    // replaced. That is a refusal that has written half of `ops/`, which
    // is not a refusal; the module doc above promises a damaged log is
    // reported, never rewritten, and "never" has to mean no file.
    //
    // A damaged file this plan does NOT touch is covered by the length
    // check above plus the plan being decided against the log as it was
    // read: the bytes we are about to write do not depend on it.
    let sources: Vec<(ActorId, ActorFile)> = plan
        .touched()
        .map(|actor| Ok((actor, read_one(actor, ops_path(&ops_dir, actor))?)))
        .collect::<Result<_, CompactError>>()?;

    // The timestamp is for the human reading the directory; the ULID is
    // what makes the name unique. Second resolution alone is not a name:
    // two `--apply` runs inside one second would share a directory and
    // `copy_durable` would overwrite the first run's files — the only
    // recovery route for a rewrite that already deleted lines. `create_dir`
    // rather than `create_dir_all` for the leaf, so an existing directory
    // is refused instead of reused.
    let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S").to_string();
    let backup_dir = backup_generation(root, &stamp);
    if let Some(parent) = backup_dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| CompactError::Io {
            path: parent.to_path_buf(),
            source: e,
        })?;
    }
    std::fs::create_dir(&backup_dir).map_err(|e| CompactError::Io {
        path: backup_dir.clone(),
        source: e,
    })?;

    for (actor, _) in &sources {
        let path = ops_path(&ops_dir, *actor);
        let name = format!("ops-{actor}.jsonl");
        copy_durable(&path, &backup_dir.join(&name))?;
    }
    // Only after every backup is on disk does anything in `ops/` move.
    //
    // Past this line every failure carries `backup_dir`. A mid-loop abort
    // leaves a partially rewritten `ops/`, and the one moment somebody
    // needs the backup's path is exactly that one — it used to be
    // reported only on success, so the recovery route was invisible in
    // the only case that needed it.
    let mut failure: Option<CompactError> = None;
    for (actor, source) in &sources {
        let drops = plan.drops.get(actor).cloned().unwrap_or_default();
        if let Err(e) = rewrite_one(&ops_dir, *actor, source, &drops) {
            failure.get_or_insert(e);
            break;
        }
        if let Err(e) = invalidate_indexes(&ops_dir, *actor) {
            // Not `?`: stopping here leaves *more* stale sidecars than
            // continuing does. Every actor after this one would keep an
            // index whose offsets point into a file this pass renumbers,
            // and a wrong offset is a silently dropped op on every
            // index-driven read (#129). Record it and keep going.
            failure.get_or_insert(e);
        }
    }
    if let Some(source) = failure {
        return Err(CompactError::RewriteFailed {
            backup_dir,
            source: Box::new(source),
        });
    }

    report.backup_dir = Some(backup_dir);
    Ok(report)
}

/// Every actor with an `ops-<actor>.jsonl` directly under `ops_dir`.
///
/// Applies the same two rules as [`read_actor_files`] about what a
/// directory in `ops/` means, because this runs between the plan and the
/// rewrite: a per-page shard directory appearing in that window is the
/// same "decide inertness against an incomplete log" problem, arriving
/// late.
fn actor_files_present(ops_dir: &Path) -> Result<BTreeSet<ActorId>, CompactError> {
    let entries = std::fs::read_dir(ops_dir).map_err(|e| CompactError::Io {
        path: ops_dir.to_path_buf(),
        source: e,
    })?;
    let mut actors = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|e| CompactError::Io {
            path: ops_dir.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() {
            if holds_op_log(&path) {
                return Err(CompactError::PerPageLayout(path));
            }
            continue;
        }
        if let Some(actor) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(parse_actor_from_ops_filename)
        {
            actors.insert(actor);
        }
    }
    Ok(actors)
}

/// The same question as [`exclusive_workspace_lock`], asked without
/// creating anything: the read side ([`super::plan_compaction`]) must not
/// materialise `.outl/` in a directory it is only inspecting.
///
/// A missing lock file means no live process has ever attached, so there
/// is nobody to race. The returned handle must be held for as long as the
/// read lasts.
pub(super) fn idle_workspace_probe(root: &Path) -> Result<Option<File>, CompactError> {
    let path = root.join(".outl").join(".lock");
    // Read *and* write, minus `create`: Windows wants write access on the
    // handle before it will grant an exclusive lock, and creating the file
    // is what this probe exists not to do.
    // A read-only workspace (a mounted backup, a restored archive) is a
    // legitimate thing to plan against, and refusing the *dry run* there
    // would be a wall built by a probe. Fall back to a read handle.
    let file = match OpenOptions::new().read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => match File::open(&path) {
            Ok(file) => file,
            Err(source) => return Err(CompactError::Io { path, source }),
        },
        Err(source) => return Err(CompactError::Io { path, source }),
    };
    file.try_lock_exclusive()
        .map_err(|_| CompactError::Busy(path))?;
    Ok(Some(file))
}

/// Every live `outl` process holds `<root>/.outl/.lock` shared, so an
/// exclusive acquire answers "is this workspace open anywhere?".
fn exclusive_workspace_lock(root: &Path) -> Result<File, CompactError> {
    let dir = root.join(".outl");
    std::fs::create_dir_all(&dir).map_err(|e| CompactError::Io {
        path: dir.clone(),
        source: e,
    })?;
    let path = dir.join(".lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| CompactError::Io {
            path: path.clone(),
            source: e,
        })?;
    file.try_lock_exclusive()
        .map_err(|_| CompactError::Busy(path))?;
    Ok(file)
}

/// Lock **every** actor in `ops/`, not just the rewritten ones: a writer
/// holds exactly one of these and it need not be one this plan touches.
///
/// Takes the actor set rather than the plan, because the set that matters
/// is the one on disk *now* unioned with the one the plan saw — an actor
/// file created since is exactly the writer this is meant to exclude.
fn lock_every_actor(
    ops_dir: &Path,
    actors: impl Iterator<Item = ActorId>,
) -> Result<Vec<ActorWriteLock>, CompactError> {
    let mut held = Vec::new();
    for actor in actors {
        match ActorWriteLock::try_acquire(ops_dir, actor) {
            Ok(lock) => held.push(lock),
            Err(LockError::AlreadyHeld(path)) => return Err(CompactError::Busy(path)),
            Err(LockError::Io { path, source }) => return Err(CompactError::Io { path, source }),
        }
    }
    Ok(held)
}

/// A backup generation directory no earlier run can already be using.
///
/// `stamp` is second-resolution, and a second resolution is not enough on
/// its own: two valid `--apply` runs land in the same second easily —
/// `--apply` then the `--no-horizon` re-run the dry run's own output
/// recommends, a script, a user re-reading the output and trying again.
/// Sharing a directory means [`copy_durable`] writes the *already
/// compacted* file over the pre-compaction one, and this module deletes
/// op-log lines: that copy is the only route back, named by
/// [RFC 0256](../../../../docs/rfcs/0256-op-log-compaction.md) and by
/// `outl compact`'s own output. Overwriting a backup looks exactly like
/// taking one, so nothing would tell the user.
///
/// The ULID is the repo's existing answer to "this filename must not
/// collide" (`sidecar::write_atomic`'s temps, and `rewrite_one` below).
/// It goes **after** the stamp so the directory still sorts
/// chronologically by name — the property `repair-backup`'s generational
/// prune leans on, and the one a prune here would need too.
///
/// Refusing a reused directory instead would be a guard whose only
/// escape hatch is a stopwatch: the colliding run is *legitimate*, so
/// there is nothing for the user to fix and nothing to force (root
/// `CLAUDE.md` invariant 9).
pub(super) fn backup_generation(root: &Path, stamp: &str) -> PathBuf {
    root.join(".outl")
        .join("compact-backup")
        .join(format!("{stamp}-{}", ulid::Ulid::new()))
}

fn copy_durable(from: &Path, to: &Path) -> Result<(), CompactError> {
    std::fs::copy(from, to).map_err(|e| CompactError::Io {
        path: to.to_path_buf(),
        source: e,
    })?;
    let file = File::open(to).map_err(|e| CompactError::Io {
        path: to.to_path_buf(),
        source: e,
    })?;
    file.sync_all().map_err(|e| CompactError::Io {
        path: to.to_path_buf(),
        source: e,
    })?;
    Ok(())
}

/// Stream the file into a sibling temp, skipping the dropped lines, then
/// rename over the original. Kept lines are copied verbatim.
/// Rewrite one actor's file from an `ActorFile` the caller already read.
///
/// Takes the parse rather than doing it, so the caller can surface a
/// damaged record **before** the first byte of `ops/` moves — see
/// `apply_compaction`.
fn rewrite_one(
    ops_dir: &Path,
    actor: ActorId,
    source: &ActorFile,
    drops: &BTreeSet<crate::hlc::Hlc>,
) -> Result<(), CompactError> {
    let path = ops_path(ops_dir, actor);
    let tmp = ops_dir.join(format!(".compact-{actor}-{}.tmp", ulid::Ulid::new()));
    let mut out = File::create(&tmp).map_err(|e| CompactError::Io {
        path: tmp.clone(),
        source: e,
    })?;

    let write = |out: &mut File, bytes: &[u8]| -> Result<(), CompactError> {
        out.write_all(bytes).map_err(|e| CompactError::Io {
            path: tmp.clone(),
            source: e,
        })
    };

    // Re-read the raw bytes alongside the decoded lines so kept records
    // are copied, not re-serialized.
    let raw = File::open(&path).map_err(|e| CompactError::Io {
        path: path.clone(),
        source: e,
    })?;
    let mut reader = BufReader::new(raw);
    let mut buf = Vec::new();
    for line in &source.lines {
        buf.clear();
        reader
            .read_until(b'\n', &mut buf)
            .map_err(|e| CompactError::Io {
                path: path.clone(),
                source: e,
            })?;
        if line.ops.is_empty() || line_is_droppable(line, drops) {
            continue;
        }
        write(&mut out, &buf)?;
    }
    out.sync_all().map_err(|e| CompactError::Io {
        path: tmp.clone(),
        source: e,
    })?;
    drop(out);
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(CompactError::Io { path, source: e });
    }
    Ok(())
}

/// Both offset sidecars are byte offsets into the file just renumbered.
/// Delete rather than rebuild: a missing index always rebuilds, a wrong
/// one is a silently dropped op on every index-driven read.
fn invalidate_indexes(ops_dir: &Path, actor: ActorId) -> Result<(), CompactError> {
    sidecar::remove_all(ops_dir, actor, &PageScope::Global).map_err(|e| CompactError::Io {
        path: ops_dir.to_path_buf(),
        source: std::io::Error::other(e.to_string()),
    })?;
    Ok(())
}
