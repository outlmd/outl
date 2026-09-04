//! Write `.md` + `.outl` projections to disk — the `apply_*` family,
//! `mutate_page_md`, and the workspace-wide sweep.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::{content_hash, file_hash, sidecar_path_for, Sidecar, SidecarBlock};

use super::paths::{page_md_path, write_md_atomic};
use super::render::render_page_md;
use crate::error::ActionError;
use crate::page::{list_all as list_pages, page_meta, PageMeta};
use crate::tree::children_of;

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

/// `Some(error)` when re-projecting over `disk` would delete content the
/// op log cannot account for.
///
/// The one place that phrases this verdict as an `ActionError`, so the
/// two callers cannot drift on what the message says or which fields it
/// carries.
///
/// **`None` covers two different situations on purpose**, and the caller
/// decides what each one means:
///
/// - nothing is at risk;
/// - the sidecar cannot answer at all (every one written before 0.11).
///
/// [`apply_page_md_with_sidecar_guarded`] treats both as "go ahead" —
/// there is a real mutation to project and refusing every pre-0.11 page
/// would freeze the app. [`apply_page_md_with_sidecar_if_stale`] asks
/// [`sidecar_can_answer`] *first* and declines the second case, because
/// re-projecting a page it cannot vouch for is how bytes go missing.
/// Reading one policy as the other is the bug this whole module guards.
fn unlogged_content_error(path: &Path, disk: &str, blocks: &[SidecarBlock]) -> Option<ActionError> {
    if !sidecar_can_answer(blocks) {
        return None;
    }
    let unlogged = content_lines_missing_from(disk, blocks);
    let sample = unlogged.first()?;
    Some(ActionError::PageMarkdownAheadOfLog {
        path: path.display().to_string(),
        lines: unlogged.len(),
        sample: format!("{sample:?}"),
    })
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
        if let Some(e) = unlogged_content_error(&path, &disk, &sidecar.blocks) {
            return Err(e);
        }
    }

    let md = render_page_md(workspace, page_root);
    write_page_projection_if_unchanged(workspace, root, page_root, &meta, &md, disk.as_deref())
}

/// Cross-process serialization for a page's guarded check-and-write.
///
/// The stable sibling stays locked while the atomic write replaces the
/// `.md` inode, closing the check-to-rename window between outl processes.
pub(crate) struct ProjectionLock {
    file: File,
}

impl ProjectionLock {
    pub(crate) fn acquire(md_path: &Path) -> Result<Self, ActionError> {
        let parent = md_path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "page path has no parent")
        })?;
        std::fs::create_dir_all(parent)?;
        let name = md_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid page filename")
            })?;
        let lock_path = md_path.with_file_name(format!(".{name}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(lock_path)?;
        file.lock_exclusive()?;
        Ok(Self { file })
    }
}

impl Drop for ProjectionLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

/// Like [`apply_page_md_with_sidecar`] but reuses an already-rendered
/// `md` instead of rendering the page again.
///
/// The GUI commit path renders the page once to diff it for undo; passing
/// that string here saves a second whole-page render (which materializes
/// every block's text). On a large journal that render is tens of ms in
/// release, hundreds in debug, and it ran on every keystroke-commit.
pub fn apply_page_md_with_sidecar_rendered(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
    md: &str,
) -> Result<PathBuf, ActionError> {
    let meta = page_meta(workspace, page_root)
        .ok_or_else(|| ActionError::NotInTree(page_root.to_string()))?;
    write_page_projection(workspace, root, page_root, &meta, md)
}

/// Write an already-rendered page `md` to its `.md` and rebuild the matching
/// sidecar from the same tree. Split out of [`apply_page_md_with_sidecar`] so a
/// caller that already rendered the page (to detect a stale projection) reuses
/// that string instead of rendering it a second time.
fn write_page_projection(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
    meta: &PageMeta,
    md: &str,
) -> Result<PathBuf, ActionError> {
    let path = page_md_path(root, meta);
    let _lock = ProjectionLock::acquire(&path)?;
    write_page_projection_unlocked(workspace, root, page_root, meta, md)
}

fn write_page_projection_unlocked(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
    meta: &PageMeta,
    md: &str,
) -> Result<PathBuf, ActionError> {
    let path = page_md_path(root, meta);
    write_md_atomic(&path, md)?;
    let sidecar = build_sidecar(workspace, page_root, md);
    outl_md::sidecar::write(&sidecar_path_for(&path), &sidecar)?;
    Ok(path)
}

/// Best-effort compare-before-replace for editors that do not honour
/// [`ProjectionLock`].
///
/// There is no portable atomic compare-and-swap for a pathname. The page lock
/// closes the window between cooperating outl processes; this final re-read
/// closes the practical window where rendering or sidecar construction gave an
/// external editor time to save. If the bytes no longer match the revision the
/// guard authorized, refuse and let the filesystem reconciliation path ingest
/// the external edit.
fn write_page_projection_if_unchanged(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
    meta: &PageMeta,
    md: &str,
    expected_disk: Option<&str>,
) -> Result<PathBuf, ActionError> {
    let path = page_md_path(root, meta);
    let unchanged = match (expected_disk, std::fs::read_to_string(&path)) {
        (Some(expected), Ok(current)) => current == expected,
        (None, Err(error)) if error.kind() == std::io::ErrorKind::NotFound => true,
        (_, Err(error)) => return Err(error.into()),
        _ => false,
    };
    if !unchanged {
        return Err(ActionError::PageMarkdownChangedDuringProjection(
            path.display().to_string(),
        ));
    }
    write_page_projection_unlocked(workspace, root, page_root, meta, md)
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
/// Four cases:
/// - `.md` absent → project it (subsumes `_if_absent`, issue #120).
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
    write_page_projection_if_unchanged(workspace, root, page_root, &meta, &rendered, Some(&disk))
        .map(Some)
}

/// The content lines in `disk` that **no block the op log knows** can
/// account for.
///
/// Re-exported from [`outl_md::unlogged`], which is where it lives so
/// that `reconcile_md` — the *producer* of the unlogged state, one
/// crate down — can ask the same question before it advances
/// `last_synced_hash`. Every existing `outl_actions::` path still
/// resolves through this re-export.
///
/// Public because `outl doctor` must reach the *same* verdict in its
/// read-only listing that `--repair` reaches when it writes; two opinions
/// about which pages are safe is how a listing promises a repair the pass
/// then silently skips.
pub use outl_md::unlogged::content_lines_missing_from;

pub use outl_md::unlogged::sidecar_can_answer;

/// Construct a sidecar that lines up with the `.md` we just rendered
/// from the workspace. Walks the page subtree in DFS preorder — the
/// same order [`render_page_md`] emits — so every block's index in
/// the walk maps 1:1 to its line in the `.md`.
fn build_sidecar(workspace: &Workspace, page_root: NodeId, md: &str) -> Sidecar {
    let mut blocks: Vec<SidecarBlock> = Vec::new();
    let mut line = 1usize;
    walk_sidecar(workspace, page_root, 0, &mut line, &mut blocks);
    Sidecar {
        // Never a literal: this builder writes whatever fields the
        // current `SidecarBlock` carries, so a hardcoded number labels a
        // v3 payload as v2 the moment the schema moves — and the reader
        // trusts the label.
        version: outl_md::sidecar::SIDECAR_VERSION,
        page_id: page_root,
        last_synced_hash: file_hash(md),
        last_synced_at: chrono::Local::now().fixed_offset(),
        blocks,
        // This builder runs after a workspace-driven render — the
        // workspace tree already holds the page properties, so by
        // construction they're in the op log. Stamp the current
        // pipeline version to keep the orphan scanner from looping
        // on this page.
        pipeline_version: outl_md::sidecar::CURRENT_PIPELINE_VERSION,
    }
}

fn walk_sidecar(
    workspace: &Workspace,
    parent: NodeId,
    indent: u32,
    line: &mut usize,
    out: &mut Vec<SidecarBlock>,
) {
    for (id, _) in children_of(workspace, parent) {
        let text = workspace.block_text(id).unwrap_or_default();
        // `from_text` keeps hash, handle and stored text derived from
        // one revision — level-2 matching diffs against that text, so a
        // hand-built literal that drifts would mis-assign ids.
        out.push(SidecarBlock::from_text(id, *line, indent, &text));
        *line += 1;
        walk_sidecar(workspace, id, indent + 1, line, out);
    }
}

/// Apply a pure-AST mutation to a page's `.md`, then rewrite both the
/// `.md` and its sidecar.
///
/// **This is the path mobile mutations should take.** The workspace
/// op log isn't on the hot edit path here — we read the `.md` as the
/// source of truth, mutate the parsed AST, render it back, and rebuild
/// the sidecar by content-hash-matching the new blocks against the
/// previous sidecar so unchanged blocks keep their `NodeId`. Anything
/// the closure inserts gets a fresh ULID. Peers reading the resulting
/// `.md` + `.outl` see consistent ids.
///
/// The closure receives a map `NodeId -> block_path` derived from the
/// sidecar so callers can translate the ids the frontend passes in
/// (e.g. "create after block ABC") into the path-based mutations that
/// [`outl_md::outline_ops`] expects.
pub fn mutate_page_md<F>(root: &Path, meta: &PageMeta, mutation: F) -> Result<PathBuf, ActionError>
where
    F: FnOnce(
        &mut outl_md::parse::ParsedPage,
        &std::collections::HashMap<NodeId, Vec<usize>>,
    ) -> Result<(), ActionError>,
{
    use std::collections::HashMap;

    let md_path = page_md_path(root, meta);
    // NOT `unwrap_or_default()`: this function renders the parsed AST
    // straight back over `md_path`, so a read that fails for any reason
    // other than "the page doesn't exist yet" would replace the page
    // with an empty one — and rebuild the sidecar to agree, hiding it
    // from every later consistency scan. See `read_for_rewrite`.
    let md_text = outl_md::read_for_rewrite(&md_path)?;

    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    // `read_for_rewrite` answers "absent" with `Ok("")`, which is only
    // legitimate for a page that does not exist yet. Everything else
    // that presents as absent is checked here, before the empty parse
    // becomes a write.
    if md_text.is_empty() {
        guard_absent_markdown(&md_path, &sidecar_path)?;
    }

    let mut parsed = outl_md::parse::parse(&md_text);

    // NOT `.ok()`: the same "unreadable reads as empty" bug the `.md`
    // was fixed for, one line down and worse. With `old_sidecar = None`
    // every block content-hash lookup misses, `build_sidecar_from_ast`
    // mints a **fresh ULID per block**, and the rewritten sidecar
    // replaces the id ↔ text mapping wholesale: every `((blk-…))`
    // pointing into this page stops resolving, and the next
    // `reconcile_md` sees N unknown ids and trashes the tree's blocks.
    // A `.md` is recoverable from the op log; that mapping is not.
    let old_sidecar = match outl_md::sidecar::read(&sidecar_path) {
        Ok(sidecar) => Some(sidecar),
        // Genuinely absent — a page this device has never projected.
        // The only case where minting ids is correct.
        Err(outl_md::sidecar::SidecarError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            None
        }
        Err(e) => return Err(e.into()),
    };

    // Build NodeId -> block_path map from the AST + sidecar (DFS
    // preorder lines up between the two).
    let mut id_to_path: HashMap<NodeId, Vec<usize>> = HashMap::new();
    if let Some(sc) = &old_sidecar {
        let mut iter = sc.blocks.iter();
        build_id_path_map(&parsed.blocks, &mut Vec::new(), &mut iter, &mut id_to_path);
    }

    mutation(&mut parsed, &id_to_path)?;

    let new_md = outl_md::render::render(&parsed);
    outl_md::write_atomic(&md_path, new_md.as_bytes())?;

    let page_id_ulid = ulid::Ulid::from_string(&meta.id)
        .map_err(|e| ActionError::NotInTree(format!("invalid page id {}: {e}", meta.id)))?;
    let page_id = NodeId(page_id_ulid);
    let new_sidecar = build_sidecar_from_ast(&parsed, old_sidecar.as_ref(), &new_md, page_id);
    outl_md::sidecar::write(&sidecar_path, &new_sidecar)?;

    Ok(md_path)
}

/// Decide whether an absent `.md` really means "this page does not
/// exist yet".
///
/// Two ways it does not, both of which used to end in the page being
/// recreated as a single block and its real blocks trashed by the next
/// `reconcile_md`:
///
/// - **A sidecar is present.** The sidecar is only ever written next to
///   a `.md` this device projected, so its existence is proof the page
///   existed. A missing `.md` beside it is a lost file — a half-finished
///   sync, an editor that deleted-and-recreated, a user emptying a
///   folder — never a new page. Rewriting over it converts a recoverable
///   loss (the `.md` is a projection; the op log still has the content)
///   into an unrecoverable one, because the rewrite rebuilds the sidecar
///   from one block and the next reconcile emits `Move`→`TRASH_ROOT` for
///   every id it can no longer find.
/// - **An iCloud placeholder sibling is present.** On iOS and on legacy
///   iCloud Drive, a file whose bytes have not been downloaded is
///   `.foo.md.icloud` and *the real name does not exist* — so the read
///   is `NotFound`, not the permission/IO error the `read_for_rewrite`
///   contract assumes. Same outcome, on a file that is not lost at all
///   and will materialise on its own.
fn guard_absent_markdown(md_path: &Path, sidecar_path: &Path) -> Result<(), ActionError> {
    // Re-check existence rather than trusting the empty read: a page
    // that legitimately renders to an empty string is not absent.
    if md_path.exists() {
        return Ok(());
    }
    if icloud_placeholder(md_path).is_some() {
        return Err(ActionError::PageMarkdownNotDownloaded(
            md_path.display().to_string(),
        ));
    }
    if sidecar_path.exists() {
        return Err(ActionError::PageMarkdownVanished(
            md_path.display().to_string(),
        ));
    }
    Ok(())
}

/// The iCloud placeholder that stands in for `md_path` while its bytes
/// are still in the cloud: `pages/foo.md` → `pages/.foo.md.icloud`.
fn icloud_placeholder(md_path: &Path) -> Option<PathBuf> {
    let name = md_path.file_name()?.to_str()?;
    let placeholder = md_path.with_file_name(format!(".{name}.icloud"));
    placeholder.exists().then_some(placeholder)
}

fn build_id_path_map<'a>(
    blocks: &[outl_md::parse::OutlineNode],
    current_path: &mut Vec<usize>,
    sidecar_iter: &mut std::slice::Iter<'a, SidecarBlock>,
    out: &mut std::collections::HashMap<NodeId, Vec<usize>>,
) {
    for (i, block) in blocks.iter().enumerate() {
        current_path.push(i);
        if let Some(sc) = sidecar_iter.next() {
            out.insert(sc.id, current_path.clone());
        }
        build_id_path_map(&block.children, current_path, sidecar_iter, out);
        current_path.pop();
    }
}

fn build_sidecar_from_ast(
    parsed: &outl_md::parse::ParsedPage,
    old_sidecar: Option<&Sidecar>,
    md: &str,
    page_id: NodeId,
) -> Sidecar {
    use std::collections::HashSet;
    let mut used: HashSet<NodeId> = HashSet::new();
    let mut blocks: Vec<SidecarBlock> = Vec::new();
    let mut line = 1usize;
    walk_ast_for_sidecar(
        &parsed.blocks,
        0,
        old_sidecar,
        &mut used,
        &mut line,
        &mut blocks,
    );
    Sidecar {
        version: outl_md::sidecar::SIDECAR_VERSION,
        page_id,
        last_synced_hash: file_hash(md),
        last_synced_at: chrono::Local::now().fixed_offset(),
        blocks,
        // Built from a parsed `.md` + workspace tree — both sources
        // already carry the page properties consistently, so this
        // sidecar represents a fully-propagated state.
        pipeline_version: outl_md::sidecar::CURRENT_PIPELINE_VERSION,
    }
}

fn walk_ast_for_sidecar(
    blocks: &[outl_md::parse::OutlineNode],
    indent: u32,
    old_sidecar: Option<&Sidecar>,
    used: &mut std::collections::HashSet<NodeId>,
    line: &mut usize,
    out: &mut Vec<SidecarBlock>,
) {
    for block in blocks {
        let hash = content_hash(&block.text);
        let id = old_sidecar
            .and_then(|sc| {
                sc.blocks
                    .iter()
                    .find(|b| b.content_hash == hash && !used.contains(&b.id))
                    .map(|b| b.id)
            })
            .unwrap_or_else(|| {
                // No content-hash match: this is a freshly inserted
                // block, so allocate a new random id.
                NodeId::new()
            });
        used.insert(id);
        out.push(SidecarBlock::from_text(id, *line, indent, &block.text));
        *line += 1;
        walk_ast_for_sidecar(&block.children, indent + 1, old_sidecar, used, line, out);
    }
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
    pub written: Vec<PathBuf>,
    pub failures: Vec<ProjectionFailure>,
}

#[derive(Debug)]
pub struct ProjectionFailure {
    pub path: PathBuf,
    pub error: ActionError,
}

fn parse_node_id(s: &str) -> Result<NodeId, ActionError> {
    use std::str::FromStr;
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|e| ActionError::NotInTree(format!("invalid id {s}: {e}")))
}
