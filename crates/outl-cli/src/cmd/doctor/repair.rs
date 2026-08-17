//! `outl doctor --repair` — the narrow set of fixes that cannot lose
//! data.
//!
//! ## What repair is allowed to touch
//!
//! Exactly three things, all of them *projections* or *caches*:
//!
//! 1. Re-project a page's `.md` (+ its sidecar) from the op log, when
//!    the file on disk is absent or is a stale-but-faithful projection.
//!    The op log is the source of truth (root `CLAUDE.md` invariant 1);
//!    a `.md` that disagrees with it is by definition the wrong side.
//! 2. Rebuild a missing sidecar for a `.md` whose bytes already equal
//!    what the tree renders.
//! 3. Delete a snapshot we read end-to-end and then failed to decode. It
//!    is a pure boot cache; the next boot rebuilds it from a full
//!    replay. A snapshot that could not be *read* is not this — see
//!    `oplog::check_snapshots`.
//!
//! Plus one piece of housekeeping over its own output: pruning stale
//! `.outl/repair-backup/` generations (below).
//!
//! ## What gates 1 and 2
//!
//! Both write a projection **rendered from the materialized tree**, and
//! the tree is only ever as complete as the op log that built it. When
//! the doctor found the log damaged, `collect_internal` clears those two
//! lists before this pass ever sees them, so nothing here can overwrite
//! a good `.md` with the render of a truncated replay. See
//! `oplog::OpLogHealth`.
//!
//! ## What repair will never do
//!
//! - Delete a `.md`. Ever. Even an orphaned one.
//! - Touch `ops/` — not a byte, not a file, not a line. A corrupt line
//!   in the op log is *reported*, never rewritten: we cannot know what
//!   the lost op said, and a "repair" that drops it would silently
//!   rewrite history on one device and diverge it from its peers.
//! - Move anything to the trash, or out of it.
//! - Merge or delete a sync-conflict copy. Choosing a winner between
//!   two forks of a page is the user's call.
//!
//! ## Reversibility
//!
//! Every file repair touches is copied to
//! `.outl/repair-backup/<timestamp>/<relative path>` **before** the
//! write. Restoring is a plain `cp` back. The backup dir is under
//! `.outl/` so it never becomes a page.
//!
//! ## Why the backups are pruned
//!
//! `.outl/` is dot-prefixed, so iCloud Documents drops it and iroh never
//! ships it — but Syncthing, Dropbox and a shared network volume all
//! replicate it happily. An unbounded pile of generations there is a
//! full copy of every `.md` a repair ever touched, forever, on every
//! peer. On the 66k-block graphs this command exists for, a handful of
//! `--repair` runs is gigabytes.
//!
//! So the pruning follows the same shape `.outl/reminders-fired.json`
//! already uses for its device-local state: keep it in `.outl/`, and put
//! a TTL on it. Two guards, and a generation has to fail **both** before
//! it goes: it must be older than [`BACKUP_MIN_AGE_DAYS`] *and* outside
//! the newest [`BACKUP_KEEP`]. Deleting a user's only undo is worse than
//! keeping a stale one, so recency wins over the age rule and every
//! prune is reported as its own [`RepairAction`].

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use outl_core::device::{ActorBinding, DeviceStore, STALE_BINDING_TTL, STALE_SCRATCH_TTL};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use serde::Serialize;

/// How many `--repair` backup generations always survive a prune,
/// however old they are.
const BACKUP_KEEP: usize = 10;

/// Minimum age before a backup generation is even a prune candidate.
const BACKUP_MIN_AGE_DAYS: u64 = 14;

/// Content lines one `--repair` may remove across the whole workspace
/// before it stops and asks.
///
/// On a healthy workspace this number is **zero**: re-projection exists
/// to restore a render the tree already agrees with, not to remove text.
/// A non-zero total means a peer genuinely deleted blocks — or that
/// something systemic is wrong, which is what [RFC 0210] measured: 1,426
/// lines across 233 pages, deleted in one pass that printed `708 fixed`.
/// A hundred lines is comfortably above ordinary peer deletes and an
/// order of magnitude below that incident.
///
/// [RFC 0210]: ../../../../../docs/rfcs/0210-md-content-outside-op-log.md
const CONFIRM_ABOVE_LINES: usize = 100;

/// Pages that would *lose content* before `--repair` stops and asks.
///
/// Counted separately from the line total because the two failures look
/// different: one page losing 400 lines is a big page, while 40 pages
/// losing 3 lines each is a pattern. Pages the write only *adds* to — the
/// whole graph on a device that just paired and has nothing projected yet
/// — are not counted at all, so the ordinary bulk case never asks for a
/// flag it does not need.
const CONFIRM_ABOVE_PAGES: usize = 20;

/// One page the repair pass would rewrite, measured **before** the write.
#[derive(Debug, Clone)]
pub(super) struct PageWrite {
    /// Page root node in the materialized tree.
    pub page_root: NodeId,
    /// Path of the `.md` the repair would write.
    pub path: PathBuf,
    /// Content lines on disk today that the new projection does not
    /// reproduce — what this one write removes.
    pub lines_removed: usize,
}

impl PageWrite {
    /// A write that cannot remove anything: the `.md` is absent, or its
    /// bytes already equal what the tree renders.
    pub fn additive(page_root: NodeId, path: PathBuf) -> Self {
        Self {
            page_root,
            path,
            lines_removed: 0,
        }
    }
}

/// How much content a `--repair` pass would remove, totalled across the
/// workspace.
///
/// Exists so the count is available *before* `run` is called. A
/// destructive operation that reports its scale only afterwards — `708
/// fixed`, with no mention of the 1,426 lines that went with it — cannot
/// be consented to.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct RepairVolume {
    /// Pages whose rewrite removes at least one content line.
    pub pages_losing_content: usize,
    /// Total content lines removed across those pages.
    pub lines_removed: usize,
}

impl RepairVolume {
    /// Whether this volume needs an explicit opt-in.
    pub fn needs_confirmation(&self) -> bool {
        self.lines_removed > CONFIRM_ABOVE_LINES || self.pages_losing_content > CONFIRM_ABOVE_PAGES
    }

    /// Whether there is any content loss at all to announce.
    pub fn is_destructive(&self) -> bool {
        self.lines_removed > 0
    }

    /// The ceilings, for a message that has to name them.
    pub fn ceilings() -> (usize, usize) {
        (CONFIRM_ABOVE_PAGES, CONFIRM_ABOVE_LINES)
    }
}

/// What the repair pass is allowed to act on, collected by the checks.
#[derive(Debug, Default)]
pub(super) struct Plan {
    /// Pages whose `.md` is missing or a stale faithful projection.
    pub reproject: Vec<PageWrite>,
    /// Pages whose `.md` already matches the tree but has no sidecar.
    pub rebuild_sidecar: Vec<PageWrite>,
    /// Snapshot files that failed to decode.
    pub corrupt_snapshots: Vec<PathBuf>,
    /// `.outl/repair-backup/` generations past **both** prune guards.
    ///
    /// Collected here rather than discovered inside [`run`] so the prune
    /// is announced like every other action and so a workspace whose
    /// only remaining work *is* a prune still counts as repairable —
    /// otherwise a healthy graph keeps every generation forever, which
    /// is the case the pruning exists for.
    pub prune_backups: Vec<PathBuf>,
    /// Device-store actor bindings whose workspace is provably gone.
    ///
    /// The only entry in this plan that lives **outside** the workspace
    /// (`<device_dir>/actors/`), and the reason it is here rather than in
    /// its own command is that `doctor` is the surface users already run
    /// when something is off. The device store is machine-global, so a
    /// run from workspace A can prune a binding that belonged to
    /// workspace B — which is why every entry is named individually in
    /// [`Plan::describe`] instead of being reported as a count.
    ///
    /// `outl-core` owns the "is this safe to drop" question entirely
    /// ([`outl_core::device::BindingVerdict`]); nothing here re-derives
    /// it, and the prune re-asks before deleting.
    pub prune_bindings: Vec<ActorBinding>,
    /// Scratch files a killed process left half-published in the device
    /// store. Not bindings, not backed up: a scratch file is by
    /// definition content that never became a record.
    pub prune_scratch: Vec<PathBuf>,
}

impl Plan {
    /// Whether there is anything at all for `--repair` to do.
    pub fn is_empty(&self) -> bool {
        self.reproject.is_empty()
            && self.rebuild_sidecar.is_empty()
            && self.corrupt_snapshots.is_empty()
            && self.prune_backups.is_empty()
            && self.prune_bindings.is_empty()
            && self.prune_scratch.is_empty()
    }

    /// How much content the page writes in this plan would remove.
    ///
    /// Only the page writes are measured: dropping a corrupt snapshot
    /// and pruning a backup generation delete files by design, and both
    /// are already bounded and announced by their own rules.
    ///
    /// `rebuild_sidecar` contributes nothing because its precondition is
    /// that the `.md` bytes already equal the render — it rewrites the
    /// file byte-identical.
    pub fn volume(&self) -> RepairVolume {
        let losing = self.reproject.iter().filter(|p| p.lines_removed > 0);
        RepairVolume {
            pages_losing_content: losing.clone().count(),
            lines_removed: losing.map(|p| p.lines_removed).sum(),
        }
    }

    /// Human-readable lines describing what a repair *would* do. Printed
    /// both in the dry (default) mode and before acting under `--repair`,
    /// so the two views never drift.
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        for page in &self.reproject {
            // The line count rides on the same line that offers the
            // action, so "what will this cost me" and "what is this"
            // cannot be read apart.
            let cost = match page.lines_removed {
                0 => "adds or reorders only, removes nothing".to_string(),
                n => format!("removes {n} content line(s) from disk"),
            };
            out.push(format!(
                "re-project {} from the op log — {cost} (backup first)",
                page.path.display()
            ));
        }
        for page in &self.rebuild_sidecar {
            out.push(format!(
                "rebuild the sidecar next to {} (content unchanged)",
                page.path.display()
            ));
        }
        for path in &self.corrupt_snapshots {
            out.push(format!(
                "delete corrupt snapshot {} (pure cache, rebuilt on next boot)",
                path.display()
            ));
        }
        for path in &self.prune_backups {
            out.push(format!(
                "prune stale backup generation {} (older than {BACKUP_MIN_AGE_DAYS} days and \
                 outside the newest {BACKUP_KEEP})",
                path.display()
            ));
        }
        for binding in &self.prune_bindings {
            // The workspace path, not the record's filename. A record is
            // named by an opaque workspace id; `root=` is the only field
            // that lets a user recognise the entry as debris from a test
            // run rather than a graph they forgot about.
            // The TTL is the *record's* age (its file mtime), not the time
            // since the workspace disappeared — a years-old binding whose
            // directory vanished today is eligible immediately, so the text
            // must not promise a grace period after the deletion.
            out.push(format!(
                "drop the device-store actor binding for {}: its directory is gone and the \
                 binding record itself is older than {} days ({})",
                // A binding with no recorded root is never `Stale` (it is
                // `Inconclusive` — nothing to check existence against), so
                // this list always has one; falling back to the record's
                // own path keeps that assumption out of the type system.
                binding.root.as_deref().unwrap_or(&binding.path).display(),
                STALE_BINDING_TTL.as_secs() / 86_400,
                binding.path.display()
            ));
        }
        for path in &self.prune_scratch {
            out.push(format!(
                "delete the abandoned device-store scratch file {} (a write that was \
                 killed before it published, untouched for over {} hour(s))",
                path.display(),
                STALE_SCRATCH_TTL.as_secs() / 3_600
            ));
        }
        out
    }
}

/// One repair attempt, as reported to the user and to `--json`.
#[derive(Debug, Clone, Serialize)]
pub struct RepairAction {
    /// `reproject` | `rebuild_sidecar` | `delete_snapshot` |
    /// `prune_backup` | `prune_binding`.
    pub kind: String,
    /// Path the action targeted.
    pub path: String,
    /// Whether it succeeded.
    pub ok: bool,
    /// Failure reason, or the backup path on success.
    pub detail: String,
}

/// Outcome of a `--repair` pass.
#[derive(Debug, Clone, Serialize)]
pub struct RepairReport {
    /// Directory every touched file was copied into first.
    pub backup_dir: String,
    /// What was attempted, in execution order.
    pub actions: Vec<RepairAction>,
    /// How many actions succeeded.
    pub repaired: usize,
    /// How many actions failed.
    pub failed: usize,
}

/// Execute `plan`.
///
/// Never returns `Err`: a failure on one file is recorded in its
/// [`RepairAction`] and the pass continues, because a permission error
/// on one page must not block re-projecting the other 200.
pub(super) fn run(ws: &Workspace, root: &Path, plan: &Plan, store: &DeviceStore) -> RepairReport {
    let stamp = chrono::Local::now().format("%Y%m%dT%H%M%S").to_string();
    let backup_dir = root.join(".outl").join("repair-backup").join(&stamp);
    let mut actions: Vec<RepairAction> = Vec::new();

    for page in &plan.reproject {
        actions.push(reproject(ws, root, &backup_dir, page));
    }
    for page in &plan.rebuild_sidecar {
        actions.push(rebuild_sidecar(ws, root, &backup_dir, page));
    }
    for path in &plan.corrupt_snapshots {
        actions.push(delete_snapshot(root, &backup_dir, path));
    }
    for binding in &plan.prune_bindings {
        actions.push(prune_binding(store, &backup_dir, binding));
    }
    for path in &plan.prune_scratch {
        actions.push(prune_scratch(store, path));
    }
    // Last, so a binding backup written above is never a candidate for
    // the generation prune in the same run.
    actions.extend(prune_backups(&plan.prune_backups));

    let repaired = actions.iter().filter(|a| a.ok).count();
    let failed = actions.len() - repaired;
    RepairReport {
        backup_dir: backup_dir.display().to_string(),
        actions,
        repaired,
        failed,
    }
}

/// Re-project one page, backing up the `.md` + sidecar first.
///
/// The write itself is `outl_actions::apply_page_md_with_sidecar_if_stale`,
/// which re-runs the "is this a faithful projection?" test and refuses to
/// clobber a `.md` carrying an unreconciled external edit. Reusing it
/// means the doctor cannot develop its own opinion about when a
/// projection is safe to overwrite.
fn reproject(ws: &Workspace, root: &Path, backup_dir: &Path, page: &PageWrite) -> RepairAction {
    let (page_root, path) = (page.page_root, page.path.as_path());
    let sidecar = outl_md::sidecar::sidecar_path_for(path);
    let mut backed_up = Vec::new();
    for f in [path, sidecar.as_path()] {
        match backup(root, backup_dir, f) {
            Ok(Some(dest)) => backed_up.push(dest.display().to_string()),
            Ok(None) => {}
            Err(e) => {
                return RepairAction {
                    kind: "reproject".to_string(),
                    path: path.display().to_string(),
                    ok: false,
                    detail: format!("backup failed, nothing written: {e}"),
                }
            }
        }
    }

    match outl_actions::apply_page_md_with_sidecar_if_stale(ws, root, page_root) {
        Ok(Some(written)) => RepairAction {
            kind: "reproject".to_string(),
            path: written.display().to_string(),
            ok: true,
            detail: if backed_up.is_empty() {
                "written (nothing on disk to back up)".to_string()
            } else {
                format!("backup: {}", backed_up.join(", "))
            },
        },
        Ok(None) => RepairAction {
            kind: "reproject".to_string(),
            path: path.display().to_string(),
            ok: false,
            detail: "skipped — the `.md` carries an unreconciled external edit; \
                     run `outl reconcile` first"
                .to_string(),
        },
        Err(e) => RepairAction {
            kind: "reproject".to_string(),
            path: path.display().to_string(),
            ok: false,
            detail: format!("{e}"),
        },
    }
}

/// Rebuild the sidecar for a `.md` whose bytes already equal what the
/// tree renders.
///
/// `apply_page_md_with_sidecar_if_stale` is the wrong tool here: it
/// treats a *missing* sidecar as "not a faithful projection" and
/// refuses to write, which is right for its own callers (they can't
/// tell a lost sidecar from an external edit) and wrong for us. So this
/// re-derives the one fact that makes the write safe — the on-disk
/// bytes are exactly what the op log renders — and only then calls the
/// unconditional writer. If the file changed between the check and now,
/// it bails.
fn rebuild_sidecar(
    ws: &Workspace,
    root: &Path,
    backup_dir: &Path,
    page: &PageWrite,
) -> RepairAction {
    let (page_root, path) = (page.page_root, page.path.as_path());
    let action = |ok: bool, detail: String| RepairAction {
        kind: "rebuild_sidecar".to_string(),
        path: path.display().to_string(),
        ok,
        detail,
    };

    let disk = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => return action(false, format!("unreadable: {e}")),
    };
    let rendered = outl_actions::render_page_md(ws, page_root);
    if outl_md::sidecar::file_hash(&disk) != outl_md::sidecar::file_hash(&rendered) {
        return action(
            false,
            "skipped — the `.md` no longer matches the op log; run `outl reconcile`".to_string(),
        );
    }
    let backed_up = match backup(root, backup_dir, path) {
        Ok(Some(dest)) => dest.display().to_string(),
        Ok(None) => "nothing to back up".to_string(),
        Err(e) => return action(false, format!("backup failed, nothing written: {e}")),
    };
    match outl_actions::apply_page_md_with_sidecar(ws, root, page_root) {
        Ok(_) => action(true, format!("backup: {backed_up}")),
        Err(e) => action(false, format!("{e}")),
    }
}

/// Move a corrupt snapshot into the backup dir instead of unlinking it,
/// so "delete" stays reversible even for a file we believe is garbage.
fn delete_snapshot(root: &Path, backup_dir: &Path, path: &Path) -> RepairAction {
    match backup(root, backup_dir, path) {
        Ok(Some(dest)) => match std::fs::remove_file(path) {
            Ok(()) => RepairAction {
                kind: "delete_snapshot".to_string(),
                path: path.display().to_string(),
                ok: true,
                detail: format!("backup: {}", dest.display()),
            },
            Err(e) => RepairAction {
                kind: "delete_snapshot".to_string(),
                path: path.display().to_string(),
                ok: false,
                detail: format!("backed up but could not delete: {e}"),
            },
        },
        Ok(None) => RepairAction {
            kind: "delete_snapshot".to_string(),
            path: path.display().to_string(),
            ok: false,
            detail: "already gone".to_string(),
        },
        Err(e) => RepairAction {
            kind: "delete_snapshot".to_string(),
            path: path.display().to_string(),
            ok: false,
            detail: format!("backup failed, file left alone: {e}"),
        },
    }
}

/// Which backup generations are past **both** guards.
///
/// Split out from the I/O so the policy is testable without backdating a
/// directory's mtime — which needs a crate this binary does not carry,
/// and which would make the one test that matters here (a young backup
/// survives) depend on the filesystem's timestamp resolution.
///
/// Sorting is lexicographic on the path, which is exactly chronological
/// because the stamp is `%Y%m%dT%H%M%S`. The age check still comes off
/// `mtime` rather than the parsed name: a directory whose name we cannot
/// interpret must not become a deletion candidate by default, and
/// `None` (unreadable metadata) keeps it for the same reason. Erring
/// toward keeping costs disk; erring toward deleting costs the user
/// their only undo.
pub(super) fn prunable_backups(
    generations: &mut [(PathBuf, Option<SystemTime>)],
    now: SystemTime,
) -> Vec<PathBuf> {
    if generations.len() <= BACKUP_KEEP {
        return Vec::new();
    }
    generations.sort();
    let surplus = generations.len() - BACKUP_KEEP;
    let ttl = Duration::from_secs(BACKUP_MIN_AGE_DAYS * 24 * 60 * 60);
    generations
        .iter()
        .take(surplus)
        .filter(|(_, mtime)| {
            mtime
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age >= ttl)
        })
        .map(|(path, _)| path.clone())
        .collect()
}

/// Which generations under `<root>/.outl/repair-backup/` are past both
/// guards, as of now.
///
/// Called during collection, so the answer lands in [`Plan`] and is
/// listed by [`Plan::describe`] before anything is deleted. Running it
/// there also means the generation this run is about to write does not
/// exist yet and cannot prune itself.
pub(super) fn collect_prunable(root: &Path) -> Vec<PathBuf> {
    let dir = root.join(".outl").join("repair-backup");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut generations: Vec<(PathBuf, Option<SystemTime>)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| {
            let mtime = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, mtime)
        })
        .collect();

    prunable_backups(&mut generations, SystemTime::now())
}

/// Copy one binding into the backup generation, then drop it.
///
/// The backup lands under `device-store/actors/` inside this run's
/// generation. Putting device-global state in one workspace's backup dir
/// is deliberate: the generation records *what this repair run removed*,
/// which is the thing a user needs to undo it, and it inherits the
/// existing generational prune instead of growing a second pile that
/// would need its own GC (root `CLAUDE.md` invariant 9, fourth question).
///
/// Restoring is a plain `cp` back into `<device_dir>/actors/`.
fn prune_binding(store: &DeviceStore, backup_dir: &Path, binding: &ActorBinding) -> RepairAction {
    let action = |ok, detail| RepairAction {
        kind: "prune_binding".to_string(),
        path: binding.path.display().to_string(),
        ok,
        detail,
    };
    let dest = backup_dir
        .join("device-store")
        .join("actors")
        .join(binding.path.file_name().unwrap_or_default());

    if let Err(e) = std::fs::create_dir_all(dest.parent().unwrap_or(backup_dir))
        .and_then(|()| std::fs::copy(&binding.path, &dest).map(|_| ()))
    {
        return action(false, format!("backup failed, nothing removed: {e}"));
    }

    let pruned = store.prune_binding(&binding.path, STALE_BINDING_TTL);
    if !matches!(pruned, Ok(true)) {
        // The generation records what this run *removed*; a backup of a
        // record still sitting in the store is clutter that reads like a
        // deletion that happened.
        let _ = std::fs::remove_file(&dest);
    }
    match pruned {
        Ok(true) => action(true, format!("backed up to {}", dest.display())),
        // `DeviceStore::prune_binding` re-judges before deleting, and
        // refused: the workspace came back between the collection and
        // now, so it keeps its actor. Nothing is wrong — the evidence
        // simply expired — so the run still succeeded.
        Ok(false) => action(
            true,
            "left in place — its workspace is reachable again".to_string(),
        ),
        Err(e) => action(false, format!("could not remove: {e}")),
    }
}

/// Delete one abandoned scratch file.
///
/// Not backed up, and that is the one way it differs from every other
/// action here. A scratch file is a write that never published — its
/// content became a record or it did not, and if it did, the record is
/// already there. Copying it would archive a fragment of nothing.
fn prune_scratch(store: &DeviceStore, path: &Path) -> RepairAction {
    let (kind, shown) = ("prune_scratch".to_string(), path.display().to_string());
    match store.prune_scratch(path, STALE_SCRATCH_TTL) {
        Ok(true) => RepairAction {
            kind,
            path: shown,
            ok: true,
            detail: "half-published write, never became a record".to_string(),
        },
        // Re-judged and refused: something touched it since the listing,
        // so it may belong to a live writer after all.
        Ok(false) => RepairAction {
            kind,
            path: shown,
            ok: true,
            detail: "left in place — it was touched since the scan".to_string(),
        },
        Err(e) => RepairAction {
            kind,
            path: shown,
            ok: false,
            detail: format!("could not remove: {e}"),
        },
    }
}

/// Delete what [`collect_prunable`] selected.
///
/// Runs at the tail of every `--repair`, so the pile is bounded by the
/// same command that grows it and nothing has to be cleaned up by hand.
fn prune_backups(selected: &[PathBuf]) -> Vec<RepairAction> {
    selected
        .iter()
        .map(|path| match std::fs::remove_dir_all(path) {
            Ok(()) => RepairAction {
                kind: "prune_backup".to_string(),
                path: path.display().to_string(),
                ok: true,
                detail: format!(
                    "older than {BACKUP_MIN_AGE_DAYS} days and outside the newest \
                     {BACKUP_KEEP} generations"
                ),
            },
            Err(e) => RepairAction {
                kind: "prune_backup".to_string(),
                path: path.display().to_string(),
                ok: false,
                detail: format!("could not remove: {e}"),
            },
        })
        .collect()
}

/// Copy `file` into `backup_dir`, preserving its path relative to the
/// workspace root. `Ok(None)` when there was nothing to copy.
fn backup(root: &Path, backup_dir: &Path, file: &Path) -> std::io::Result<Option<PathBuf>> {
    if !file.exists() {
        return Ok(None);
    }
    let rel = file.strip_prefix(root).unwrap_or(file);
    // An absolute `rel` (file outside the workspace — shouldn't happen,
    // but `strip_prefix` failing would produce one) would make `join`
    // discard `backup_dir` entirely and overwrite the original.
    let rel: PathBuf = rel
        .components()
        .filter(|c| matches!(c, std::path::Component::Normal(_)))
        .collect();
    let dest = backup_dir.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(file, &dest)?;
    Ok(Some(dest))
}
