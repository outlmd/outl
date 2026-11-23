//! The nouns a rename / merge summary counts, so four surfaces name one
//! fact one way.
//!
//! [`rename_page`](crate::rename_page) and
//! [`merge_pages`](crate::merge_pages()) return counts, never sentences:
//! a terminal status line, a GUI toast and `outl page merge`'s stdout
//! have genuinely different densities, and
//! [`crate::refusal::ahead_of_log_notice`]'s doc already settles that a
//! surface owns *where* a notice goes and how dense it is. What it does
//! not get to own is **what the thing is called**.
//!
//! It did. One field, `skipped_namespace_refs`, reached the user in
//! three spellings:
//!
//! | Surface | Said |
//! |---|---|
//! | `outl-tui` | `1 sub-page link left alone` |
//! | desktop + mobile | `1 namespace link left unchanged` |
//! | `outl page merge` | `left 1 namespace reference(s) alone (they name a sub-page)` |
//!
//! and none of the three used the word [`UX.md`'s glossary] defines for
//! the hierarchy, which is a **nested page**. A user who renames on the
//! phone and merges in the terminal is told about two different things.
//!
//! `rewritten_blocks` had the sharper version of the same problem: the
//! TUI called it blocks and the GUI called it links. It counts **blocks
//! whose text was edited**, and one block can carry several mentions,
//! so "3 links updated" was not a denser phrasing of "links updated in
//! 3 blocks" — it was a different, wrong number.
//!
//! # The mirror
//!
//! `@outl/shared/pages/rename-merge` keeps its own copy, for the reason
//! [`crate::refusal::ahead_of_log_notice`] gives: a Solid component
//! cannot call into this crate, and shipping prose over the IPC on
//! every rename to avoid retyping two nouns is not a trade worth
//! making. So it is a **declared mirror**, pinned rather than trusted —
//! `the_shared_frontend_copy_counts_the_same_things` reads that file
//! and compares both spellings.
//!
//! [`UX.md`'s glossary]: https://github.com/outlmd/outl/blob/main/UX.md

/// A noun a summary counts, with the plural spelled out.
///
/// Every noun below happens to pluralise with `s`, so a suffix rule
/// would work today. It is spelled out because the mirrored set on the
/// TypeScript side already contains one that does not (`alias` /
/// `aliases`), so a suffix rule here is a trap for whoever moves that
/// one over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Noun {
    /// The singular, for exactly one.
    pub one: &'static str,
    /// The plural, for every other count including zero.
    pub many: &'static str,
}

impl Noun {
    /// `"1 block"`, `"3 blocks"`, `"0 blocks"`.
    pub fn count(&self, n: usize) -> String {
        if n == 1 {
            format!("1 {}", self.one)
        } else {
            format!("{n} {}", self.many)
        }
    }
}

/// A block, the unit both reports count.
pub const BLOCK: Noun = Noun {
    one: "block",
    many: "blocks",
};

/// A mention left as it was because it names a **nested page** that is
/// not being renamed or merged (`[[old/child]]`, `#old/child`).
///
/// `RenameReport::skipped_namespace_refs` /
/// `MergeReport::skipped_namespace_refs`. The field keeps the
/// implementation's word (`namespace`); the user reads the glossary's.
pub const NESTED_PAGE_LINK: Noun = Noun {
    one: "nested-page link",
    many: "nested-page links",
};

/// A property both pages set to a different value, the target's kept.
pub const PROPERTY_CONFLICT: Noun = Noun {
    one: "property conflict",
    many: "property conflicts",
};

/// What happened to the mentions that *were* respelled, and to the ones
/// that were not.
///
/// Both reports carry the same pair, so the two halves are worded here
/// instead of in each surface. Only the nonzero halves are returned: a
/// rename that moved no links has nothing to say about them.
///
/// `rewritten` is a count of **blocks**, deliberately. One block can
/// carry several mentions of the page, so calling it a count of links
/// overstates it.
pub fn rewrite_parts(rewritten: usize, skipped: usize) -> Vec<String> {
    let mut parts = Vec::new();
    if rewritten > 0 {
        parts.push(format!("links updated in {}", BLOCK.count(rewritten)));
    }
    if skipped > 0 {
        parts.push(format!(
            "{} left unchanged",
            NESTED_PAGE_LINK.count(skipped)
        ));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_is_singular_and_zero_is_not() {
        assert_eq!(BLOCK.count(1), "1 block");
        assert_eq!(BLOCK.count(3), "3 blocks");
        assert_eq!(BLOCK.count(0), "0 blocks");
    }

    #[test]
    fn an_irregular_plural_is_spelled_not_suffixed() {
        // The reason `Noun` carries both forms. A suffix rule gives
        // `property conflicts` right and would give `aliass` wrong, and
        // the GUI's summary counts aliases.
        assert_eq!(PROPERTY_CONFLICT.count(2), "2 property conflicts");
        assert_eq!(NESTED_PAGE_LINK.count(1), "1 nested-page link");
    }

    #[test]
    fn only_the_nonzero_halves_are_worded() {
        assert!(rewrite_parts(0, 0).is_empty());
        assert_eq!(rewrite_parts(2, 0), vec!["links updated in 2 blocks"]);
        assert_eq!(
            rewrite_parts(0, 1),
            vec!["1 nested-page link left unchanged"]
        );
    }

    #[test]
    fn a_rewrite_is_counted_in_blocks_never_in_links() {
        // One block can carry several mentions, so the number is a
        // count of blocks and the sentence has to say so.
        let parts = rewrite_parts(3, 0);
        assert_eq!(parts, vec!["links updated in 3 blocks"]);
        assert!(
            !parts[0].starts_with("3 links"),
            "that would claim a mention count this number is not"
        );
    }

    /// `@outl/shared/pages/rename-merge` is a declared mirror of the
    /// nouns above, so the two spellings are compared rather than
    /// trusted. Density is free to differ; the words are not.
    #[test]
    fn the_shared_frontend_copy_counts_the_same_things() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../outl-frontend-shared/src/pages/rename-merge.ts"
        );
        let ts =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

        for noun in [NESTED_PAGE_LINK, BLOCK] {
            assert!(
                ts.contains(&format!("\"{}\"", noun.one))
                    && ts.contains(&format!("\"{}\"", noun.many)),
                "{path} no longer spells {:?} / {:?} — the same rename would \
                 name different things in the terminal and in the app",
                noun.one,
                noun.many
            );
        }
        // Quoted, so this guards the string the user reads and not
        // prose about the field, which is named `skipped_namespace_refs`
        // and keeps the implementation's word.
        assert!(
            !ts.contains("\"namespace link\""),
            "{path} is back to showing a nested page as a namespace, which \
             is the implementation's word and not the glossary's"
        );
    }
}
