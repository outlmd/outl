//! Single-block text editor buffer.
//!
//! Holds a `Vec<char>` plus a cursor index; exposes the small set of
//! cursor + edit operations the TUI needs. Block text in outl is
//! single-line for now — multi-line support lives behind the same
//! API for when block content grows (Yrs already supports it).

/// Edit buffer with cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditBuffer {
    /// Character contents.
    pub chars: Vec<char>,
    /// Cursor position in chars (0..=chars.len()).
    pub cursor: usize,
}

impl EditBuffer {
    /// Build from a string with the cursor placed at the end.
    pub fn from_text(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let cursor = chars.len();
        Self { chars, cursor }
    }

    /// Empty buffer with cursor at 0.
    pub fn empty() -> Self {
        Self {
            chars: Vec::new(),
            cursor: 0,
        }
    }

    /// Render as `String`.
    pub fn as_string(&self) -> String {
        self.chars.iter().collect()
    }

    /// Length in characters.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Insert one character at the cursor.
    pub fn insert_char(&mut self, ch: char) {
        self.chars.insert(self.cursor, ch);
        self.cursor += 1;
    }

    /// Insert a string at the cursor.
    pub fn insert_str(&mut self, s: &str) {
        for ch in s.chars() {
            self.insert_char(ch);
        }
    }

    /// Insert a pair like `(`/`)`, leaving the cursor between them.
    pub fn insert_pair(&mut self, open: char, close: char) {
        self.chars.insert(self.cursor, open);
        self.chars.insert(self.cursor + 1, close);
        self.cursor += 1;
    }

    /// Backspace inside an empty doubled pair like `[[|]]` or `((|))`
    /// — delete both opener and closer in one shot so the user
    /// doesn't have to backspace four times to undo an aborted ref.
    /// Returns `true` when the pair was collapsed, `false` when no
    /// such pair surrounds the cursor (caller should fall back to
    /// the normal one-char backspace).
    pub fn delete_pair_back(&mut self) -> bool {
        if self.cursor < 2 || self.cursor + 2 > self.chars.len() {
            return false;
        }
        let left = (self.chars[self.cursor - 2], self.chars[self.cursor - 1]);
        let right = (self.chars[self.cursor], self.chars[self.cursor + 1]);
        let is_brackets = left == ('[', '[') && right == (']', ']');
        let is_parens = left == ('(', '(') && right == (')', ')');
        if !is_brackets && !is_parens {
            return false;
        }
        // Remove [opener, opener, closer, closer] around the cursor.
        for _ in 0..2 {
            self.chars.remove(self.cursor); // both closers (shift left)
        }
        for _ in 0..2 {
            self.cursor -= 1;
            self.chars.remove(self.cursor); // both openers
        }
        true
    }

    /// Delete the character before the cursor (Backspace).
    /// Returns `true` if a character was removed.
    pub fn delete_back(&mut self) -> bool {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
            true
        } else {
            false
        }
    }

    /// Delete the character at the cursor (Delete).
    pub fn delete_forward(&mut self) -> bool {
        if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
            true
        } else {
            false
        }
    }

    /// Move cursor left one character.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor right one character.
    pub fn move_right(&mut self) {
        if self.cursor < self.chars.len() {
            self.cursor += 1;
        }
    }

    /// Move cursor to the start of the **current line**.
    ///
    /// Equivalent to `cursor = 0` for a single-line buffer, which is
    /// every ordinary block. It differs inside a multi-line one (a
    /// fenced code block typed with soft newlines), where `Home` means
    /// the start of the line the cursor is on — not the top of the
    /// fence, which is what this used to do while its doc comment
    /// already said "line".
    ///
    /// Line math is [`outl_md::view`]'s, like [`Self::move_up`]'s: one
    /// owner for the `(line, col) ↔ char_idx` mapping, shared with the
    /// renderer.
    pub fn move_home(&mut self) {
        let text = self.as_string();
        let (line, _) = outl_md::view::char_to_line_col(&text, self.cursor);
        self.cursor = outl_md::view::line_col_to_char(&text, line, 0);
    }

    /// Move cursor to the end of the **current line**. Mirror of
    /// [`Self::move_home`].
    pub fn move_end(&mut self) {
        let text = self.as_string();
        let (line, _) = outl_md::view::char_to_line_col(&text, self.cursor);
        // `usize::MAX` is the documented way to ask `line_col_to_char`
        // for a line's end: it clamps a column past EOL to the last
        // char of that line, which is the same vim-style clamp
        // `move_up` / `move_down` lean on for the preferred column.
        self.cursor = outl_md::view::line_col_to_char(&text, line, usize::MAX);
    }

    /// Move cursor one *visual* line up inside the buffer (across
    /// `\n` boundaries — soft newlines inside the same block).
    ///
    /// Returns `true` when the cursor moved, `false` when it was
    /// already on the buffer's first line. The caller uses the false
    /// branch to spill out of the current block (cross-block nav).
    ///
    /// Preserves the cursor's column from the current line (vim-style
    /// preferred column). When the target line is shorter, the
    /// cursor clamps to its end instead of overshooting into the
    /// next line.
    ///
    /// Line/column math is delegated to
    /// [`outl_md::view::char_to_line_col`] / [`outl_md::view::line_col_to_char`]
    /// — the same primitives `block_to_rows` already uses to render
    /// the buffer. Keeping a single owner for the mapping prevents
    /// drift between the cursor logic and the renderer.
    pub fn move_up(&mut self) -> bool {
        let text = self.as_string();
        let (line, col) = outl_md::view::char_to_line_col(&text, self.cursor);
        if line == 0 {
            return false;
        }
        self.cursor = outl_md::view::line_col_to_char(&text, line - 1, col);
        true
    }

    /// Mirror of [`Self::move_up`] — one visual line down.
    pub fn move_down(&mut self) -> bool {
        let text = self.as_string();
        let (line, col) = outl_md::view::char_to_line_col(&text, self.cursor);
        let last_line = text.chars().filter(|c| *c == '\n').count();
        if line >= last_line {
            return false;
        }
        self.cursor = outl_md::view::line_col_to_char(&text, line + 1, col);
        true
    }

    /// Park the cursor on the buffer's first or last line at `col`,
    /// clamped to that line's end. This is where a cross-block `↓` / `↑`
    /// arrives, carrying the preferred column of the block it left.
    ///
    /// The line is the one that **faces** the block being left: stepping
    /// up arrives on the last line, stepping down on the first. A raw
    /// `cursor = col` gets both wrong inside a multi-line buffer (it
    /// always lands on the first line, and overshoots into the second
    /// when `col` is past the first line's end), which is where
    /// `@outl/shared/caret-nav`'s `entryCaretOffset` and this client
    /// used to disagree.
    pub fn land_on_edge_line(&mut self, last_line: bool, col: usize) {
        let text = self.as_string();
        let line = if last_line {
            text.chars().filter(|c| *c == '\n').count()
        } else {
            0
        };
        self.cursor = outl_md::view::line_col_to_char(&text, line, col);
    }

    /// Column the cursor sits on within its current visual line.
    /// Equivalent to `cursor` for single-line buffers; for multi-line
    /// buffers it's the offset past the last `\n` before the cursor.
    ///
    /// Used by cross-block nav to seed `App.cursor_col` so the cursor
    /// lands at a comparable position in the next block.
    pub fn visual_column(&self) -> usize {
        outl_md::view::char_to_line_col(&self.as_string(), self.cursor).1
    }

    /// Move cursor to the previous word boundary.
    pub fn move_word_left(&mut self) {
        while self.cursor > 0 && self.chars[self.cursor - 1].is_whitespace() {
            self.cursor -= 1;
        }
        while self.cursor > 0 && !self.chars[self.cursor - 1].is_whitespace() {
            self.cursor -= 1;
        }
    }

    /// Move cursor to the next word boundary.
    pub fn move_word_right(&mut self) {
        let len = self.chars.len();
        while self.cursor < len && !self.chars[self.cursor].is_whitespace() {
            self.cursor += 1;
        }
        while self.cursor < len && self.chars[self.cursor].is_whitespace() {
            self.cursor += 1;
        }
    }

    /// Delete from the cursor to the start of the current word.
    pub fn delete_word_back(&mut self) {
        let start = self.cursor;
        self.move_word_left();
        self.chars.drain(self.cursor..start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_text_places_cursor_at_end() {
        let b = EditBuffer::from_text("hello");
        assert_eq!(b.cursor, 5);
        assert_eq!(b.as_string(), "hello");
    }

    #[test]
    fn insert_char_advances_cursor() {
        let mut b = EditBuffer::from_text("hllo");
        b.cursor = 1;
        b.insert_char('e');
        assert_eq!(b.as_string(), "hello");
        assert_eq!(b.cursor, 2);
    }

    #[test]
    fn insert_pair_leaves_cursor_between() {
        let mut b = EditBuffer::empty();
        b.insert_pair('[', ']');
        assert_eq!(b.as_string(), "[]");
        assert_eq!(b.cursor, 1);
    }

    #[test]
    fn delete_back_at_zero_is_noop() {
        let mut b = EditBuffer::from_text("abc");
        b.cursor = 0;
        assert!(!b.delete_back());
        assert_eq!(b.as_string(), "abc");
    }

    #[test]
    fn delete_back_removes_prev_char() {
        let mut b = EditBuffer::from_text("abc");
        assert!(b.delete_back());
        assert_eq!(b.as_string(), "ab");
        assert_eq!(b.cursor, 2);
    }

    #[test]
    fn move_word_boundaries() {
        let mut b = EditBuffer::from_text("hello world today");
        b.move_word_left();
        assert_eq!(b.cursor, 12); // start of "today"
        b.move_word_left();
        assert_eq!(b.cursor, 6); // start of "world"
        b.move_word_left();
        assert_eq!(b.cursor, 0); // start of "hello"
    }

    #[test]
    fn delete_word_back_removes_word() {
        let mut b = EditBuffer::from_text("hello world");
        b.delete_word_back();
        assert_eq!(b.as_string(), "hello ");
    }

    #[test]
    fn insert_str_handles_multi_char() {
        let mut b = EditBuffer::empty();
        b.insert_str("[[ref]]");
        assert_eq!(b.as_string(), "[[ref]]");
        assert_eq!(b.cursor, 7);
    }

    #[test]
    fn delete_pair_back_collapses_empty_brackets() {
        let mut b = EditBuffer::from_text("foo [[]]");
        b.cursor = 6; // between [[ and ]]
        assert!(b.delete_pair_back());
        assert_eq!(b.as_string(), "foo ");
        assert_eq!(b.cursor, 4);
    }

    #[test]
    fn delete_pair_back_collapses_empty_parens() {
        let mut b = EditBuffer::from_text("see (())");
        b.cursor = 6; // between (( and ))
        assert!(b.delete_pair_back());
        assert_eq!(b.as_string(), "see ");
        assert_eq!(b.cursor, 4);
    }

    #[test]
    fn delete_pair_back_skips_when_pair_has_content() {
        let mut b = EditBuffer::from_text("[[ave]]");
        b.cursor = 5; // between "ave" and ]]
        assert!(!b.delete_pair_back());
        assert_eq!(b.as_string(), "[[ave]]");
        assert_eq!(b.cursor, 5);
    }

    #[test]
    fn delete_pair_back_skips_at_buffer_edges() {
        let mut b = EditBuffer::from_text("[[]]");
        b.cursor = 0;
        assert!(!b.delete_pair_back());
        b.cursor = 4;
        assert!(!b.delete_pair_back());
    }

    #[test]
    fn delete_pair_back_skips_cross_mixed_pairs() {
        let mut b = EditBuffer::from_text("[[))");
        b.cursor = 2;
        assert!(!b.delete_pair_back());
        assert_eq!(b.as_string(), "[[))");
    }

    #[test]
    fn move_up_on_single_line_buffer_returns_false() {
        let mut b = EditBuffer::from_text("hello world");
        b.cursor = 5;
        assert!(!b.move_up());
        assert_eq!(b.cursor, 5, "cursor stays put when no move happened");
    }

    #[test]
    fn move_down_on_single_line_buffer_returns_false() {
        let mut b = EditBuffer::from_text("hello world");
        b.cursor = 5;
        assert!(!b.move_down());
        assert_eq!(b.cursor, 5);
    }

    #[test]
    fn move_up_crosses_soft_newline_preserving_column() {
        // "foo\nbar baz"
        //   line 0: "foo"          (chars 0..3)
        //   '\n'                   (char 3)
        //   line 1: "bar baz"      (chars 4..11)
        // cursor=6 → column 2 of line 1 (the `r` of "bar").
        let mut b = EditBuffer::from_text("foo\nbar baz");
        b.cursor = 6;
        assert!(b.move_up());
        // Column 2 of "foo" is the second `o`.
        assert_eq!(b.cursor, 2);
    }

    #[test]
    fn move_up_lands_at_same_column_when_line_is_long_enough() {
        let mut b = EditBuffer::from_text("hello\nbar");
        b.cursor = 7; // column 1 of "bar" (the `a`)
        assert!(b.move_up());
        // Column 1 of "hello" is `e`.
        assert_eq!(b.cursor, 1);
    }

    #[test]
    fn move_down_crosses_soft_newline_preserving_column() {
        let mut b = EditBuffer::from_text("hi\nworld");
        b.cursor = 1; // column 1 of "hi"
        assert!(b.move_down());
        // Column 1 of "world" is `o`.
        assert_eq!(b.cursor, 4);
    }

    #[test]
    fn move_down_clamps_to_shorter_target_line() {
        let mut b = EditBuffer::from_text("hello\nhi");
        b.cursor = 4; // column 4 of "hello" (the second `l`)
        assert!(b.move_down());
        // Line "hi" has length 2 → column 4 clamps to end.
        assert_eq!(b.cursor, 8);
    }

    #[test]
    fn visual_column_inside_multiline_buffer() {
        let mut b = EditBuffer::from_text("foo\nbar baz");
        b.cursor = 7; // 3 chars into line 1
        assert_eq!(b.visual_column(), 3);
    }

    #[test]
    fn home_and_end_stay_on_the_cursors_own_line() {
        // A fenced code block typed with soft newlines. `Home` used to
        // jump to the top of the fence and `End` to the bottom of it,
        // while both doc comments already said "line".
        let text = "```rust\nfn main() {}\n```";
        let body_start = "```rust\n".chars().count();

        let mut b = EditBuffer::from_text(text);
        b.cursor = body_start + 3; // inside `fn main() {}`
        b.move_home();
        assert_eq!(b.cursor, body_start);
        b.move_end();
        assert_eq!(b.cursor, body_start + "fn main() {}".chars().count());
    }

    #[test]
    fn home_and_end_are_the_whole_buffer_on_a_single_line_block() {
        // Which is every ordinary block, so the fix must not move this.
        let mut b = EditBuffer::from_text("ship it");
        b.cursor = 3;
        b.move_home();
        assert_eq!(b.cursor, 0);
        b.move_end();
        assert_eq!(b.cursor, 7);
    }

    #[test]
    fn landing_from_below_arrives_on_the_last_line() {
        let mut b = EditBuffer::from_text("abc\nde");
        b.land_on_edge_line(true, 1);
        assert_eq!(b.cursor, "abc\n".chars().count() + 1);
        // A column past the line's end clamps to it.
        b.land_on_edge_line(true, 9);
        assert_eq!(b.cursor, "abc\nde".chars().count());
    }

    #[test]
    fn landing_from_above_stays_on_the_first_line() {
        let mut b = EditBuffer::from_text("ab\ncdef");
        b.land_on_edge_line(false, 1);
        assert_eq!(b.cursor, 1);
        // Past the first line's end clamps there instead of spilling
        // into the second line.
        b.land_on_edge_line(false, 4);
        assert_eq!(b.cursor, 2);
    }

    #[test]
    fn end_on_an_empty_trailing_line_stays_there() {
        let mut b = EditBuffer::from_text("one\n");
        b.cursor = 4; // the empty line after the newline
        b.move_end();
        assert_eq!(b.cursor, 4);
        b.move_home();
        assert_eq!(b.cursor, 4);
    }
}
