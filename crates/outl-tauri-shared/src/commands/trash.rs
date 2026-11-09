//! Reading the trash back, as commands.
//!
//! Invariant 6 makes delete a `Move(node, TRASH_ROOT)`, so every client
//! has always *preserved* deletions. Until this module, only the CLI and
//! the MCP tools could read one back — so a GUI user who deleted the
//! wrong block had no way to it from inside the app, and the capability
//! catalog's nudge told them to open a terminal (issue #287).
//!
//! **Every policy question belongs to `outl_actions::trash`.** What can
//! be restored, where it lands, why it would be refused and when it was
//! deleted are all that module's, and this one neither re-asks nor
//! re-words any of them. In particular [`TrashEntryDto::refusal`] is
//! `trash::refusal_for`'s own sentence, carried per row, so a listing
//! cannot offer a restore the mutation then declines — the same
//! one-owner-of-the-verdict shape as root `CLAUDE.md` invariant 8.
//!
//! ## A refusal computed at list time can go stale
//!
//! It is the right shape and it is not a guarantee. The verdict depends
//! on the live tree, and between the listing and the click a peer can
//! trash the parent the block would land under, or move that parent
//! inside the block's own subtree. So the per-row `refusal` is a
//! *filter*, never a promise, and [`restore_trash_entry`] asks again
//! under the mutation lock.
//!
//! That second answer is deliberately **the same verdict**: both come
//! off `trash::refusal_for`'s own error, so a row that flips from
//! actionable to refused says what it would have said had the listing
//! been a second later. A client's job is to show it and re-list; the
//! wording is not the client's to invent.
//!
//! ## The way out is per surface, and this crate serves two of them
//!
//! A refusal's diagnosis is true everywhere; what the user can *do*
//! about it is not. A phone has no `outl` binary, and neither GUI shows
//! block ids (invariant 2), so a shared string handed a desktop user
//! `outl block tree <ULID>` and a phone user `outl doctor` — both
//! measured, both printed verbatim by `TrashPanel.tsx`.
//! [`trash::list`] therefore takes a [`Surface`], and `SURFACE` is
//! which one this build is.
//!
//! [`restore_trash_entry`]'s error is deliberately left as the bare
//! diagnosis: both clients re-list after a failed restore *on purpose*
//! (see the paragraph above), so the worded half arrives in the row the
//! sentence is about rather than twice in two voices. The one-shot
//! surfaces — `outl trash restore`, `outl_trash_restore` — have no
//! listing to come back to and word their own.

use serde::Serialize;

use outl_actions::trash;

use outl_actions::refusal::Surface;

use crate::helpers::{finish_in_page_with, parse_node_id, with_ws_mut};

/// Which surface a refusal reaching this backend is worded for.
///
/// The two GUI clients share this crate and do **not** share the answer:
/// a desktop user is sitting next to a shell, a phone has no `outl`
/// binary at all. So `trash::list` cannot be handed one shared string
/// (it used to name `outl doctor` and `outl block tree <ULID>` on both).
///
/// A `cfg` rather than an [`AppHost`] method, the same way
/// `capture_inbox` picks its inbox location in this crate: the fact is a
/// property of the build target, not of the host's runtime state, and an
/// associated const with no default would make the ten test hosts in
/// `tests/` declare a client they are not.
#[cfg(any(target_os = "ios", target_os = "android"))]
const SURFACE: Surface = Surface::Mobile;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
const SURFACE: Surface = Surface::Desktop;
use crate::host::AppHost;
use crate::offload;
use crate::state::PageView;

/// One top-level deletion on the wire.
///
/// "Top-level" is the node the `Move` named: deleting a subtree is one
/// op on its root, so everything underneath rides along and is **not** a
/// row of its own. [`Self::blocks`] is how many come back together.
#[derive(Debug, Clone, Serialize)]
pub struct TrashEntryDto {
    /// The deleted node's id, as [`restore_trash_entry`] takes it.
    pub id: String,
    /// One line of what the block said, for recognising it in a list.
    ///
    /// Named `text` rather than `preview` because that is what it is to
    /// a reader; the truncation is `outl_actions::trash`'s, and a client
    /// never re-truncates (it would cut the ellipsis the owner added).
    pub text: String,
    /// `Some(slug)` when the deletion is a whole page rather than a
    /// block. Restoring one is refused — see [`Self::refusal`] — so a
    /// listing that could not tell them apart would offer an action that
    /// always fails.
    pub page: Option<String>,
    /// The node plus everything under it: what the user gets back.
    pub blocks: usize,
    /// Wall-clock half of the HLC on the move that trashed it, in
    /// milliseconds since the epoch.
    ///
    /// A **clock**, not an ordering: it comes from whichever device did
    /// the deleting, so a client renders it and never re-sorts on it.
    /// The rows arrive in `outl_actions::trash::list`'s order, which is
    /// stable because it cannot depend on a peer's clock.
    pub deleted_at_ms: Option<u64>,
    /// Why a restore would be refused, or `None` when it would work.
    ///
    /// `outl_actions::trash::refusal_for`'s own sentence. Not recomputed
    /// here and not re-phrased here — and not a promise either, see the
    /// module doc.
    pub refusal: Option<String>,
}

/// What came back out of the trash.
#[derive(Debug, Clone, Serialize)]
pub struct RestoreTrashReply {
    /// The restored node's id.
    pub id: String,
    /// How many blocks came back with it.
    pub blocks: usize,
    /// The page it landed on, refreshed, so a client can show the user
    /// where their block went.
    ///
    /// `None` only when the block was deleted from directly under the
    /// tree root — every pre-page-model block, and anything
    /// `migrate_legacy_into_today` has not swept yet. There is no `.md`
    /// for that position, so there is nothing to project and nothing to
    /// navigate to; the restore still happened.
    pub view: Option<PageView>,
}

/// Every top-level deletion currently in the trash.
///
/// Async: `trash::list` walks every child of the trash root and folds
/// each one's parent trail out of the op log, so it is O(deletions ×
/// trail) under the workspace lock. The panel that calls it is opened
/// deliberately, which is exactly the shape [`crate::offload`] exists
/// for.
pub async fn list_trash<S: AppHost>(state: &S) -> Result<Vec<TrashEntryDto>, String> {
    let workspace = state.workspace_arc();
    offload::run("list_trash", move || {
        offload::locked(&workspace, |ws| {
            Ok(trash::list(ws, SURFACE).into_iter().map(to_dto).collect())
        })
    })
    .await
}

/// Project one `outl_actions::trash::TrashEntry` onto the wire shape.
fn to_dto(entry: trash::TrashEntry) -> TrashEntryDto {
    TrashEntryDto {
        id: entry.node.to_string(),
        text: entry.preview,
        page: entry.page,
        blocks: entry.subtree_len,
        deleted_at_ms: entry.deleted_at_ms,
        refusal: entry.refusal,
    }
}

/// Put one deleted block back where it was deleted from.
///
/// Routed through [`finish_in_page_with`] so the landing page gets the
/// whole commit pipeline — undo snapshot, backlink invalidation, peer
/// announce, `.md` + sidecar projection — rather than a bare `Move` that
/// leaves the page's `.md` still saying the block is gone.
///
/// The landing page has to be resolved **before** the mutation, because
/// that is what `commit_page` is scoped to, and a trashed node's own
/// ancestor walk reaches the trash root rather than a page. So this
/// reads under one lock and commits under another, and a peer can
/// re-parent the landing block in between.
///
/// If the landing is *gone*, the mutation declines: `trash::restore`
/// re-asks `trash::refusal_for` under its own lock.
///
/// If the landing merely **moved to another page**, the restore is
/// correct and the reply is stale, in three ways worth naming rather
/// than implying:
///
/// - the op log holds the block under its real new parent — that half
///   is right, and it is the half that survives;
/// - `view` is the page this resolved *first*, so a client that
///   navigates with it shows a page the restored block is not on, and
///   the user has to find it;
/// - `commit_page` snapshotted that same wrong page for undo, so `Cmd+Z`
///   right after offers the wrong page's previous state.
///
/// Both stale halves self-heal: the real landing page re-projects on its
/// next open or `reproject_stale_pages` sweep, and the undo stack is
/// per-page so the wrong entry is simply never offered on the right one.
/// Closing the window properly means resolving the page inside the
/// mutation lock, which `commit_page`'s one-page scope does not express
/// today ([#264](https://github.com/outlmd/outl/issues/264)).
pub fn restore_trash_entry<S: AppHost>(state: &S, id: String) -> Result<RestoreTrashReply, String> {
    let node = parse_node_id(&id)?;
    let landing_page = crate::helpers::with_ws(state, |ws| Ok(trash::landing_page(ws, node)))?;

    let Some(page) = landing_page else {
        // No page to commit against: the block was deleted from directly
        // under the tree root, its landing is a pre-page-model block, or
        // the trail cannot say where from. Only the last of those is an
        // error, and `trash::restore` is what reports it — with the
        // sentence the listing already carried.
        let entry = with_ws_mut(state, |ws| {
            trash::restore(ws, state.hlc(), node).map_err(|e| e.to_string())
        })?;
        return Ok(RestoreTrashReply {
            id: entry.node.to_string(),
            blocks: entry.subtree_len,
            view: None,
        });
    };

    let (entry, view) =
        finish_in_page_with(state, page, |ws| trash::restore(ws, state.hlc(), node))?;
    Ok(RestoreTrashReply {
        id: entry.node.to_string(),
        blocks: entry.subtree_len,
        view: Some(view),
    })
}
