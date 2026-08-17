//! Thin re-export shim.
//!
//! Historically the entire TUI lived in this file. It grew past 3k
//! lines and got split into responsibility-focused siblings:
//!
//! - [`crate::state`]   — plain data: `App`, modes, overlays, snapshots.
//! - [`crate::actions`] — methods on `App` that mutate state.
//! - [`crate::input`]   — key handlers that route events into actions.
//! - [`crate::view`]    — ratatui rendering.
//! - [`crate::runtime`] — `pub fn run`, terminal lifecycle, event loop.
//!
//! Callers (the `outl` CLI, the `outl-tui` binary) still import
//! `outl_tui::run` and `outl_tui::run_with_theme_override`; those are
//! re-exported from here for backwards compatibility with existing
//! consumers.

pub use crate::runtime::{run, run_with_theme_override};

#[cfg(test)]
mod tests {
    use crate::actions::autocomplete::detect_trigger;
    use crate::actions::block::{cycle_todo_inline, cycle_todo_state};
    use crate::edit_buffer::EditBuffer;
    use crate::state::AutocompleteKind;
    use crate::theme::{self, default_theme};
    use crate::view::{highlight_inline, render_markdown_inline};
    use outl_md::index::WorkspaceIndex;

    // Tokenization and ref-at-cursor logic itself is tested in
    // `outl_md::inline`. The TUI-side tests below cover only the
    // ratatui-specific mapping (raw vs pretty rendering).

    fn t() -> crate::theme::Theme {
        default_theme()
    }

    /// Empty index for tests that don't care about page icons.
    fn idx() -> WorkspaceIndex {
        WorkspaceIndex::default()
    }

    #[test]
    fn pretty_render_strips_bold_markers() {
        let spans = render_markdown_inline("a **brave** soul", &t(), &idx());
        // The literal `**` must not appear in any span; the inner
        // word must.
        for s in &spans {
            assert!(!s.content.contains("**"), "found ** in {:?}", s.content);
        }
        assert!(spans.iter().any(|s| s.content == "brave"));
    }

    #[test]
    fn pretty_render_strips_page_ref_brackets() {
        let spans = render_markdown_inline("see [[Avelino]] today", &t(), &idx());
        // `[[` / `]]` are gone; the bare name is present.
        for s in &spans {
            assert!(!s.content.contains("[["));
            assert!(!s.content.contains("]]"));
        }
        assert!(spans.iter().any(|s| s.content == "Avelino"));
    }

    #[test]
    fn raw_render_keeps_delimiters() {
        // Raw is used for the cursor-bearing block: cursor columns must
        // map to source bytes, so delimiters stay visible.
        let spans = highlight_inline("a **brave** soul", &t());
        let joined: String = spans.iter().map(|s| s.content.clone()).collect();
        assert_eq!(joined, "a **brave** soul");
    }

    #[test]
    fn raw_render_multibyte_does_not_panic() {
        // Regression for the "está" crash: the `á` is 2 bytes.
        let _ = highlight_inline("isso parece que está", &t());
        let _ = highlight_inline("veja [[orçamento]] e #ação", &t());
        let _ = render_markdown_inline("isso parece que está", &t(), &idx());
    }

    #[test]
    fn every_theme_preset_provides_complete_palette() {
        // Sanity: all preset themes share the same set of named styles
        // (compiler-enforced by the struct shape, but a quick smoke
        // here catches accidental Color::Reset placeholders).
        for name in theme::PRESETS {
            let t = theme::by_name(name).unwrap();
            assert_eq!(t.name, *name);
        }
    }

    #[test]
    fn detect_trigger_finds_page_ref() {
        let chars: Vec<char> = "see [[Av".chars().collect();
        let t = detect_trigger(&chars, chars.len()).unwrap();
        assert_eq!(t.0, AutocompleteKind::PageRef);
        assert_eq!(t.1, "Av");
    }

    #[test]
    fn detect_trigger_finds_tag_at_word_start() {
        let chars: Vec<char> = "urgent #ta".chars().collect();
        let t = detect_trigger(&chars, chars.len()).unwrap();
        assert_eq!(t.0, AutocompleteKind::Tag);
        assert_eq!(t.1, "ta");
    }

    #[test]
    fn detect_trigger_skips_hash_inside_word() {
        // `foo#bar` is not a tag.
        let chars: Vec<char> = "foo#ba".chars().collect();
        assert!(detect_trigger(&chars, chars.len()).is_none());
    }

    #[test]
    fn detect_trigger_closes_on_space() {
        let chars: Vec<char> = "see [[Av today".chars().collect();
        assert!(detect_trigger(&chars, chars.len()).is_none());
    }

    #[test]
    fn detect_trigger_closes_when_brackets_closed() {
        // After `]]`, the trigger is gone.
        let chars: Vec<char> = "see [[Avelino]]a".chars().collect();
        assert!(detect_trigger(&chars, chars.len()).is_none());
    }

    #[test]
    fn cycle_todo_state_cycles_none_todo_doing_done_none() {
        assert_eq!(cycle_todo_state("buy milk"), "TODO buy milk");
        assert_eq!(cycle_todo_state("TODO buy milk"), "DOING buy milk");
        assert_eq!(cycle_todo_state("DOING buy milk"), "DONE buy milk");
        assert_eq!(cycle_todo_state("DONE buy milk"), "buy milk");
        // Empty block: still cycles cleanly.
        assert_eq!(cycle_todo_state(""), "TODO ");
        // No false positives on TODOlist or DONErama (no space).
        assert_eq!(cycle_todo_state("TODOlist"), "TODO TODOlist");
        assert_eq!(cycle_todo_state("DONErama"), "TODO DONErama");
    }

    #[test]
    fn cycle_todo_inline_adds_prefix_and_shifts_cursor() {
        let mut buf = EditBuffer::from_text("buy milk");
        let original_cursor = buf.cursor; // 8 (end)
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "TODO buy milk");
        assert_eq!(buf.cursor, original_cursor + 5);
    }

    #[test]
    fn cycle_todo_inline_todo_to_doing_shifts_cursor_by_one() {
        // `DOING ` is one character wider than `TODO `, so the caret
        // has to follow the body right by one to stay on the same
        // letter. A fixed five-character assumption parks it inside
        // the marker instead.
        let mut buf = EditBuffer::from_text("TODO buy milk");
        buf.cursor = 7; // on the "y" of "buy"
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "DOING buy milk");
        assert_eq!(buf.cursor, 8);
        assert_eq!(buf.chars[buf.cursor], 'y');
    }

    #[test]
    fn cycle_todo_inline_doing_to_done_shifts_cursor_back() {
        let mut buf = EditBuffer::from_text("DOING buy milk");
        buf.cursor = 8; // on the "y" of "buy"
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "DONE buy milk");
        assert_eq!(buf.cursor, 7);
        assert_eq!(buf.chars[buf.cursor], 'y');
    }

    #[test]
    fn cycle_todo_inline_done_strips_prefix_and_shifts_cursor() {
        let mut buf = EditBuffer::from_text("DONE buy milk");
        buf.cursor = 8;
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "buy milk");
        assert_eq!(buf.cursor, 3); // 8 - 5
    }

    #[test]
    fn cycle_todo_inline_cursor_in_prefix_clamps_to_zero() {
        // Cursor inside the `DONE ` prefix — after strip, cursor lands
        // at 0 (instead of going negative).
        let mut buf = EditBuffer::from_text("DONE buy");
        buf.cursor = 2; // inside "DONE"
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "buy");
        assert_eq!(buf.cursor, 0);
    }

    #[test]
    fn cycle_todo_inline_keeps_a_caret_at_column_zero() {
        // The width change happens *after* a caret sitting at the very
        // start of the line, so the caret must not move: shifting it by
        // the total delta parked it one column inside the marker, and
        // the next keystroke landed inside `DOING`.
        let mut buf = EditBuffer::from_text("TODO buy milk");
        buf.cursor = 0;
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "DOING buy milk");
        assert_eq!(buf.cursor, 0);
    }

    #[test]
    fn cycle_todo_inline_keeps_a_caret_inside_the_marker_at_its_column() {
        // A caret inside the rewritten marker has no character to
        // follow; it keeps its column instead of drifting with the
        // width delta.
        let mut buf = EditBuffer::from_text("TODO buy milk");
        buf.cursor = 3; // inside "TODO"
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "DOING buy milk");
        assert_eq!(buf.cursor, 3);
    }

    #[test]
    fn cycle_todo_inline_handles_short_strings() {
        // `TODOlist` (no space) — doesn't match the `TODO ` prefix, so
        // it gets a `TODO ` prepended.
        let mut buf = EditBuffer::from_text("TODOlist");
        let cur = buf.cursor;
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "TODO TODOlist");
        assert_eq!(buf.cursor, cur + 5);
    }

    #[test]
    fn cycle_todo_inline_follows_the_caret_through_a_legacy_quote_shape() {
        // `"> DOING foo"` is the shape external markdown authors — the
        // quote before the marker. `cycle_todo` normalises it to
        // `"DONE > foo"`, which moves the body left by one. Measuring
        // only the task prefix read the input as unmarked (the quote
        // is first), added five instead of subtracting one, and parked
        // the caret at the end of the line.
        let mut buf = EditBuffer::from_text("> DOING foo");
        buf.cursor = 8; // on the "f" of "foo"
        cycle_todo_inline(&mut buf);
        assert_eq!(buf.as_string(), "DONE > foo");
        assert_eq!(buf.chars[buf.cursor], 'f');
    }

    #[test]
    fn cycle_todo_inline_four_calls_returns_to_original() {
        let mut buf = EditBuffer::from_text("milk");
        let original = buf.as_string();
        let original_cursor = buf.cursor;
        cycle_todo_inline(&mut buf); // TODO milk
        cycle_todo_inline(&mut buf); // DOING milk
        cycle_todo_inline(&mut buf); // DONE milk
        cycle_todo_inline(&mut buf); // milk
        assert_eq!(buf.as_string(), original);
        // The caret comes home too — the widths have to cancel out
        // across the full cycle, not just per step.
        assert_eq!(buf.cursor, original_cursor);
    }
}
