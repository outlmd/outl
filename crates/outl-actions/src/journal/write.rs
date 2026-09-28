//! The on-disk transaction: page lock, `.md`, sidecar.
//!
//! Split out of the `apply_*` family because every one of them ends
//! here, and the ordering is the part that has to be identical: take the
//! page lock, write the `.md` atomically, then rebuild the `.outl`
//! sidecar **from the same tree that produced those bytes**. A `.md`
//! written without its matching sidecar makes a peer's 3-level matcher
//! see "different content, old sidecar" and emit phantom `Create` /
//! `Delete` ops in cascade.
//!
//! Deciding *whether* to write is not this module's business — that is
//! `super::guard`, run by the callers in `super::apply`. The one
//! exception is [`write_page_projection_if_unchanged`], whose check is
//! about the bytes changing underfoot between the guard and the rename,
//! not about what the op log knows.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_md::sidecar::sidecar_path_for;

use super::paths::{page_md_path, write_md_atomic};
use super::sidecar::build_sidecar;
use crate::error::ActionError;
use crate::page::PageMeta;

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
        // The lock file carries no content; it exists only to be `flock`ed,
        // so there is nothing to truncate or preserve.
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
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
/// Write an already-rendered page `md` to its `.md` and rebuild the matching
/// sidecar from the same tree. Split out of [`apply_page_md_with_sidecar`] so a
/// caller that already rendered the page (to detect a stale projection) reuses
/// that string instead of rendering it a second time.
pub(super) fn write_page_projection(
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

pub(super) fn write_page_projection_unlocked(
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
pub(super) fn write_page_projection_if_unchanged(
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
