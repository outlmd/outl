//! Block mutation command bodies.
//!
//! Each parses ids, delegates the mutation to `outl-actions`, and
//! projects the result back to `.md` + sidecar via
//! [`crate::helpers::finish_in_page`]. The only deliberate exception is
//! [`set_block_collapsed`], which bypasses reprojection — see its doc.

use outl_actions::{
    append_block, apply_page_md_with_sidecar_guarded, copy_markdown as action_copy_markdown,
    create_after_or_append, create_before_or_append, delete, edit_text, enclosing_page_id, indent,
    move_after, move_down, move_up, outdent, paste_markdown as action_paste_markdown,
    paste_plain as action_paste_plain, render_block_md,
    set_block_collapsed as action_set_block_collapsed, split_block as action_split_block,
    toggle_quote as action_toggle_quote, toggle_todo as action_toggle_todo, ActionError,
    PasteAnchor,
};
use outl_md::index::WorkspaceIndex;
use tracing::warn;

use crate::helpers::{
    announce_after_commit, build_page_view, finish_in_page, finish_in_page_with, parse_node_id,
    with_ws, with_ws_mut,
};
use crate::host::AppHost;
use crate::state::{CreateBlockReply, CutBlockReply, PageView};

/// Create a block. Precedence: `before_id` (vim `O` /
/// `Cmd/Ctrl+Shift+Enter` at col 0) wins over `after_id` (vim `o` /
/// `Enter`); falling back to "last child of `parent_id`" (defaults to
/// the page root) when neither is set. The `after_id` branch tolerates a
/// stale anchor: a peer reload / re-mint can leave the node out of the
/// tree, and appending at the page end beats surfacing "block X is not
/// in the tree" when the user hit `o`. That fallback lives in
/// `outl-actions` so every client shares it.
pub fn create_block<S: AppHost>(
    state: &S,
    page_id: String,
    after_id: Option<String>,
    before_id: Option<String>,
    parent_id: Option<String>,
    text: Option<String>,
) -> Result<CreateBlockReply, String> {
    let page = parse_node_id(&page_id)?;
    let text_owned = text.clone();
    let (new_id, view) = finish_in_page_with(state, page, |ws| {
        if let Some(id) = &before_id {
            let node = parse_node_id(id).map_err(ActionError::NotInTree)?;
            create_before_or_append(ws, state.hlc(), page, node, text_owned.as_deref())
        } else if let Some(id) = &after_id {
            let node = parse_node_id(id).map_err(ActionError::NotInTree)?;
            create_after_or_append(ws, state.hlc(), page, node, text_owned.as_deref())
        } else {
            let parent = match &parent_id {
                Some(id) => parse_node_id(id).map_err(ActionError::NotInTree)?,
                None => page,
            };
            append_block(ws, state.hlc(), Some(parent), text_owned.as_deref())
        }
    })?;
    Ok(CreateBlockReply {
        view,
        new_id: new_id.to_string(),
    })
}

/// Split a block at the caret: the text up to `char_offset` stays in the
/// block, the rest moves into a new sibling created right below. Returns
/// the new sibling's id so the client parks the caret at its start.
///
/// `char_offset` is a **codepoint** offset (the client converts the
/// textarea's UTF-16 `selectionStart` first, same as `paste_plain_at`).
/// `char_offset == 0` empties the block and pushes its text down (open a
/// block above); `char_offset` at/after the end leaves the text and
/// creates an empty sibling (plain "Enter at end of line").
///
/// Tolerates a stale anchor exactly like [`create_block`]: if a sync
/// reload re-minted or trashed the node between the caret read and this
/// commit, it degrades to the old "empty sibling below" so Enter still
/// yields a block instead of surfacing a dead-id Retry toast.
pub fn split_block<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
    char_offset: u32,
) -> Result<CreateBlockReply, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    let (new_id, view) = finish_in_page_with(state, page, |ws| {
        match action_split_block(ws, state.hlc(), node, char_offset as usize) {
            Err(ActionError::NotInTree(_)) => {
                warn!("split_block: node {node} not in tree (re-minted by a sync reload?); creating an empty sibling instead");
                create_after_or_append(ws, state.hlc(), page, node, None)
            }
            other => other,
        }
    })?;
    Ok(CreateBlockReply {
        view,
        new_id: new_id.to_string(),
    })
}

pub fn edit_block<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
    text: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    let registry = state.exec_registry();
    finish_in_page(state, page, |ws| {
        // Tolerate a stale block id. Between the frontend reading the block's
        // id and this commit landing, a concurrent sync reload can re-mint or
        // trash that node (its content already converged via the peer's ops),
        // leaving `edit_text` with nowhere to write. Surfacing the raw
        // `block <id> is not in the tree` as a Retry toast is the wrong UX —
        // Retry re-fails forever against the same dead id. Treat it as a no-op
        // and let the projection below refresh the page to the merged state, so
        // the client swaps its stale `editingId` for the live tree. This mirrors
        // the stale-anchor tolerance `create_after_or_append` already has.
        match edit_text(ws, state.hlc(), node, &text) {
            Ok(()) => {}
            Err(ActionError::NotInTree(_)) => {
                warn!(
                    "edit_block: node {node} not in tree (re-minted by a sync reload?); skipping"
                );
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        // Finishing an edit on a `call:<name>` block re-runs it so the
        // `> **result:**` reflects the freshly-typed params. Best-effort:
        // a failing template must never drop the edit itself.
        if let Some((name, params)) = outl_actions::parse_call_invocation(&text) {
            let _ =
                outl_actions::run_callable_block(ws, state.hlc(), &registry, &name, &params, node);
        }
        Ok(())
    })
}

pub fn toggle_todo<S: AppHost>(state: &S, page_id: String, id: String) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(state, page, |ws| action_toggle_todo(ws, state.hlc(), node))
}

pub fn toggle_quote<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(state, page, |ws| action_toggle_quote(ws, state.hlc(), node))
}

pub fn delete_block<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(state, page, |ws| delete(ws, state.hlc(), node))
}

pub fn indent_block<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(state, page, |ws| match indent(ws, state.hlc(), node) {
        Err(ActionError::NoPreviousSibling(_)) => Ok(()),
        other => other,
    })
}

pub fn outdent_block<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(state, page, |ws| match outdent(ws, state.hlc(), node) {
        Err(ActionError::AlreadyAtRoot(_)) => Ok(()),
        other => other,
    })
}

pub fn move_block_up<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(state, page, |ws| move_up(ws, state.hlc(), node))
}

pub fn move_block_down<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    finish_in_page(state, page, |ws| move_down(ws, state.hlc(), node))
}

/// Move `node` (`id`) to sit immediately after `after_id`, re-parenting
/// it under the target's parent. This is the workspace side of the
/// cut-and-paste-block gesture (`Cmd+X` then `Cmd+V` in view mode): the
/// block keeps its identity, so every `((blk-…))` ref and backlink
/// pointing at it stays valid.
///
/// `page_id` is the page the user is viewing (the target's page). When
/// the cut block came from a *different* page, that source page is
/// re-rendered too so its `.md` no longer lists the moved block. A paste
/// that would drop the block inside its own subtree is rejected upstream
/// (`WouldCreateCycle`) — the frontend nudges instead of emitting a
/// no-op move.
pub fn move_block_after<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
    after_id: String,
) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    let after = parse_node_id(&after_id)?;
    with_ws_mut(state, |ws| {
        // Capture the source page *before* the move so a cross-page
        // paste re-renders the page the block left behind, not only the
        // one it landed on.
        let source_page = enclosing_page_id(ws, node);
        move_after(ws, state.hlc(), node, after).map_err(|e| e.to_string())?;
        // Guarded: a mutation must project, but projecting must not
        // delete content the op log never saw (invariant 8). The move
        // itself is already in the log; only the on-disk `.md` lags, and
        // the open path raises a banner explaining why.
        if let Err(e) = apply_page_md_with_sidecar_guarded(ws, &root, page) {
            warn!("destination page md+sidecar sync skipped: {e}");
        }
        if let Some(src) = source_page {
            if src != page {
                if let Err(e) = apply_page_md_with_sidecar_guarded(ws, &root, src) {
                    warn!("source page md+sidecar sync skipped: {e}");
                }
            }
        }
        build_page_view(ws, &root, page).map_err(|e| e.to_string())
    })
}

/// Render the block `id` and its subtree to clean outl markdown for the
/// block clipboard (`Cmd+C` in view mode). Read-only — the paste
/// re-ingests this text via [`paste_block_after`] and mints fresh ids,
/// so a copy duplicates rather than moves.
pub fn copy_block_markdown<S: AppHost>(state: &S, id: String) -> Result<String, String> {
    let node = parse_node_id(&id)?;
    with_ws(state, |ws| Ok(render_block_md(ws, node)))
}

/// Paste clipboard `text` (clean outl markdown) as the sibling(s)
/// immediately after `after_id` — the `Cmd+V` of a *copied* block in
/// view mode. Routes through the same `paste_markdown` pipeline the
/// external-clipboard paste uses, so the duplicated subtree gets fresh
/// ids.
pub fn paste_block_after<S: AppHost>(
    state: &S,
    page_id: String,
    after_id: String,
    text: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let after = parse_node_id(&after_id)?;
    finish_in_page(state, page, |ws| {
        action_paste_markdown(ws, state.hlc(), PasteAnchor::AfterBlock(after), &text).map(|_| ())
    })
}

/// Cut the block `id` (and its subtree): renders it to clipboard
/// markdown exactly like [`copy_block_markdown`], then removes it via
/// `outl_actions::delete` — `Op::Move(node, TRASH_ROOT)`, never a
/// physical removal (root `CLAUDE.md` invariant 6).
///
/// Paste with [`paste_block_after`], which mints fresh ids for the
/// pasted copy — a cut+paste round trip duplicates identity rather
/// than preserving it. That is deliberately different from the
/// desktop frontend's own view-mode Cmd+X/Cmd+V (which reparents the
/// block via [`move_block_after`] and keeps every `((blk-…))` ref
/// valid): this command exists for a client whose block clipboard is
/// a plain markdown string, with no anchor back to the original node
/// once the cut has happened — the mobile long-press "Cut" gesture,
/// RFC 0254 phase 4.
pub fn cut_block<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
) -> Result<CutBlockReply, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    let markdown = with_ws(state, |ws| Ok(render_block_md(ws, node)))?;
    let view = finish_in_page(state, page, |ws| delete(ws, state.hlc(), node))?;
    Ok(CutBlockReply { markdown, view })
}

/// Set or flip the `collapsed` flag on a block. Deliberately bypasses
/// `finish_in_page` — `Op::SetCollapsed` changes neither the `.md` body
/// nor the sidecar, so reprojecting would just bump file-transport
/// upload metadata for every fold gesture.
pub fn set_block_collapsed<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
    collapsed: bool,
) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    with_ws_mut(state, |ws| {
        action_set_block_collapsed(ws, state.hlc(), node, collapsed).map_err(|e| e.to_string())?;
        // SetCollapsed is still a real op that must converge — announce
        // it like any other commit (this path bypasses `finish_in_page`,
        // which is where the announce normally lives).
        announce_after_commit(state, ws, page);
        build_page_view(ws, &root, page).map_err(|e| e.to_string())
    })
}

/// Paste external clipboard markdown as a tree of blocks. `caret` is a
/// Unicode codepoint offset into the host block's text — the frontend
/// converts `textarea.selectionStart` (UTF-16) via
/// `utf16OffsetToCharOffset` from `@outl/shared/paste` first.
pub fn paste_markdown_at<S: AppHost>(
    state: &S,
    page_id: String,
    block_id: String,
    caret: u32,
    text: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;
    finish_in_page(state, page, |ws| {
        action_paste_markdown(
            ws,
            state.hlc(),
            PasteAnchor::AtCaret {
                block,
                caret: caret as usize,
            },
            &text,
        )
        .map(|_| ())
    })
}

/// Paste clipboard text **without formatting**: the raw string is
/// spliced into the host block at `caret` — no outline detection, no
/// syntax normalization, no paragraph splitting. The "with formatting"
/// counterpart is [`paste_markdown_at`].
pub fn paste_plain_at<S: AppHost>(
    state: &S,
    page_id: String,
    block_id: String,
    caret: u32,
    text: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;
    finish_in_page(state, page, |ws| {
        action_paste_plain(
            ws,
            state.hlc(),
            PasteAnchor::AtCaret {
                block,
                caret: caret as usize,
            },
            &text,
        )
        .map(|_| ())
    })
}

/// Serialize the given blocks (each with its subtree) to clean outl
/// markdown for the OS clipboard. Read-only — the frontend writes the
/// returned string to `navigator.clipboard` itself.
///
/// `block_ids` arrives in document order (a single yank, or a Visual
/// range top-to-bottom); the markdown preserves that order. A
/// **malformed** id fails the whole call at parse time; an id that
/// simply isn't in the tree (stale / re-minted selection) is silently
/// skipped by `outl_actions::copy_markdown` rather than emitting a blank
/// bullet. That serializer (the inverse of `paste_markdown`) is shared
/// so the TUI and every GUI client produce byte-identical output.
pub fn copy_markdown<S: AppHost>(state: &S, block_ids: Vec<String>) -> Result<String, String> {
    let roots: Vec<_> = block_ids
        .iter()
        .map(|id| parse_node_id(id))
        .collect::<Result<_, _>>()?;
    with_ws(state, |ws| Ok(action_copy_markdown(ws, &roots)))
}

/// Produce the `((blk-XXXXXX))` reference string for block `id`
/// (issue #18) — the mobile/desktop counterpart of the TUI's `y r`
/// chord (`outl-tui/src/actions/yank.rs::yank_current_ref`).
///
/// Reuses the **same** handle the TUI copies: `WorkspaceIndex` (built
/// fresh from disk, same as [`crate::commands::page::search_blocks`])
/// is the one owner of ref-handle assignment, including the lazy
/// collision expansion past the default 6-char tail — deriving the
/// handle straight from the `NodeId` here would produce a ref that
/// silently disagrees with the sidecar's expanded form and never
/// resolves. Errors when the block has no sidecar entry yet (created
/// this session, not saved) — an unresolvable ref is worse than none.
pub fn copy_block_ref<S: AppHost>(state: &S, id: String) -> Result<String, String> {
    let node = parse_node_id(&id)?;
    let root = state.storage_root()?;
    let index = WorkspaceIndex::build(&root);
    let entry = index
        .block_index()
        .get(node)
        .ok_or_else(|| "no ref handle yet — save and retry".to_string())?;
    Ok(format!("(({}))", entry.ref_handle))
}
