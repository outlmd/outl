//! `rename_page` and `merge_pages`: renaming a page and folding one page
//! into another, for the GUI clients.
//!
//! The policy is `outl_actions::rename_page` / `outl_actions::merge_pages`
//! (RFC 0557): what a name collision means, which mentions are rewritten,
//! how properties merge. This module is the IPC adapter and owns only what
//! a client has to get right around that call, in the order
//! `outl page rename` (`outl-cli`) established:
//!
//! 1. the mutation, in one batch, under the workspace lock;
//! 2. **every** slug in `report.touched_pages` re-projected to `.md` +
//!    sidecar, not only the page that moved: a rename rewrites `[[old]]`
//!    in other pages' blocks through ordinary edit ops, and a `.md` left
//!    behind the op log is a page that reads "link to nothing";
//! 3. the stale projection (the old slug's `.md` + sidecar, or the merged
//!    source's) removed **last**, and only once the page that now holds
//!    the content projected. A refused write must never cost the user the
//!    only `.md` that still holds the page;
//! 4. the backlinks cache dropped under the same lock (it serializes with
//!    the rebuild `page_backlinks` does);
//! 5. peers woken, outside the lock.
//!
//! ## Why this is not `finish_in_page`
//!
//! `finish_in_page` / `commit_page` carry one page: a snapshot, a
//! mutation, one projection. A rename touches N pages and retires a
//! projection, and a merge does the same. Looping `commit_page` per slug
//! would snapshot for undo and announce N times around mutations that
//! already happened in one batch, so the steps are split by what they are
//! per:
//!
//! - **once:** the mutation, the backlinks invalidation, the removal of
//!   the stale projection;
//! - **per touched page:** the projection (synchronous on purpose: the
//!   stale file is removed right after, and a queued write landing behind
//!   that removal could recreate it) and the peer announce;
//! - **dropped, not recorded:** undo. A pre-mutation `.md` snapshot of a
//!   page whose slug just changed, or whose blocks were rewritten by a
//!   rename elsewhere, restores text that names a page that no longer
//!   exists. The stacks of every page the call touched are cleared
//!   instead, the same call `invalidate_changed_history` makes across a
//!   peer reload.
//!
//! Async for the reason [`crate::commands::page_open`] documents: a merge
//! re-projects every page that linked to the source, which on a large
//! workspace is real disk work, and the IPC thread is the UI main thread.

use std::path::Path;
use std::sync::Arc;

use outl_actions::{
    apply_page_md_with_sidecar_guarded, find_by_slug, is_trashed, page_meta,
    remove_page_projection, ActionError, MergeReport, PageMeta, PropertyConflict, RenameReport,
    SyncTransport,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use serde::Serialize;
use tracing::warn;

use crate::helpers::{build_page_view, build_page_view_from_tree, parse_node_id};
use crate::host::AppHost;
use crate::offload;
use crate::state::{PageView, ProjectionWriteFailed};

/// One property both pages held with different values. The target kept
/// `kept`; `dropped` is the source's value, which was not copied. The
/// client lists these so the user can reconcile by hand.
#[derive(Debug, Clone, Serialize)]
pub struct PropertyConflictView {
    pub key: String,
    pub kept: String,
    pub dropped: String,
}

impl From<&PropertyConflict> for PropertyConflictView {
    fn from(c: &PropertyConflict) -> Self {
        Self {
            key: c.key.clone(),
            kept: c.kept.clone(),
            dropped: c.dropped.clone(),
        }
    }
}

/// What a rename did, in the counts a toast shows.
#[derive(Debug, Clone, Serialize)]
pub struct RenamePageReport {
    pub old_slug: String,
    pub old_title: String,
    /// Blocks (in any page) whose text was rewritten to follow the new name.
    pub rewritten_blocks: usize,
    /// Namespace-child mentions (`[[old/child]]`) left as they were: they
    /// name a sub-page that was not renamed.
    pub skipped_namespace_refs: usize,
}

/// Reply for `rename_page`.
///
/// Internally tagged on `status`, so the one command answers both
/// outcomes the client branches on: the rename happened, or the name is
/// taken and the client may offer a merge (`merge_pages`).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RenamePageReply {
    /// The page now lives under the new name.
    Renamed {
        /// The renamed page, refreshed. Boxed so the `Exists` reply does
        /// not pay for a whole `PageView` (`clippy::large_enum_variant`).
        view: Box<PageView>,
        report: RenamePageReport,
    },
    /// Another page already owns the new name. **Nothing was written.**
    Exists {
        /// The page that already holds the name.
        existing: PageMeta,
    },
}

/// What a merge did, in the counts and conflicts a summary shows.
#[derive(Debug, Clone, Serialize)]
pub struct MergePagesReport {
    pub source_slug: String,
    pub source_title: String,
    pub target_slug: String,
    pub target_title: String,
    /// Top-level blocks moved under the target (each carries its subtree).
    pub moved_blocks: usize,
    /// Blocks whose text was rewritten to follow the new name.
    pub rewritten_blocks: usize,
    pub skipped_namespace_refs: usize,
    /// Properties both pages held with different values (target kept).
    pub property_conflicts: Vec<PropertyConflictView>,
    /// Names the target now answers to: the source's aliases, slug and title.
    pub aliases_added: Vec<String>,
    /// `true` when the source was already merged away on entry (a repeat).
    pub already_merged: bool,
}

/// Reply for `merge_pages`: the **target** page, refreshed, plus the report.
#[derive(Debug, Clone, Serialize)]
pub struct MergePagesReply {
    pub view: PageView,
    pub report: MergePagesReport,
}

impl From<&RenameReport> for RenamePageReport {
    fn from(r: &RenameReport) -> Self {
        Self {
            old_slug: r.old_slug.clone(),
            old_title: r.old_title.clone(),
            rewritten_blocks: r.rewritten_blocks.len(),
            skipped_namespace_refs: r.skipped_namespace_refs.len(),
        }
    }
}

impl From<&MergeReport> for MergePagesReport {
    fn from(r: &MergeReport) -> Self {
        Self {
            source_slug: r.source.slug.clone(),
            source_title: r.source.title.clone(),
            target_slug: r.target.slug.clone(),
            target_title: r.target.title.clone(),
            moved_blocks: r.moved_blocks,
            rewritten_blocks: r.rewritten_blocks.len(),
            skipped_namespace_refs: r.skipped_namespace_refs.len(),
            property_conflicts: r
                .property_conflicts
                .iter()
                .map(PropertyConflictView::from)
                .collect(),
            aliases_added: r.aliases_added.clone(),
            already_merged: r.already_merged,
        }
    }
}

/// Everything the off-thread body needs from the host, cloned out of the
/// borrowed [`AppHost`] before the `await` (a blocking task is `'static`).
struct Host {
    workspace: Arc<Mutex<Option<Workspace>>>,
    root: std::path::PathBuf,
    hlc: HlcGenerator,
    transport: Option<Arc<dyn SyncTransport>>,
    backlinks: Option<Arc<Mutex<Option<outl_actions::BacklinkIndex>>>>,
}

impl Host {
    fn of<S: AppHost>(state: &S) -> Result<Self, String> {
        Ok(Self {
            workspace: state.workspace_arc(),
            root: state.storage_root()?,
            hlc: state.hlc().clone(),
            transport: state.sync_transport(),
            backlinks: state.backlink_index(),
        })
    }

    /// Wake peers for each slug. Best-effort, like `announce_after_commit`.
    fn announce(&self, slugs: &[String]) {
        if let Some(transport) = &self.transport {
            for slug in slugs {
                transport.announce_local_ops(slug, self.hlc.next());
            }
        }
    }
}

/// How the projection tail went for the page the user lands on.
struct Projected {
    /// Ids whose undo stacks must be dropped.
    touched_ids: Vec<NodeId>,
    /// The landing page's projection failure, if any. Reported on the
    /// view (the mutation committed, so this is not an error reply).
    primary_failure: Option<ActionError>,
}

/// Steps 2 to 4 of the module doc: project every touched page, then
/// remove the stale projection last, then drop the backlinks cache.
fn project_and_retire(
    ws: &Workspace,
    host: &Host,
    touched: &[String],
    primary: NodeId,
    stale: Option<&PageMeta>,
) -> Projected {
    let mut touched_ids = Vec::with_capacity(touched.len() + 1);
    let mut primary_projected = false;
    let mut primary_failure = None;
    for slug in touched {
        let Some(id) = find_by_slug(ws, slug) else {
            continue;
        };
        touched_ids.push(id);
        // Step 5 of `commit_page`, taken alone on purpose: see the module
        // doc for why the other four steps do not apply per page here.
        match apply_page_md_with_sidecar_guarded(ws, &host.root, id) {
            Ok(_) => primary_projected |= id == primary,
            Err(e) if id == primary => primary_failure = Some(e),
            // A rewritten page whose write was refused keeps its old
            // `.md`; the op log holds the rewrite and the next open
            // re-projects (and surfaces a refusal on that page's view).
            Err(e) => warn!("page merge/rename: could not project {slug}: {e}"),
        }
    }
    if primary_projected {
        if let Some(stale) = stale {
            retire_projection(&host.root, stale);
        }
    }
    if let Some(index) = &host.backlinks {
        // The caller holds the workspace lock: free the old index elsewhere.
        crate::commit::discard_slot(index);
    }
    Projected {
        touched_ids,
        primary_failure,
    }
}

fn retire_projection(root: &Path, stale: &PageMeta) {
    // Logged, not returned: the content is in the op log and the new
    // projection, so a leftover file is clutter the orphan scanner owns.
    if let Err(e) = remove_page_projection(root, stale) {
        warn!(
            "page merge/rename: could not remove projection for {}: {e}",
            stale.slug
        );
    }
}

/// The landing page's view. Read off disk when it projected; off the tree
/// when it did not, because a page that failed to project may have no
/// `.md` at its new path and would otherwise open blank.
fn landing_view(
    ws: &Workspace,
    host: &Host,
    page: NodeId,
    failure: Option<&ActionError>,
) -> Result<PageView, String> {
    let Some(failure) = failure else {
        return build_page_view(ws, &host.root, page).map_err(|e| e.to_string());
    };
    let mut view = build_page_view_from_tree(ws, page).map_err(|e| e.to_string())?;
    let failed = ProjectionWriteFailed::from_error(page, failure);
    view.md_ahead_of_log = failed.md_ahead_of_log.clone();
    view.projection_error = failed.md_ahead_of_log.is_none().then_some(failed.error);
    Ok(view)
}

/// A page by slug, else by the id of a live page root.
fn resolve_live_page(ws: &Workspace, page: &str) -> Option<NodeId> {
    find_by_slug(ws, page).or_else(|| {
        let id = parse_node_id(page).ok()?;
        (page_meta(ws, id).is_some() && !is_trashed(ws, id)).then_some(id)
    })
}

/// Rename `page` (slug or page id) to the human text `new_name`.
///
/// The new slug is `slugify(new_name)` and the new title is the trimmed
/// `new_name`, so "Café Notes" becomes slug `cafe-notes` titled
/// `Café Notes`. A name that only changes case or punctuation keeps the
/// slug and is a title-only rename.
///
/// A name another page already owns is **not an error**: it returns
/// [`RenamePageReply::Exists`] with that page, nothing written, and the
/// client decides whether to offer `merge_pages`. Every other refusal
/// (journal, blank or unspellable name, unknown page) is an `Err` string
/// the client shows as a toast.
pub async fn rename_page<S: AppHost>(
    state: &S,
    page: String,
    new_name: String,
) -> Result<RenamePageReply, String> {
    let title = new_name.trim().to_string();
    if title.is_empty() {
        return Err("a page name cannot be empty".to_string());
    }
    let new_slug = outl_md::slug::slugify(&title);
    let host = Host::of(state)?;

    let (reply, ids) = offload::run("rename_page", move || {
        let (reply, ids, announce) = offload::locked_mut(&host.workspace, |ws| {
            rename_locked(ws, &host, &page, &new_slug, &title)
        })?;
        host.announce(&announce);
        Ok((reply, ids))
    })
    .await?;

    forget_undo(state, &ids);
    Ok(reply)
}

/// What a body hands back past the lock: the reply, the page ids whose
/// undo stacks to drop, and the slugs to announce.
type Done<T> = (T, Vec<NodeId>, Vec<String>);

/// [`rename_page`]'s half that runs under the workspace lock.
fn rename_locked(
    ws: &mut Workspace,
    host: &Host,
    page: &str,
    new_slug: &str,
    title: &str,
) -> Result<Done<RenamePageReply>, String> {
    let id = resolve_live_page(ws, page).ok_or_else(|| format!("page not found: {page}"))?;
    let old_slug = page_meta(ws, id)
        .map(|m| m.slug)
        .ok_or_else(|| ActionError::NotInTree(id.to_string()).to_string())?;
    let report = match outl_actions::rename_page(ws, &host.hlc, &old_slug, new_slug, Some(title)) {
        Ok(report) => report,
        Err(ActionError::PageExists { existing }) => {
            let owner = find_by_slug(ws, &existing)
                .and_then(|existing_id| page_meta(ws, existing_id))
                .ok_or_else(|| format!("page not found: {existing}"))?;
            let reply = RenamePageReply::Exists { existing: owner };
            return Ok((reply, Vec::new(), Vec::new()));
        }
        Err(e) => return Err(e.to_string()),
    };
    let projected = project_and_retire(
        ws,
        host,
        &report.touched_pages,
        id,
        report.stale_projection.as_ref(),
    );
    let view = landing_view(ws, host, id, projected.primary_failure.as_ref())?;
    let mut announce = report.touched_pages.clone();
    announce.push(report.old_slug.clone());
    let reply = RenamePageReply::Renamed {
        view: Box::new(view),
        report: RenamePageReport::from(&report),
    };
    Ok((reply, projected.touched_ids, announce))
}

/// Fold `source` into `target` (each a slug or a page id).
///
/// Moves the source's blocks under the target, merges properties (the
/// target wins a conflict and the conflict is reported), keeps the
/// source's names as aliases of the target, rewrites mentions of the
/// source, and trashes the source. Confirm before calling: this command
/// does not re-prompt. Merging a page into itself, a journal day as the
/// source, or an unknown page is an `Err` string.
pub async fn merge_pages<S: AppHost>(
    state: &S,
    source: String,
    target: String,
) -> Result<MergePagesReply, String> {
    let host = Host::of(state)?;

    let (reply, ids) = offload::run("merge_pages", move || {
        let (reply, ids, announce) = offload::locked_mut(&host.workspace, |ws| {
            merge_locked(ws, &host, &source, &target)
        })?;
        host.announce(&announce);
        Ok((reply, ids))
    })
    .await?;

    forget_undo(state, &ids);
    Ok(reply)
}

/// [`merge_pages`]'s half that runs under the workspace lock.
fn merge_locked(
    ws: &mut Workspace,
    host: &Host,
    source: &str,
    target: &str,
) -> Result<Done<MergePagesReply>, String> {
    let report =
        outl_actions::merge_pages(ws, &host.hlc, source, target).map_err(|e| e.to_string())?;
    let target_id = find_by_slug(ws, &report.target.slug)
        .ok_or_else(|| format!("page not found: {}", report.target.slug))?;
    let projected = project_and_retire(
        ws,
        host,
        &report.touched_pages,
        target_id,
        Some(&report.source),
    );
    let view = landing_view(ws, host, target_id, projected.primary_failure.as_ref())?;
    let mut announce = report.touched_pages.clone();
    announce.push(report.source.slug.clone());
    // The source is trashed, so its slug no longer resolves; its id does.
    let mut ids = projected.touched_ids;
    if let Ok(id) = parse_node_id(&report.source.id) {
        ids.push(id);
    }
    let reply = MergePagesReply {
        view,
        report: MergePagesReport::from(&report),
    };
    Ok((reply, ids, announce))
}

/// Drop the undo / redo stacks of every page the call touched.
///
/// Taken after the workspace lock is released: it only needs the history
/// mutex, and lock order stays workspace then history.
fn forget_undo<S: AppHost>(state: &S, ids: &[NodeId]) {
    if let Some(history) = state.history() {
        let mut history = history.lock();
        for id in ids {
            history.remove(id);
        }
    }
}
