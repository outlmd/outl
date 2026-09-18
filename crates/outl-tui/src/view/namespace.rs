//! Nested-pages section — every page under the current one's
//! namespace (`os` → `os/linux`, `os/linux/debian`), issue #275.
//!
//! Rendered below the inline backlinks, with its own `─` rule, and
//! **not** gated on `show_backlinks`: this answers "what lives under
//! here" from page titles, the backlinks section answers "what points
//! here" from referencing blocks. One toggle for both would hide one
//! when the user muted the other.
//!
//! The hierarchy itself is decided in `outl_actions::namespace` — the
//! rows arrive with `depth` and `label` already computed, so nothing
//! here splits a title on `/`. That is the point: the desktop and
//! mobile clients render the same rows from the same function, and a
//! per-client idea of what `OS/Linux` means cannot exist.
//!
//! **Not cursor-navigable.** `j`/`k` stop at the backlinks section;
//! opening a nested page goes through the picker. The gap is recorded
//! as `Capability::NestedPages` → `Support::Partial` in
//! `outl_shortcuts::capability_support`, and the section's own footer
//! says so, so the user is never left guessing whether the keys are
//! broken (root `CLAUDE.md` invariant 12).

use crate::state::App;
use ratatui::text::{Line, Span};

/// Render the nested-pages section.
///
/// `inner_width` is the drawable width of the outline panel, so the
/// separator rule spans the full visible width. Empty result when the
/// current page has nothing nested under it — an empty section does
/// not earn its rows.
pub(crate) fn render_nested_pages(app: &App, inner_width: u16) -> Vec<Line<'static>> {
    let children = app.namespace_children_for_current();
    if children.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<Line<'static>> = Vec::new();
    out.push(Line::from(""));
    let rule = "─".repeat(inner_width.max(1) as usize);
    out.push(Line::from(Span::styled(rule, app.theme.border)));
    out.push(Line::from(vec![
        Span::styled(
            format!(" Nested pages · {}  ", children.len()),
            app.theme.heading,
        ),
        // The nudge, shown where the limitation is, not in a log.
        Span::styled("(open with the picker: ^P)", app.theme.dim),
    ]));
    out.push(Line::from(""));

    for child in &children {
        // Two spaces per level below the root, so the tree reads as a
        // tree. `depth` is 1-based (a direct child is 1), matching what
        // every other client indents by.
        let indent = " ".repeat(1 + (child.depth - 1) * 2);
        let icon = child.page.icon.as_deref().unwrap_or(app.icons.file);
        out.push(Line::from(vec![
            Span::styled(format!("{indent}{icon}  "), app.theme.dim),
            Span::styled(child.label.clone(), app.theme.foreground),
        ]));
    }

    out
}
