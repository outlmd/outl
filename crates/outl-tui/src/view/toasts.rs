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
use unicode_width::UnicodeWidthStr;

/// Border rows a card always spends (top + bottom).
const CARD_BORDERS: u16 = 2;
/// Most body rows one toast may grow to.
///
/// A card used to be one row, so a long `reconcile failed: {e}` lost
/// its path at the border ([#435](https://github.com/outlmd/outl/issues/435)).
/// Four rows holds the errors this channel actually carries without
/// letting one toast take the screen.
const MAX_BODY_LINES: usize = 4;
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

    // Anchor: bottom-right, walking up the stack. Cards size to their
    // message now, so the walk tracks the bottom edge and subtracts
    // each card's own height rather than a constant.
    let right_x = full.x + full.width.saturating_sub(width + 2);
    let mut bottom = full.y + full.height.saturating_sub(BOTTOM_GAP);

    // Walk newest-first so the most recent toast sits on top.
    for toast in app.toasts.iter().rev() {
        let (icon, accent) = icon_and_color(toast.kind, &app.icons);
        let prefix = format!(" {icon} ");
        // The icon sits on the first row and continuations line up
        // under the text, not under the glyph, so a wrapped error
        // reads as one message.
        let gutter = prefix.width();
        let text_width = (width as usize)
            .saturating_sub(CARD_BORDERS as usize)
            .saturating_sub(gutter);
        let rows = wrap_message(&toast.message, text_width, MAX_BODY_LINES);
        if rows.is_empty() {
            continue;
        }
        let height = CARD_BORDERS + rows.len() as u16;
        if bottom < full.y + CARD_BORDERS + height {
            break;
        }

        let area = Rect {
            x: right_x,
            y: bottom - height,
            width,
            height,
        };
        f.render_widget(Clear, area);

        let accent_bold = Style::default().fg(accent).add_modifier(Modifier::BOLD);
        let mut body: Vec<Line> = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            if i == 0 {
                body.push(Line::from(vec![
                    Span::styled(prefix.clone(), accent_bold),
                    Span::raw(row.clone()),
                ]));
            } else {
                body.push(Line::from(vec![
                    Span::raw(" ".repeat(gutter)),
                    Span::raw(row.clone()),
                ]));
            }
        }

        let card = Paragraph::new(body)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(accent))
                    .title(Span::styled(
                        format!(" {} ", label_for(toast.kind)),
                        accent_bold,
                    )),
            )
            .style(app.theme.popup_style());
        f.render_widget(card, area);

        bottom = bottom.saturating_sub(height);
    }
}

/// Split `msg` into at most `max_lines` rows of `width` columns.
///
/// A toast used to be one unwrapped `Paragraph` row inside a 50-cell
/// card, so anything longer was cut at the border with nothing to say
/// it had been: `reconcile failed: {e}` lost the path or the errno,
/// which is the only part worth reading
/// ([#435](https://github.com/outlmd/outl/issues/435)).
///
/// Word-based, falling back to a hard break for a token longer than
/// the card (a path with no spaces, which is the common case here).
///
/// `overlays::truncate` is deliberately not reused for the final row.
/// It answers "is this string longer than `max`", and the question
/// here is "was anything dropped" — a row that happens to land exactly
/// on `width` still has to carry the ellipsis, or the user reads a
/// clipped error as a complete one.
///
/// `view::wrap::push_wrapped` is deliberately not reused: it wraps
/// styled spans against the outline's indent rails and protects the
/// cursor cell, none of which exists in a toast.
fn wrap_message(msg: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut rows: Vec<String> = Vec::new();
    let mut current = String::new();

    for word in msg.split_whitespace() {
        if word.chars().count() > width {
            // A token wider than the card: break it hard rather than
            // overflow the border. Error messages here are mostly
            // paths, which have no spaces to break on.
            if !current.is_empty() {
                rows.push(std::mem::take(&mut current));
            }
            let mut chars = word.chars().peekable();
            while chars.peek().is_some() {
                let chunk: String = chars.by_ref().take(width).collect();
                rows.push(chunk);
            }
            // Keep the trailing chunk open so the next word can share
            // its row.
            current = rows.pop().unwrap_or_default();
            continue;
        }
        let needed = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if needed > width {
            rows.push(std::mem::take(&mut current));
            current = word.to_string();
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }

    if rows.len() > max_lines {
        rows.truncate(max_lines);
        if let Some(last) = rows.last_mut() {
            let mut elided: String = last.chars().take(width.saturating_sub(1)).collect();
            elided.push('\u{2026}');
            *last = elided;
        }
    }
    rows
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

#[cfg(test)]
mod tests {
    use super::wrap_message;
    use crate::state::{App, ToastKind};
    use outl_core::{ActorId, Workspace};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use tempfile::TempDir;

    /// A long error, painted through the real render path.
    ///
    /// The pure `wrap_message` tests below pass whether or not
    /// `render_toasts` calls it, so this is what actually pins the fix:
    /// drop the call and the cause stops reaching the screen.
    fn painted_with_toast(message: &str) -> String {
        let dir = TempDir::new().unwrap();
        let actor = ActorId::new();
        let ws = Workspace::open_in_memory(actor).unwrap();
        let mut app = App::new_for_tests(
            dir.path().to_path_buf(),
            ws,
            actor,
            crate::theme::default_theme(),
            false,
        )
        .unwrap();
        app.toast(ToastKind::Error, message);
        let mut terminal = Terminal::new(TestBackend::new(90, 30)).unwrap();
        terminal
            .draw(|f| crate::view::render_app(f, &mut app))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn a_long_error_reaches_the_screen_instead_of_stopping_at_the_border() {
        // The tail is the part worth reading, and the part a one-row
        // card dropped.
        // The assertion targets the *tail*. A 50-cell card fits
        // roughly the first 45 columns, so anything this test proves
        // has to live past that: "permission denied" would have been
        // visible even unwrapped.
        let screen = painted_with_toast(
            "reconcile failed: cannot write /Users/a/outl/pages/deeply-nested-page.md because the device is full",
        );
        assert!(
            screen.contains("device is full"),
            "the tail of the error never reached the screen: {screen}"
        );
    }

    #[test]
    fn a_short_toast_still_paints_on_one_row() {
        let screen = painted_with_toast("saved");
        assert!(screen.contains("saved"), "{screen}");
    }

    #[test]
    fn a_short_message_stays_on_one_row() {
        assert_eq!(wrap_message("saved", 40, 4), vec!["saved"]);
    }

    #[test]
    fn a_long_message_wraps_instead_of_being_cut() {
        let rows = wrap_message("reconcile failed: permission denied on the sidecar", 24, 4);
        assert!(rows.len() > 1, "expected a wrap, got {rows:?}");
        for r in &rows {
            assert!(r.chars().count() <= 24, "row over width: {r:?}");
        }
        let joined = rows.join(" ");
        assert!(
            joined.contains("permission denied"),
            "the cause was lost: {joined:?}"
        );
    }

    #[test]
    fn a_message_past_the_row_budget_is_elided() {
        let long = "reconcile failed: ".to_string() + &"verbose ".repeat(40);
        let rows = wrap_message(&long, 20, 3);
        assert_eq!(rows.len(), 3, "must respect max_lines");
        assert!(
            rows.last().unwrap().ends_with('\u{2026}'),
            "the cut row must say it was cut: {:?}",
            rows.last()
        );
    }

    #[test]
    fn a_token_longer_than_the_card_is_hard_broken() {
        // A path with no spaces is the common shape of these errors.
        let path = "/Users/a/very/deeply/nested/workspace/pages/some-page.md";
        let rows = wrap_message(path, 16, 6);
        for r in &rows {
            assert!(r.chars().count() <= 16, "row over width: {r:?}");
        }
        assert!(rows.len() > 1, "a long path must wrap, got {rows:?}");
    }

    #[test]
    fn a_zero_width_card_returns_nothing_instead_of_looping() {
        assert!(wrap_message("anything", 0, 4).is_empty());
    }

    #[test]
    fn a_zero_row_budget_returns_nothing() {
        assert!(wrap_message("anything", 20, 0).is_empty());
    }
}
