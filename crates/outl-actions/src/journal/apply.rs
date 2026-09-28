//! The `apply_*` family: render a page and put the projection on disk.
//!
//! Every entry point here renders the same string; they differ only in
//! **when the write is allowed** — unconditionally, only when the `.md`
//! is absent, only when the tree has moved past it, or only when the
//! write deletes nothing the op log cannot account for.
//!
//! The three things they are built out of live next door, one owner
//! each, because each is asked by more than one of them:
//!
//! - `guard` — the verdicts ("may these bytes be overwritten")
//! - `write` — the locked on-disk transaction (`.md` + sidecar)
//! - `mutate` — the `.md`-as-source-of-truth rewrite path
//!
//! `apply_page_md_with_sidecar_guarded` and
//! `apply_page_md_with_sidecar_if_stale` are the two **write gates**,
//! and they run the same guards in the same order for opposite reasons;
//! root `CLAUDE.md` invariant 8 is the contract both implement. Read
//! `guard`'s module doc before touching either.

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::{file_hash, sidecar_path_for};

use super::guard::{
    frontmatter_loss_error, guard_absent_markdown, sidecar_can_answer, unlogged_content_error,
};
use super::paths::{page_md_path, write_md_atomic};
use super::render::render_page_md;
use super::write::{
    write_page_projection, write_page_projection_if_unchanged, write_page_projection_unlocked,
    ProjectionLock,
};
use crate::error::ActionError;
use crate::page::{list_all as list_pages, page_meta};

/// Render `page_root`'s sub-tree and write it to its canonical path
/// under `root`.
pub fn apply_page_md(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<PathBuf, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let md = render_page_md(workspace, page_root);
    let path = page_md_path(root, &meta);
    write_md_atomic(&path, &md)?;
    Ok(path)
}

/// Render the page, write the `.md`, and (re)write its `.outl` sidecar
/// to match the workspace tree exactly.
///
/// This is the call clients use when they want peers to read the
/// projection consistently. Writing `.md` without updating the sidecar
/// is dangerous: a peer running the 3-level matching algorithm would
/// see "different content, old sidecar" and emit phantom `Create` /
/// `Delete` ops in cascade. By regenerating the sidecar from the same
/// workspace tree we just rendered, the peer's matcher sees identical
/// hashes and the reconcile is a no-op.
pub fn apply_page_md_with_sidecar(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<PathBuf, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let md = render_page_md(workspace, page_root);
    write_page_projection(workspace, root, page_root, &meta, &md)
}
/// [`apply_page_md_with_sidecar`], but refusing when the write would
/// delete content the op log has never seen.
///
/// **Why this exists next to the unconditional one.**
/// The re-projection guard in [`apply_page_md_with_sidecar_if_stale`]
/// only covers the *read* paths — opening a page. The background
/// projection writer runs after a real mutation, and it wrote
/// unconditionally, so the very deletion the open path refuses happened
/// anyway on the user's next keystroke commit. Same invariant 8, a door
/// nobody had checked.
///
/// It cannot simply call `_if_stale`: that one declines whenever the
/// `.md` carries an unreconciled external edit, which is exactly the
/// state a page is in *while the user is typing into it*. The write has
/// to happen; what must not happen is losing bytes to it.
///
/// So this asks the single question that matters and nothing else:
/// **does the file hold content the log cannot account for?** If it
/// does, the projection is skipped and [`ActionError::PageMarkdownAheadOfLog`]
/// comes back. The user's edit is not lost — it went through
/// `Workspace::apply` and lives in the op log; only the on-disk
/// projection stays behind, which is the recoverable direction.
///
/// Three outcomes, and the third was missing from the first version:
///
/// - **no `.md` yet** → write; there is nothing on disk to lose.
/// - **sidecar present but cannot answer** (every one written before
///   0.11 carries `text: ""`) → write; refusing here would freeze every
///   pre-0.11 page. See [`sidecar_can_answer`].
/// - **sidecar missing, corrupt, or from a newer binary** → **refuse**,
///   with [`ActionError::PageSidecarUnreadable`]. That is not "nothing
///   at risk", it is "I cannot tell", and writing on it reopens this
///   very door on a different hinge. It is a *different* error from the
///   one above on purpose: "the file holds lines that exist in no op,
///   run `reconcile --ahead-of-log`" names a condition this branch has
///   not established and a recovery that would not apply.
pub fn apply_page_md_with_sidecar_guarded(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<PathBuf, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let path = page_md_path(root, &meta);
    let _lock = ProjectionLock::acquire(&path)?;

    // Absent `.md` → nothing on disk can be lost. An unreadable one is
    // *not* treated as absent: that is how a transient I/O error or an
    // undownloaded iCloud placeholder turns into an overwrite.
    let disk = match std::fs::read_to_string(&path) {
        Ok(disk) => Some(disk),
        // Absent `.md` → nothing on disk can be lost.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        // Present but unreadable is *not* absent: that is how a
        // transient I/O error or an undownloaded iCloud placeholder
        // turns into an overwrite.
        Err(e) => return Err(e.into()),
    };
    // The sidecar's hash, kept for the frontmatter channel below: it is the
    // witness for "the fence on disk is the one the log held".
    let mut last_synced_hash = String::new();
    if let Some(disk) = disk.as_deref() {
        // **An unreadable sidecar is a refusal, not a fall-through.**
        //
        // Missing, corrupt, or written by a newer binary
        // (`UnsupportedVersion`) are three states where "does the log
        // know this line" has one honest answer: *I cannot tell*. The
        // first version of this function used `if let Ok(sidecar)` and
        // wrote anyway on all three — so a page the read path protects
        // was overwritten on the next keystroke commit, which is the
        // door this function was added to close, standing open on a
        // different hinge.
        //
        // `_if_stale` already declines these, and liveness is not at
        // risk: `sync::needs_reconcile` maps `Err(_)` to `true`, so the
        // orphan pass rebuilds the sidecar and the page projects on the
        // pass after.
        //
        // It gets its **own** error rather than borrowing
        // `PageMarkdownAheadOfLog`: that one states as fact that the
        // file holds N lines the log lacks and tells the user to run
        // `outl reconcile --ahead-of-log`. Here neither is established
        // — the refusal is precisely because nothing could be
        // established — and a `lines: 0` with a synthetic sample would
        // reach the banner as a permanent sync failure instead of the
        // transient local condition this is.
        let Ok(sidecar) = outl_md::sidecar::read(&sidecar_path_for(&path)) else {
            return Err(ActionError::PageSidecarUnreadable(
                path.display().to_string(),
            ));
        };
        if let Some(e) = unlogged_content_error(&path, disk, &sidecar.blocks) {
            return Err(e);
        }
        last_synced_hash = sidecar.last_synced_hash;
    }

    let md = render_page_md(workspace, page_root);
    // The frontmatter channel, asked after the render because the render is
    // its reference; see `frontmatter_loss_error`. This is the writer an
    // unreconciled fence edit on disk reaches first (a local mutation lands
    // before the orphan reconcile), so the hash matters here.
    if let Some(disk) = disk.as_deref() {
        if let Some(e) = frontmatter_loss_error(&path, disk, &md, &last_synced_hash) {
            return Err(e);
        }
    }
    write_page_projection_if_unchanged(workspace, root, page_root, &meta, &md, disk.as_deref())
}
/// Like [`apply_page_md_with_sidecar`], but **skips the write when the
/// `.md` file already exists on disk**.
///
/// Use this on read paths (e.g. `open_page_by_slug`) where the goal is
/// to lazily materialise a page that a peer synced into the CRDT tree
/// but never projected to disk on this device.
/// Calling the unconditional variant on every page open would rewrite
/// the `.outl` sidecar on every navigation because `build_sidecar`
/// stamps `last_synced_at: now()` — turning the hottest nav path into
/// constant sync churn even when nothing changed.
///
/// Returns `Some(path)` when the file was absent and was written, or
/// `None` when the file already existed and no I/O was performed.
pub fn apply_page_md_with_sidecar_if_absent(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<Option<PathBuf>, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let path = page_md_path(root, &meta);
    let _lock = ProjectionLock::acquire(&path)?;
    if path.exists() {
        return Ok(None);
    }
    let rendered = render_page_md(workspace, page_root);
    write_page_projection_unlocked(workspace, root, page_root, &meta, &rendered).map(Some)
}

/// Like [`apply_page_md_with_sidecar`], but writes **only when the on-disk
/// `.md` is missing or stale relative to the tree**.
///
/// This is the re-projection counterpart to
/// [`apply_page_md_with_sidecar_if_absent`]: that one only covers an *absent*
/// `.md` (a page synced into the tree but never projected here — issue #120).
/// It leaves a page **projected empty before its content synced** stale
/// forever: the file then exists, so the `_if_absent` guard skips it, and the
/// view — which reads the `.md` via [`crate::outline::read_page_outline`] —
/// keeps rendering blank even though the tree holds the blocks. That is the
/// "day created on one device shows empty on another" bug.
///
/// Five cases:
/// - `.md` absent **and nothing says otherwise** → project it (subsumes
///   `_if_absent`, issue #120).
/// - `.md` absent but an iCloud placeholder or a live sidecar says its
///   bytes exist and are not here yet → refuse
///   (`ActionError::PageMarkdownNotDownloaded` /
///   `PageMarkdownVanished`). See `guard_absent_markdown`.
/// - `.md` present and a **faithful projection** (its hash matches the
///   sidecar's `last_synced_hash`, i.e. no unreconciled external edit) but the
///   tree now renders to something different → re-project it. This is the sync
///   case the bug lives in.
/// - `.md` present but **not** matching its sidecar → an external edit is
///   pending; leave it untouched (`.md → tree` reconcile owns that), so this
///   never clobbers a hand-edited file.
/// - `.md` present, hash-faithful, tree ahead — but the file holds content
///   that exists in no op, or its sidecar cannot answer whether it does →
///   refuse (`ActionError::PageMarkdownAheadOfLog`) or leave it alone. The
///   hash proves outl wrote these bytes, never that the log holds them; see
///   root `CLAUDE.md` invariant 8.
///
/// Only writes on a real change, so it does not churn the sidecar's
/// `last_synced_at` on a page already in sync.
///
/// Returns `Some(path)` when it (re)projected, `None` when it left disk alone.
pub fn apply_page_md_with_sidecar_if_stale(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Result<Option<PathBuf>, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    let path = page_md_path(root, &meta);
    let _lock = ProjectionLock::acquire(&path)?;
    let disk = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // **`NotFound` is not the same question as "this page has no
            // `.md`".** An undownloaded iCloud file answers `NotFound`
            // too: the real name does not exist, only `.notes.md.icloud`
            // does. So does a `.md` that went missing beside a live
            // sidecar. `guard_absent_markdown` is the single owner of
            // that distinction and had exactly one caller
            // (`mutate_page_md`); this door was never checked, which was
            // survivable only while `_if_stale` ran on page-open with a
            // user present. `outl serve`'s sweep runs it over every page,
            // unattended, every 30 seconds.
            guard_absent_markdown(&path, &sidecar_path_for(&path))?;
            // Genuinely absent → project it (issue #120). The page lock is
            // already held, so call the unlocked transaction tail directly.
            let rendered = render_page_md(workspace, page_root);
            return write_page_projection_unlocked(workspace, root, page_root, &meta, &rendered)
                .map(Some);
        }
        // Present but unreadable (permissions, non-UTF8, …): do NOT treat as
        // absent — re-projecting would clobber a file that may hold an external
        // edit. Surface the error; the caller logs and leaves the file alone.
        Err(e) => return Err(e.into()),
    };
    let disk_hash = file_hash(&disk);
    // Read the sidecar **once**: the hash gate below needs `last_synced_hash`
    // and the unlogged-content check needs `blocks`, and nothing holds a lock
    // between them. Reading it twice leaves a window where a peer's sync, the
    // background projection writer, or an `outl serve` in the same folder
    // rewrites the file mid-call, so the hash that authorised the write and
    // the blocks that vetted it would describe different revisions. Same
    // defect `reconcile_md` was fixed for.
    let sidecar_path = sidecar_path_for(&path);
    // Only re-project a file that is a faithful projection of the tree its
    // sidecar was built from. A `.md` whose hash no longer matches its sidecar
    // carries an external edit — that is the orphan reconcile's job
    // (`.md → tree`); re-projecting here would clobber it. No readable sidecar
    // means the same thing: nothing establishes that outl wrote these bytes.
    let Ok(sidecar) = outl_md::sidecar::read(&sidecar_path) else {
        return Ok(None);
    };
    if sidecar.last_synced_hash != disk_hash {
        // **One exception: the empty hash is not a stale projection, it
        // is a withheld one.**
        //
        // `reconcile_md` writes `last_synced_hash = ""` when it read
        // content it could not turn into ops (invariant 8). Every gate
        // downstream tests hash-equality, so without this arm the page
        // that the producer flagged is the one page the user is never
        // told about: no `PageMarkdownAheadOfLog`, so no banner — the
        // fix erasing the signal the same release built.
        //
        // Asking the content question here costs one comparison on a
        // page that is already known to need attention, and it is the
        // honest answer: the page really does hold lines the log lacks.
        if sidecar.last_synced_hash.is_empty() && sidecar_can_answer(&sidecar.blocks) {
            if let Some(e) = unlogged_content_error(&path, &disk, &sidecar.blocks) {
                return Err(e);
            }
        }
        return Ok(None);
    }
    // The tree has moved past the projection iff rendering it now differs from
    // what is on disk. Render once and reuse it for the write below.
    let rendered = render_page_md(workspace, page_root);
    if file_hash(&rendered) == disk_hash {
        return Ok(None);
    }
    // The hash gate above proves the sidecar agrees with these bytes — it
    // does NOT prove the bytes came from the op log. A `reconcile_md` that rewrote
    // the sidecar without emitting ops for everything it read leaves a
    // page in exactly that state, and re-rendering the tree over it drops
    // the difference for good.
    //
    // The question is "does the op log know this line", and the sidecar
    // is what answers it: its blocks are what the log held at the last
    // agreement. Asking the *render* instead answers a different
    // question, "do disk and tree disagree", which every remote edit and
    // every remote delete also answers yes to — that version of this
    // guard froze any page a peer had touched, reintroducing #166 for
    // the most ordinary sync case there is.
    //
    // A sidecar that cannot answer at all does not get to authorise the
    // write either — see `sidecar_can_answer`.
    if !sidecar_can_answer(&sidecar.blocks) {
        return Ok(None);
    }
    // Same verdict as the post-mutation guard, phrased once — see
    // `unlogged_content_error`. The policy split is above: this path
    // declines a sidecar that cannot answer, that one writes anyway.
    if let Some(e) = unlogged_content_error(&path, &disk, &sidecar.blocks) {
        return Err(e);
    }
    // And the frontmatter channel, which the block list above cannot answer
    // for — see `frontmatter_loss_error`. Without it this is the one gate a
    // page whose fence the log does not know walks straight through, and
    // `outl serve`'s sweep runs it over every page, unattended.
    if let Some(e) = frontmatter_loss_error(&path, &disk, &rendered, &sidecar.last_synced_hash) {
        return Err(e);
    }
    write_page_projection_if_unchanged(workspace, root, page_root, &meta, &rendered, Some(&disk))
        .map(Some)
}
/// Render **every** page in the workspace to its `.md` file. Useful
/// after a workspace-wide change (sync pull, migration, …) when we
/// don't know which pages actually moved.
///
/// Each page uses the post-mutation guard: a bulk plugin mutation may
/// advance the tree, but it must never overwrite content ahead of the log.
/// Pages are independent, so the pass continues after a refusal and returns
/// every success and failure in [`ProjectionSweep`].
pub fn apply_all_pages_md(workspace: &Workspace, root: &Path) -> ProjectionSweep {
    let mut report = ProjectionSweep::default();
    for meta in list_pages(workspace) {
        let result = parse_node_id(&meta.id)
            .and_then(|id| apply_page_md_with_sidecar_guarded(workspace, root, id));
        match result {
            Ok(path) => report.written.push(path),
            Err(error) => report.failures.push(ProjectionFailure {
                path: page_md_path(root, &meta),
                error,
            }),
        }
    }
    report
}

/// Non-atomic result of a workspace-wide projection pass.
///
/// Pages are independent projections, so one refusal must not leave every page
/// after it stale. Callers surface `failures` separately from the plugin ops
/// that were already committed.
#[derive(Debug, Default)]
pub struct ProjectionSweep {
    /// `.md` paths the pass re-projected.
    pub written: Vec<PathBuf>,
    /// Pages the pass could not project, each with the guard's refusal.
    pub failures: Vec<ProjectionFailure>,
}

/// One page a [`ProjectionSweep`] left untouched, and why.
#[derive(Debug)]
pub struct ProjectionFailure {
    /// The `.md` path that was not rewritten.
    pub path: PathBuf,
    /// The refusal, typically [`ActionError::PageMarkdownAheadOfLog`] or
    /// [`ActionError::PageSidecarUnreadable`].
    pub error: ActionError,
}

fn parse_node_id(s: &str) -> Result<NodeId, ActionError> {
    use std::str::FromStr;
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|e| ActionError::NotInTree(format!("invalid id {s}: {e}")))
}
