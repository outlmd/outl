//! Bottom-right toast stack — visual confirmation for transient
//! actions (saved, reloaded, undid, …). Stacks multiple at once;
//! each one expires on its own timer.
//!
//! The stack draws *over* everything else (after main, overlays,
//! help) so a save toast still pops up even when a modal is open.

use crate::icons::IconSet;
use crate::state::{App, ToastKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Per-toast height (border + 1 content row).
const TOAST_HEIGHT: u16 = 3;
/// Gap below the footer where toasts anchor.
const BOTTOM_GAP: u16 = 4;

pub(crate) fn render_toasts(f: &mut ratatui::Frame<'_>, full: Rect, app: &App) {
    if app.toasts.is_empty() {
        return;
    }

    // Width: cap at 50 chars, but never more than half the screen.
    // Real terminals get a comfortable card; narrow ones still see
    // something readable.
    let width = 50u16.min(full.width.saturating_sub(4));
    if width < 12 {
        return; // not worth drawing in a sliver
    }

    // Anchor: bottom-right, walking up the stack.
    let right_x = full.x + full.width.saturating_sub(width + 2);
    let mut y_cursor = full.y + full.height.saturating_sub(BOTTOM_GAP + TOAST_HEIGHT);

    // Walk newest-first so the most recent toast sits on top.
    for toast in app.toasts.iter().rev() {
        if y_cursor < full.y + 2 {
            break;
        }
        let area = Rect {
            x: right_x,
            y: y_cursor,
            width,
            height: TOAST_HEIGHT,
        };
        f.render_widget(Clear, area);

        let (icon, accent) = icon_and_color(toast.kind, &app.icons);
        let body = Line::from(vec![
            Span::styled(
                format!(" {icon} "),
                Style::default().fg(accent).add_modifier(Modifier::BOLD),
            ),
            // Borrow rather than clone — render runs every poll
            // tick; a per-frame allocation per toast adds up.
            Span::raw(toast.message.as_str()),
        ]);
        let card = Paragraph::new(body)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(accent))
                    .title(Span::styled(
                        format!(" {} ", label_for(toast.kind)),
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    )),
            )
            .style(app.theme.popup_style());
        f.render_widget(card, area);

        // Step up for the next toast. Saturating sub so we never
        // wrap into the chrome.
        y_cursor = y_cursor.saturating_sub(TOAST_HEIGHT);
    }
}

fn icon_and_color(kind: ToastKind, icons: &IconSet) -> (&'static str, Color) {
    match kind {
        ToastKind::Success => (icons.success, Color::LightGreen),
        ToastKind::Info => (icons.info, Color::LightCyan),
        ToastKind::Warning => (icons.warning, Color::LightYellow),
        ToastKind::Error => (icons.error, Color::LightRed),
    }
}

fn label_for(kind: ToastKind) -> &'static str {
    match kind {
        ToastKind::Success => "ok",
        ToastKind::Info => "info",
        ToastKind::Warning => "warn",
        ToastKind::Error => "error",
    }
}
