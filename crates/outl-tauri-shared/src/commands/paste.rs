//! The paste command bodies: a clipboard payload into the open page.
//!
//! Three commands, two shapes. [`paste_markdown_at`] and
//! [`paste_plain_at`] splice into the block the user is typing in, at
//! the caret. [`paste_block_after`] drops a payload next to a selected
//! block in view mode (desktop `p` / `Cmd+V`, mobile's block paste).
//! All three answer a [`PasteReply`], built by one `paste_reply`.
//!
//! Split out of `block.rs` under the file-size ratchet; the policy is
//! `outl_actions::paste`'s, and this is the anchor resolution plus the
//! commit.

use outl_actions::{
    paste_markdown as action_paste_markdown, paste_plain as action_paste_plain, ActionError,
    PasteAnchor, PasteOutcome,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::commands::backlink_edit::row_is_live_on;
use crate::commit::finish_in_page_with;
use crate::helpers::parse_node_id;
use crate::host::AppHost;
use crate::state::{PageView, PasteReply};

/// Paste `text` next to a selected block — the view-mode paste.
///
/// Routes through the same `paste_markdown` pipeline the caret paste
/// uses, so a copied subtree comes back with fresh ids and external
/// clipboard text is converted on the way in.
///
/// The anchor is resolved **once, inside the commit and before any
/// op**, by `paste_anchor`: `parent_id` (zoom root, empty zoom) when
/// it is live on the page, else `after_id` when it is, else the page
/// itself. A client captures both ids before an `await` on the OS
/// clipboard, so either can be stale by the time this runs; landing on
/// the page beats refusing a paste the user just asked for, and
/// landing on another page (an anchor a peer moved) is never right.
pub fn paste_block_after<S: AppHost>(
    state: &S,
    page_id: String,
    after_id: Option<String>,
    parent_id: Option<String>,
    text: String,
) -> Result<PasteReply, String> {
    let page = parse_node_id(&page_id)?;
    let after = after_id.as_deref().map(parse_node_id).transpose()?;
    let parent = parent_id.as_deref().map(parse_node_id).transpose()?;
    finish_in_page_with(state, page, |ws| {
        let anchor = paste_anchor(ws, page, after, parent)?;
        action_paste_markdown(ws, state.hlc(), anchor, &text)
    })
    .map(paste_reply)
}

/// Where a view-mode paste on `page` lands. See [`paste_block_after`].
///
/// "Live on the page" is [`row_is_live_on`], the predicate the backlink
/// row commands already gate on: in the tree, not under the trash, and
/// still enclosed by `page`. `after` naming the page root itself is
/// refused as an anchor, because a sibling of a page is a new page.
///
/// A page that is not live is `NotInTree` before anything is written,
/// which is what stops a paste landing under a page a peer deleted.
fn paste_anchor(
    ws: &Workspace,
    page: NodeId,
    after: Option<NodeId>,
    parent: Option<NodeId>,
) -> Result<PasteAnchor, ActionError> {
    if let Some(parent) = parent.filter(|&p| row_is_live_on(ws, page, p)) {
        return Ok(PasteAnchor::AsLastChildOf(parent));
    }
    if let Some(after) = after.filter(|&a| a != page && row_is_live_on(ws, page, a)) {
        return Ok(PasteAnchor::AfterBlock(after));
    }
    if row_is_live_on(ws, page, page) {
        return Ok(PasteAnchor::AsLastChildOf(page));
    }
    Err(ActionError::NotInTree(page.to_string()))
}

/// Paste external clipboard markdown as a tree of blocks. `caret` is a
/// Unicode codepoint offset into the host block's text — the frontend
/// converts `textarea.selectionStart` (UTF-16) via
/// `utf16OffsetToCharOffset` from `@outl/shared/paste` first.
///
/// Returns a [`PasteReply`], not a bare `PageView`: the host block's
/// new text rides along so a client editing it can re-seed its draft
/// instead of committing the pre-paste string back over the paste
/// ([issue #488](https://github.com/outlmd/outl/issues/488)).
pub fn paste_markdown_at<S: AppHost>(
    state: &S,
    page_id: String,
    block_id: String,
    caret: u32,
    text: String,
) -> Result<PasteReply, String> {
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;
    finish_in_page_with(state, page, |ws| {
        action_paste_markdown(
            ws,
            state.hlc(),
            PasteAnchor::AtCaret {
                block,
                caret: caret as usize,
            },
            &text,
        )
    })
    .map(paste_reply)
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
) -> Result<PasteReply, String> {
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;
    finish_in_page_with(state, page, |ws| {
        action_paste_plain(
            ws,
            state.hlc(),
            PasteAnchor::AtCaret {
                block,
                caret: caret as usize,
            },
            &text,
        )
    })
    .map(paste_reply)
}

/// Shape a paste's outcome into the wire reply, for every paste command.
///
/// One function because the three differ only in which `outl-actions`
/// entry point ran and where it was anchored; a second copy of this
/// mapping is how one of them would start forgetting to carry the caret
/// or the landing.
fn paste_reply((outcome, view): (PasteOutcome, PageView)) -> PasteReply {
    PasteReply {
        view,
        host_text: outcome.host_text,
        host_caret: outcome.host_caret.map(|c| c as u32),
        new_id: outcome.last_root.map(|id| id.to_string()),
    }
}
