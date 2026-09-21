//! Row chrome — everything a block draws around its own text.
//!
//! The fold slot, the `auto-run::` marker, the pad every non-bullet row
//! spends to reach the text column, and the `key:: value` rows. One
//! module because they are one measurement: change the fold slot and
//! the property row has to move with it, which is exactly the drift
//! [issue 319](https://github.com/outlmd/outl/issues/319) was.

use crate::state::App;
use crate::view::wrap::push_wrapped;
use ratatui::text::{Line, Span};

/// Marker drawn before the bullet on a block carrying `auto-run::`,
/// so the user can see at a glance which cells re-run themselves on
/// page open.
pub(crate) const AUTO_RUN_GLYPH: &str = "⚡";

/// Blank cells standing in for [`AUTO_RUN_GLYPH`] on the rows that
/// don't draw it. Two, because `⚡` is two wide; the single space this
/// used to be left every continuation row of an `auto-run::` block a
/// column short. `the_auto_run_pad_matches_the_glyph` keeps the pair
/// honest.
const AUTO_RUN_PAD: &str = "  ";

/// Pad the cells a bullet row spends between the indent guides and
/// the block's text: the two-cell fold slot, the optional `⚡`, and
/// the `- ` bullet.
///
/// Every other row a block emits pads by exactly this, so all of them
/// start in the block's own text column (#319).
pub(crate) fn push_body_indent(spans: &mut Vec<Span<'static>>, has_auto_run: bool) {
    spans.push(Span::raw("    "));
    if has_auto_run {
        spans.push(Span::raw(AUTO_RUN_PAD));
    }
}

/// Emit one `key:: value` row under the block that carries it.
///
/// Single owner for the property row, because there are two callers
/// (the outline and the backlinks mini-outline) and they had already
/// drifted: backlinks never drew the [`property_glyph`], so the same
/// `remind::` read differently depending on which pane you saw it in.
///
/// The row wraps like any block row — a long `template::` used to run
/// off the right edge and get clipped with nothing to say it had been.
/// The glyph rides in the `head`, so a wrapped value re-indents under
/// the key rather than under the glyph.
#[allow(clippy::too_many_arguments)]
pub(crate) fn push_property_row(
    indent: u32,
    key: &str,
    value: &str,
    has_auto_run: bool,
    app: &App,
    out: &mut Vec<Line<'static>>,
    text_width: u16,
) {
    let mut guides: Vec<Span<'static>> = Vec::new();
    for _ in 0..indent {
        guides.push(Span::styled("│ ", app.theme.dim));
    }
    let mut head: Vec<Span<'static>> = Vec::new();
    push_body_indent(&mut head, has_auto_run);
    if let Some(glyph) = property_glyph(key) {
        head.push(Span::raw(format!("{glyph} ")));
    }
    let content = vec![
        Span::styled(format!("{key}:: "), app.theme.property_key),
        Span::styled(value.to_string(), app.theme.property_value),
    ];
    push_wrapped(guides, head, content, text_width, out);
}

/// Fold indicator drawn before the bullet on the bullet row.
///
/// `None` keeps a two-cell gap so leaf rows align with their parent
/// at the same indent — without it, a leaf's `-` would slide left
/// the moment a sibling grew children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FoldMarker {
    /// Block has no children — no marker, gap only.
    None,
    /// Block has children and they're visible. `▼ ` prefix.
    Expanded,
    /// Block has children but they're folded away. `▶ ` prefix.
    Collapsed,
}

/// Leading glyph for a property key outl gives a meaning to.
///
/// Mirrors `KNOWN_PROPERTIES` in
/// `@outl/shared/markdown/properties` — a const can't cross the
/// Rust/TS boundary any more than a DTO field can, so the two tables
/// are edited together. A user's own key (`priority::`) gets no glyph;
/// interpreting it isn't ours to do.
pub(crate) fn property_glyph(key: &str) -> Option<&'static str> {
    match key.to_ascii_lowercase().as_str() {
        outl_md::remind::REMIND_KEY => Some("⏰"),
        "auto-run" => Some("▶"),
        "template" => Some("📋"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    /// The blank stand-in has to measure what the glyph measures, or
    /// every row that doesn't draw `⚡` sits a column off the one that
    /// does. That was the bug, for as long as the pad was a literal.
    #[test]
    fn the_auto_run_pad_matches_the_glyph() {
        assert_eq!(AUTO_RUN_PAD.width(), AUTO_RUN_GLYPH.width());
    }
}
