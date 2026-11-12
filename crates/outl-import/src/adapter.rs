//! The [`SourceAdapter`] trait — the boundary every importer implements.

use crate::ir::ImportGraph;
use crate::report::ImportReport;
use std::path::Path;

/// Errors an import can fail with. Non-fatal fidelity issues go into
/// the [`ImportReport`] instead — an adapter only errors when it
/// cannot produce a graph at all.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// Filesystem failure reading the source or writing the destination.
    #[error("io on {path}: {source}")]
    Io {
        /// Offending path.
        path: String,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The source didn't parse as the expected format.
    #[error("parsing source: {0}")]
    Parse(String),
    /// The destination workspace rejected an operation.
    #[error("workspace: {0}")]
    Workspace(String),
}

impl ImportError {
    /// Convenience constructor for IO errors.
    pub fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.display().to_string(),
            source,
        }
    }
}

/// One source dialect (Roam, Logseq, Obsidian, …).
///
/// An adapter's single job is `source on disk → typed IR`. It never
/// writes to the destination — emission is shared and lives in
/// [`crate::emit`]. All dialect-specific inline translation
/// (`__italic__` → `*italic*`, `{{[[TODO]]}}` → task state, …)
/// happens here, because only the adapter knows what the source
/// syntax means.
pub trait SourceAdapter {
    /// Stable id (`"roam"`, `"logseq"`, `"obsidian"`) used by the CLI
    /// and the report.
    fn id(&self) -> &'static str;

    /// Cheap heuristic for `outl import auto <src>`.
    fn detect(src: &Path) -> bool
    where
        Self: Sized;

    /// Parse the source into an [`ImportGraph`]. Fidelity notes go
    /// into `report`; only unrecoverable failures return `Err`.
    fn parse(&self, src: &Path, report: &mut ImportReport) -> Result<ImportGraph, ImportError>;

    /// [`Self::parse`] for an import run with `opts`. An adapter whose IR
    /// depends on an option overrides this; the rest ignore the options.
    fn parse_with(
        &self,
        src: &Path,
        opts: &crate::ImportOptions,
        report: &mut ImportReport,
    ) -> Result<ImportGraph, ImportError> {
        let _ = opts;
        self.parse(src, report)
    }
}
