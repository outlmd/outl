//! Vim-style char / line text ops in Normal mode.
//!
//! Each method mutates the selected outline block's `text` directly
//! (via `outline_ops::node_at_path_mut`), snapshots for undo, and
//! flushes through the standard `save()`. None of these touch the
//! backlink-source path — Vim-style char ops in Normal are an outline
//! affordance only. Backlink rows fall through to a no-op (the focus
//! check skips them silently); use `Enter` to jump to the source then
//! edit there.

use crate::actions::block::InsertCursor;
use crate::outline_ops::{node_at_path, node_at_path_mut, path_for_index};
use crate::state::{App, Focus};

impl App {
    /// `A` — enter Insert mode with the cursor at the end of the block.
    /// Equivalent to `$` then `i`, kept as a single action so the
    /// shortcut catalog stays declarative.
    pub(crate) fn enter_insert_at_end(&mut self) {
        self.cursor_to_end();
        self.enter_insert(InsertCursor::AtCursor);
    }

    /// `x` — delete the char under the cursor in Normal mode. No-op on
    /// an empty block. The cursor stays put unless it sat past the
    /// last char (delete on the right edge), in which case it shifts
    /// one left so it doesn't dangle past EOL.
    pub(crate) fn delete_char_under_cursor(&mut self) {
        self.mutate_current_text(|chars, cursor| {
            if *cursor >= chars.len() {
                return false;
            }
            chars.remove(*cursor);
            if *cursor > 0 && *cursor >= chars.len() {
                *cursor -= 1;
            }
            true
        });
    }

    /// `X` — delete the char before the cursor (Normal-mode Backspace).
    /// No-op at column 0.
    pub(crate) fn delete_char_before_cursor(&mut self) {
        self.mutate_current_text(|chars, cursor| {
            if *cursor == 0 {
                return false;
            }
            *cursor -= 1;
            chars.remove(*cursor);
            true
        });
    }

    /// `D` — delete from the cursor through the end of the block.
    /// Cursor clamps to the new EOL.
    pub(crate) fn delete_to_end_of_block(&mut self) {
        self.mutate_current_text(|chars, cursor| {
            if *cursor >= chars.len() {
                return false;
            }
            chars.truncate(*cursor);
            true
        });
    }

    /// `C` — delete to end of block, then enter Insert at the new EOL.
    /// vim's `c$`. Snapshots once.
    pub(crate) fn change_to_end_of_block(&mut self) {
        self.delete_to_end_of_block();
        self.enter_insert(InsertCursor::AtCursor);
    }

    /// `S` / `cc` — clear the block's text and enter Insert at column 0.
    /// vim's "substitute line" — rewrite this block from scratch.
    pub(crate) fn substitute_block(&mut self) {
        self.mutate_current_text(|chars, cursor| {
            if chars.is_empty() && *cursor == 0 {
                return false;
            }
            chars.clear();
            *cursor = 0;
            true
        });
        self.enter_insert(InsertCursor::Start);
    }

    /// `s` — delete the char under the cursor and enter Insert (vim's
    /// substitute char = `xi`). On an empty block lands at column 0 in
    /// Insert without touching the buffer.
    pub(crate) fn substitute_char(&mut self) {
        self.delete_char_under_cursor();
        self.enter_insert(InsertCursor::AtCursor);
    }

    /// `r{ch}` — replace the char under the cursor with `ch` without
    /// entering Insert. No-op on empty block / cursor past EOL.
    pub(crate) fn replace_char_under_cursor(&mut self, ch: char) {
        self.mutate_current_text(|chars, cursor| {
            if *cursor >= chars.len() {
                return false;
            }
            chars[*cursor] = ch;
            true
        });
    }

    /// `~` — toggle case of the char under the cursor and advance one
    /// position. vim moves past the changed char so `~~~` toggles three
    /// in a row; we follow.
    pub(crate) fn toggle_case_under_cursor(&mut self) {
        let advanced = self.mutate_current_text(|chars, cursor| {
            if *cursor >= chars.len() {
                return false;
            }
            let ch = chars[*cursor];
            let swapped = if ch.is_uppercase() {
                ch.to_lowercase().next().unwrap_or(ch)
            } else if ch.is_lowercase() {
                ch.to_uppercase().next().unwrap_or(ch)
            } else {
                ch
            };
            chars[*cursor] = swapped;
            *cursor += 1;
            true
        });
        if advanced {
            // `mutate_current_text` already wrote the new cursor into
            // `self.cursor_col`; nothing else to do.
        }
    }

    /// `Y` — alias of `yy`. vim's older "yank line" matches `yy`
    /// exactly; modern vim defaults to `y$` instead, but users who
    /// reach for `Y` invariably mean "yank this whole line". Outline
    /// = block, so the line is the block.
    pub(crate) fn yank_current_alias(&mut self) {
        self.yank_current();
    }

    /// `e` — move the cursor to the end of the current / next word.
    /// vim's `e` lands on the **last** char of the word (one before
    /// the boundary). Empty block / EOL is a no-op.
    pub(crate) fn cursor_word_end(&mut self) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        if chars.is_empty() {
            return;
        }
        let len = chars.len();
        let mut i = self.cursor_col.min(len);
        // Step one forward so a cursor sitting on the last char of a
        // word moves to the next word's end (vim semantics).
        if i < len {
            i += 1;
        }
        // Skip whitespace.
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        // Walk through the word.
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        // Vim lands on the last non-whitespace char, not past it.
        if i > 0 && i <= len && (i == len || chars[i].is_whitespace()) {
            i -= 1;
        }
        self.cursor_col = i;
    }

    /// `f{ch}` — find the next occurrence of `ch` on the current
    /// block's text (after the cursor). Lands the cursor on the match.
    /// No-op if not found.
    pub(crate) fn find_char_forward(&mut self, ch: char) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        let start = self.cursor_col.saturating_add(1).min(chars.len());
        if let Some(off) = chars[start..].iter().position(|c| *c == ch) {
            self.cursor_col = start + off;
        } else {
            self.status = format!("'{ch}' not found");
        }
    }

    /// `F{ch}` — find the previous occurrence of `ch` before the
    /// cursor. Lands the cursor on the match. No-op if not found.
    pub(crate) fn find_char_backward(&mut self, ch: char) {
        let text = self.current_block_text();
        let chars: Vec<char> = text.chars().collect();
        if self.cursor_col == 0 {
            self.status = format!("'{ch}' not found");
            return;
        }
        let end = self.cursor_col.min(chars.len());
        if let Some(off) = chars[..end].iter().rposition(|c| *c == ch) {
            self.cursor_col = off;
        } else {
            self.status = format!("'{ch}' not found");
        }
    }

    /// Apply a closure to the current block's `(text_chars, cursor)`.
    /// Snapshots for undo, writes the result back, syncs the App's
    /// `cursor_col`, persists. Returns whatever the closure returned —
    /// `false` means "nothing changed", so we skip the snapshot + save
    /// and don't pollute history.
    ///
    /// The Backlink focus path is intentionally a no-op: char-level
    /// edits in Normal mode on a backlink row would mutate someone
    /// else's file silently. Use `Enter` to jump to the source first.
    fn mutate_current_text<F>(&mut self, f: F) -> bool
    where
        F: FnOnce(&mut Vec<char>, &mut usize) -> bool,
    {
        if matches!(self.focus, Focus::Backlink { .. }) {
            return false;
        }
        let Some(path) = path_for_index(&self.page.blocks, self.selected) else {
            return false;
        };
        let Some(node) = node_at_path(&self.page.blocks, &path) else {
            return false;
        };
        let mut chars: Vec<char> = node.text.chars().collect();
        let mut cursor = self.cursor_col.min(chars.len());
        let changed = f(&mut chars, &mut cursor);
        if !changed {
            self.cursor_col = cursor;
            return false;
        }
        self.snapshot_for_undo();
        if let Some(node) = node_at_path_mut(&mut self.page.blocks, &path) {
            node.text = chars.iter().collect();
        }
        self.cursor_col = cursor.min(self.current_block_char_count());
        self.save();
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::state::App;
    use outl_core::{ActorId, Workspace};
    use tempfile::TempDir;

    fn fresh_app() -> (App, TempDir) {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let app = App::new(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        (app, dir)
    }

    fn seed_single_block(app: &mut App, text: &str) {
        // Replace whatever the journal has with a single block whose
        // text is `text`. Cheap enough since we're operating on the
        // in-memory `ParsedPage` directly.
        app.page.blocks.clear();
        app.page.blocks.push(outl_md::parse::OutlineNode {
            text: text.to_string(),
            children: vec![],
            properties: vec![],
        });
        app.flat_len = 1;
        app.selected = 0;
    }

    #[test]
    fn delete_char_under_cursor_removes_and_clamps() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "abcd");
        app.cursor_col = 3;
        app.delete_char_under_cursor();
        assert_eq!(app.current_block_text(), "abc");
        assert_eq!(app.cursor_col, 2, "cursor stepped left off the end");
    }

    #[test]
    fn delete_char_before_cursor_at_zero_is_noop() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "abc");
        app.cursor_col = 0;
        app.delete_char_before_cursor();
        assert_eq!(app.current_block_text(), "abc");
        assert_eq!(app.cursor_col, 0);
    }

    #[test]
    fn delete_to_end_of_block_truncates() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "hello world");
        app.cursor_col = 5;
        app.delete_to_end_of_block();
        assert_eq!(app.current_block_text(), "hello");
        assert_eq!(app.cursor_col, 5);
    }

    #[test]
    fn substitute_block_clears_and_resets_cursor() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "old text");
        app.cursor_col = 3;
        app.substitute_block();
        assert_eq!(app.current_block_text(), "");
        // After substitute_block, App enters Insert mode — current_block_text
        // still reads from page.blocks (not the in-flight EditBuffer).
    }

    #[test]
    fn replace_char_under_cursor_swaps_char() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "hello");
        app.cursor_col = 1;
        app.replace_char_under_cursor('a');
        assert_eq!(app.current_block_text(), "hallo");
        assert_eq!(app.cursor_col, 1, "vim's r doesn't advance");
    }

    #[test]
    fn replace_char_on_empty_block_is_noop() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "");
        app.cursor_col = 0;
        app.replace_char_under_cursor('z');
        assert_eq!(app.current_block_text(), "");
    }

    #[test]
    fn toggle_case_under_cursor_swaps_and_advances() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "hello");
        app.cursor_col = 0;
        app.toggle_case_under_cursor();
        assert_eq!(app.current_block_text(), "Hello");
        assert_eq!(app.cursor_col, 1, "advances past the toggled char");
        app.toggle_case_under_cursor();
        assert_eq!(app.current_block_text(), "HEllo");
        assert_eq!(app.cursor_col, 2);
    }

    #[test]
    fn cursor_word_end_lands_on_last_char_of_word() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "hello world today");
        app.cursor_col = 0;
        app.cursor_word_end();
        assert_eq!(app.cursor_col, 4, "end of 'hello'");
        app.cursor_word_end();
        assert_eq!(app.cursor_col, 10, "end of 'world'");
        app.cursor_word_end();
        assert_eq!(app.cursor_col, 16, "end of 'today'");
    }

    #[test]
    fn find_char_forward_lands_on_match() {
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "find a char");
        app.cursor_col = 0;
        app.find_char_forward('a');
        assert_eq!(app.cursor_col, 5, "first 'a' after cursor");
    }

    #[test]
    fn find_char_forward_skips_cursor_position() {
        // vim's f starts searching *after* the cursor, not at it, so
        // pressing `fx` twice walks through every `x`.
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "axbxcx");
        app.cursor_col = 1;
        app.find_char_forward('x');
        assert_eq!(app.cursor_col, 3);
        app.find_char_forward('x');
        assert_eq!(app.cursor_col, 5);
    }

    #[test]
    fn find_char_backward_lands_on_match() {
        // "find a char" — chars: f(0) i(1) n(2) d(3) ' '(4) a(5) ' '(6) c(7) h(8) a(9) r(10).
        let (mut app, _dir) = fresh_app();
        seed_single_block(&mut app, "find a char");
        app.cursor_col = 10;
        app.find_char_backward('a');
        assert_eq!(app.cursor_col, 9, "'a' in 'char'");
        app.find_char_backward('a');
        assert_eq!(app.cursor_col, 5, "'a' standalone");
    }
}
