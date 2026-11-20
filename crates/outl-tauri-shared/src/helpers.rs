//! Cross-cutting helpers used by every command body.
//!
//! Argument parsing, workspace-lock acquisition, building a [`PageView`]
//! off the tree or off disk, and the stale-`.md` refresh every open path
//! runs. Pure glue — anything that mutates the workspace semantically
//! belongs in `outl-actions`, and what happens *around* a mutation
//! (the `finish_in_page*` pipeline, the backlink-index policy, the peer
//! announce) is [`crate::commit`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use chrono::NaiveDate;
use outl_actions::{
    date_from_slug, page_meta as page_meta_action, project_outline,
    read_page_outline_with_workspace, render_page_md, ActionError, HistoryStacks, PageOutline,
    ProjectionRefusal,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use tracing::warn;

use crate::host::AppHost;
use crate::state::{MdAheadOfLog, PageView};

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
///
/// Delegates to [`crate::offload::locked`], which a body that has been
/// moved onto the blocking pool calls directly (it holds the slot's
/// `Arc`, not an `AppHost`). One owner, so the on-thread and off-thread
/// paths cannot answer "is a workspace open" two different ways.
pub fn with_ws<S, F, T>(state: &S, f: F) -> Result<T, String>
where
    S: AppHost,
    F: FnOnce(&Workspace) -> Result<T, String>,
{
    crate::offload::locked(state.workspace(), f)
}

/// Normalize a path the OS handed us into a plain filesystem path
/// `std::fs` can open.
///
/// On desktop the Tauri dialog returns a bare path and this is a no-op. On
/// iOS it returns a `file://` URL (often percent-encoded), so we strip the
/// scheme and decode `%XX` before `import_asset` reads it. Android's
/// `content://` URIs are not filesystem paths and can't be handled here —
/// they're left as-is and fail loudly downstream (Android is not a target
/// platform yet).
///
/// Shared by the file picker (`commands::asset`) and the OS "Open With"
/// gesture (`commands::open_with`), which receive the same two shapes
/// from different places — a second copy would be a second answer to
/// "is this a `file://` URL".
pub fn normalize_picker_path(raw: &str) -> String {
    let Some(rest) = raw.strip_prefix("file://") else {
        return raw.to_string();
    };
    // `file:///path` → `/path`; drop an empty authority if present.
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .into_owned()
}

/// Acquire a mutable handle to the workspace.
pub fn with_ws_mut<S, F, T>(state: &S, f: F) -> Result<T, String>
where
    S: AppHost,
    F: FnOnce(&mut Workspace) -> Result<T, String>,
{
    crate::offload::locked_mut(state.workspace(), f)
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
        projection_error: None,
        // Set by the one command that can discover it, `edit_block`, via
        // `PageView::with_dropped_edit`.
        dropped_edit: None,
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
        projection_error: None,
        dropped_edit: None,
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
/// The split itself is [`outl_actions::ProjectionRefusal`], not this
/// function: the TUI needs the same verdict on its own open path and
/// cannot depend on this crate ([#292]). What is left here is the DTO
/// and the log line.
///
/// [#292]: https://github.com/outlmd/outl/issues/292
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
    let Some(refusal) = outl_actions::reproject_stale_md(workspace, storage_root, page_id) else {
        return ReprojectOutcome::default();
    };
    match refusal {
        ProjectionRefusal::AheadOfLog {
            path,
            lines,
            sample,
        } => {
            warn!("{context}: reproject stale .md refused: {path} is ahead of the log");
            ReprojectOutcome {
                ahead_of_log: Some(MdAheadOfLog {
                    path,
                    lines,
                    sample,
                }),
                other_error: None,
            }
        }
        ProjectionRefusal::Retryable(message) => {
            warn!("{context}: reproject stale .md failed: {message}");
            ReprojectOutcome {
                ahead_of_log: None,
                other_error: Some(message),
            }
        }
    }
}

/// Surgical undo invalidation across a peer-driven reload.
///
/// Drops the undo / redo stacks of **only** the pages whose `.md`
/// projection actually changed between `old` and `fresh`. Restoring a
/// pre-reload snapshot of a page the peer DID change would silently
/// revert the peer's edits, so those stacks have to go — but a blanket
/// clear caps undo at a single step whenever a peer (another device, or
/// the TUI on the same workspace) wrote anything, because every peer
/// write fires a reload. Pages the reload left untouched keep their
/// full undo depth.
///
/// `old` is `None` only when the workspace wasn't loaded yet; with no
/// baseline to diff against, every stale stack is dropped.
///
/// Shared by every client whose `AppHost::history()` is wired (moved
/// here from `outl-desktop/src-tauri/src/helpers.rs` when mobile's
/// `reload_workspace` gained the same undo stacks to protect — RFC
/// 0254 phase 1). Each client's `reload_workspace` calls this against
/// its own pre/post workspace before swapping the `Mutex` slot, so the
/// rule is unit-testable without a Tauri `AppHandle`.
pub fn invalidate_changed_history(
    old: Option<&Workspace>,
    fresh: &Workspace,
    history: &mut HashMap<NodeId, HistoryStacks<String>>,
) {
    let stale: Vec<NodeId> = match old {
        Some(old) => history
            .keys()
            .filter(|id| render_page_md(old, **id) != render_page_md(fresh, **id))
            .copied()
            .collect(),
        None => history.keys().copied().collect(),
    };
    for id in stale {
        history.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::invalidate_changed_history;
    use crate::state::MdAheadOfLog;
    use outl_actions::{
        append_block, open_or_create_page as open_or_create, render_page_md, ActionError, PageKind,
    };
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::{ActorId, NodeId};
    use outl_core::workspace::Workspace;
    use std::collections::HashMap;

    #[test]
    fn the_refusal_carries_the_count_and_the_sample_forward() {
        let err = ActionError::PageMarkdownAheadOfLog {
            path: "/w/pages/infra.md".into(),
            lines: 12,
            sample: "\"restarted the ingest worker\"".into(),
        };
        let info = MdAheadOfLog::from_error(&err).expect("this is the variant the user must see");
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
        assert!(MdAheadOfLog::from_error(&err).is_none());
    }

    /// Build a workspace with two pages (`alpha`, `beta`), each with a
    /// single block. Page ids are derived deterministically from the
    /// slug (`page_id_from_slug`), so a second workspace built the same
    /// way agrees on the ids — exactly what a peer reload relies on.
    fn workspace_with_two_pages(actor: ActorId, hlc: &HlcGenerator) -> (Workspace, NodeId, NodeId) {
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        let alpha = open_or_create(&mut ws, hlc, "alpha", "Alpha", PageKind::Page).unwrap();
        append_block(&mut ws, hlc, Some(alpha), Some("alpha one")).unwrap();
        let beta = open_or_create(&mut ws, hlc, "beta", "Beta", PageKind::Page).unwrap();
        append_block(&mut ws, hlc, Some(beta), Some("beta one")).unwrap();
        (ws, alpha, beta)
    }

    /// Avelino's requested integration test for the surgical history
    /// invalidation (#82): two pages, history recorded on both, a peer
    /// reload that changed **only one** of them must drop only that
    /// page's stack and leave the untouched page's `can_undo` intact.
    #[test]
    fn surgical_invalidation_keeps_untouched_page_undoable() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);

        // 1. Two pages, each with a block.
        let (old, alpha, beta) = workspace_with_two_pages(actor, &hlc);

        // 2. Record an undo snapshot on both (the pre-mutation render,
        //    exactly as `finish_in_page_with` does).
        let mut history: HashMap<NodeId, outl_actions::HistoryStacks<String>> = HashMap::new();
        history
            .entry(alpha)
            .or_default()
            .record(render_page_md(&old, alpha));
        history
            .entry(beta)
            .or_default()
            .record(render_page_md(&old, beta));
        assert!(history[&alpha].can_undo());
        assert!(history[&beta].can_undo());

        // 3. A peer reload that touched ONLY `beta`. Rebuilding the
        //    same two slugs yields the same page ids; `beta` gets an
        //    extra block so its projection diverges, `alpha`'s doesn't.
        let (mut fresh, _, fresh_beta) = workspace_with_two_pages(actor, &hlc);
        append_block(&mut fresh, &hlc, Some(fresh_beta), Some("beta two")).unwrap();
        assert_eq!(
            render_page_md(&old, alpha),
            render_page_md(&fresh, alpha),
            "alpha must be byte-identical across the reload"
        );
        assert_ne!(
            render_page_md(&old, beta),
            render_page_md(&fresh, beta),
            "beta must have diverged"
        );

        invalidate_changed_history(Some(&old), &fresh, &mut history);

        // 4. The untouched page keeps its undo depth; the changed page
        //    lost its (now-misleading) stack.
        assert!(
            history.get(&alpha).is_some_and(|h| h.can_undo()),
            "alpha was untouched by the reload — its undo must survive"
        );
        assert!(
            !history.contains_key(&beta),
            "beta changed across the reload — its stack must be dropped"
        );
    }

    /// With no prior workspace to diff against, every recorded stack is
    /// stale and must be dropped (the `None` arm).
    #[test]
    fn invalidation_with_no_baseline_clears_everything() {
        let actor = ActorId::new();
        let hlc = HlcGenerator::new(actor);
        let (fresh, alpha, beta) = workspace_with_two_pages(actor, &hlc);

        let mut history: HashMap<NodeId, outl_actions::HistoryStacks<String>> = HashMap::new();
        history.entry(alpha).or_default().record(String::new());
        history.entry(beta).or_default().record(String::new());

        invalidate_changed_history(None, &fresh, &mut history);

        assert!(history.is_empty(), "no baseline → drop every stack");
    }
}
