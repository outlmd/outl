//! Paste external markdown as a tree of blocks.
//!
//! When a user copies a chunk of bullet-list markdown from another app
//! (Roam, Logseq, GitHub issue, Notion export, Notes.app draft) and
//! pastes it into outl, we want the hierarchy to come across — not a
//! single block with literal `\n- ` characters in it. This module is
//! the **single** entry point both clients (TUI, mobile) call to make
//! that happen, so the semantics stay identical between surfaces.
//!
//! ## What gets converted to outl syntax on the way in
//!
//! `paste_markdown` runs the raw clipboard text through
//! [`normalize::normalize_external_syntax`] before parsing it. The
//! conversions cover the most common syntaxes the user might copy
//! from:
//!
//! | Input (external) | Output (outl) | Origin |
//! |------------------|---------------|--------|
//! | `{{[[TODO]]}} foo` | `TODO foo` | Roam |
//! | `{{[[DONE]]}} foo` | `DONE foo` | Roam |
//! | `- [ ] foo` | `- TODO foo` | GitHub / CommonMark task list |
//! | `- [x] foo` / `- [X] foo` | `- DONE foo` | GitHub / CommonMark |
//! | `{{embed: ((blk-XXXXXX))}}` | `!((blk-XXXXXX))` | Roam |
//! | `{{[[query]]: foo}}` | `{{query: foo}}` | Roam |
//! | `^^highlight^^` | (stripped) | Roam |
//! | `{{video: url}}` and other unknown `{{…}}` | (stripped) | various |
//! | `id:: 01HXY…` (alone on a line) | (line dropped) | Logseq |
//! | 4-space indent | 2-space indent | Roam/Notion export |
//! | tab-separated rows | a markdown table | spreadsheet / SQL client / `column -t` |
//!
//! The unknown-token strip is deliberate: blocks come into outl clean.
//! We never invent information; we only delete tokens that aren't
//! part of our syntax.
//!
//! ## Properties
//!
//! Lines of the form `key:: value` that the parser attaches to a block
//! are re-applied to the freshly minted node as `Op::SetProp` via
//! [`crate::page::set_property`]. They converge across devices the
//! same way every other op does.
//!
//! ## Adding a new external source
//!
//! When the user shows up wanting to paste from a tool we don't
//! cover yet (Obsidian, RemNote, Bear, Apple Notes, etc.), extend
//! [`normalize::normalize_external_syntax`]. The pipeline runs in a
//! fixed order and the order matters — follow these rules:
//!
//! 1. **Specific conversions first.** Map every concrete external
//!    token to its outl equivalent BEFORE the generic-strip step.
//!    Otherwise the catch-all stripper deletes the token before the
//!    converter has a chance to see it (this is why `{{[[TODO]]}}`
//!    has its own `replace` call ahead of the `{{…}}` strip).
//! 2. **Line-level transforms own indent.** When a converter touches
//!    a bullet line (`- [ ]` → `- TODO`), do it after the indent
//!    normaliser so the regex doesn't have to chase 2/4-space drift.
//! 3. **Strip is last.** Anything still wrapped in `{{…}}` /
//!    `^^…^^` after the conversions is unknown to outl and gets
//!    deleted. Use the allowlist callback in `strip_pair` (see how
//!    `{{query: …}}` is preserved) when the wrapper happens to be
//!    outl-native — don't add a new strip helper.
//! 4. **No new dependencies for parsing.** The pipeline is plain
//!    `str` manipulation on purpose: `regex` would pull a 40+kb dep
//!    into a crate that runs on the mobile binary. Manual scans
//!    (see `rewrite_roam_embed`) are the pattern.
//! 5. **Test the conversion AND the order-of-operations.** Every new
//!    entry in the table above gets a unit test in
//!    `normalize.rs::tests`, plus one assertion that the conversion
//!    runs before the generic strip (mirror the
//!    `known_token_wins_over_generic_strip` test).
//! 6. **Update the docs.** Add a row to the table above and to
//!    `docs/markdown-format.md` so users know what we silently
//!    rewrite on paste.
//! 7. **Mirror the heuristic in JS** when the change affects
//!    bullet detection. `looks_like_outline` lives both in this
//!    crate and in `crates/outl-mobile/src/lib/paste.ts`; the JS
//!    copy gates the Tauri round-trip on the client. They must
//!    stay in lockstep.

mod anchors;
mod detect;
mod normalize;

#[cfg(test)]
mod tests;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::ActionError;

pub use detect::{looks_like_outline, looks_like_table, looks_like_tabular, looks_structured};
pub use normalize::normalize_external_syntax;

use detect::split_paragraphs;

/// Where in the workspace the pasted markdown should be grafted.
#[derive(Debug, Clone)]
pub enum PasteAnchor {
    /// Append the pasted blocks as new last children of `parent`.
    AsLastChildOf(NodeId),
    /// Insert the pasted blocks as siblings immediately after `after`.
    AfterBlock(NodeId),
    /// The user is editing `block` and the caret sits at char offset
    /// `caret` inside its text.
    ///
    /// The first parsed bullet (if any) is appended to the text on the
    /// left of the caret and becomes the new text of `block`; any
    /// children of that first bullet land as children of `block`.
    /// Subsequent root-level bullets become siblings after `block`.
    /// Finally, whatever text was on the right of the caret is added
    /// as one more sibling so nothing the user typed gets lost.
    AtCaret {
        /// Block currently being edited.
        block: NodeId,
        /// Caret position, measured in `char` offsets into the block's
        /// text (not byte offsets).
        caret: usize,
    },
}

/// What `paste_markdown` did, so the caller can update UI state.
#[derive(Debug, Clone, Default)]
pub struct PasteOutcome {
    /// Ids of newly created blocks, in DFS / sibling order.
    pub new_blocks: Vec<NodeId>,
    /// New text of the host block when `AtCaret` was used and the
    /// host's text changed. `None` for the other anchors.
    pub host_text: Option<String>,
    /// Number of *outline* root-level bullets the user pasted, after
    /// normalisation and parsing. UI clients show this to confirm
    /// "pasted N blocks" — it counts what the heuristic actually
    /// detected, not blocks created.
    ///
    /// **Always zero on the plain-text fallback path**, even when
    /// the anchor (AfterBlock / AsLastChildOf) caused one literal
    /// block to be created: that block exists only because the
    /// caller asked us where to drop the raw text, not because the
    /// payload had bullet structure. Plain-text via AtCaret returns
    /// zero with no new block at all.
    pub root_count: usize,
}

/// Apply pasted markdown to the workspace at `anchor`.
///
/// `raw` is the clipboard contents verbatim. The function:
///
/// 1. Normalises external syntax to outl (see module docs).
/// 2. Detects whether the result is an outline (any line starting
///    with `- `). If not, falls back to "plain text" behaviour
///    appropriate for the anchor.
/// 3. Parses the normalised text via `outl_md::parse::parse`.
/// 4. Materialises blocks through [`crate::block::append_tree`] /
///    [`crate::block::create_after`].
/// 5. Re-applies block properties via `Op::SetProp`.
pub fn paste_markdown(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    anchor: PasteAnchor,
    raw: &str,
) -> Result<PasteOutcome, ActionError> {
    // A paste is one user-visible action but materialises many blocks
    // (append_forest + sibling inserts + property ops). Batch the whole
    // entry so it flushes once per destination instead of per op.
    let mut batch = workspace.begin_batch();
    let outcome = paste_markdown_inner(&mut batch, hlc, anchor, raw)?;
    batch.commit()?;
    Ok(outcome)
}

fn paste_markdown_inner(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    anchor: PasteAnchor,
    raw: &str,
) -> Result<PasteOutcome, ActionError> {
    // Detect outline shape on the **raw** payload. Running
    // `normalize_external_syntax` first would strip unknown tokens
    // (`{{video: …}}`, `^^…^^`) and collapse whitespace runs before
    // we ever decide to fall back to plain text — that means a user
    // pasting "look at {{video: https://x}}!" into a block would
    // land a mangled string, not what they copied. Normalisation is
    // only legitimate when we're actually going to parse bullets.
    // Tabular data first: a spreadsheet, a database client, a terminal
    // that prints columns all put **tab-separated** lines on the
    // clipboard, and that means one table — not one block per row,
    // which is what the paragraph path below would make of it.
    // Converting here rather than in a branch of its own is what keeps
    // the two tabular sources on one path: the result *is* a markdown
    // table, so `looks_like_table` claims it and the parser builds the
    // same single block a pasted markdown table builds.
    //
    // `normalize_external_syntax` later collapses the column padding
    // `render_table` wrote. Deliberate — the cells are what carry the
    // data, and reformatting the user's file is not this function's job
    // (the TUI pads when it paints, the GUI clients hand the cells to
    // `<table>`).
    let tabular = outl_md::tsv_to_markdown(raw);
    let raw = tabular.as_deref().unwrap_or(raw);

    if !looks_like_outline(raw) && !looks_like_table(raw) {
        // Plain text (no bullets). If it has two or more blank-line
        // separated paragraphs, graft one block per paragraph so a
        // pasted chat reply lands as a readable outline instead of one
        // wall-of-text block. Soft line breaks (single `\n`) stay inside
        // their paragraph's block. A single paragraph (a URL, a snippet,
        // one line) keeps the verbatim splice — never fragment that.
        let paragraphs = split_paragraphs(raw);
        if paragraphs.len() > 1 {
            let blocks: Vec<outl_md::parse::OutlineNode> = paragraphs
                .into_iter()
                .map(|text| outl_md::parse::OutlineNode {
                    text,
                    properties: Vec::new(),
                    children: Vec::new(),
                })
                .collect();
            return anchors::paste_nodes(workspace, hlc, anchor, &blocks);
        }
        return anchors::paste_plain_text(workspace, hlc, anchor, raw);
    }

    let normalized = normalize_external_syntax(raw);
    let trimmed = normalized.trim_end_matches('\n');

    // `parse_fragment`, not `parse`: clipboard text is a fragment, so a
    // leading `---` is something the user copied and not the header of a
    // page that does not exist here. `parse` would split it off as
    // frontmatter and this function only reads `blocks`, so the fence
    // would be silently dropped from the paste.
    let parsed = outl_md::parse::parse_fragment(trimmed);
    if parsed.blocks.is_empty() {
        // Heuristic said outline but parser disagreed (mangled input).
        // Fall back to plain-text behaviour with the **raw** payload
        // so we never edit the user's text behind their back.
        return anchors::paste_plain_text(workspace, hlc, anchor, raw);
    }

    anchors::paste_nodes(workspace, hlc, anchor, &parsed.blocks)
}

/// Paste `raw` as **plain text** — no outline detection, no external
/// syntax normalization, no paragraph splitting. The clipboard contents
/// land verbatim at the anchor: spliced into the host block at the caret
/// (`AtCaret`), or as one new block (`AfterBlock` / `AsLastChildOf`).
///
/// This is the "paste without formatting" path the clients bind to a
/// distinct chord (desktop `Cmd+Shift+V`, TUI `P`). [`paste_markdown`] is
/// the "with formatting" counterpart that converts and splits.
pub fn paste_plain(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    anchor: PasteAnchor,
    raw: &str,
) -> Result<PasteOutcome, ActionError> {
    anchors::paste_plain_text(workspace, hlc, anchor, raw)
}
