//! Quote state, encoded as a prefix on a block's text.
//!
//! Mirrors the TODO/DONE convention (see [`crate::todo`]): the marker
//! lives **in the block's text** as a `"> "` prefix instead of a new
//! field on the AST. Consumers (TUI, mobile, desktop) decide how to
//! render it; the wire format stays the same and round-trips through
//! CommonMark `.md` as `> body` continuation lines.
//!
//! Rules:
//!
//! 1. A block is a quote when its text starts with `"> "` — one space
//!    after `>`, matching CommonMark.
//! 2. Quote state is **per block**: children of a quoted block are not
//!    implicitly quoted (same policy as TODO/DONE).
//! 3. Nested quotes (`">> "`) are out of scope for the first cut — the
//!    body is whatever follows the single `"> "` prefix, verbatim, so a
//!    user who types `"> > foo"` gets a quoted block whose body
//!    *starts* with `"> foo"` but no extra unwrapping happens.

/// Wire prefix for a quoted block. Two characters including the
/// trailing space; consumers can rely on `chars().count() == 2` for
/// cursor math.
pub const QUOTE_PREFIX: &str = "> ";

/// Does the block's raw text carry the quote marker?
pub fn is_quote(text: &str) -> bool {
    text.starts_with(QUOTE_PREFIX)
}

/// Split a block's raw text into `(quoted, body)`. The body never
/// includes the prefix or its trailing space when `quoted` is true.
/// When the prefix is absent, `quoted` is `false` and `body` is the
/// untouched input.
pub fn split_quote(raw: &str) -> (bool, &str) {
    match raw.strip_prefix(QUOTE_PREFIX) {
        Some(rest) => (true, rest),
        None => (false, raw),
    }
}

/// Toggle the quote prefix on a block's raw text.
///
/// Canonical encoding is **`"TODO > body"`** (TODO/DONE before the
/// quote marker). The toggle peels both prefixes off, flips the
/// quote flag, and re-emits in canonical order so `split_todo` in
/// the backend (and every reader of `OutlineNode.text`) keeps
/// detecting the task state when the block is also quoted.
///
/// Without this, a `toggle_quote` on `"TODO ship it"` would produce
/// `"> TODO ship it"`, `outl_actions::outline::project_outline_node`
/// would call `split_todo`, miss the marker, and the DTO would land
/// in mobile / desktop with `todo = null` and the literal `> TODO`
/// in `text`. Checkbox disappears mid-flight. Same hazard the
/// **other** direction in [`crate::todo::cycle_todo`].
pub fn toggle_quote(raw: &str) -> String {
    use crate::todo::split_todo;
    let (todo_state, after_todo) = split_todo(raw);
    let (quoted, body) = split_quote(after_todo);
    let next_quoted = !quoted;
    let mut out = String::new();
    if let Some(state) = todo_state {
        // `TodoState::prefix` is the single owner of the marker
        // spelling — a local match here is a second one, and it is
        // how this function missed `DOING` when that state landed.
        out.push_str(state.prefix());
    }
    if next_quoted {
        out.push_str(QUOTE_PREFIX);
    }
    out.push_str(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_recognises_the_marker() {
        assert_eq!(split_quote("> a quote"), (true, "a quote"));
        assert_eq!(split_quote("plain block"), (false, "plain block"));
    }

    #[test]
    fn marker_requires_trailing_space() {
        // ">foo" without the space stays plain (CommonMark requires
        // the space too).
        assert_eq!(split_quote(">foo"), (false, ">foo"));
        // A bare ">" is also not a quote (no body, no trailing space).
        assert_eq!(split_quote(">"), (false, ">"));
        assert!(!is_quote(">foo"));
        assert!(is_quote("> "));
    }

    #[test]
    fn toggle_walks_through_two_states() {
        let s0 = "ship the feature";
        let s1 = toggle_quote(s0);
        let s2 = toggle_quote(&s1);
        assert_eq!(s1, "> ship the feature");
        assert_eq!(s2, "ship the feature");
    }

    #[test]
    fn toggle_preserves_inner_text_verbatim_including_inline_tokens() {
        // Inline tokens inside a quote stay untouched — the wrapper
        // is transparent to the inline tokenizer.
        let s = toggle_quote("**bold** [[ref]] #tag");
        assert_eq!(s, "> **bold** [[ref]] #tag");

        let (quoted, body) = split_quote(&s);
        assert!(quoted);
        assert_eq!(body, "**bold** [[ref]] #tag");
    }

    #[test]
    fn empty_body_is_legal() {
        // "> " alone is a quoted block with an empty body — we treat
        // it as legal so the user can type the marker and then the
        // body without an intermediate "not a quote" state.
        assert_eq!(split_quote("> "), (true, ""));
        assert!(is_quote("> "));
    }

    #[test]
    fn toggle_preserves_leading_todo_in_canonical_order() {
        // Canonical encoding is `"TODO > body"` — TODO/DONE stay
        // before the quote marker so the backend's `split_todo`
        // keeps detecting the task state when the block is also
        // quoted. Without this, `split_todo("> TODO foo")` would
        // miss the marker and the DTO would land with `todo = null`.
        assert_eq!(toggle_quote("TODO ship it"), "TODO > ship it");
        assert_eq!(toggle_quote("DONE ship it"), "DONE > ship it");
        assert_eq!(toggle_quote("TODO > ship it"), "TODO ship it");
        assert_eq!(toggle_quote("DONE > ship it"), "DONE ship it");
    }

    #[test]
    fn toggle_normalises_legacy_quote_before_todo_authoring() {
        // A user who types `"> TODO foo"` by hand (TODO inside the
        // quote body) gets normalised to canonical order on toggle:
        // the inner TODO is plain text, so removing the quote leaves
        // it sitting as text. Re-toggling builds canonical TODO+quote.
        assert_eq!(toggle_quote("> TODO foo"), "TODO foo");
        // …and a second toggle puts everything in canonical order.
        assert_eq!(toggle_quote("TODO foo"), "TODO > foo");
    }
}
