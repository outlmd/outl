//! Cross-cutting helpers used by every command body.
//!
//! Argument parsing, workspace-lock acquisition, and the
//! `finish_in_page` pattern that every mutation funnels through. Pure
//! glue — anything that mutates the workspace semantically belongs in
//! `outl-actions`.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use outl_actions::{
    apply_page_md_with_sidecar_if_stale, apply_page_md_with_sidecar_rendered, date_from_slug,
    page_meta as page_meta_action, project_outline, read_page_outline_with_workspace,
    render_page_md, ActionError, PageOutline,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use tracing::warn;

use crate::host::AppHost;
use crate::state::{MdAheadOfLog, PageView, ERR_LOADING};

pub fn parse_node_id(s: &str) -> Result<NodeId, String> {
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|e| format!("invalid node id {s}: {e}"))
}

pub fn parse_date(slug: &str) -> Result<NaiveDate, String> {
    date_from_slug(slug).ok_or_else(|| format!("invalid date slug: {slug}"))
}

/// Acquire a read-only handle to the workspace. Returns the
/// `workspace_loading` sentinel string while the background opener is
/// still running or the user hasn't picked a workspace yet.
pub fn with_ws<S, F, T>(state: &S, f: F) -> Result<T, String>
where
    S: AppHost,
    F: FnOnce(&Workspace) -> Result<T, String>,
{
    let guard = state.workspace().lock();
    match guard.as_ref() {
        Some(ws) => f(ws),
        None => Err(ERR_LOADING.to_string()),
    }
}

/// Acquire a mutable handle to the workspace.
pub fn with_ws_mut<S, F, T>(state: &S, f: F) -> Result<T, String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<T, String>,
{
    let mut guard = state.workspace().lock();
    match guard.as_mut() {
        Some(ws) => f(ws),
        None => Err(ERR_LOADING.to_string()),
    }
}

/// The current storage root, or the `workspace_loading` sentinel.
/// Convenience alias over [`AppHost::storage_root`] so command bodies
/// read like the pre-extraction client code.
pub fn storage_root_or_err<S: AppHost>(state: &S) -> Result<PathBuf, String> {
    state.storage_root()
}

pub fn build_page_view(
    workspace: &Workspace,
    storage_root: &Path,
    page_id: NodeId,
) -> Result<PageView, ActionError> {
    let meta = page_meta_action(workspace, page_id)
        .ok_or_else(|| ActionError::NotInTree(page_id.to_string()))?;
    // Read the outline straight from the page's `.md` (+ sidecar for
    // stable block ids) — the `.md` is the projection the user sees on
    // disk. The workspace-aware variant overlays `Op::SetCollapsed` so
    // `OutlineNode.collapsed` reflects the op log (the only place that
    // state legitimately lives).
    let page_outline = read_page_outline_with_workspace(storage_root, &meta, workspace)
        .unwrap_or_else(|_| PageOutline {
            nodes: Vec::new(),
            warnings: Vec::new(),
        });
    // Backlinks are deliberately NOT computed here. `backlinks_for_page`
    // is an O(blocks-in-workspace) scan, and this function is on the
    // first-paint path (every open) AND the post-mutation path (every
    // block edit, via `finish_in_page`). On a ~66k-node workspace that
    // made the journal take seconds to appear on desktop and much longer
    // on mobile. The frontend fetches backlinks lazily via the
    // `page_backlinks` command after the outline paints — same policy the
    // TUI has always used (lazy, cached, toggle-gated).
    let backlinks_order = outl_config::load().display.backlinks_order;
    Ok(PageView {
        page: meta,
        outline: page_outline.nodes,
        backlinks: Vec::new(),
        backlinks_order,
        page_properties: outl_actions::property::page_properties(workspace, page_id),
        warnings: page_outline.warnings,
        // Set by the open commands, which are the ones that attempt the
        // re-projection this flag reports on. Reading the `.md` never
        // discovers the condition on its own, so the view leaves both
        // halves unset and `PageView::with_ahead_of_log_check` stamps
        // them where the check actually ran.
        md_ahead_of_log: None,
        md_ahead_of_log_checked: false,
    })
}

/// Build a page view **straight from the workspace tree**, without
/// reading the `.md` off disk.
///
/// This is the async-projection twin of [`build_page_view`]: when the
/// `.md` write is deferred to the background [`crate::ProjectionWriter`],
/// the on-disk `.md` is momentarily behind the tree, so the reply must
/// come from the tree (the source of truth) instead. `project_outline`
/// carries the same shape `read_page_outline` produces — tokens,
/// `collapsed` (from the op log), alpha-sorted block props — so the
/// client renders identically either way. `warnings` is always empty
/// here: parse warnings come from re-reading external `.md`, and the
/// tree the user just mutated has none.
pub fn build_page_view_from_tree(
    workspace: &Workspace,
    page_id: NodeId,
) -> Result<PageView, ActionError> {
    let meta = page_meta_action(workspace, page_id)
        .ok_or_else(|| ActionError::NotInTree(page_id.to_string()))?;
    let outline = project_outline(workspace, page_id);
    let backlinks_order = outl_config::load().display.backlinks_order;
    Ok(PageView {
        page: meta,
        outline,
        backlinks: Vec::new(),
        backlinks_order,
        page_properties: outl_actions::property::page_properties(workspace, page_id),
        warnings: Vec::new(),
        md_ahead_of_log: None,
        md_ahead_of_log_checked: false,
    })
}

/// What a stale-`.md` refresh attempt has to say back to its caller.
///
/// Both fields are already logged by [`reproject_stale_md`]; they are
/// carried out so a caller with a user-facing surface can use them.
#[derive(Debug, Default)]
pub struct ReprojectOutcome {
    /// The write was refused because the `.md` holds content that exists
    /// in no op — the one condition the user must be told about, since
    /// nothing else will resolve it.
    pub ahead_of_log: Option<MdAheadOfLog>,
    /// Any other failure (I/O, permissions, unreadable sidecar). Local
    /// and retryable, so most callers ignore it; `open_ref` reports it
    /// through its own `ref-projection-failed` event.
    pub other_error: Option<String>,
}

/// Refresh `page_id`'s `.md` when the tree has moved past it, and report
/// the one failure the **user** has to know about.
///
/// Every GUI open path calls this before reading the `.md` back into a
/// view (issue #166: a peer's ops landed, the projection was never
/// refreshed, the page renders stale or empty).
///
/// Two failures, deliberately handled differently:
///
/// - [`ActionError::PageMarkdownAheadOfLog`] is **the user's problem**.
///   The write was refused because the file holds content that exists in
///   no op, so the page has stopped converging in both directions until
///   `outl reconcile --ahead-of-log` runs (root `CLAUDE.md` invariant 8,
///   [RFC 0210]). Logging it and moving on is what left a page silently
///   frozen with nothing on screen. It comes back as an
///   [`MdAheadOfLog`] the caller hangs on the [`PageView`].
/// - Everything else (I/O, permissions, an unreadable sidecar) is a
///   local, retryable condition the next open or the orphan scanner
///   picks up; it stays a log line.
///
/// Never fails the open. The page opens either way — the guard withheld
/// a *write*, and the `.md` on disk is still perfectly readable. Turning
/// this into an error would trade a stale page for no page at all, on
/// the hottest path in the app.
///
/// [RFC 0210]: https://github.com/outlmd/outl/blob/main/docs/rfcs/0210-md-content-outside-op-log.md
pub fn reproject_stale_md(
    workspace: &Workspace,
    storage_root: &Path,
    page_id: NodeId,
    context: &str,
) -> ReprojectOutcome {
    let Err(e) = apply_page_md_with_sidecar_if_stale(workspace, storage_root, page_id) else {
        return ReprojectOutcome::default();
    };
    warn!("{context}: reproject stale .md failed: {e}");
    match md_ahead_of_log(&e) {
        Some(ahead) => ReprojectOutcome {
            ahead_of_log: Some(ahead),
            other_error: None,
        },
        None => ReprojectOutcome {
            ahead_of_log: None,
            other_error: Some(e.to_string()),
        },
    }
}

/// Project the one [`ActionError`] variant the user must see onto its
/// wire shape; every other variant maps to `None`.
///
/// Split from [`reproject_stale_md`] so the classification is testable
/// without a workspace on disk — it is the piece that decides whether
/// anything reaches the screen at all.
fn md_ahead_of_log(err: &ActionError) -> Option<MdAheadOfLog> {
    match err {
        ActionError::PageMarkdownAheadOfLog {
            path,
            lines,
            sample,
        } => Some(MdAheadOfLog {
            path: path.clone(),
            lines: *lines,
            sample: sample.clone(),
        }),
        _ => None,
    }
}

/// Apply a workspace mutation `f` and project the result back to
/// `.md` + sidecar.
///
/// The op log is the source of truth: the `.md` and the sidecar are
/// projections regenerated after the mutation so what the user reads on
/// disk matches the op-log state. We do **not** run `reconcile_md`
/// before `f` — the op log is already up to date with whatever peers
/// delivered, and "catching up" from a lagging on-disk `.md` would risk
/// emitting Delete cascades.
pub fn finish_in_page<S, F>(state: &S, page_id: NodeId, f: F) -> Result<PageView, String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<(), ActionError>,
{
    finish_in_page_with(state, page_id, f).map(|(_, view)| view)
}

/// Variant of [`finish_in_page`] that also returns whatever value the
/// mutation produced (the new `NodeId` for `create_block`, etc.) so the
/// frontend never has to re-discover it from a DFS diff of the outline.
///
/// When the host exposes undo stacks ([`AppHost::history`] returns
/// `Some`), the pre-mutation `.md` render is recorded — but only when
/// the mutation actually changed the page render, so no-op commands
/// don't turn `Cmd+Z` into a visible nothing.
pub fn finish_in_page_with<S, F, T>(
    state: &S,
    page_id: NodeId,
    f: F,
) -> Result<(T, PageView), String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<T, ActionError>,
{
    let root = state.storage_root()?;
    with_ws_mut(state, |ws| {
        // Undo snapshot: record the pre-mutation `.md` only when the page
        // actually changed. The renders here also cover the async path's
        // diff; the sync fallback reuses `after` for its projection.
        let before = state.history().map(|_| render_page_md(ws, page_id));
        let value = f(ws).map_err(|e| e.to_string())?;
        if let (Some(history), Some(before)) = (state.history(), before) {
            if render_page_md(ws, page_id) != before {
                history.lock().entry(page_id).or_default().record(before);
            }
        }
        // The mutation changed the tree, so the cached backlinks index is
        // stale. Drop it (under the workspace lock, so it serializes with
        // the rebuild `page_backlinks` does); the next read rebuilds it.
        invalidate_backlink_index(state);
        announce_after_commit(state, ws, page_id);

        if let Some(writer) = state.projection_writer() {
            // Async-writes default: the op log already has the truth, so
            // queue the `.md` + sidecar write off-thread and build the
            // reply straight from the tree. The commit never blocks the
            // next keystroke on a render + SHA-256 + disk write.
            writer.queue(page_id);
            let view = build_page_view_from_tree(ws, page_id).map_err(|e| e.to_string())?;
            Ok((value, view))
        } else {
            // Synchronous fallback (host without a projection worker):
            // project inline and read the view back off the `.md`.
            let after = render_page_md(ws, page_id);
            if let Err(e) = apply_page_md_with_sidecar_rendered(ws, &root, page_id, &after) {
                warn!("page md+sidecar sync failed: {e}");
            }
            let view = build_page_view(ws, &root, page_id).map_err(|e| e.to_string())?;
            Ok((value, view))
        }
    })
}

/// Drop the host's cached backlinks index so the next `page_backlinks`
/// rebuilds it. Call after any change that can alter backlinks — a local
/// mutation (via [`finish_in_page_with`]) or a peer reload. No-op when
/// the host doesn't cache one ([`AppHost::backlink_index`] is `None`).
pub fn invalidate_backlink_index<S: AppHost>(state: &S) {
    if let Some(index) = state.backlink_index() {
        *index.lock() = None;
    }
}

/// Post-commit hook: tell connected peers this device just produced new
/// ops so a peer pulls **immediately** over iroh gossip, instead of
/// waiting for the catch-up loop's maintenance re-sync.
///
/// Best-effort and non-fatal by design:
/// - no transport wired (file-sync / not yet started) → nothing to
///   announce;
/// - the announce never crosses (flaky link) → the catch-up loop's
///   periodic re-sync still converges.
///
/// This is the GUI mirror of the TUI's `save()` tail. Without it, edits
/// committed the op locally but never woke peers — so propagation
/// depended entirely on the catch-up timing.
pub fn announce_after_commit<S: AppHost>(state: &S, ws: &Workspace, page_id: NodeId) {
    let Some(transport) = state.sync_transport() else {
        return;
    };
    let Some(meta) = page_meta_action(ws, page_id) else {
        return;
    };
    transport.announce_local_ops(&meta.slug, state.hlc().next());
}

#[cfg(test)]
mod tests {
    use super::md_ahead_of_log;
    use outl_actions::ActionError;

    #[test]
    fn the_refusal_carries_the_count_and_the_sample_forward() {
        let err = ActionError::PageMarkdownAheadOfLog {
            path: "/w/pages/infra.md".into(),
            lines: 12,
            sample: "\"restarted the ingest worker\"".into(),
        };
        let info = md_ahead_of_log(&err).expect("this is the variant the user must see");
        assert_eq!(info.lines, 12);
        assert_eq!(info.path, "/w/pages/infra.md");
        assert!(info.sample.contains("ingest worker"));
    }

    #[test]
    fn an_unrelated_failure_puts_nothing_on_screen() {
        // A banner that fires on ordinary I/O trouble is a banner users
        // learn to ignore, and then the one that means "this page is not
        // syncing" goes unread too. Only the refusal gets the surface.
        let err = ActionError::NotInTree("01ABC".into());
        assert!(md_ahead_of_log(&err).is_none());
    }
}
