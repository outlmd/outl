//! Parser-warning banner shown above the outline when the loaded
//! `.md` had lines outside the outl dialect (e.g. a leading
//! `# heading`, free paragraph, imported markdown). The parser
//! preserves them verbatim as blocks — this banner tells the user
//! why they're seeing rows that don't look like normal bullets.

use crate::state::App;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

/// Banner height in lines (including its single-line border).
///
/// Zero when there's nothing to surface, so the layout collapses
/// silently on a clean page.
pub(crate) fn banner_height(app: &App) -> u16 {
    if app.parse_warnings.is_empty() {
        0
    } else {
        3
    }
}

/// Render the banner. Caller must size the area to
/// [`banner_height`] — anything else looks broken.
pub(crate) fn render_banner(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    if app.parse_warnings.is_empty() || area.height == 0 {
        return;
    }
    let first = &app.parse_warnings[0];
    // Trim the offending line so the banner never explodes
    // horizontally. 60 chars is enough for the user to identify
    // the row in their editor.
    let mut preview: String = first.raw.chars().take(60).collect();
    if first.raw.chars().count() > 60 {
        preview.push('…');
    }
    let extra = app.parse_warnings.len().saturating_sub(1);
    let summary = if extra == 0 {
        format!("line {}: {}", first.line, preview)
    } else {
        format!("line {}: {} (+{} more)", first.line, preview, extra)
    };
    let title = format!(
        " {} {} line(s) outside outl dialect — preserved as blocks ",
        app.icons.warning,
        app.parse_warnings.len()
    );
    let line = Line::from(vec![Span::styled(
        summary,
        Style::default().add_modifier(Modifier::DIM),
    )]);
    // Yellow is the universal "heads up, not fatal" colour across
    // every theme — picking it directly (instead of routing through
    // `Theme`) keeps the banner readable on dark, light, and the
    // colour-scheme presets without each theme having to declare a
    // `warning` slot. Themes can opt in later by gaining one.
    let warning = Style::default().fg(Color::Yellow);
    let widget = Paragraph::new(line).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(warning)
            .title(Span::styled(title, warning.add_modifier(Modifier::BOLD))),
    );
    f.render_widget(widget, area);
}
