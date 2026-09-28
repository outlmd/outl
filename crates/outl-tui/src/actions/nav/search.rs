//! `*` / `#` — search the workspace for the word under the cursor.
//!
//! The search itself belongs to the `/` overlay; this module only
//! extracts the word and drives that machinery, so the two can never
//! disagree about what a hit is.

use crate::state::App;
use anyhow::Result;

impl App {
    /// `*` / `#` — extract the word under the cursor and feed it to
    /// the workspace search. Direction is `forward = true` for `*`,
    /// `false` for `#`. No-op when the cursor isn't sitting on a word.
    pub(crate) fn search_word_under_cursor(&mut self, forward: bool) -> Result<()> {
        let text = self.current_block_text();
        let Some(word) = word_under_cursor(&text, self.cursor_col) else {
            self.status = "no word under cursor".into();
            return Ok(());
        };
        self.run_inline_search(&word, forward)
    }

    /// Build a search the same way the `/` overlay does, jump straight
    /// to the first hit, persist the rest into `last_search` so
    /// `n`/`N` can walk through them. Used by `*` / `#`.
    fn run_inline_search(&mut self, query: &str, forward: bool) -> Result<()> {
        // Reuse the overlay's machinery: open, set query, refresh,
        // accept. Cheaper than reimplementing the workspace walk.
        self.open_search();
        if let Some(crate::state::Overlay::Search(ref mut s)) = self.overlay {
            s.query = query.to_string();
        }
        self.refresh_search();
        self.accept_search()?;
        if forward {
            // `accept_search` already lands on hit 0; nothing else to do.
        } else {
            // `accept_search` set `last_search.cursor = 0`. `search_prev`
            // wraps 0 → len-1, which is exactly the last hit `#` wants.
            self.search_prev()?;
        }
        Ok(())
    }
}

/// Extract the word under `cursor` from `text`. A "word" is a
/// contiguous run of non-whitespace chars. Returns `None` on
/// whitespace / empty text.
fn word_under_cursor(text: &str, cursor: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let pos = cursor.min(chars.len().saturating_sub(1));
    if chars[pos].is_whitespace() {
        return None;
    }
    let mut start = pos;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }
    let mut end = pos;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    let word: String = chars[start..end].iter().collect();
    if word.is_empty() {
        None
    } else {
        Some(word)
    }
}

#[cfg(test)]
mod word_tests {
    use super::word_under_cursor;

    #[test]
    fn returns_word_under_cursor() {
        assert_eq!(
            word_under_cursor("hello world", 2),
            Some("hello".to_string())
        );
        assert_eq!(
            word_under_cursor("hello world", 6),
            Some("world".to_string())
        );
    }

    #[test]
    fn returns_none_on_whitespace_or_empty() {
        assert_eq!(word_under_cursor("hello world", 5), None);
        assert_eq!(word_under_cursor("", 0), None);
        assert_eq!(word_under_cursor("   ", 1), None);
    }

    #[test]
    fn clamps_past_end() {
        assert_eq!(
            word_under_cursor("hello", 99),
            Some("hello".to_string()),
            "cursor past EOL still picks up the last word"
        );
    }
}
