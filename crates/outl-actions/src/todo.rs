//! Task state, encoded as a prefix on a block's text.
//!
//! The TUI established this convention and we keep it for wire-format
//! compatibility: `"TODO foo"`, `"DOING foo"` and `"DONE foo"` are the
//! canonical marker shapes, separated from the body by a single space.
//!
//! # Two spellings in, one spelling out
//!
//! CommonMark task-list syntax (`"[ ] foo"`, `"[/] foo"`, `"[x] foo"`)
//! is **also** recognised, because a user who types `- [ ] buy milk`
//! means a task and every other outliner agrees with them
//! ([issue #230](https://github.com/outlmd/outl/issues/230)). Before
//! this, that line was inert prose: no checkbox, no `status:` hit, no
//! toggle, and the literal `[ ]` rendered as text.
//!
//! Reading both while writing only one is [RFC 0008]'s clause 3
//! (parse permissively) applied to a block prefix, and it has a
//! consequence worth stating out loud: **the first mutation rewrites
//! the spelling.** `cycle_todo` and `set_todo` re-emit through
//! [`TodoState::prefix`], so `"[ ] buy milk"` becomes `"DOING buy
//! milk"` on its first toggle and never returns to bracket form.
//! Until then the bytes on disk are untouched — recognition alone
//! changes nothing, so a `.md` full of `[ ]` stays as written until
//! the user acts on those blocks.
//!
//! The alternative (rewriting on sight, at parse time) was rejected:
//! it edits the file the moment it is read, which turns opening a page
//! into a write and makes the text change under the cursor.
//!
//! [RFC 0008]: ../../../docs/rfcs/0008-markdown-dialect-and-sidecar-tokens.md

/// Wire prefix for an open task.
pub const TODO_PREFIX: &str = "TODO ";
/// Wire prefix for a task in progress.
pub const DOING_PREFIX: &str = "DOING ";
/// Wire prefix for a completed task.
pub const DONE_PREFIX: &str = "DONE ";

/// Every prefix [`split_todo`] accepts, paired with the state it
/// means. The write side is [`TodoState::prefix`], which only ever
/// emits the three canonical words — that asymmetry is the whole
/// "two spellings in, one out" rule from the module docs.
///
/// Order is irrelevant: no entry is a prefix of another.
///
/// `[X]` rides along with `[x]` because GitHub accepts both and a
/// user who holds shift should not get prose.
const READ_PREFIXES: [(&str, TodoState); 7] = [
    (TODO_PREFIX, TodoState::Todo),
    (DOING_PREFIX, TodoState::Doing),
    (DONE_PREFIX, TodoState::Done),
    ("[ ] ", TodoState::Todo),
    ("[/] ", TodoState::Doing),
    ("[x] ", TodoState::Done),
    ("[X] ", TodoState::Done),
];

/// Recognised task states. The order also defines the cycle order in
/// [`cycle_todo`]: `None → TODO → DOING → DONE → None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoState {
    /// The block is an open task.
    Todo,
    /// The block is a task somebody has started.
    Doing,
    /// The block is a completed task.
    Done,
}

impl TodoState {
    /// Stringified wire form used in block text and rendered markdown.
    pub fn as_str(self) -> &'static str {
        match self {
            TodoState::Todo => "TODO",
            TodoState::Doing => "DOING",
            TodoState::Done => "DONE",
        }
    }

    /// Prefix used when this state lives inline in a block's text,
    /// e.g. `"TODO "` or `"DOING "`.
    ///
    /// **Not a fixed width.** `"DOING "` is one character longer than
    /// the other two, so a caller doing cursor math must measure the
    /// prefix it is actually adding or removing rather than assuming
    /// five. The TUI's inline cycle got this wrong for exactly one
    /// release and put the caret inside the marker.
    pub fn prefix(self) -> &'static str {
        match self {
            TodoState::Todo => TODO_PREFIX,
            TodoState::Doing => DOING_PREFIX,
            TodoState::Done => DONE_PREFIX,
        }
    }
}

/// Split a block's raw text into `(state, body)`. The body never
/// includes the prefix or its trailing space.
///
/// Accepts both the canonical word form (`"TODO foo"`) and the
/// CommonMark checkbox form (`"[ ] foo"`); see the module docs for why
/// only the first is ever written back.
pub fn split_todo(raw: &str) -> (Option<TodoState>, &str) {
    for (prefix, state) in READ_PREFIXES {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return (Some(state), rest);
        }
    }
    (None, raw)
}

/// Cycle the TODO state of `raw` to the next stop. Returns the new
/// text, ready to be stored as the block's content.
///
/// Aware of an optional leading quote prefix so the canonical
/// encoding stays **`"TODO > body"`** (TODO before the quote marker).
/// Without the awareness, cycling a `"> foo"` block would yield
/// `"TODO > foo"` only by lucky string concatenation, but cycling a
/// `"> TODO foo"` block (TODO already after the quote, the legacy
/// shape) would yield `"TODO > TODO foo"` — a double TODO that
/// `split_todo` would misread. Peeling both prefixes and re-emitting
/// in canonical order makes the operation idempotent across either
/// authoring shape, and keeps mobile / desktop happy with `block.todo`
/// populated.
pub fn cycle_todo(raw: &str) -> String {
    let (quoted, after_quote) = crate::quote::split_quote(raw);
    let (state, body) = split_todo(after_quote);
    let next = match state {
        None => Some(TodoState::Todo),
        Some(TodoState::Todo) => Some(TodoState::Doing),
        Some(TodoState::Doing) => Some(TodoState::Done),
        Some(TodoState::Done) => None,
    };
    let mut out = String::new();
    if let Some(s) = next {
        out.push_str(s.prefix());
    }
    if quoted {
        out.push_str(crate::quote::QUOTE_PREFIX);
    }
    out.push_str(body);
    out
}

/// Set the TODO state of `raw` outright, rather than stepping to the
/// next one. Returns the new text, ready to store as block content.
///
/// Same quote handling as [`cycle_todo`], for the same reason.
///
/// Exists because "mark this done" and "advance one state" are not the
/// same request, and a caller that only had `cycle_todo` had to guess.
/// The reminders panel's ✓ button guessed wrong: on a block carrying a
/// rule but no marker (`g r` attaches to any block, `remind:: 3pm`
/// needs no task), one cycle produced `TODO`, so the button labelled
/// "mark done, cancels the reminder" armed the nag instead.
pub fn set_todo(raw: &str, state: Option<TodoState>) -> String {
    let (quoted, after_quote) = crate::quote::split_quote(raw);
    let (_, body) = split_todo(after_quote);
    let mut out = String::new();
    if let Some(s) = state {
        out.push_str(s.prefix());
    }
    if quoted {
        out.push_str(crate::quote::QUOTE_PREFIX);
    }
    out.push_str(body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_todo_reaches_done_from_every_starting_state() {
        // The reminders ✓ has to mean DONE from a plain block, not
        // "one step towards it".
        for start in ["ship it", "TODO ship it", "DONE ship it"] {
            assert_eq!(set_todo(start, Some(TodoState::Done)), "DONE ship it");
        }
    }

    #[test]
    fn set_todo_clears_a_marker_and_keeps_the_quote() {
        assert_eq!(set_todo("TODO > ship it", None), "> ship it");
        assert_eq!(
            set_todo("> ship it", Some(TodoState::Done)),
            "DONE > ship it"
        );
        // Legacy shape (marker after the quote) normalises too.
        assert_eq!(
            set_todo("> TODO ship it", Some(TodoState::Done)),
            "DONE > ship it"
        );
    }

    #[test]
    fn set_todo_is_idempotent() {
        let once = set_todo("ship it", Some(TodoState::Done));
        assert_eq!(set_todo(&once, Some(TodoState::Done)), once);
    }

    #[test]
    fn split_recognises_every_marker() {
        assert_eq!(
            split_todo("TODO write report"),
            (Some(TodoState::Todo), "write report")
        );
        assert_eq!(
            split_todo("DOING writing it"),
            (Some(TodoState::Doing), "writing it")
        );
        assert_eq!(
            split_todo("DONE shipped it"),
            (Some(TodoState::Done), "shipped it")
        );
        assert_eq!(split_todo("plain block"), (None, "plain block"));
    }

    #[test]
    fn the_commonmark_checkbox_form_is_read_as_a_task() {
        // Issue #230: typing `- [ ] buy milk` means a task. Before
        // this it was inert prose — no checkbox, no `status:` hit, no
        // toggle, and a literal `[ ]` on screen.
        assert_eq!(
            split_todo("[ ] buy milk"),
            (Some(TodoState::Todo), "buy milk")
        );
        assert_eq!(
            split_todo("[/] buy milk"),
            (Some(TodoState::Doing), "buy milk")
        );
        assert_eq!(
            split_todo("[x] buy milk"),
            (Some(TodoState::Done), "buy milk")
        );
        // GitHub accepts the capital, so a user holding shift gets a
        // task rather than prose.
        assert_eq!(
            split_todo("[X] buy milk"),
            (Some(TodoState::Done), "buy milk")
        );
    }

    #[test]
    fn a_markdown_link_is_not_a_checkbox() {
        // `[x](url)` is a real link whose anchor text is "x", and
        // `[ ](url)` is a link with an empty anchor. The trailing
        // space in the checkbox prefix is what separates them: a link
        // continues into `(`, never into a space. Getting this wrong
        // would eat the opening of the user's link and leave `(url)`
        // stranded in the body.
        assert_eq!(
            split_todo("[x](https://example.com) is a link"),
            (None, "[x](https://example.com) is a link")
        );
        assert_eq!(
            split_todo("[ ](https://example.com)"),
            (None, "[ ](https://example.com)")
        );
        // A wiki-link or a citation opening a block stays prose too.
        assert_eq!(split_todo("[[page]] mention"), (None, "[[page]] mention"));
    }

    #[test]
    fn an_unchecked_box_with_no_space_is_prose() {
        assert_eq!(split_todo("[]"), (None, "[]"));
        assert_eq!(split_todo("[ ]"), (None, "[ ]"));
        assert_eq!(split_todo("[y] foo"), (None, "[y] foo"));
    }

    #[test]
    fn cycling_a_checkbox_rewrites_it_to_the_canonical_word() {
        // Stated in the module docs as a deliberate consequence: two
        // spellings in, one out. The user's bracket form survives
        // untouched until they act on the block.
        assert_eq!(cycle_todo("[ ] buy milk"), "DOING buy milk");
        assert_eq!(cycle_todo("[x] buy milk"), "buy milk");
        assert_eq!(
            set_todo("[ ] buy milk", Some(TodoState::Done)),
            "DONE buy milk"
        );
    }

    #[test]
    fn a_marker_word_without_its_space_is_prose() {
        // The marker is the prefix *including* the trailing space.
        // `DOING` opens more prose than `TODO` does ("DOINGs"), and a
        // body starting with the bare word must stay untouched.
        assert_eq!(
            split_todo("DOINGs are piling up"),
            (None, "DOINGs are piling up")
        );
        assert_eq!(split_todo("DOING"), (None, "DOING"));
    }

    #[test]
    fn cycle_walks_through_four_states() {
        let s0 = "deploy frontend";
        let s1 = cycle_todo(s0);
        let s2 = cycle_todo(&s1);
        let s3 = cycle_todo(&s2);
        let s4 = cycle_todo(&s3);
        assert_eq!(s1, "TODO deploy frontend");
        assert_eq!(s2, "DOING deploy frontend");
        assert_eq!(s3, "DONE deploy frontend");
        assert_eq!(s4, "deploy frontend");
    }

    #[test]
    fn cycle_preserves_quote_marker_in_canonical_order() {
        // Quote marker survives a full cycle and stays in canonical
        // position (after the task state, before the body).
        let s0 = "> deploy frontend";
        let s1 = cycle_todo(s0);
        let s2 = cycle_todo(&s1);
        let s3 = cycle_todo(&s2);
        let s4 = cycle_todo(&s3);
        assert_eq!(s1, "TODO > deploy frontend");
        assert_eq!(s2, "DOING > deploy frontend");
        assert_eq!(s3, "DONE > deploy frontend");
        assert_eq!(s4, "> deploy frontend");
    }

    #[test]
    fn cycle_normalises_legacy_todo_after_quote_authoring() {
        // A user who imported `"> TODO foo"` (legacy / external
        // markdown shape) gets normalised: cycling promotes the
        // TODO inside the quote body to canonical TODO-first.
        // Without this normalisation, `cycle_todo("> TODO foo")`
        // would output `"TODO > TODO foo"` — a double TODO that
        // `split_todo` would misread.
        assert_eq!(cycle_todo("> TODO foo"), "DOING > foo");
        assert_eq!(cycle_todo("> DOING foo"), "DONE > foo");
        assert_eq!(cycle_todo("> DONE foo"), "> foo");
    }

    #[test]
    fn set_todo_reaches_doing_from_every_starting_state() {
        for start in ["ship it", "TODO ship it", "DOING ship it", "DONE ship it"] {
            assert_eq!(set_todo(start, Some(TodoState::Doing)), "DOING ship it");
        }
    }
}
