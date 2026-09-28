//! What one reconcile pass hands back to its caller: the report on a
//! pass that ran, the error on one that could not or would not.
//!
//! Split out from the pass itself because these two are what every
//! caller imports, and because a refusal is as much a result as a
//! success here — [`ReconcileError::BulkDelete`] leaves the `.md` and
//! the tree untouched on purpose, and
//! [`ReconcileReport::unlogged_lines`] reports a pass that deliberately
//! did not advance `last_synced_hash` (invariant 8).

use crate::sidecar;
use outl_core::workspace::WorkspaceError;
use std::io;
use std::path::{Path, PathBuf};

/// Outcome of one reconcile pass.
#[derive(Debug, Clone)]
pub struct ReconcileReport {
    /// Path of the `.md` file processed.
    pub md_path: PathBuf,
    /// Number of ops produced and applied.
    pub ops_applied: usize,
    /// Number of orphan ids logged.
    pub orphans: usize,
    /// Whether the sidecar was created fresh.
    pub created_sidecar: bool,
    /// Content lines this pass read from the `.md` but could not emit an
    /// op for.
    ///
    /// Non-zero means the sidecar's `last_synced_hash` was deliberately
    /// **not** advanced (invariant 8), so the page stays dirty and the
    /// next reconcile looks at it again. Callers should surface the
    /// count: a page that quietly reconciles forever is the symptom the
    /// user gets to see, and the cause is content the log cannot hold.
    pub unlogged_lines: usize,
}

/// Errors a reconcile pass may surface.
#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    /// Filesystem error reading or writing files.
    #[error("io error on {path}: {source}")]
    Io {
        /// Path involved in the failure.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
    /// Invalid sidecar payload.
    #[error("sidecar error: {0}")]
    Sidecar(#[from] sidecar::SidecarError),
    /// Workspace failed to apply an op.
    #[error("workspace error: {0}")]
    Workspace(#[from] WorkspaceError),
    /// The `.md` would delete more of the page than a guard allows.
    ///
    /// Not a failure of the reconcile — a refusal. The `.md` on disk and
    /// the tree are both untouched, so the caller can re-run with
    /// [`crate::matching::guard::OrphanGuard::Disabled`] once the user
    /// says the deletion was intended.
    #[error("{0}")]
    BulkDelete(#[from] crate::matching::guard::MatchGuardError),
}

pub(super) fn io_err(path: &Path, source: io::Error) -> ReconcileError {
    ReconcileError::Io {
        path: path.to_path_buf(),
        source,
    }
}
