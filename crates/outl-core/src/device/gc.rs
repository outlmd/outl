//! Which actor bindings the device store may drop, and — far more
//! importantly — which it may not.
//!
//! The store has never had a GC. `actors/` gains one record per workspace
//! this device has ever opened and loses none, so a workspace the user
//! deleted keeps its binding forever. On a dev machine that reached 1,208
//! records, 1,166 of them orphaned and 1,144 pointing at `TempDir` paths
//! from test runs ([#211] item 3, root `CLAUDE.md` invariant 9's fourth
//! question — *what cleans it up?*).
//!
//! ## Why this is not "delete the ones whose root is missing"
//!
//! That rule is the obvious one and it is **wrong on its own**. Dropping a
//! binding is not free: the next open of that workspace mints a *fresh*
//! actor, which is a second `ops-<actor>.jsonl` for a device that already
//! had one, and every op it wrote under the old actor stops being
//! attributed to it. That is precisely the fork the device store exists to
//! prevent, so a GC that guesses wrong causes the bug it is cleaning up
//! after.
//!
//! And "the root is missing" has several innocent causes:
//!
//! - an external drive that is unplugged right now,
//! - a network volume that is not mounted yet,
//! - an iCloud / OneDrive folder that has not been materialized on this
//!   machine,
//! - a workspace the user archived and will restore next week.
//!
//! ## The three conditions, and why all of them
//!
//! A binding is [`BindingVerdict::Stale`] only when **all** of these hold:
//!
//! 1. **The root is gone** — `try_exists` says `false`, not "I could not
//!    tell". Anything unreadable is [`BindingVerdict::Inconclusive`], the
//!    same conservative reading `prior_instance_is_gone` already takes.
//! 2. **The root's parent directory is present.** This is what separates
//!    "the folder was deleted" from "the volume holding it is not here".
//!    An unmounted `/Volumes/Backup` makes `/Volumes/Backup/notes` missing
//!    *and* its parent missing, so the whole mount reads as inconclusive
//!    and every binding on it survives. A deleted `~/notes` leaves `~`
//!    right where it was, so the absence is something we actually
//!    observed rather than something we failed to observe.
//! 3. **The record is older than the TTL.**
//!
//! Condition 2 is the one that does the real work. Condition 3 is
//! narrower than it looks, and the exact shape matters enough to state
//! rather than imply.
//!
//! **The TTL measures the age of the *record*, not the time since the
//! workspace went away.** Age comes from the file's mtime, and
//! `actor_for_instance` only rewrites a binding when its root *moves* —
//! an ordinary open reads and returns. So mtime is "when this device
//! first bound this workspace", and nothing anywhere records when the
//! directory disappeared.
//!
//! The consequence is worth being blunt about: a workspace bound two
//! years ago and deleted this morning is `Stale` immediately. It does
//! **not** get thirty days. What condition 3 actually buys is a floor
//! under *young* bindings — a workspace created and removed inside the
//! window keeps its actor — plus a hedge against a root that briefly
//! reads as absent on a store this device only just started using.
//!
//! That is a deliberately weaker guarantee than "restore from the trash
//! within a month", and the cost of getting it wrong is bounded: a
//! dropped binding forks one extra `ops-<actor>.jsonl` on the next open,
//! and no op is lost, because every reader merges every `ops-*.jsonl` in
//! the directory. Buying the stronger guarantee means writing a
//! `seen=<unix>` stamp on every open, which turns the common read path
//! into a write on a store that may legitimately be read-only. That
//! trade has not been made; `an_old_binding_whose_workspace_just_vanished_is_stale`
//! pins the behaviour so a future change to it is deliberate.
//!
//! ## What is deliberately not covered
//!
//! - **A binding stamped by a foreign machine id** (a store cloned onto
//!   another machine and since reminted) is unusable by this device
//!   forever, so it is provably garbage — but it is *not* pruned here.
//!   Reading it requires the machine id, the rule would have a second
//!   shape, and the case is rare. It falls under the ordinary two
//!   conditions like any other binding.
//! - **`iroh/`, `machine-id`, `actor`, `backups/`.** This module lists and
//!   judges `actors/` and nothing else. `identity.key` **is** this
//!   device's node id and deleting it voids every pairing — the exact
//!   failure the store was moved out of `target/` to prevent.
//!
//! [#211]: https://github.com/outlmd/outl/issues/211

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::{read_record, DeviceError, DeviceStore};

/// How long a leftover scratch file is kept before it counts as debris.
///
/// `record.rs` composes every write in a sibling `.<name>.<pid>.<seq>`
/// file and publishes it with `rename`/`link`, removing the scratch
/// afterwards. A process killed between those two steps leaves the
/// scratch behind, and nothing has ever removed one — the same "what
/// cleans it up?" the bindings had.
///
/// A real in-flight write lives for microseconds, so a day is four orders
/// of magnitude of headroom. It is also **not** load-bearing: deleting a
/// scratch a live writer still holds makes its `hard_link` fail with
/// something other than `AlreadyExists`, which falls through to
/// `exclusive_create` and writes the same record anyway.
pub const STALE_SCRATCH_TTL: Duration = Duration::from_secs(60 * 60 * 24);

/// How long a binding whose root is gone is kept anyway.
///
/// Long enough to cover a restore-from-trash, a drive left in a drawer
/// over a holiday, or a workspace archived between two releases. The cost
/// of waiting is ~190 bytes per entry; the cost of acting early is a
/// forked actor, so the asymmetry sets the direction.
pub const STALE_BINDING_TTL: Duration = Duration::from_secs(60 * 60 * 24 * 30);

/// What the GC concluded about one binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingVerdict {
    /// The workspace directory is right where the binding says it is.
    Live,
    /// The root is gone and its parent is still there, but the binding is
    /// younger than the TTL. A candidate, later.
    RecentlyGone,
    /// The root is gone, its parent is present, and the record is past the
    /// TTL. Safe to drop.
    Stale,
    /// Not enough was readable to say. Covers an unmounted volume, an
    /// unreadable record, a record with no `root=` at all, and a file
    /// whose mtime the platform would not give us.
    ///
    /// **Always keeps the binding.** Guessing about a record we could not
    /// read is how a GC deletes something live.
    Inconclusive,
}

impl BindingVerdict {
    /// Whether this binding may be removed.
    pub fn is_prunable(self) -> bool {
        matches!(self, Self::Stale)
    }
}

/// One record under `<device_dir>/actors/`, with the GC's verdict on it.
#[derive(Debug, Clone)]
pub struct ActorBinding {
    /// The record's own path in the store.
    pub path: PathBuf,
    /// Workspace root the binding names, when it names one.
    ///
    /// The only field a caller needs beyond the verdict: a record is named
    /// by an opaque workspace id, so this is what lets a user recognise
    /// the entry as debris rather than a graph they forgot about.
    pub root: Option<PathBuf>,
    /// Whether it may be dropped, and why not when it may not.
    pub verdict: BindingVerdict,
}

impl DeviceStore {
    /// Every actor binding in the store, judged against `ttl`.
    ///
    /// Reads only; nothing here writes or deletes. The caller decides what
    /// to do with a [`BindingVerdict::Stale`] entry, and
    /// [`DeviceStore::prune_binding`] is the only thing that acts.
    ///
    /// A missing `actors/` directory is an empty list, not an error: a
    /// store that has never bound a workspace is a normal state.
    pub fn actor_bindings(&self, ttl: Duration) -> Result<Vec<ActorBinding>, DeviceError> {
        let dir = self.dir().join("actors");
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(DeviceError::Io { path: dir, source }),
        };

        let now = SystemTime::now();
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            // A scratch file is a half-published write, not a binding, so
            // it never appears here. `stale_scratch` is what collects the
            // ones a crash orphaned.
            if !entry.file_type().is_ok_and(|t| t.is_file()) || is_scratch(&path) {
                continue;
            }
            out.push(judge(&path, now, ttl));
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Leftover scratch files under `actors/` older than `ttl`.
    ///
    /// Read-only, like [`DeviceStore::actor_bindings`]; the caller decides
    /// and [`DeviceStore::prune_scratch`] is what acts.
    ///
    /// These are not bindings and are deliberately kept out of the binding
    /// listing: a scratch file names no workspace, so reporting one as "an
    /// actor binding whose workspace is gone" would be a phantom.
    pub fn stale_scratch(&self, ttl: Duration) -> Result<Vec<PathBuf>, DeviceError> {
        let dir = self.dir().join("actors");
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(DeviceError::Io { path: dir, source }),
        };

        let now = SystemTime::now();
        let mut out: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.path())
            .filter(|p| is_scratch(p) && older_than(p, now, ttl))
            .collect();
        out.sort();
        Ok(out)
    }

    /// Delete one scratch file, after re-checking that it is still stale.
    ///
    /// Same two-pass safety as [`DeviceStore::prune_binding`], and the same
    /// refusal to touch anything outside `actors/`.
    pub fn prune_scratch(&self, path: &Path, ttl: Duration) -> Result<bool, DeviceError> {
        if path.parent() != Some(self.dir().join("actors").as_path())
            || !is_scratch(path)
            || !older_than(path, SystemTime::now(), ttl)
        {
            return Ok(false);
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(DeviceError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Bindings [`DeviceStore::actor_bindings`] judged safe to drop.
    pub fn stale_actor_bindings(&self, ttl: Duration) -> Result<Vec<ActorBinding>, DeviceError> {
        Ok(self
            .actor_bindings(ttl)?
            .into_iter()
            .filter(|b| b.verdict.is_prunable())
            .collect())
    }

    /// Delete one binding, after re-checking that it is still stale.
    ///
    /// The re-check is not ceremony. Listing and pruning are two passes,
    /// and between them the user can plug the drive back in, restore the
    /// folder, or let a sync client materialize it — at which point the
    /// binding is live again and dropping it forks that workspace's actor.
    /// A GC that trusts a verdict it computed a minute ago is a GC that
    /// deletes on stale evidence.
    ///
    /// Returns whether anything was removed. An already-absent file is
    /// `Ok(false)`, not an error.
    pub fn prune_binding(&self, path: &Path, ttl: Duration) -> Result<bool, DeviceError> {
        // `parent ==`, not `starts_with`. `Path::starts_with` compares
        // components without normalising, so `<dir>/actors/../../x`
        // passes it — and this guard is what the crate doc advertises as
        // the thing keeping `iroh/identity.key` safe. Every real binding
        // sits directly in `actors/`, so the stricter test costs nothing.
        if path.parent() != Some(self.dir().join("actors").as_path()) {
            return Ok(false);
        }
        if !judge(path, SystemTime::now(), ttl).verdict.is_prunable() {
            return Ok(false);
        }
        match std::fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(DeviceError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }
}

/// Whether `path` is one of `record.rs`'s in-flight scratch files
/// (`.<name>.<pid>.<seq>.<suffix>`) rather than a binding.
///
/// The dot prefix is the whole test, and it is sound in both directions:
/// `workspace_key` only ever emits `[A-Za-z0-9_-]` or an `h-<hex>` hash,
/// and `instance_key` appends `.<hex>` to that, so no real binding can
/// start with one.
fn is_scratch(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

fn older_than(path: &Path, now: SystemTime, ttl: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age >= ttl)
}

/// The whole policy, in one place so the listing and the prune cannot
/// develop separate opinions about what is safe.
fn judge(path: &Path, now: SystemTime, ttl: Duration) -> ActorBinding {
    // A record the parser had to normalise did not survive the round
    // trip, so the `root` it yields is not the path that was written —
    // and a path that was never written names a directory that does not
    // exist, which is the one verdict that authorises a delete. Drop the
    // root instead of trusting it; `verdict_for` then says
    // `Inconclusive`, which is the answer we want for a record we cannot
    // read faithfully.
    let root = read_record(path)
        .ok()
        .flatten()
        .filter(|r| !r.is_lossy())
        .and_then(|r| r.get("root").map(PathBuf::from));
    let age = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| now.duration_since(t).ok());

    ActorBinding {
        path: path.to_path_buf(),
        verdict: verdict_for(root.as_deref(), age, ttl),
        root,
    }
}

fn verdict_for(root: Option<&Path>, age: Option<Duration>, ttl: Duration) -> BindingVerdict {
    // No `root=` line: a legacy binding written before roots were
    // recorded, or a torn one. Either way there is nothing to check the
    // workspace's existence against, so there is nothing to conclude.
    let Some(root) = root else {
        return BindingVerdict::Inconclusive;
    };
    match root.try_exists() {
        Ok(true) => return BindingVerdict::Live,
        // Permission denied, a broken symlink chain, an I/O error on the
        // volume: we did not observe an absence, we failed to look.
        Err(_) => return BindingVerdict::Inconclusive,
        Ok(false) => {}
    }
    // The absence has to be one we could actually see. If the parent is
    // gone too, the volume or the mount is what is missing, and every
    // binding under it must survive.
    let parent_present = root
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .is_some_and(|p| p.try_exists().unwrap_or(false));
    if !parent_present {
        return BindingVerdict::Inconclusive;
    }
    match age {
        Some(age) if age >= ttl => BindingVerdict::Stale,
        Some(_) => BindingVerdict::RecentlyGone,
        // No mtime means no TTL, and the TTL is the margin that covers
        // what the parent check cannot see. Without it, keep.
        None => BindingVerdict::Inconclusive,
    }
}

#[cfg(test)]
mod tests;
