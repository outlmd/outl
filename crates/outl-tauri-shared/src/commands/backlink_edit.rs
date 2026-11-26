//! The cross-page mutations: everything a client does to a page it is
//! **not** showing, because the user acted on a backlink row.
//!
//! Its own module beside [`super::block`], which sits at its size
//! baseline — the `commands/page_backlinks.rs` answer to the same
//! pressure. It belongs apart for a reason beyond the line count: every
//! body in `block.rs` acts on the page the user is looking at and may
//! therefore be *tolerant* of a stale id, and nothing here may be.
//! Keeping them in one file is how a later reader "unifies" the two.
//!
//! **Twelve gestures, one answer to a stale id.** A text edit is
//! [`edit_backlink_block`]; the other eleven are [`backlink_row_op`] and
//! [`create_backlink_sibling`], and all three take the verdict from
//! `stale_row_block` so the sentence cannot differ per gesture. Nine
//! `block.rs` commands each growing a backlink-aware arm was the
//! alternative, and it is nine places for the wording to drift from —
//! which is exactly the state this replaced: the eight chords routed
//! through `block.rs` and printed `ActionError`'s `Display`, a ULID and
//! an implementation noun, while
//! [`outl_actions::refusal::stale_backlink_block_unchanged_refusal`]
//! sat written and called only by the TUI (root `CLAUDE.md`
//! invariant 12 — *the nudge wording belongs in the catalog, never in a
//! client*).

use outl_actions::refusal::{
    ahead_of_log_notice, stale_backlink_block_refusal, stale_backlink_block_unchanged_refusal,
};
use outl_actions::{
    create_after, create_before, edit_text, enclosing_page_id, is_live, ActionError,
};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use serde::{Deserialize, Serialize};

mod row_op;
pub use row_op::BacklinkRowOp;

use crate::commit::{announce_after_commit, finish_in_page_with_refresh, BacklinkRefresh};
use crate::helpers::{build_page_view, parse_node_id, reproject_stale_md, with_ws, with_ws_mut};
use crate::host::AppHost;
use crate::state::{CreateBlockReply, PageView};
use crate::surface::SURFACE;

/// Replace a block's text **through a backlink row**, where the block
/// lives on a page the user is not looking at.
///
/// Everything it does, [`super::block::edit_block`] already does. The difference is
/// how the two *recover*, and that is why this is a second command
/// rather than a flag:
///
/// - **A stale block id is refused, never re-targeted.** A backlink's
///   `block_id` can be transient: `build_backlink_index_from_disk` reads
///   each page through `read_page_outline`, and `outline_from_parsed`
///   mints a fresh `NodeId` per block the sidecar does not cover. On the
///   open page `edit_block`'s append keeps the text where the user can
///   still see it; here the end of a page they are not on is not a place
///   their text may be moved to — root `CLAUDE.md` invariant 8's shape
///   through a different door. So the edit is refused, in the words of
///   [`stale_backlink_block_refusal`] (a deleted block and an id that
///   never named one are told apart there) rather than the
///   `ActionError`'s `Display`, which is a raw id and an
///   implementation noun.
///   It is refused **first**, before the ahead-of-log check below,
///   because that check *writes* on a merely-stale page — see
///   `refuse_a_dead_row`, which owns the ordering and why the
///   in-closure verdict is still asked underneath it.
/// - **A page ahead of its log is refused before anything is written.**
///   Those lines exist in no op, so the edit would apply to the *logged*
///   text — and here that is **not** the text the user edited: both
///   clients seed the draft from `link.source_block`, which
///   `build_backlink_index_from_disk` built from the `.md`
///   (`BacklinkRowEditor.tsx` → `rawTextWithTodo(props.link.source_block)`).
///   So the user typed over bytes the log does not hold, the projection
///   would then be refused, and the row would come back reading what it
///   read before. The check is [`reproject_stale_md`], the same one
///   every open path asks, and every word of the refusal is
///   [`outl_actions::refusal::ahead_of_log_notice`]'s: a client decides
///   where a notice goes, never what it says.
///   **The refusal reaches the user before they type**, which is the
///   other half of why it is right here: a client's rule is that a
///   refused edit keeps the editor up on its draft, so nothing of
///   theirs is dropped either way.
///   The TUI deliberately does *not* ask this question for its text
///   edit, and the asymmetry is the draft's origin rather than a rule:
///   `enter_insert_backlink` seeds its buffer from the **op log**, so
///   there the logged text *is* what the user edited. See
///   `outl_tui::actions::block::backlink_gesture`.
///
/// It also keeps the host's cached backlink index instead of dropping
/// it ([`BacklinkRefresh::PatchPage`]). Editing text in a backlink row
/// is the daily `#fup` flow, and a whole-workspace rebuild per commit is
/// root `CLAUDE.md` invariant 15's third question asked of a path that
/// is about to get much hotter.
///
/// The reply is a `PageView` of the **source** page, which is not the
/// page on screen unless it cites itself. A client applies it only in
/// that case.
pub fn edit_backlink_block<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
    text: String,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    refuse_a_dead_row(state, page, node, stale_backlink_block_refusal)?;
    let root = state.storage_root()?;
    let outcome = with_ws(state, |ws| {
        Ok(reproject_stale_md(ws, &root, page, "edit_backlink_block"))
    })?;
    // `outcome.other_error` is deliberately not read: a retryable I/O
    // condition is a `tracing::warn!` and nothing else here, exactly as
    // on every open path. The edit still goes through `Workspace::apply`,
    // and the next open or the orphan scanner picks the page up.
    if let Some(info) = outcome.ahead_of_log {
        return Err(ahead_of_log_notice(SURFACE, info.lines, &info.sample).one_line());
    }
    // Worded inside the commit closure, where the workspace is in hand
    // and the verdict is about the same state the edit just failed on.
    let mut gone = None;
    finish_in_page_with_refresh(state, page, BacklinkRefresh::PatchPage, |ws| {
        let result = edit_text(ws, state.hlc(), node, &text);
        if matches!(result, Err(ActionError::NotInTree(_))) {
            gone = Some(stale_backlink_block_refusal(ws, node));
        }
        result
    })
    .map(|((), view)| view)
    .map_err(|e| gone.map_or(e, str::to_string))
}

/// Which side of the citing block a new sibling lands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BacklinkSide {
    /// `o` — the sibling immediately after the citing block.
    Below,
    /// `O` — the sibling immediately before it.
    Above,
}

/// Run a structural or task-state mutation on the block a backlink row
/// shows, on **its own** page.
///
/// Everything here `block.rs` already does. What this adds is the one
/// thing a cross-page gesture needs and an open-page one must not: a
/// stale row id is **refused in the catalog's words**, before any op is
/// written.
///
/// That is not decoration. A backlink's `block_id` is routinely
/// transient — `build_backlink_index_from_disk` reads each page through
/// `read_page_outline`, and `outline_from_parsed` mints a fresh
/// `NodeId` per block the sidecar does not cover — so this refusal is
/// the common case on a graph whose sidecars lag, not an edge. Routed
/// through `block.rs` these eight printed
/// `"block 01J… is not in the tree"`, while the TUI printed *"That
/// block was deleted, so nothing changed."* for the same keystroke.
///
/// **Two of the nine also refuse a source page ahead of its log,
/// before anything is written**, and which two is
/// `BacklinkRowOp::shape`'s mapping put to
/// [`CrossPageWrite::computed_from_the_logged_text`](outl_actions::CrossPageWrite::computed_from_the_logged_text) rather than a
/// classification of "structural". `ToggleTodo` and `MarkDone` are text
/// read-modify-write: they compute an op from the logged text while the
/// row the user tapped was rendered from the `.md`. The check and the
/// wording are [`edit_backlink_block`]'s, asked the same way for the
/// same reason, and it runs **before** the commit so a refusal does not
/// pay a `render_page_md` for the undo snapshot of a mutation that will
/// not happen.
///
/// Nothing is typed on any of these nine, so a refusal costs the user
/// nothing to weigh against it — which is why
/// `outl_actions::refusal::stale_backlink_block_unchanged_refusal` is
/// the half `stale_row_block` returns.
///
/// **The liveness verdict is asked twice, deliberately, and the first
/// ask comes before the ahead-of-log check** — which *writes* on a
/// merely-stale page, so a chord aimed at a deleted block used to
/// rewrite a `.md` and then refuse. `refuse_a_dead_row` owns that
/// ordering, the TUI's argument for it, and why `stale_row_block` is
/// still asked under the commit's own lock.
///
/// Three of the ops would not even have refused. `move_up` / `move_down`
/// answer `Ok(())` when they find no sibling, and a node the tree never
/// held has no parent to look under — so a stale id was a silent
/// success, and a *trashed* one would have reordered blocks inside the
/// trash. `indent` is the same shape through `NoPreviousSibling`, which
/// `block.rs` swallows on purpose. So the liveness gate is **this
/// function's**, asked once up front, rather than each action's own
/// error: four of the nine do not raise one.
///
/// The no-op arms `block.rs` swallows are swallowed here too
/// (`NoPreviousSibling`, `AlreadyAtRoot`): a block already at the top of
/// its page has nothing to indent into, which is not a refusal.
///
/// [`BacklinkRefresh::DropAll`] deliberately, and now measured rather
/// than merely declined. `PatchPage` is **not wrong** here — the index is
/// keyed by source page and every one of these gestures is confined to
/// `page_id` — it is dearer where the user can feel it: it forces a
/// synchronous `.md` + sidecar fsync pair on the IPC thread, 21-25 ms per
/// gesture and flat in page size, to save a 280 ms index rebuild on a
/// background thread nothing awaits. `edit_backlink_block` is the one
/// gesture that bought the exception with a measurement of its own.
/// The fixture, the table and the held-key case are in
/// `tests/backlink_commit_cost.rs`, whose `POLICY` census is what stops
/// the three commands being "unified" onto one policy — no behavioural
/// test can fail on that, because both policies leave the same `.md`.
///
/// `Collapse` / `Expand` take neither policy: they skip the commit
/// pipeline the way the outline's `set_block_collapsed` does
/// (`BacklinkRowOp::reprojects`), so `fold_row` writes one
/// `Op::SetCollapsed`, announces it, and touches no file.
pub fn backlink_row_op<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
    op: BacklinkRowOp,
) -> Result<PageView, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    refuse_a_dead_row(state, page, node, stale_backlink_block_unchanged_refusal)?;
    if op.shape().computed_from_the_logged_text() {
        let root = state.storage_root()?;
        let outcome = with_ws(state, |ws| {
            Ok(reproject_stale_md(ws, &root, page, "backlink_row_op"))
        })?;
        // `outcome.other_error` is deliberately not read, for
        // `edit_backlink_block`'s reason: a retryable I/O condition is a
        // `tracing::warn!` and nothing else, and the op log is still the
        // right place for the write.
        if let Some(info) = outcome.ahead_of_log {
            return Err(ahead_of_log_notice(SURFACE, info.lines, &info.sample).one_line());
        }
    }
    if !op.reprojects() {
        return fold_row(state, page, node, op);
    }
    // Worded inside the commit closure for `edit_backlink_block`'s
    // reason: the workspace is in hand, so the verdict describes the
    // state the gesture actually landed on.
    let mut refused = None;
    finish_in_page_with_refresh(state, page, BacklinkRefresh::DropAll, |ws| {
        if let Some(words) = stale_row_block(ws, page, node) {
            refused = Some(words);
            return Err(ActionError::NotInTree(node.to_string()));
        }
        op.write(ws, state.hlc(), node)
    })
    .map(|((), view)| view)
    .map_err(|e| refused.map_or(e, str::to_string))
}

/// A fold from a backlink row: the outline's `set_block_collapsed`
/// shape, aimed at the source page and behind the same in-lock liveness
/// verdict every other row op asks.
///
/// No commit pipeline, so no undo snapshot, no projection and no index
/// refresh — see [`BacklinkRowOp::reprojects`]. The announce is the one
/// step it still owes, because `Op::SetCollapsed` has to converge.
fn fold_row<S: AppHost>(
    state: &S,
    page: NodeId,
    node: NodeId,
    op: BacklinkRowOp,
) -> Result<PageView, String> {
    let root = state.storage_root()?;
    with_ws_mut(state, |ws| {
        if let Some(words) = stale_row_block(ws, page, node) {
            return Err(words.to_string());
        }
        op.write(ws, state.hlc(), node).map_err(|e| e.to_string())?;
        announce_after_commit(state, ws, page);
        build_page_view(ws, &root, page).map_err(|e| e.to_string())
    })
}

/// Create an empty sibling beside the block a backlink row shows, on
/// **its own** page.
///
/// `create_after` / `create_before`, never the `*_or_append` pair
/// [`super::block::create_block`] routes through. That pair's append
/// tolerance is documented for *the open page only* — "the frontend's
/// selected-block id can go stale … this appends the new block at the
/// end of `page` so the keystroke still produces a block" — and the end
/// of a page the user is not looking at is not a place their keystroke
/// may land.
///
/// With it, `o` on a stale backlink row created an empty block at the
/// end of the source page and then reported *"Added an empty block
/// after this reference"*, which is false about the one thing the
/// sentence exists to say. Held down, it accumulated one junk block per
/// repeat on a page nothing on screen showed. That is the exact
/// recovery the `edit_backlink_block` / `edit_block` split was created
/// to prevent, reached through a different door.
pub fn create_backlink_sibling<S: AppHost>(
    state: &S,
    page_id: String,
    id: String,
    side: BacklinkSide,
) -> Result<CreateBlockReply, String> {
    let page = parse_node_id(&page_id)?;
    let node = parse_node_id(&id)?;
    let mut refused = None;
    let (new_id, view) = finish_in_page_with_refresh(state, page, BacklinkRefresh::DropAll, |ws| {
        // **Not redundant with `create_after`'s own `ensure_live`, and
        // each half was measured separately.** The strict pair below is
        // what stops the append: with this gate off and `create_after`
        // in place, nothing lands on the page. This gate is what makes
        // the refusal *worded*: with it off, the same call reports
        // `"block 01M7… is not in the tree"` — a ULID and an
        // implementation noun, which is the state the eight chords
        // shipped in.
        if let Some(words) = stale_row_block(ws, page, node) {
            refused = Some(words);
            return Err(ActionError::NotInTree(node.to_string()));
        }
        match side {
            BacklinkSide::Below => create_after(ws, state.hlc(), node, Some("")),
            BacklinkSide::Above => create_before(ws, state.hlc(), node, Some("")),
        }
    })
    .map_err(|e| refused.map_or(e, str::to_string))?;
    Ok(CreateBlockReply {
        view,
        new_id: new_id.to_string(),
    })
}

/// The sentence for a row whose block is no longer live, or `None` when
/// it is.
///
/// **The one owner of that verdict for every cross-page gesture**, and
/// the reason the ten of them cannot say different things about the same
/// block. The wording itself is
/// [`outl_actions::refusal::stale_backlink_block_unchanged_refusal`]'s —
/// the *chord* half of the catalog, because nothing was typed, so the
/// only news is that the gesture did not land. Claiming "your text could
/// not be saved" on `Ctrl+T` names a loss that did not happen; that half
/// belongs to [`edit_backlink_block`] alone.
///
/// The liveness test is [`outl_actions::is_live`], the `bool` half of
/// `block::ensure_live`: a node the tree never held, **or** one sitting
/// under the trash (a peer's delete is an `Op::Move`, so the record
/// survives and `contains` still answers yes — root `CLAUDE.md`
/// invariant 6) — **and** the block must still live under the row's
/// page, because a child row names a block a peer can move to another
/// page without deleting it.
///
/// Asked, not re-derived. The pair used to be spelled out inline here,
/// because `ensure_live` was private to `outl-actions`, and the TUI had
/// a third copy — so what "live" means could have come apart between
/// the guard behind the actions and the two gates in front of them.
fn stale_row_block(workspace: &Workspace, page: NodeId, node: NodeId) -> Option<&'static str> {
    if row_is_live_on(workspace, page, node) {
        return None;
    }
    Some(stale_backlink_block_unchanged_refusal(workspace, node))
}

/// Whether `node` is live **and still under `page`**.
///
/// A row carries `(page_id, block_id)` from the moment the index was
/// built. A peer's edit or a cross-page move can put the block on
/// another page afterwards, and a commit scoped to `page` (undo
/// snapshot, projection) would then land on the wrong one. The page
/// walk is [`outl_actions::enclosing_page_id`], an `O(depth)` climb up
/// parent pointers with no workspace scan; a block moved elsewhere gets
/// the "could not be found" sentence, because from this row it is.
///
/// `pub(crate)` because the open page's block paste asks the same
/// question of its anchor (`commands::paste`): a block a peer trashed
/// or moved to another page is not somewhere a paste on *this* page may
/// land. Asked of the page itself it answers "is the page live", since
/// [`outl_actions::enclosing_page_id`] of a page root is that root.
pub(crate) fn row_is_live_on(workspace: &Workspace, page: NodeId, node: NodeId) -> bool {
    is_live(workspace, node) && enclosing_page_id(workspace, node) == Some(page)
}

/// The same verdict, asked **before** the command touches the disk.
///
/// The second half of a pair, and both halves are load-bearing.
///
/// **Why a pre-check.** [`edit_backlink_block`] and [`backlink_row_op`]
/// ran [`reproject_stale_md`] *first* and asked about the row's block
/// only inside the commit closure. That call does not merely read: on a
/// page whose `.md` is **merely stale** it refreshes it, so a gesture
/// aimed at a block a peer had already deleted (or moved off the
/// row's page) rewrote a file on a page
/// the user was not looking at and then refused. The TUI asks in the
/// other order and documents why —
/// *"a chord aimed at a block that is gone should not touch the disk at
/// all"* — and it is right: the disk write is a side effect of a check
/// for a gesture that is about to be declined, and the commit behind it
/// renders the page once more for the undo snapshot before the closure
/// gets to refuse. Two clients, opposite order, different outcome for
/// one gesture.
///
/// It also decides the **sentence**. On a page ahead of its log the old
/// order reported the ahead-of-log notice for a row whose block was
/// deleted, where the TUI said the block was gone. The block is the
/// nearer fact and the one the user acted on.
///
/// **Why the in-closure verdict stays.** `stale_row_block` is still
/// asked under the commit's own lock, because this read and that one are
/// not the same question asked twice: a peer's delete can land in the
/// window between them (`backlink_edit_concurrent.rs` is the suite for
/// exactly that window), and there the workspace is in hand so the
/// sentence describes the state the gesture actually landed on. Dropping
/// it would trade a refusal for a write into the trash.
///
/// **What the second read costs.** [`is_live`] is `contains` plus a
/// bounded parent walk, so the live path pays one extra lock
/// acquisition and an `O(depth)` walk — against a `render_page_md` of
/// the whole source page, and on two of the nine gestures a `.md` +
/// sidecar fsync pair, on the refused path. `tests/backlink_commit_cost.rs`
/// is the census that would have to change if this ever stopped being
/// worth it; it counts *page walks*, and this is not one.
///
/// `wording` is a parameter rather than a constant because the two
/// commands owe different costs: a chord typed nothing, a text edit has
/// a draft behind it. Both sentences are the catalog's either way.
fn refuse_a_dead_row<S: AppHost>(
    state: &S,
    page: NodeId,
    node: NodeId,
    wording: fn(&Workspace, NodeId) -> &'static str,
) -> Result<(), String> {
    match with_ws(state, |ws| {
        Ok((!row_is_live_on(ws, page, node)).then(|| wording(ws, node)))
    })? {
        Some(words) => Err(words.to_string()),
        None => Ok(()),
    }
}
