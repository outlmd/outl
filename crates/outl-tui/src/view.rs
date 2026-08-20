//! ratatui rendering: turn the current `App` into a frame.
//!
//! This file is the orchestrator — it composes the panels but delegates
//! the actual painting to siblings:
//!
//! - `overlays` — every modal popup (quick switcher, search, slash,
//!   command bar, error, help, inline autocomplete).
//! - `properties` — the `g p` property editor popup.
//! - `outline` — the current page's block tree.
//! - `backlinks` — the inline backlinks section below the outline.
//! - `inline` — span-level markdown (used by `outline` and
//!   `backlinks`).
//!
//! Only `render_app` is callable from outside the module — the rest is
//! `pub(crate)` for cross-file reuse inside `view/`.

mod backlinks;
mod chrome;
mod inline;
mod outline;
pub(crate) mod overlays;
mod properties;
mod sidebar;
mod toasts;
mod warnings_banner;
mod wrap;

use crate::state::{App, Focus, Overlay, View};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{
    Block, Borders, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};

// Inline-rendering helpers used by tests in `app.rs` — keep them
// reachable via `crate::view::…` for backwards compat with the
// pre-split layout.
#[cfg(test)]
pub(crate) use inline::{highlight_inline, render_markdown_inline};

// Re-export the help-tabs constant so `input` can compute the tab
// count without needing the whole `overlays` module visible.
pub(crate) use overlays::HELP_TABS;

pub(crate) fn render_app(f: &mut ratatui::Frame<'_>, app: &mut App) {
    let area = f.area();
    // Reset the whole frame buffer before composing the new view.
    //
    // Without this, navigating from a long page (many outline lines +
    // multi-block backlinks) to a shorter one leaves behind the cells
    // ratatui's diff renderer thinks are still up to date — the new
    // paint covers fewer rows than the previous one. The leftover
    // cells look like random truncated text after a journal switch,
    // an empty page, or a TUI resize while a popup was up.
    //
    // `Clear` writes the default background style across `area`, so
    // every subsequent `render_widget` paints onto a known-empty
    // buffer. The cost is one extra full-area write per frame, which
    // is well inside the 60fps budget the TUI targets.
    f.render_widget(Clear, area);
    // Paint the theme's canvas (background + base text color) across
    // the whole frame. On the palette-derived RGB presets this is what
    // makes a light theme readable on a dark terminal (and vice
    // versa): spans that don't set an explicit fg/bg inherit these
    // cell attributes instead of the terminal's defaults. The two
    // ANSI presets (`default-dark`, `light`) carry `Color::Reset`
    // here, so for them this paint is a no-op and the terminal's own
    // palette keeps showing through — that behavior is intentional
    // and documented in docs/theming.md.
    f.render_widget(
        Block::default().style(
            ratatui::style::Style::default()
                .fg(app.theme.foreground)
                .bg(app.theme.background),
        ),
        area,
    );
    render_main(f, area, app);

    // Overlays draw on top of everything else.
    match &app.overlay {
        Some(Overlay::QuickSwitch(qs)) => overlays::render_quick_switch(f, area, app, qs),
        Some(Overlay::Search(s)) => overlays::render_search_overlay(f, area, app, s),
        Some(Overlay::Command(c)) => overlays::render_command_bar(f, area, app, c),
        Some(Overlay::Error(e)) => overlays::render_error_overlay(f, area, app, e),
        Some(Overlay::Slash(s)) => overlays::render_slash_overlay(f, area, app, s),
        Some(Overlay::TemplatePicker(tp)) => overlays::render_template_picker(f, area, app, tp),
        Some(Overlay::Reminders(r)) => overlays::render_reminders(f, area, app, r),
        Some(Overlay::PluginSettings(ps)) => overlays::render_plugin_settings(f, area, app, ps),
        Some(Overlay::Properties(p)) => properties::render_properties(f, area, app, p),
        None => {}
    }

    if let Some(ac) = &app.autocomplete {
        overlays::render_autocomplete(f, area, app, ac);
    }

    if app.show_help {
        overlays::render_help_popup(f, area, app);
    }

    // Toasts render last so they sit visually on top of every other
    // surface (even open overlays). They're harmless decoration —
    // never block input, never own focus.
    toasts::render_toasts(f, area, app);
}

fn render_main(f: &mut ratatui::Frame<'_>, area: Rect, app: &mut App) {
    let banner_h = warnings_banner::banner_height(app);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(banner_h),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(area);

    chrome::render_header(f, outer[0], app);

    // Warning banner — collapses to zero height when `parse_warnings`
    // is empty (clean files), so this slot is invisible most of the
    // time.
    warnings_banner::render_banner(f, outer[1], app);

    let middle = outer[2];

    // Optional left sidebar: splits the middle row horizontally when
    // toggled on (`\` in Normal mode). Default off keeps the classic
    // single-pane layout intact for users who never opt in.
    let body_area = if app.show_sidebar {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(sidebar::SIDEBAR_WIDTH),
                Constraint::Min(20),
            ])
            .split(middle);
        sidebar::render_sidebar(f, cols[0], app);
        cols[1]
    } else {
        middle
    };

    // Build the single scrollable region: outline lines, then the
    // inline backlinks section. The `─` separator and headers live
    // inside `backlinks::render_backlinks_inline`.
    let inner_width = body_area.width.saturating_sub(2);
    let (mut all_lines, sel_outline, block_starts) =
        outline::render_outline(&app.page, app, inner_width);
    let outline_len = all_lines.len();
    let (bl_lines, sel_bl) = backlinks::render_backlinks_inline(app, inner_width);
    let bl_offset = all_lines.len();
    all_lines.extend(bl_lines);

    let title = match &app.view {
        View::Journal(_) | View::Page(_) => "Outline",
    };

    // Pick the selected line based on where the focus currently is.
    let selected_line = match &app.focus {
        Focus::Outline => sel_outline,
        Focus::Backlink { .. } => sel_bl.map(|n| n + bl_offset),
    };
    let _ = outline_len; // kept for future scroll heuristics

    // Viewport math: body_area is the outline area (borders included).
    // Subtract 2 for top + bottom border lines to get the actually
    // drawable region.
    let viewport_h = body_area.height.saturating_sub(2);
    app.viewport_height = viewport_h;

    // Auto-scroll: keep the selection visible. If it scrolled off the
    // top, drop the offset down to it; if it scrolled off the bottom,
    // push the offset up so the bullet sits on the last row.
    if let Some(sel) = selected_line {
        let sel = sel as u16;
        if sel < app.scroll_y {
            app.scroll_y = sel;
        } else if viewport_h > 0 && sel >= app.scroll_y + viewport_h {
            app.scroll_y = sel + 1 - viewport_h;
        }
    }
    // Clamp: never scroll past `last_line - viewport_h + 1`.
    let total = all_lines.len() as u16;
    if total > viewport_h {
        let max_scroll = total - viewport_h;
        if app.scroll_y > max_scroll {
            app.scroll_y = max_scroll;
        }
    } else {
        app.scroll_y = 0;
    }

    let body = Paragraph::new(all_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(title.to_string()),
        )
        .scroll((app.scroll_y, 0));
    // NB: no ratatui `.wrap(...)`. It expands one logical line into N
    // visual lines *after* layout, whose count depends on width — that
    // would invalidate the `selected_line` index the scroll math above
    // relies on. Instead the outline pre-wraps to `inner_width` itself
    // (`view::wrap::push_wrapped`), so `all_lines` already holds the
    // final visual rows and the index stays honest. See issue #99.
    f.render_widget(body, body_area);

    // Persist what the mouse handler needs to map a click back to a
    // block next frame: the outline's on-screen rect (borders included),
    // the block-start map, and how many lines the outline itself took
    // (so a click in the backlinks tail below it is ignored).
    app.outline_area = Some(body_area);
    app.block_starts = block_starts;
    app.outline_line_count = outline_len;

    // Vertical scrollbar on the right border. Only meaningful when the
    // body actually overflows the viewport; the widget renders nothing
    // when `content_length <= viewport_height`, but we skip the call
    // entirely to keep the body title pristine on short pages.
    if total > viewport_h && viewport_h > 0 {
        let mut sb_state = ScrollbarState::new(total as usize)
            .viewport_content_length(viewport_h as usize)
            .position(app.scroll_y as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .track_symbol(Some("│"))
            .thumb_symbol("█")
            .style(app.theme.dim)
            .thumb_style(app.theme.heading);
        f.render_stateful_widget(scrollbar, body_area, &mut sb_state);
    }

    chrome::render_footer(f, outer[3], app);
}
