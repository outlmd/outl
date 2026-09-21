//! Tag-boundary predicates over inline block text.
//!
//! `text.contains("#tag")` is the bug this module exists to delete:
//! the substring form matches `#tag-longer`, `#tagged`, and `#tag/sub`
//! as false positives. The inline tokenizer ([`crate::inline`])
//! already knows exactly where a `#tag` token starts and ends, so the
//! predicate here routes through it instead of re-deriving boundary
//! rules with a regex.
//!
//! Matching follows the tokenizer's behavior: `tag` is the bare name
//! without the leading `#`. Tags nested inside emphasis (`**#tag**`)
//! still match because bold / italic / strike inners are re-tokenized;
//! a `#tag` inside a `` `code` `` span is *not* a tag and does not
//! match.
//!
//! Two predicates, because two callers want two different questions
//! answered and neither should re-derive boundary rules:
//!
//! - [`text_contains_tag`] — exact, case-sensitive. `#tag/sub` is a
//!   *different* tag from `#tag`. Backlinks and tag counting use this.
//! - [`text_contains_tag_or_child`] — case-insensitive, and a parent
//!   name also answers for its namespace children, so `#ops/deploy`
//!   satisfies `ops`. The ` ```query ` DSL's `tag:` / `not-tag:` use
//!   this, which is the semantics `docs/query.md` has always
//!   documented.
//!
//! Both are boundary-aware, and that is the property the negative
//! filter depends on: `not-tag: work` must not silently swallow
//! `#workflow`.

use crate::inline::{tokenize, InlineTok};

/// True when `text` mentions `#tag` as a whole tag token.
///
/// `tag` is the tag name without the leading `#` (e.g. `"project"`).
/// `#tag-longer` / `#tag/sub` do **not** match `"tag"` — the token
/// name must be equal, not a prefix.
pub fn text_contains_tag(text: &str, tag: &str) -> bool {
    let tag = normalize(tag);
    any_tag(&tokenize(text), &|name| name == tag)
}

/// Drop a leading `#` and surrounding whitespace from a needle.
///
/// `#` cannot appear *inside* a tag token, so a needle carrying one
/// could only ever match nothing — and "matches nothing" is invisible
/// on the negative side of a filter, where it means the caller gets
/// back exactly the rows they asked to hide. Since the hash is how a
/// tag is spelled everywhere the user reads one, accept it here, at
/// the single owner, rather than in each caller.
fn normalize(tag: &str) -> &str {
    tag.trim().strip_prefix('#').unwrap_or(tag).trim()
}

/// True when `text` mentions `#tag` or any tag nested under it.
///
/// Case-insensitive, and `#ops/deploy` satisfies `"ops"` — the
/// namespace-prefix semantics `docs/query.md` documents for the
/// ` ```query ` DSL. `#opsec` does **not** match: the name has to end
/// at the boundary or continue with `/`.
pub fn text_contains_tag_or_child(text: &str, tag: &str) -> bool {
    let needle = normalize(tag).to_lowercase();
    if needle.is_empty() {
        return false;
    }
    let child = format!("{needle}/");
    any_tag(&tokenize(text), &|name| {
        let name = name.to_lowercase();
        name == needle || name.starts_with(&child)
    })
}

fn any_tag(toks: &[InlineTok<'_>], pred: &dyn Fn(&str) -> bool) -> bool {
    toks.iter().any(|tok| match tok {
        InlineTok::Tag { name } => pred(name),
        InlineTok::Bold { inner }
        | InlineTok::Italic { inner, .. }
        | InlineTok::Strike { inner }
        | InlineTok::Highlight { inner } => any_tag(inner, pred),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_tag_matches() {
        assert!(text_contains_tag("working on #outl today", "outl"));
    }

    #[test]
    fn longer_tag_is_not_a_prefix_match() {
        // The substring bug this predicate replaces: `#tag-longer`
        // must not satisfy a query for `tag`.
        assert!(!text_contains_tag("see #tag-longer here", "tag"));
        assert!(!text_contains_tag("see #tagged here", "tag"));
        assert!(!text_contains_tag("see #tag/sub here", "tag"));
    }

    #[test]
    fn nested_tag_still_matches_its_full_name() {
        assert!(text_contains_tag("see #tag/sub here", "tag/sub"));
    }

    #[test]
    fn tag_at_end_of_line_matches() {
        assert!(text_contains_tag("ship it #urgent", "urgent"));
    }

    #[test]
    fn tag_followed_by_punctuation_matches() {
        assert!(text_contains_tag("done (#urgent), moving on", "urgent"));
        assert!(text_contains_tag("#urgent: fix the build", "urgent"));
        assert!(text_contains_tag("really #urgent!", "urgent"));
    }

    #[test]
    fn matching_is_case_sensitive_like_the_tokenizer() {
        // The tokenizer preserves case verbatim; so does the predicate.
        assert!(!text_contains_tag("see #Urgent", "urgent"));
        assert!(text_contains_tag("see #Urgent", "Urgent"));
    }

    #[test]
    fn tag_inside_emphasis_matches() {
        assert!(text_contains_tag("**#urgent** fix", "urgent"));
        assert!(text_contains_tag("*#urgent* fix", "urgent"));
        assert!(text_contains_tag("~~#urgent~~ fix", "urgent"));
        // A tag inside a highlight must stay findable, or backlinks and
        // tag search silently miss `==see #project==`.
        assert!(text_contains_tag("==#urgent== fix", "urgent"));
    }

    #[test]
    fn hash_inside_code_span_is_not_a_tag() {
        assert!(!text_contains_tag("run `#urgent` literally", "urgent"));
    }

    #[test]
    fn plain_word_without_hash_does_not_match() {
        assert!(!text_contains_tag("urgent but untagged", "urgent"));
    }

    #[test]
    fn or_child_matches_the_tag_itself_and_its_namespace() {
        assert!(text_contains_tag_or_child("ship it #ops", "ops"));
        assert!(text_contains_tag_or_child("ship it #ops/deploy", "ops"));
        assert!(text_contains_tag_or_child("ship it #ops/deploy/eu", "ops"));
        assert!(text_contains_tag_or_child(
            "ship it #ops/deploy",
            "ops/deploy"
        ));
    }

    #[test]
    fn or_child_stops_at_the_tag_boundary() {
        // The whole reason `not-tag:` can be trusted: a longer name is
        // a different tag, not a match. `not-tag: work` excluding
        // `#workflow` would be silent over-exclusion.
        assert!(!text_contains_tag_or_child("ship it #workflow", "work"));
        assert!(!text_contains_tag_or_child("ship it #opsec", "ops"));
        assert!(!text_contains_tag_or_child("ship it #ops-team", "ops"));
    }

    #[test]
    fn or_child_is_case_insensitive() {
        // The DSL lowercases everything else it matches on; a tag
        // filter that did not would be the odd one out.
        assert!(text_contains_tag_or_child("ship it #Ops", "ops"));
        assert!(text_contains_tag_or_child("ship it #ops", "OPS"));
        assert!(text_contains_tag_or_child("ship it #OPS/Deploy", "ops"));
    }

    #[test]
    fn or_child_ignores_code_spans_and_bare_words() {
        assert!(!text_contains_tag_or_child("run `#ops` literally", "ops"));
        assert!(!text_contains_tag_or_child("ops but untagged", "ops"));
    }

    #[test]
    fn a_leading_hash_on_the_needle_is_tolerated() {
        // `#` never appears inside a tag token, so a needle carrying
        // one could only ever match nothing — invisible on the
        // negative side of a query filter.
        assert!(text_contains_tag("ship it #urgent", "#urgent"));
        assert!(text_contains_tag_or_child("roll out #ops/deploy", "#ops"));
        assert!(!text_contains_tag("ship it #urgent", "#other"));
        // Still not a wildcard.
        assert!(!text_contains_tag("ship it #urgent", "#"));
        assert!(!text_contains_tag_or_child("ship it #urgent", "#"));
    }

    #[test]
    fn or_child_with_an_empty_needle_matches_nothing() {
        // `tag:` with no value would otherwise match every tag, which
        // makes `not-tag:` drop the whole workspace.
        assert!(!text_contains_tag_or_child("ship it #ops", ""));
    }
}
