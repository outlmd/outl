//! What a block's text mentions — the single owner of that rule.
//!
//! `[[ref]]` and `#tag` both come out of one walk over
//! [`outl_md::inline::tokenize`], which is also what renders the block
//! and what `ref_at_cursor` resolves when the user clicks. That matters
//! because a mention decides a backlink
//! ([`crate::backlinks_index::BacklinkIndex`]) and a reminder's date
//! anchor ([`crate::reminders::scan`]), and both used to read `[[ref]]`
//! with a raw byte scan for `[[` … `]]` instead.
//!
//! The scan made this the only one of the workspace's four `[[ref]]`
//! readers blind to the inline grammar — `outl_md::reference`, the Roam
//! importer and the Logseq importer all tokenize — and left the two
//! channels disagreeing *inside a single block*: `#tag` asked the
//! tokenizer, `[[ref]]` did not, so a `` `code` `` span was inert for
//! one and live for the other.
//!
//! What that swap moved, measured on a real 2,862-page workspace, lives
//! in `tests/ref_extraction_divergence.rs`.

use outl_md::inline::InlineTok;

/// Extract every `[[ref]]` target out of a block's text, in document
/// order, as the inline tokenizer sees them.
///
/// A ref inside a `` `code` `` span is not one, `[[` and `]]` may not
/// straddle a newline, and `[[a [[b]] ]]` names the page the renderer
/// draws (`a [[b`), not the inner `b`. Emphasis wrappers stay
/// transparent.
pub fn extract_refs(text: &str) -> Vec<String> {
    extract_refs_and_tags(text).0
}

/// Every `[[ref]]` and `#tag` in one pass over the token tree.
///
/// `mentions_of` needs both channels for one block and used to ask two
/// different oracles. One tokenization, one traversal, one rule.
pub(crate) fn extract_refs_and_tags(text: &str) -> (Vec<String>, Vec<String>) {
    let toks = outl_md::inline::tokenize(text);
    let (mut refs, mut tags) = (Vec::new(), Vec::new());
    collect(&toks, &mut refs, &mut tags);
    (refs, tags)
}

/// DFS the token tree, descending only through the wrappers a client
/// renders *through*. The `match` is exhaustive on purpose: a new
/// wrapper variant must not compile until someone says which side it is
/// on. `Bold` and friends carry recursively tokenized contents, and a
/// flat `filter(PageRef)` drops 403 references on that workspace.
/// `Code` being opaque is the whole point; `Link` / `Image` carry raw
/// `&str`, and a `[[ref]]` inside a URL is not clickable.
fn collect(toks: &[InlineTok<'_>], refs: &mut Vec<String>, tags: &mut Vec<String>) {
    for tok in toks {
        match tok {
            InlineTok::PageRef { name } => refs.push((*name).to_string()),
            InlineTok::Tag { name } => tags.push((*name).to_string()),
            InlineTok::Bold { inner }
            | InlineTok::Italic { inner, .. }
            | InlineTok::Strike { inner }
            | InlineTok::Highlight { inner } => collect(inner, refs, tags),
            InlineTok::Plain(_)
            | InlineTok::Code { .. }
            | InlineTok::Link { .. }
            | InlineTok::Image { .. }
            | InlineTok::BlockRef { .. }
            | InlineTok::Embed { .. }
            | InlineTok::Emoji { .. } => {}
        }
    }
}
