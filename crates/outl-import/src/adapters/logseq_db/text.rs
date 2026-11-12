//! Block text: `[[<uuid>]]` and `((<uuid>))` → something outl reads.
//!
//! In a DB graph a block's `:block/title` never spells a page name
//! (renames would have to rewrite every referrer). A page ref is
//! stored as `[[<uuid>]]`, a tag as `#[[<uuid>]]`, and a block ref as
//! `[[<uuid>]]` too (older graphs also carry `((<uuid>))`); see
//! `deps/db/src/logseq/db/frontend/content.cljs` at tag 2.0.2.
//!
//! This pass rewrites them into the file-graph spellings the rest of
//! the pipeline already understands: `[[Page Title]]`, `#tag`, and
//! `((<block uuid>))`, which the Logseq inline tokenizer then turns
//! into the IR's block ref so the resolve pass mints a real handle.

use std::collections::{HashMap, HashSet};

/// What a uuid in the text points at.
#[derive(Debug, Default)]
pub(super) struct RefTargets {
    /// uuid → page title (journals are already spelled `YYYY-MM-DD`).
    pub(super) pages: HashMap<String, String>,
    /// uuids of blocks that are part of the import.
    pub(super) blocks: HashSet<String>,
}

/// One uuid that pointed nowhere.
#[derive(Debug, Clone)]
pub(super) struct Dangling {
    /// The uuid as written.
    pub(super) uuid: String,
}

/// Outcome of one rewrite.
#[derive(Debug, Default)]
pub(super) struct Rewritten {
    /// Text with every resolvable uuid replaced.
    pub(super) text: String,
    /// uuids found in the text that match nothing imported.
    pub(super) dangling: Vec<Dangling>,
}

impl RefTargets {
    /// Rewrite every uuid reference in `text`. Code spans are left
    /// alone.
    pub(super) fn rewrite(&self, text: &str) -> Rewritten {
        self.rewrite_with(text, true)
    }

    /// Like [`Self::rewrite`] for text that no tokenizer reads
    /// afterwards (property values): a block ref has nowhere to
    /// resolve, so it is reported rather than turned into `((uuid))`.
    pub(super) fn rewrite_plain(&self, text: &str) -> Rewritten {
        self.rewrite_with(text, false)
    }

    fn rewrite_with(&self, text: &str, block_refs: bool) -> Rewritten {
        let mut out = Rewritten::default();
        let mut s = String::with_capacity(text.len());
        let mut i = 0usize;
        while i < text.len() {
            let rest = &text[i..];
            if let Some(len) = code_span_len(rest) {
                s.push_str(&rest[..len]);
                i += len;
                continue;
            }
            if let Some((adv, rep)) = self.reference_at(rest, block_refs, &mut out) {
                s.push_str(&rep);
                i += adv;
                continue;
            }
            let ch = rest.chars().next().expect("rest is non-empty");
            s.push(ch);
            i += ch.len_utf8();
        }
        out.text = s;
        out
    }

    /// Try the four reference shapes at the start of `rest`; returns
    /// `(bytes consumed, replacement)`.
    fn reference_at(
        &self,
        rest: &str,
        block_refs: bool,
        out: &mut Rewritten,
    ) -> Option<(usize, String)> {
        let (hash, open, close, body) = if let Some(r) = rest.strip_prefix("#[[") {
            (true, 3, "]]", r)
        } else if let Some(r) = rest.strip_prefix("[[") {
            (false, 2, "]]", r)
        } else {
            (false, 2, "))", rest.strip_prefix("((")?)
        };
        let uuid = body.get(..36).filter(|u| is_uuid(u))?;
        if !body[36..].starts_with(close) {
            return None;
        }
        // `open` already counts the leading `#` of a tag ref.
        let consumed = open + 36 + 2;
        let uuid = uuid.to_ascii_lowercase();
        let verbatim = || rest[..consumed].to_string();
        let rep = if let Some(title) = self.pages.get(&uuid) {
            if hash {
                tag_text(title)
            } else {
                format!("[[{title}]]")
            }
        } else if block_refs && self.blocks.contains(&uuid) {
            format!("(({uuid}))")
        } else {
            out.dangling.push(Dangling { uuid });
            verbatim()
        };
        Some((consumed, rep))
    }
}

/// `#tag` for a title the outl tokenizer reads as one tag, otherwise
/// the `#[[…]]` form (the Logseq tokenizer then rewrites it to a page
/// ref and counts it).
pub(super) fn tag_text(title: &str) -> String {
    if is_simple_tag(title) {
        format!("#{title}")
    } else {
        format!("#[[{title}]]")
    }
}

/// The spelling of a tag where no tokenizer runs afterwards (page and
/// property values): a bare `#tag`, or a plain `[[page]]` ref.
pub(super) fn tag_value(title: &str) -> String {
    if is_simple_tag(title) {
        format!("#{title}")
    } else {
        format!("[[{title}]]")
    }
}

/// outl's tag alphabet (`outl-md/src/reference.rs::try_tag`).
fn is_simple_tag(title: &str) -> bool {
    !title.is_empty()
        && title
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '/')
}

/// `8-4-4-4-12` hex.
fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Length of a single-backtick span at the start of `rest`.
fn code_span_len(rest: &str) -> Option<usize> {
    let r = rest.strip_prefix('`')?;
    let close = r.find('`')?;
    Some(1 + close + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "11111111-1111-4111-8111-111111111111";
    const BLOCK: &str = "22222222-2222-4222-8222-222222222222";
    const GONE: &str = "33333333-3333-4333-8333-333333333333";

    fn targets() -> RefTargets {
        let mut t = RefTargets::default();
        t.pages.insert(PAGE.into(), "My Page".into());
        t.blocks.insert(BLOCK.into());
        t
    }

    #[test]
    fn a_page_ref_becomes_the_title() {
        let r = targets().rewrite(&format!("see [[{PAGE}]] now"));
        assert_eq!(r.text, "see [[My Page]] now");
        assert!(r.dangling.is_empty());
    }

    #[test]
    fn a_block_ref_in_either_spelling_becomes_a_double_paren_ref() {
        let r = targets().rewrite(&format!("[[{BLOCK}]] and (({BLOCK}))"));
        assert_eq!(r.text, format!("(({BLOCK})) and (({BLOCK}))"));
    }

    #[test]
    fn a_tag_ref_becomes_a_tag_or_a_bracketed_tag() {
        let mut t = targets();
        t.pages.insert(GONE.into(), "book".into());
        let r = t.rewrite(&format!("#[[{GONE}]] and #[[{PAGE}]]"));
        assert_eq!(r.text, "#book and #[[My Page]]");
    }

    #[test]
    fn an_unknown_uuid_is_kept_verbatim_and_reported() {
        let r = targets().rewrite(&format!("x [[{GONE}]] y (({GONE}))"));
        assert_eq!(r.text, format!("x [[{GONE}]] y (({GONE}))"));
        assert_eq!(r.dangling.len(), 2);
    }

    #[test]
    fn code_spans_and_plain_brackets_are_left_alone() {
        let src = format!("`[[{PAGE}]]` [[plain]] ((not a uuid))");
        assert_eq!(targets().rewrite(&src).text, src);
    }

    #[test]
    fn uppercase_uuids_resolve() {
        let r = targets().rewrite(&format!("[[{}]]", PAGE.to_ascii_uppercase()));
        assert_eq!(r.text, "[[My Page]]");
    }

    #[test]
    fn multibyte_text_survives() {
        let r = targets().rewrite(&format!("café ☕ [[{PAGE}]] ñ"));
        assert_eq!(r.text, "café ☕ [[My Page]] ñ");
    }
}
