//! The page-open refresh: bring a page's `.md` up to the tree before a
//! client reads a view off it, and say which refusals the **user** has
//! to hear about.
//!
//! One call, two jobs, because they are the same call. A peer's ops land
//! in the op log and not in the `.md`, so reading the file without this
//! shows a projection from before they arrived (issue #166). And the
//! write that fixes that can be **refused** — root `CLAUDE.md`
//! invariant 8 forbids overwriting a `.md` holding content that exists
//! in no op — which is the one thing on an open path that a log line
//! cannot carry (issue #292).
//!
//! It lives beside the gate rather than in a client because the
//! classification *is* the policy, and it had two homes before: the
//! Tauri backend's `reproject_stale_md` (unreachable from the TUI — it
//! is the Tauri crate) and, in the TUI, nothing at all. A second
//! opinion about which refusals reach the user is how one client ends
//! up shipping a page that silently stopped syncing.

use std::path::Path;

use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use super::apply::apply_page_md_with_sidecar_if_stale;
use crate::error::ActionError;

/// Why a page-open re-projection declined, split by **who has to hear
/// it**.
///
/// The split is the whole point. [`ActionError`] has about thirty variants
/// and exactly one of them means "this page has stopped converging with
/// your other devices until a command runs" — everything else here is
/// an I/O condition the next open retries out of. A client that treats
/// them alike either nags about a transient permission error or stays
/// silent about a frozen page, and the second is the failure root
/// `CLAUDE.md` invariant 8 exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionRefusal {
    /// Invariant 8's refusal: the `.md` holds content that exists in no
    /// op, so it was left byte-for-byte alone and the page is frozen in
    /// **both** directions until `outl reconcile --ahead-of-log` runs.
    ///
    /// The user has to be told. Fields come straight off
    /// [`ActionError::PageMarkdownAheadOfLog`] so nothing recounts them.
    AheadOfLog {
        /// The `.md` that was left alone.
        path: String,
        /// How many content lines exist only on disk.
        lines: usize,
        /// One of those lines, quoted, so a surface can name what is at
        /// risk instead of only counting it.
        sample: String,
    },
    /// Everything else — I/O, permissions, a sidecar that cannot be
    /// read, a page id not in the tree. Local and retryable, so a log
    /// line is the honest surface.
    Retryable(String),
}

/// Refresh `page_root`'s `.md` when the tree has moved past it, and
/// classify the one failure the **user** has to know about.
///
/// Every client's page-open path wants this, not the bare gate: reading
/// a view off a `.md` the tree has outrun shows a stale or empty page
/// (issue #166), and the refusal that can come back instead is the one
/// thing on that path a log line cannot carry (issue #292).
///
/// `None` means the projection was fine — written, or already in sync.
///
/// **Never fails the open.** The guard withheld a *write*; the `.md` on
/// disk is still perfectly readable, so the caller shows what is on
/// disk and hangs the notice beside it. Turning this into an error
/// would trade a stale page for no page at all, on the hottest path in
/// every client.
///
pub fn reproject_stale_md(
    workspace: &Workspace,
    root: &Path,
    page_root: NodeId,
) -> Option<ProjectionRefusal> {
    let Err(e) = apply_page_md_with_sidecar_if_stale(workspace, root, page_root) else {
        return None;
    };
    Some(ProjectionRefusal::from_error(&e))
}

impl ProjectionRefusal {
    /// Project one [`ActionError`] onto "must the user hear this".
    ///
    /// Split from [`reproject_stale_md`] so the classification is
    /// testable without a workspace on disk — it is the piece that
    /// decides whether anything reaches a screen at all.
    pub fn from_error(error: &ActionError) -> Self {
        match error {
            ActionError::PageMarkdownAheadOfLog {
                path,
                lines,
                sample,
            } => ProjectionRefusal::AheadOfLog {
                path: path.clone(),
                lines: *lines,
                sample: sample.clone(),
            },
            other => ProjectionRefusal::Retryable(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_carries_the_count_and_the_sample_forward() {
        let err = ActionError::PageMarkdownAheadOfLog {
            path: "/w/pages/infra.md".into(),
            lines: 12,
            sample: "\"restarted the ingest worker\"".into(),
        };
        let ProjectionRefusal::AheadOfLog {
            path,
            lines,
            sample,
        } = ProjectionRefusal::from_error(&err)
        else {
            panic!("this is the variant the user must see");
        };
        assert_eq!(lines, 12);
        assert_eq!(path, "/w/pages/infra.md");
        assert!(sample.contains("ingest worker"));
    }

    /// A banner that fires on ordinary I/O trouble is a banner users
    /// learn to ignore, and then the one that means "this page is not
    /// syncing" goes unread too. Only the refusal gets a surface; the
    /// rest keep their message for the log.
    #[test]
    fn an_unrelated_failure_is_retryable_and_keeps_its_message() {
        let err = ActionError::NotInTree("01ABC".into());
        match ProjectionRefusal::from_error(&err) {
            ProjectionRefusal::Retryable(message) => assert!(
                message.contains("01ABC"),
                "the log line must keep what the error said: {message}"
            ),
            other => panic!("expected Retryable, got {other:?}"),
        }
    }
}
