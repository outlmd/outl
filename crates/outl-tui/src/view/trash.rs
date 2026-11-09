//! Renderer for the trash overlay (`/trash`). Pure projection of
//! [`TrashState`]; the listing, the restore and every refusal live in
//! `actions/trash.rs`.
//!
//! Its own file rather than a section of `view/overlays.rs`, which is at
//! its frozen size baseline — the same reason `view/sync_panel.rs` and
//! `view/properties.rs` are separate.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::actions::trash::TrashState;
use crate::state::App;

/// How long ago, for a deletion's wall clock.
///
/// Rendered, never ordered on: `deleted_at_ms` comes from whichever
/// device did the deleting, so a peer with a skewed clock can show a
/// time that reads out of order. The row order is `trash::list`'s, which
/// cannot depend on a clock.
fn ago(ms: Option<u64>) -> String {
    let Some(ms) = ms else {
        return String::new();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(ms);
    let seconds = now.saturating_sub(ms) / 1000;
    match seconds {
        0..=89 => "just now".to_string(),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// The overlay's body, plus the line index the selected row starts on.
///
/// A row is one to three lines (the entry, its age, its refusal), so the
/// cursor's *line* is not its index — which is what the scroll owner
/// needs.
pub(crate) fn lines(app: &App, state: &TrashState) -> (Vec<Line<'static>>, usize) {
    let t = &app.theme;
    let mut out = Vec::new();

    let mut selected_line = 0usize;

    if state.entries.is_empty() {
        out.push(Line::from(Span::styled(
            " Nothing has been deleted in this workspace.",
            t.dim,
        )));
        out.push(Line::from(Span::styled(
            " A deleted block comes here instead of disappearing, from any device.",
            t.dim,
        )));
        return (out, selected_line);
    }

    for (i, entry) in state.entries.iter().enumerate() {
        if i == state.selected {
            selected_line = out.len();
        }
        let marker = if i == state.selected { ">" } else { " " };
        // A deleted page is named by its slug; a deleted block by its
        // text. `restore` refuses the first, so telling them apart is
        // what stops the overlay offering an action that always fails.
        let what = match &entry.page {
            Some(slug) => format!("page `{slug}`"),
            None => entry.preview.clone(),
        };
        let carried = if entry.subtree_len > 1 {
            format!("  (+{} block(s) under it)", entry.subtree_len - 1)
        } else {
            String::new()
        };
        let when = ago(entry.deleted_at_ms);
        let style = if i == state.selected {
            t.help_title
        } else {
            t.dim
        };
        out.push(Line::from(Span::styled(
            format!(" {marker} {what}{carried}"),
            style,
        )));
        if !when.is_empty() {
            out.push(Line::from(Span::styled(format!("     {when}"), t.dim)));
        }
        // Printed verbatim: `ActionError::Display` owns the whole
        // sentence, "cannot restore" included, so prefixing it here
        // would state it twice.
        if let Some(refusal) = &entry.refusal {
            out.push(Line::from(Span::styled(format!("     {refusal}"), t.dim)));
        }
    }

    if let Some(message) = &state.message {
        out.push(Line::from(""));
        out.push(Line::from(Span::styled(
            format!(" {message}"),
            t.help_title,
        )));
    }
    (out, selected_line)
}

pub(crate) fn render_trash(f: &mut ratatui::Frame<'_>, full: Rect, app: &App, state: &TrashState) {
    let area = super::overlays::centered_rect(full, 76, 70);
    f.render_widget(Clear, area);
    let (body_lines, selected_line) = lines(app, state);
    // `scroll_to_keep_visible` is the one owner of a list popup's offset
    // (`view/overlays.rs`), and it is asked about the selected row's
    // *line* because a row here is one to three of them. The hand-rolled
    // `selected * 3` this replaced both over-scrolled a list that already
    // fit and used `as u16`, which wraps to the top past 65,535 rows
    // instead of clamping.
    let visible = usize::from(area.height.saturating_sub(2));
    let scroll = super::overlays::scroll_to_keep_visible(selected_line, visible);
    let body = Paragraph::new(body_lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(
                    " Trash · j/k move · r restore · Enter open its page · Esc close ",
                    app.theme.help_title,
                ))
                .style(app.theme.popup_style()),
        );
    f.render_widget(body, area);
}
