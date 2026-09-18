//! Runtime-selected icons for TUI chrome.
//!
//! Emoji is the default because it works with ordinary terminal fonts.
//! Nerd Font glyphs are opt-in through `[tui] icons = "nerd-font"`.
//!
//! Scope: every TUI-owned icon — status/footer chips, fold markers,
//! property/command/palette glyphs. What stays Unicode in both sets by
//! design: task checkboxes (`☐`/`◐`/`☑`, they mirror the document
//! state), calendar day dots, scrollbar symbols, and plain geometric
//! separators (`↪`, `≡`, `✕`, `·`).

use crate::theme::Theme;
use crate::view::outline::FoldMarker;
use outl_config::TuiIconStyle;
use ratatui::text::Span;

/// Icons used by the TUI's own chrome and placeholders.
///
/// `calendar`/`week` and `clock`/`stamp` are split so Emoji mode can
/// preserve the upstream glyphs (`📅`/`📆`, `🕐`/`🕒`) for the
/// `/date` vs `/week` and `/time` vs `/stamp` commands respectively,
/// even though Nerd Font collapses each pair to one codepoint.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IconSet {
    pub(crate) calendar: &'static str,
    pub(crate) week: &'static str,
    pub(crate) file: &'static str,
    pub(crate) image: &'static str,
    pub(crate) clock: &'static str,
    pub(crate) stamp: &'static str,
    pub(crate) star: &'static str,
    pub(crate) history: &'static str,
    pub(crate) bolt: &'static str,
    pub(crate) search: &'static str,
    pub(crate) cog: &'static str,
    pub(crate) paint_brush: &'static str,
    pub(crate) warning: &'static str,
    /// Toast accents — one per `ToastKind` arm, so the four states do
    /// not drift between Emoji and Nerd Font.
    pub(crate) success: &'static str,
    pub(crate) info: &'static str,
    pub(crate) error: &'static str,
    pub(crate) save: &'static str,
    pub(crate) clipboard: &'static str,
    /// Snoozed-reminder chip in the reminders overlay.
    pub(crate) snooze: &'static str,
    pub(crate) hashtag: &'static str,
    pub(crate) bell: &'static str,
    pub(crate) play: &'static str,
    /// TODO-progress header chip (`nf-fa-check-square-o`).
    pub(crate) todo_chip: &'static str,
    /// Insert-mode footer chip (`nf-fa-circle`).
    pub(crate) editing: &'static str,
    /// "saved Ns ago" freshness chip (`nf-fa-refresh`).
    pub(crate) freshness: &'static str,
    /// Workspace-name footer chip (`nf-fa-circle-thin`).
    pub(crate) workspace: &'static str,
    /// Backlink-count footer chip (`nf-fa-link`).
    pub(crate) backlinks: &'static str,
    /// Fold marker before an expanded parent; carries its own padding
    /// space so columns stay flush (`nf-fa-chevron-down`).
    pub(crate) fold_open: &'static str,
    /// Fold marker before a collapsed parent (`nf-fa-chevron-right`).
    pub(crate) fold_closed: &'static str,
    /// Help-overlay legend line explaining the two fold markers.
    pub(crate) fold_legend: &'static str,
}

impl IconSet {
    pub(crate) fn new(style: TuiIconStyle) -> Self {
        match style {
            TuiIconStyle::Emoji => Self::emoji(),
            TuiIconStyle::NerdFont => Self::nerd_font(),
        }
    }

    pub(crate) fn property_glyph(&self, key: &str) -> Option<&'static str> {
        match key.to_ascii_lowercase().as_str() {
            outl_md::remind::REMIND_KEY => Some(self.bell),
            "auto-run" => Some(self.play),
            "template" => Some(self.clipboard),
            _ => None,
        }
    }

    pub(crate) fn category_glyph(&self, category: &str) -> &'static str {
        match category {
            "Actions" => self.bolt,
            "Navigation" => "↪",
            "Search" => self.search,
            "Settings" => self.cog,
            "Dates & time" => self.calendar,
            _ => "•",
        }
    }

    /// The fold marker as a styled span, glyph and colour together.
    /// The `None` arm keeps the two-cell gap so leaf bullets stay
    /// aligned with their parent's.
    pub(crate) fn fold_span(&self, marker: FoldMarker, theme: &Theme) -> Span<'static> {
        match marker {
            FoldMarker::None => Span::raw("  "),
            FoldMarker::Expanded => Span::styled(self.fold_open, theme.dim),
            FoldMarker::Collapsed => Span::styled(self.fold_closed, theme.hint),
        }
    }

    pub(crate) fn command_glyph(&self, name: &str) -> &'static str {
        match name {
            "run" => self.play,
            "prop" => "≡",
            "search" | "find" => self.search,
            "theme" => self.paint_brush,
            "open" | "switch" => "↪",
            "quit" | "q" => "✕",
            n if n.starts_with("date") || n == "dt" || n == "dy" || n == "dtm" => self.calendar,
            n if n.starts_with("time") => self.clock,
            n if n.starts_with("iso") => self.hashtag,
            n if n.starts_with("week") => self.week,
            "stamp" => self.stamp,
            _ => "·",
        }
    }

    fn emoji() -> Self {
        Self {
            calendar: "📅",
            week: "📆",
            file: "📄",
            image: "🖼",
            clock: "🕐",
            stamp: "🕒",
            star: "⭐",
            history: "🕘",
            bolt: "⚡",
            search: "🔎",
            cog: "⚙",
            paint_brush: "🎨",
            warning: "⚠",
            success: "✓",
            info: "ℹ",
            error: "✕",
            save: "💾",
            clipboard: "📋",
            snooze: "💤",
            hashtag: "🔢",
            bell: "⏰",
            play: "▶",
            todo_chip: "☑",
            editing: "●",
            freshness: "⟳",
            workspace: "◌",
            backlinks: "⇇",
            fold_open: "▼ ",
            fold_closed: "▶ ",
            fold_legend: "              (▼ expanded · ▶ collapsed · synced via op log)",
        }
    }

    fn nerd_font() -> Self {
        Self {
            calendar: "\u{f073}",
            week: "\u{f073}",
            file: "\u{f016}",
            image: "\u{f03e}",
            clock: "\u{f017}",
            stamp: "\u{f017}",
            star: "\u{f005}",
            history: "\u{f1da}",
            bolt: "\u{f0e7}",
            search: "\u{f002}",
            cog: "\u{f013}",
            paint_brush: "\u{f07c0}",
            warning: "\u{f071}",
            success: "\u{f00c}",
            info: "\u{f05a}",
            error: "\u{f00d}",
            save: "\u{f0c7}",
            clipboard: "\u{f0ea}",
            snooze: "\u{f1f6}",
            hashtag: "\u{f292}",
            bell: "\u{f0f3}",
            play: "\u{f04b}",
            todo_chip: "\u{f046}",
            editing: "\u{f111}",
            freshness: "\u{f021}",
            workspace: "\u{f1db}",
            backlinks: "\u{f0c1}",
            fold_open: "\u{f078} ",
            fold_closed: "\u{f054} ",
            fold_legend:
                "              (\u{f078} expanded · \u{f054} collapsed · synced via op log)",
        }
    }
}

impl Default for IconSet {
    fn default() -> Self {
        Self::new(TuiIconStyle::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_is_the_default_and_contains_no_private_use_glyphs() {
        let icons = IconSet::default();
        assert_eq!(icons.calendar, "📅");
        assert!(icons
            .file
            .chars()
            .all(|ch| !(0xE000..=0xF8FF).contains(&(ch as u32))));
    }

    #[test]
    fn nerd_font_is_explicit() {
        let icons = IconSet::new(TuiIconStyle::NerdFont);
        assert_eq!(icons.calendar, "\u{f073}");
        assert!(icons
            .calendar
            .chars()
            .any(|ch| (0xE000..=0xF8FF).contains(&(ch as u32))));
    }

    #[test]
    fn nerd_font_uses_only_pua_glyphs() {
        // Nerd Font mode renders nothing the user's font cannot
        // draw as a single-colour cell. Every codepoint that ships
        // in this set is in some Unicode PUA plane (BMP PUA,
        // PUA-A in plane 15, or PUA-B in plane 16), so a terminal
        // without a Nerd Font shows the well-known "missing box"
        // glyph rather than a colour emoji.
        fn in_some_pua(ch: char) -> bool {
            let cp = ch as u32;
            (0xE000..=0xF8FF).contains(&cp)
                || (0xF0000..=0xFFFFD).contains(&cp)
                || (0x100000..=0x10FFFD).contains(&cp)
        }
        let nerd = IconSet::new(TuiIconStyle::NerdFont);
        for glyph in [
            nerd.calendar,
            nerd.week,
            nerd.file,
            nerd.image,
            nerd.clock,
            nerd.stamp,
            nerd.star,
            nerd.history,
            nerd.bolt,
            nerd.search,
            nerd.cog,
            nerd.paint_brush,
            nerd.warning,
            nerd.success,
            nerd.info,
            nerd.error,
            nerd.save,
            nerd.clipboard,
            nerd.snooze,
            nerd.hashtag,
            nerd.bell,
            nerd.play,
            nerd.todo_chip,
            nerd.editing,
            nerd.freshness,
            nerd.workspace,
            nerd.backlinks,
            nerd.fold_open,
            nerd.fold_closed,
        ] {
            assert!(
                glyph.chars().all(|ch| ch == ' ' || in_some_pua(ch)),
                "nerd glyph must be PUA-only (or a padding space): {glyph:?}"
            );
        }
    }

    #[test]
    fn play_routes_through_the_icon_set() {
        let emoji = IconSet::new(TuiIconStyle::Emoji);
        assert_eq!(emoji.property_glyph("auto-run"), Some("▶"));
        assert_eq!(emoji.command_glyph("run"), "▶");

        let nerd = IconSet::new(TuiIconStyle::NerdFont);
        assert_eq!(nerd.property_glyph("auto-run"), Some("\u{f04b}"));
        assert_eq!(nerd.command_glyph("run"), "\u{f04b}");
    }

    #[test]
    fn emoji_preserves_the_pre_iconset_glyphs() {
        let emoji = IconSet::new(TuiIconStyle::Emoji);

        // Every field literal that used to be a hardcoded glyph in
        // `view/outline.rs` / `view/overlays.rs` / `view/sidebar.rs` /
        // `view/chrome.rs` / `view/inline.rs` / `view/toasts.rs`.
        // Reverting any of these is a silent visual regression.
        assert_eq!(emoji.calendar, "📅");
        assert_eq!(emoji.week, "📆");
        assert_eq!(emoji.file, "📄");
        assert_eq!(emoji.image, "🖼");
        assert_eq!(emoji.clock, "🕐");
        assert_eq!(emoji.stamp, "🕒");
        assert_eq!(emoji.star, "⭐");
        assert_eq!(emoji.history, "🕘");
        assert_eq!(emoji.bolt, "⚡");
        assert_eq!(emoji.search, "🔎");
        assert_eq!(emoji.cog, "⚙");
        assert_eq!(emoji.paint_brush, "🎨");
        assert_eq!(emoji.warning, "⚠");
        assert_eq!(emoji.success, "✓");
        assert_eq!(emoji.info, "ℹ");
        assert_eq!(emoji.error, "✕");
        assert_eq!(emoji.save, "💾");
        assert_eq!(emoji.clipboard, "📋");
        assert_eq!(emoji.snooze, "💤");
        assert_eq!(emoji.hashtag, "🔢");
        assert_eq!(emoji.bell, "⏰");
        assert_eq!(emoji.play, "▶");
        assert_eq!(emoji.todo_chip, "☑");
        assert_eq!(emoji.editing, "●");
        assert_eq!(emoji.freshness, "⟳");
        assert_eq!(emoji.workspace, "◌");
        assert_eq!(emoji.backlinks, "⇇");
        assert_eq!(emoji.fold_open, "▼ ");
        assert_eq!(emoji.fold_closed, "▶ ");
        assert_eq!(
            emoji.fold_legend,
            "              (▼ expanded · ▶ collapsed · synced via op log)"
        );

        // Property glyphs (was `view::outline::property_glyph`).
        assert_eq!(
            emoji.property_glyph(outl_md::remind::REMIND_KEY),
            Some("⏰")
        );
        assert_eq!(emoji.property_glyph("auto-run"), Some("▶"));
        assert_eq!(emoji.property_glyph("template"), Some("📋"));
        assert_eq!(emoji.property_glyph("pinned"), None);

        // Category glyphs (was `view::overlays::category_icon`).
        assert_eq!(emoji.category_glyph("Actions"), "⚡");
        assert_eq!(emoji.category_glyph("Navigation"), "↪");
        assert_eq!(emoji.category_glyph("Search"), "🔎");
        assert_eq!(emoji.category_glyph("Settings"), "⚙");
        assert_eq!(emoji.category_glyph("Dates & time"), "📅");
        assert_eq!(emoji.category_glyph("Other"), "•");

        // Command glyphs (was `view::overlays::command_icon`).
        assert_eq!(emoji.command_glyph("run"), "▶");
        assert_eq!(emoji.command_glyph("prop"), "≡");
        assert_eq!(emoji.command_glyph("search"), "🔎");
        assert_eq!(emoji.command_glyph("find"), "🔎");
        assert_eq!(emoji.command_glyph("theme"), "🎨");
        assert_eq!(emoji.command_glyph("open"), "↪");
        assert_eq!(emoji.command_glyph("switch"), "↪");
        assert_eq!(emoji.command_glyph("quit"), "✕");
        assert_eq!(emoji.command_glyph("q"), "✕");
        assert_eq!(emoji.command_glyph("date-today"), "📅");
        assert_eq!(emoji.command_glyph("dt"), "📅");
        assert_eq!(emoji.command_glyph("dy"), "📅");
        assert_eq!(emoji.command_glyph("dtm"), "📅");
        assert_eq!(emoji.command_glyph("time-now"), "🕐");
        assert_eq!(emoji.command_glyph("iso-date-today"), "🔢");
        assert_eq!(emoji.command_glyph("week"), "📆");
        assert_eq!(emoji.command_glyph("week-tag"), "📆");
        assert_eq!(emoji.command_glyph("stamp"), "🕒");
        assert_eq!(emoji.command_glyph("anything-else"), "·");
    }

    #[test]
    fn chrome_and_fold_glyphs_route_through_the_set() {
        let nerd = IconSet::new(TuiIconStyle::NerdFont);
        for glyph in [
            nerd.todo_chip,
            nerd.editing,
            nerd.freshness,
            nerd.workspace,
            nerd.backlinks,
            nerd.fold_open,
            nerd.fold_closed,
        ] {
            assert!(
                glyph
                    .chars()
                    .all(|ch| ch == ' ' || (0xE000..=0xF8FF).contains(&(ch as u32))),
                "nerd chip must be PUA-only: {glyph:?}"
            );
        }
        assert!(nerd.fold_legend.contains('\u{f078}'));
        assert!(nerd.fold_legend.contains('\u{f054}'));
    }
}
