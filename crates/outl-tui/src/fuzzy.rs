//! Simple fuzzy matcher — `query` chars match in order, prefer
//! consecutive runs and word-boundary hits. Used by the quick switcher
//! and the universal search popup.
//!
//! Not a battle-hardened fzf clone — but enough for "type a few letters,
//! find a page by title or filename." Returns a score where higher is
//! better, plus the byte indices of the matched chars (for highlight).
//!
//! No dependency added: the algorithm is a 50-line linear scan.

/// Per-match score modifiers. Tuned by feel.
const SCORE_CONSECUTIVE: i32 = 30;
const SCORE_WORD_START: i32 = 60;
const SCORE_CAMEL_BOUNDARY: i32 = 40;
const SCORE_BASE_MATCH: i32 = 10;
const SCORE_LEADING_PENALTY: i32 = -3;
const SCORE_GAP_PENALTY: i32 = -1;

/// Result of a fuzzy match. `score == 0` is the sentinel for "miss".
#[derive(Debug, Clone)]
pub struct FuzzyMatch {
    /// Higher is better.
    pub score: i32,
    /// Char positions inside `haystack` that matched, in order.
    /// Useful for the popup to highlight the matched chars.
    pub indices: Vec<usize>,
}

/// Match `query` against `haystack`, case-insensitively, requiring every
/// query character to appear in order. Returns `None` on a miss.
///
/// Special case: an empty query matches everything with a flat score so
/// the original order survives.
pub fn fuzzy_match(query: &str, haystack: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch {
            score: 1,
            indices: Vec::new(),
        });
    }

    let hay_chars: Vec<char> = haystack.chars().collect();
    let q_chars: Vec<char> = query.chars().collect();

    let mut indices = Vec::with_capacity(q_chars.len());
    let mut score = 0i32;
    let mut q_idx = 0usize;
    let mut last_match: Option<usize> = None;

    for (i, &hc) in hay_chars.iter().enumerate() {
        if q_idx >= q_chars.len() {
            break;
        }
        let qc = q_chars[q_idx];
        if char_equal_ci(hc, qc) {
            score += SCORE_BASE_MATCH;
            // Bonuses.
            if i == 0 {
                score += SCORE_WORD_START;
            } else if let Some(prev_char) = hay_chars.get(i - 1) {
                // Treat `_` and `/` as identifier-internal: they don't
                // create a word boundary. Without this, fuzzy match
                // scores spike inside path-like or snake_case
                // identifiers in ways users find counter-intuitive.
                let is_word_boundary =
                    !prev_char.is_alphanumeric() && *prev_char != '/' && *prev_char != '_';
                let is_camel = prev_char.is_lowercase() && hc.is_uppercase();
                if is_word_boundary {
                    score += SCORE_WORD_START;
                } else if is_camel {
                    score += SCORE_CAMEL_BOUNDARY;
                }
            }
            if let Some(prev) = last_match {
                if i == prev + 1 {
                    score += SCORE_CONSECUTIVE;
                } else {
                    // Gap penalty proportional to skip distance, capped.
                    let gap = ((i - prev - 1) as i32).min(10);
                    score += SCORE_GAP_PENALTY * gap;
                }
            } else {
                // Leading gap from start to first match.
                score += SCORE_LEADING_PENALTY * (i as i32).min(15);
            }
            indices.push(i);
            last_match = Some(i);
            q_idx += 1;
        }
    }

    if q_idx == q_chars.len() {
        Some(FuzzyMatch { score, indices })
    } else {
        None
    }
}

/// Score-only convenience.
pub fn fuzzy_score(query: &str, haystack: &str) -> Option<i32> {
    fuzzy_match(query, haystack).map(|m| m.score)
}

/// Best score across a page's names: its primary display name plus any
/// `alias::` alternatives (issue #302). Aliases score beside the title
/// so `NYC` finds "New York" in every candidate list.
pub fn best_score(query: &str, primary: &str, aliases: &[String]) -> Option<i32> {
    std::iter::once(primary)
        .chain(aliases.iter().map(String::as_str))
        .filter_map(|name| fuzzy_score(query, name))
        .max()
}

fn char_equal_ci(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert!(fuzzy_match("", "any").is_some());
        assert!(fuzzy_match("", "").is_some());
    }

    #[test]
    fn miss_returns_none() {
        assert!(fuzzy_match("xyz", "hello").is_none());
        // Order matters.
        assert!(fuzzy_match("ba", "abc").is_none());
    }

    #[test]
    fn exact_prefix_scores_well() {
        let a = fuzzy_score("foo", "foobar").unwrap();
        let b = fuzzy_score("foo", "barfoobaz").unwrap();
        assert!(a > b, "prefix at position 0 should outscore mid-string");
    }

    #[test]
    fn word_boundaries_beat_letters_in_middle() {
        // The second `s` in "ws-app" sits at the start of a word
        // (the `s` after `-`), so it earns a fresh word-start bonus.
        // The `s` in "wxyzs" is buried mid-token.
        let a = fuzzy_score("ws", "ws-app").unwrap();
        let b = fuzzy_score("ws", "wxyzs").unwrap();
        assert!(a > b, "ws-app ({a}) should outscore wxyzs ({b})");
    }

    #[test]
    fn consecutive_run_beats_scattered() {
        let a = fuzzy_score("foo", "myfoo").unwrap();
        let b = fuzzy_score("foo", "f___o____o").unwrap();
        assert!(a > b);
    }

    #[test]
    fn case_insensitive() {
        assert!(fuzzy_match("Foo", "foobar").is_some());
        assert!(fuzzy_match("foo", "FooBar").is_some());
    }

    #[test]
    fn indices_point_to_matched_chars() {
        let m = fuzzy_match("hl", "hello").unwrap();
        assert_eq!(m.indices, vec![0, 2]);
    }

    #[test]
    fn ranking_a_list_works_as_expected() {
        let haystacks = ["readme", "release-notes", "meals", "remember"];
        let mut scored: Vec<_> = haystacks
            .iter()
            .filter_map(|h| fuzzy_score("re", h).map(|s| (s, *h)))
            .collect();
        scored.sort_by_key(|b| std::cmp::Reverse(b.0));
        // The two that *start* with "re" should rank above "meals" / "remember".
        let top_two: Vec<&str> = scored.iter().take(2).map(|(_, h)| *h).collect();
        assert!(top_two.contains(&"readme"));
        assert!(top_two.contains(&"release-notes"));
    }
}
