//! The properties overlay's painting.
//!
//! Its own file rather than another function in `overlays.rs`: that
//! module is already at the size the file-size guard nags about, and
//! this popup has a second surface (the in-flight edit plus its key
//! completion strip) that would push it over.

use crate::state::{App, PropertiesState, PropertyField, PropertyScope};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// How many completions fit on the hint strip under the key field.
const KEY_HINT_LIMIT: usize = 6;

pub(crate) fn render_properties(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    state: &PropertiesState,
) {
    let area = centered_rect(full, 68, 56);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, (key, value)) in state.rows.iter().enumerate() {
        let selected = i == state.selected && state.editing.is_none();
        let editing_here = state.editing.as_ref().is_some_and(|e| {
            e.original_key
                .as_deref()
                .is_some_and(|k| k.eq_ignore_ascii_case(key))
        });
        let prefix = if selected || editing_here {
            "❯ "
        } else {
            "  "
        };
        let shown = match state.editing.as_ref() {
            Some(edit) if editing_here => format!("{}▏", edit.value),
            _ => value.clone(),
        };
        let style = if selected || editing_here {
            app.theme.help_title
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{prefix}{key}"), style),
            Span::styled(":: ", app.theme.dim),
            Span::styled(shown, style),
        ]));
    }

    if state.rows.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  no {} properties yet — `o` adds one", state.scope.label()),
            app.theme.dim,
        )));
    }

    // A brand new property has no row to render inside the list yet.
    if let Some(edit) = state.editing.as_ref() {
        if edit.original_key.is_none() {
            lines.push(Line::from(Span::raw("")));
            let key_span = match edit.field {
                PropertyField::Key => format!("❯ {}▏", edit.key),
                PropertyField::Value => format!("  {}", edit.key),
            };
            let value_span = match edit.field {
                PropertyField::Key => String::new(),
                PropertyField::Value => format!("{}▏", edit.value),
            };
            lines.push(Line::from(vec![
                Span::styled(key_span, app.theme.help_title),
                Span::styled(":: ", app.theme.dim),
                Span::styled(value_span, app.theme.help_title),
            ]));
            if matches!(edit.field, PropertyField::Key) {
                lines.push(Line::from(Span::styled(
                    format!("    Tab: {}", key_hint(&edit.key_matches)),
                    app.theme.dim,
                )));
            }
        }
    }

    let scroll = state.selected.saturating_sub(8) as u16;
    let title = format!(
        " Properties · {} — `p` switches to {} ",
        state.scope.label(),
        match state.scope {
            PropertyScope::Block => "page",
            PropertyScope::Page => "block",
        }
    );
    let list = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(title, app.theme.help_title))
                .style(app.theme.popup_style()),
        )
        .scroll((scroll, 0));
    f.render_widget(list, outer[0]);

    let help = match (&state.message, state.editing.as_ref()) {
        (Some(msg), _) => msg.clone(),
        (None, Some(edit)) if matches!(edit.field, PropertyField::Key) => {
            "Tab complete key · Enter → value · Esc cancel".to_string()
        }
        (None, Some(_)) => "`[[` picks a page · `#` a tag · Enter save · Esc cancel".to_string(),
        (None, None) => {
            "j/k move · Enter edit · o add · dd delete · p page/block · q close".to_string()
        }
    };
    let footer =
        Paragraph::new(Line::from(Span::raw(format!(" {help}")))).style(app.theme.popup_style());
    f.render_widget(footer, outer[1]);
}

/// The completion strip under the key field.
///
/// Shows what `Tab` will reach for, in the catalogue's frequency
/// order, so the user can see whether the key they want is one Tab or
/// four away instead of pressing blind.
fn key_hint(matches: &[String]) -> String {
    if matches.is_empty() {
        return "(no matching key — type a new one)".to_string();
    }
    let shown: Vec<&str> = matches
        .iter()
        .take(KEY_HINT_LIMIT)
        .map(String::as_str)
        .collect();
    let more = matches.len().saturating_sub(shown.len());
    let head = shown.join("  ");
    if more > 0 {
        format!("{head}  +{more}")
    } else {
        head
    }
}

fn centered_rect(full: Rect, w_pct: u16, h_pct: u16) -> Rect {
    let w = (full.width as u32 * w_pct as u32 / 100) as u16;
    let h = (full.height as u32 * h_pct as u32 / 100) as u16;
    Rect {
        x: full.x + (full.width.saturating_sub(w)) / 2,
        y: full.y + (full.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::{key_hint, KEY_HINT_LIMIT};

    #[test]
    fn an_empty_catalogue_says_so_instead_of_rendering_a_blank_strip() {
        assert!(key_hint(&[]).contains("no matching key"));
    }

    #[test]
    fn the_strip_keeps_the_catalogue_order() {
        let keys = vec!["related".to_string(), "icon".to_string()];
        assert_eq!(key_hint(&keys), "related  icon");
    }

    #[test]
    fn a_long_catalogue_is_truncated_with_a_count() {
        let keys: Vec<String> = (0..KEY_HINT_LIMIT + 3).map(|i| format!("k{i}")).collect();
        let hint = key_hint(&keys);
        assert!(hint.ends_with("+3"), "got {hint}");
        assert!(!hint.contains("k7"), "past the limit must not render");
    }
}
