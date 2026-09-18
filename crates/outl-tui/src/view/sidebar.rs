//! Left sidebar — mini-calendar, pinned pages, recent.
//!
//! Default hidden. The orchestrator (`view.rs`) only calls
//! [`render_sidebar`] when `app.show_sidebar == true`. The width is
//! fixed (24 cols) so the outline column has a stable layout when the
//! sidebar toggles on/off — no jitter mid-edit.
//!
//! Each section is rendered as a labelled `Block`. The section that
//! currently has keyboard focus (Tab inside the sidebar) gets a
//! highlighted border so the user always knows where the cursor lives.

use crate::state::{App, SidebarSection, View};
use chrono::{Datelike, NaiveDate};
use outl_actions::clock;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};

/// Width the orchestrator should reserve for the sidebar. Fixed so
/// the main column has a predictable Rect for cursor math.
pub(crate) const SIDEBAR_WIDTH: u16 = 24;

pub(crate) fn render_sidebar(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    // Three stacked sections. The fixed heights keep `recent` getting
    // the leftover space — recent is the longest list in practice and
    // benefits from being elastic.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(11),
            Constraint::Length(7),
            Constraint::Min(5),
        ])
        .split(area);

    render_calendar(f, chunks[0], app);
    render_pinned(f, chunks[1], app);
    render_recent(f, chunks[2], app);
}

// ─── calendar ─────────────────────────────────────────────────────────

fn render_calendar(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let today = clock::today();
    let viewing = match &app.view {
        View::Journal(d) => *d,
        View::Page(_) => today,
    };
    let first = NaiveDate::from_ymd_opt(viewing.year(), viewing.month(), 1).unwrap_or(today);
    // Weekday index, 0 = Monday — line up with the `Mo Tu …` header.
    let weekday_offset = first.weekday().num_days_from_monday() as usize;
    let days_in_month = days_in_month(viewing.year(), viewing.month());

    let mut lines: Vec<Line<'static>> = vec![Line::from(Span::styled(
        "Mo Tu We Th Fr Sa Su",
        app.theme.dim,
    ))];

    let mut row_spans: Vec<Span<'static>> = Vec::new();
    // Pad the first week with blanks so day 1 aligns with its weekday.
    for _ in 0..weekday_offset {
        row_spans.push(Span::raw("   "));
    }
    for day in 1..=days_in_month {
        let date = NaiveDate::from_ymd_opt(viewing.year(), viewing.month(), day)
            .expect("constructed from validated y/m");
        let has_journal = app
            .index
            .by_slug(&date.format("%Y-%m-%d").to_string())
            .is_some_and(|e| e.is_journal);

        let style = if date == today {
            // Bullseye on today, regardless of journal status.
            app.theme.heading.add_modifier(Modifier::BOLD)
        } else if date == viewing {
            app.theme.selected_bullet
        } else if has_journal {
            app.theme.ref_link
        } else {
            app.theme.dim
        };

        let glyph = if date == today {
            format!("◉{day:>2}")
        } else if has_journal {
            format!("●{day:>2}")
        } else {
            format!("·{day:>2}")
        };
        row_spans.push(Span::styled(glyph, style));

        // 7 days per row. After Sunday, flush.
        if (weekday_offset + day as usize).is_multiple_of(7) {
            lines.push(Line::from(std::mem::take(&mut row_spans)));
        }
    }
    if !row_spans.is_empty() {
        lines.push(Line::from(row_spans));
    }

    let focused = app.sidebar_focus == Some(SidebarSection::Calendar);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            app.theme.heading
        } else {
            app.theme.border
        })
        .title(Span::styled(
            format!(" {} {} ", app.icons.calendar, viewing.format("%B %Y")),
            app.theme.hint,
        ));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn days_in_month(year: i32, month: u32) -> u32 {
    // Trick: first day of next month minus one day.
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.day())
        .unwrap_or(30)
}

// ─── pinned ───────────────────────────────────────────────────────────

fn render_pinned(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let mut pinned: Vec<(&str, Option<&str>)> = app
        .index
        .pages()
        .filter(|p| p.pinned)
        .map(|p| (p.title.as_str(), p.icon.as_deref()))
        .collect();
    pinned.sort_by_key(|(t, _)| t.to_lowercase());

    let focused = app.sidebar_focus == Some(SidebarSection::Pinned);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            app.theme.heading
        } else {
            app.theme.border
        })
        .title(Span::styled(
            format!(" {} Pinned ", app.icons.star),
            app.theme.hint,
        ));

    if pinned.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " (pin pages with pinned:: true)",
                app.theme.dim,
            )))
            .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem<'_>> = pinned
        .into_iter()
        .map(|(title, icon)| {
            let label = match icon {
                Some(ic) => format!(" {ic} {title}"),
                None => format!(" {} {title}", app.icons.file),
            };
            ListItem::new(Line::from(Span::raw(label)))
        })
        .collect();

    // `ListState` carries the selected index AND the scroll offset.
    // ratatui auto-scrolls so the highlighted row stays visible —
    // we just feed it the cursor and let it figure out the offset.
    let mut state = ListState::default();
    if focused {
        state.select(Some(app.sidebar_cursor));
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(app.theme.list_selected);
    f.render_stateful_widget(list, area, &mut state);
}

// ─── recent ───────────────────────────────────────────────────────────

fn render_recent(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let focused = app.sidebar_focus == Some(SidebarSection::Recent);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            app.theme.heading
        } else {
            app.theme.border
        })
        .title(Span::styled(
            format!(" {} Recent ", app.icons.history),
            app.theme.hint,
        ));

    if app.recent_paths.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                " (opened pages show here)",
                app.theme.dim,
            )))
            .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem<'_>> = app
        .recent_paths
        .iter()
        .take(20)
        .map(|path| {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
            let entry = app.index.by_slug(stem);
            let (icon, label) = match entry {
                Some(e) if e.is_journal => (app.icons.calendar.to_string(), e.title.clone()),
                Some(e) => (
                    e.icon.clone().unwrap_or_else(|| app.icons.file.to_string()),
                    e.title.clone(),
                ),
                None => (app.icons.file.to_string(), stem.to_string()),
            };
            ListItem::new(Line::from(Span::raw(format!(" {icon} {label}"))))
        })
        .collect();

    // ListState handles offset for us — feeding it the cursor lets
    // ratatui keep the highlighted row inside the (variable-height)
    // Recent panel even when the user walks past the visible window.
    let mut state = ListState::default();
    if focused {
        state.select(Some(app.sidebar_cursor));
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(app.theme.list_selected);
    f.render_stateful_widget(list, area, &mut state);
}
