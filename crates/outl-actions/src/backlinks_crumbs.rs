//! The breadcrumb above a citing block: what one step of it is, and
//! how the index walk builds one.
//!
//! Split out of [`crate::backlinks`] (which owns *what counts* as a
//! backlink) and [`crate::backlinks_index`] (which owns the storage and
//! the traversal), for the same reason [`crate::backlinks_sort`] is its
//! own file: the breadcrumb is a distinct question — *where does this
//! reference live* — and both halves of the answer belong together.
//! The shape a client renders and the rule that decides when one is
//! materialized are two readings of one decision, and splitting them
//! across the two modules above is how the cheap one stops being cheap.

use serde::Serialize;

use crate::outline::OutlineNode;

/// One ancestor step in a backlink's breadcrumb.
///
/// Carries the ancestor's text **and** its inline tokens, because a
/// breadcrumb is read, not merely measured. Shipping the raw source
/// made a trail read `#alinhamento [[buser/team/vendas]] internet 0800
/// [mail](https://mail.google.com/...)` where the same block one page
/// over reads as prose with a tag and a link — the branch a reference
/// sits in was the one thing on the panel written in markup.
///
/// *Dimmed context* is a styling rule, never a parsing one. The crumb
/// carries no behaviour of its own: a client decides what a press on
/// the line does. Desktop, mobile and the TUI (`g o`) open the source
/// page at the **last** crumb's block, which is why `id` is on the wire.
///
/// `text` already has the `TODO `/`DONE ` prefix stripped, mirroring
/// [`crate::Backlink::block_text`], and `tokens` is that same body
/// through the inline tokenizer.
#[derive(Debug, Clone, Serialize)]
pub struct BacklinkCrumb {
    /// Node id of the ancestor block.
    pub id: String,
    /// Ancestor block's text, TODO/DONE prefix stripped. Full block text, so it may span lines.
    pub text: String,
    /// Inline tokens of [`Self::text`] — what every GUI client renders.
    /// A client holds no tokenizer of its own, so a crumb without this
    /// can only be drawn as source.
    pub tokens: Vec<outl_md::InlineToken>,
}

/// A breadcrumb the index walk has **not** materialized yet.
///
/// Every block visited pushes one and almost none is ever read: a
/// frame becomes a [`BacklinkCrumb`] only for an ancestor of a block
/// that actually hosts a mention. Since a crumb carries inline tokens,
/// materializing at push time would tokenize (or deep-clone the tokens
/// of) every block in the workspace and throw the result away on the
/// pop — root `CLAUDE.md` invariant 15's second question, asked of a
/// pass that already runs over every page.
pub(crate) enum CrumbFrame<'a> {
    /// The from-disk walk: the parse already produced the tokens, so
    /// materializing is a clone.
    Parsed(&'a OutlineNode),
    /// The workspace walk: text materialized off the tree, tokenized
    /// only if this frame is ever read.
    Tree { id: String, text: String },
}

impl CrumbFrame<'_> {
    fn materialize(&self) -> BacklinkCrumb {
        match self {
            Self::Parsed(node) => BacklinkCrumb {
                id: node.id.clone(),
                text: node.text.clone(),
                tokens: node.tokens.clone(),
            },
            Self::Tree { id, text } => BacklinkCrumb {
                id: id.clone(),
                text: text.clone(),
                tokens: outl_md::tokenize_owned(text),
            },
        }
    }
}

/// The trail above the block being visited, in the shape a
/// [`crate::Backlink`] carries it.
pub(crate) fn crumbs(frames: &[CrumbFrame<'_>]) -> Vec<BacklinkCrumb> {
    frames.iter().map(CrumbFrame::materialize).collect()
}
