//! The caret inside the selected block's text.
//!
//! Character granularity — moving between blocks is
//! `super::selection`. Every reader here honours `App::focus`, so a
//! backlink block reports its own text rather than the outline's.

use crate::outline_ops::{node_at_path, path_for_index};
use crate::state::{App, Focus};

impl App {
    /// Current selected block's text (or empty if no selection).
    /// Honours `app.focus` so backlink blocks return their own text.
    pub(crate) fn current_block_text(&self) -> String {
        match &self.focus {
            Focus::Outline => {
                let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
                    return String::new();
                };
                node_at_path(&self.page.blocks, &path)
                    .map(|n| n.text.clone())
                    .unwrap_or_default()
            }
            Focus::Backlink { idx, sub_path } => {
                let backlinks = self.backlinks_for_current();
                let Some(bl) = backlinks.get(*idx) else {
                    return String::new();
                };
                let mut node = &bl.source_block;
                for &i in sub_path {
                    let Some(child) = node.children.get(i) else {
                        return String::new();
                    };
                    node = child;
                }
                node.text.clone()
            }
        }
    }

    pub(crate) fn current_block_char_count(&self) -> usize {
        self.current_block_text().chars().count()
    }

    pub(crate) fn move_cursor_col(&mut self, delta: i32) {
        let max = self.current_block_char_count() as i32;
        let next = (self.cursor_col as i32 + delta).clamp(0, max);
        self.cursor_col = next as usize;
    }

    pub(crate) fn cursor_to_home(&mut self) {
        self.cursor_col = 0;
    }

    pub(crate) fn cursor_to_end(&mut self) {
        self.cursor_col = self.current_block_char_count();
    }

    pub(crate) fn cursor_word_left(&mut self) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        let mut i = self.cursor_col;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        self.cursor_col = i;
    }

    pub(crate) fn cursor_word_right(&mut self) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = self.cursor_col;
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor_col = i;
    }
}
