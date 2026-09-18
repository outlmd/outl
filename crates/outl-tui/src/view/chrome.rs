//! Top header + bottom footer — the "chrome" around the main outline.
//!
//! Split out from `view.rs` so the orchestrator stays small and the
//! semantics of each segment (breadcrumb, chips, powerline footer) get
//! room to breathe without leaking into outline / overlay rendering.
//!
//! Two public entry points: [`render_header`] paints the top bar,
//! [`render_footer`] paints the powerline at the bottom. Both consume
//! the `Rect` the orchestrator already laid out — no layout decisions
//! here.

use crate::outline_ops::count_todos;
use crate::state::{App, Mode, View, HELP_HINT_INSERT, HELP_HINT_NORMAL, HELP_HINT_VISUAL};
use outl_actions::clock;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_width::UnicodeWidthStr;

/// Header (top, 3 lines): breadcrumb on the left, status chips on the
/// right. Splits the row in half so chips never overlap the title even
/// when one side is long.
pub(crate) fn render_header(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    // Frame block first — chips and breadcrumb both draw *inside* the
    // bordered area, so we render the outer block as a single widget
    // and then split the inner Rect for content.
    let frame = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border)
        .title(Span::styled(
            format!(" outl · {} ", app.theme.name),
            app.theme.hint,
        ));
    let inner = frame.inner(area);
    f.render_widget(frame, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(chips_width(app))])
        .split(inner);

    f.render_widget(Paragraph::new(breadcrumb(app)), cols[0]);
    f.render_widget(
        Paragraph::new(chips(app)).alignment(ratatui::layout::Alignment::Right),
        cols[1],
    );
}

/// Footer (bottom, 3 lines): powerline-style segmented status bar.
/// Each segment carries its own bg/fg so the user can scan the bar
/// for mode + workspace + clock + save state at a glance.
pub(crate) fn render_footer(f: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let frame = Block::default()
        .borders(Borders::ALL)
        .border_style(app.theme.border);
    let inner = frame.inner(area);
    f.render_widget(frame, area);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(20),
            Constraint::Length(right_segments_width(app)),
        ])
        .split(inner);

    f.render_widget(Paragraph::new(left_segments(app)), cols[0]);
    f.render_widget(
        Paragraph::new(right_segments(app)).alignment(ratatui::layout::Alignment::Right),
        cols[1],
    );
}

// ─── header ───────────────────────────────────────────────────────────

fn breadcrumb(app: &App) -> Line<'static> {
    let workspace_label = app
        .workspace_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();

    let (icon, title) = view_icon_and_title(app);
    let sep = Span::styled(" ▸ ", app.theme.dim);

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(" outl", app.theme.heading.add_modifier(Modifier::BOLD)),
        sep.clone(),
        Span::styled(workspace_label, app.theme.hint),
        sep,
    ];
    if let Some(ic) = icon {
        spans.push(Span::raw(format!("{ic} ")));
    }
    spans.push(Span::styled(title, app.theme.heading));

    // When zoomed into a block (Roam/Workflowy), extend the breadcrumb
    // with the ancestor chain down to the focused root so the user can
    // see `page ▸ parent ▸ block` and knows they're not looking at the
    // whole page.
    for crumb in app.zoom_breadcrumb() {
        spans.push(Span::styled(" ▸ ", app.theme.dim));
        spans.push(Span::styled(crumb, app.theme.hint));
    }
    Line::from(spans)
}

/// Pick the icon + title for the current view. Falls back to the slug
/// when the workspace index hasn't indexed the page yet (race on cold
/// start).
fn view_icon_and_title(app: &App) -> (Option<String>, String) {
    match &app.view {
        View::Journal(date) => (
            Some(app.icons.calendar.to_string()),
            format!("Journal · {}", date.format("%A, %Y-%m-%d")),
        ),
        View::Page(p) => {
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
            let entry = app.index.by_slug(stem);
            let title = entry
                .map(|e| e.title.clone())
                .unwrap_or_else(|| stem.to_string());
            let icon = entry.and_then(|e| e.icon.clone());
            (icon, title)
        }
    }
}

/// Right-aligned status chips: index throbber · TODOs · unsaved · freshness.
///
/// Each chip is a `Span` with its own bg. Order matters — most
/// important first (rightmost in raw token order, since we then
/// right-align the whole line).
fn chips(app: &App) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();

    // Background index rebuild indicator. Animates against a 100 ms
    // tick derived from the system clock so the spinner moves even
    // when no key is pressed — the event loop redraws on the same
    // poll cadence.
    if app.has_pending_index() {
        let frame = throbber_frame();
        spans.push(Span::styled(
            format!(" {frame} indexing "),
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }

    let (done, total) = count_todos(&app.page.blocks);
    if total > 0 {
        let chip_style = if done == total {
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::LightGreen)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD)
        };
        spans.push(Span::styled(
            format!(" {} {done}/{total} ", app.icons.todo_chip),
            chip_style,
        ));
        spans.push(Span::raw(" "));
    }

    if matches!(app.mode, Mode::Insert { .. }) {
        spans.push(Span::styled(
            format!(" {} editing ", app.icons.editing),
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }

    if let Some(at) = app.last_saved_at {
        let secs = at.elapsed().as_secs();
        let label = format_age(secs);
        spans.push(Span::styled(
            format!(" {} {label} ", app.icons.freshness),
            Style::default().bg(Color::DarkGray).fg(Color::Gray),
        ));
        spans.push(Span::raw(" "));
    }

    Line::from(spans)
}

/// Tight upper bound on the chips line width so the layout split
/// doesn't truncate it. We over-estimate by 4 chars to keep the
/// breadcrumb side from squeezing on a wide chip set.
fn chips_width(app: &App) -> u16 {
    let mut w: u16 = 0;
    if app.has_pending_index() {
        w += 13;
    }
    let (_, total) = count_todos(&app.page.blocks);
    if total > 0 {
        w += 14;
    }
    if matches!(app.mode, Mode::Insert { .. }) {
        w += 12;
    }
    if app.last_saved_at.is_some() {
        w += 14;
    }
    w.min(70)
}

/// Pick a Braille spinner frame from the system clock. 10 frames at
/// 100 ms each = one full rotation per second. Works without holding
/// any tick counter in `App` — the render path is called frequently
/// enough (every poll + every keypress) that the user sees motion.
fn throbber_frame() -> char {
    const FRAMES: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    FRAMES[((millis / 100) as usize) % FRAMES.len()]
}

fn format_age(secs: u64) -> String {
    match secs {
        0..=2 => "just now".to_string(),
        s @ 3..=59 => format!("{s}s ago"),
        s @ 60..=3599 => format!("{}m ago", s / 60),
        s => format!("{}h ago", s / 3600),
    }
}

// ─── footer ───────────────────────────────────────────────────────────

fn left_segments(app: &App) -> Line<'static> {
    let (mode_label, mode_style) = match app.mode {
        Mode::Normal => (" NORMAL ", app.theme.status_normal),
        Mode::Insert { .. } => (" INSERT ", app.theme.status_insert),
        Mode::Visual { .. } => (" VISUAL ", app.theme.status_visual),
    };

    let workspace_label = app
        .workspace_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace")
        .to_string();

    let mut spans: Vec<Span<'static>> = vec![
        Span::styled(mode_label, mode_style.add_modifier(Modifier::BOLD)),
        // Powerline-style "arrow" between segments: a solid char on
        // the previous bg followed by spaces on the next. Works on
        // any terminal without nerd-font.
        Span::styled(" ", Style::default().bg(Color::DarkGray)),
        Span::styled(
            format!(" {} {workspace_label} ", app.icons.workspace),
            Style::default().bg(Color::DarkGray).fg(Color::Gray),
        ),
        Span::raw(" "),
    ];

    // Backlink count — kept in the footer (was already here pre-refactor).
    let bl_count = app.backlinks_count_for_current();
    if bl_count > 0 {
        spans.push(Span::styled(
            format!(
                " {} {bl_count} backlink{} ",
                app.icons.backlinks,
                if bl_count == 1 { "" } else { "s" }
            ),
            Style::default().bg(Color::DarkGray).fg(Color::LightCyan),
        ));
        spans.push(Span::raw(" "));
    }

    let hint = match app.mode {
        Mode::Insert { .. } => HELP_HINT_INSERT,
        Mode::Visual { .. } => HELP_HINT_VISUAL,
        Mode::Normal => HELP_HINT_NORMAL,
    };
    spans.push(Span::styled(hint, app.theme.hint));

    // Transient status message ("saved", "reconcile failed: …"). When
    // empty, no extra padding.
    if !app.status.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(app.status.clone(), app.theme.status_message));
    }

    Line::from(spans)
}

fn right_segments(app: &App) -> Line<'static> {
    let now = clock::now_local().format("%H:%M").to_string();
    let saved = match app.last_saved_at {
        Some(_) if app.status.is_empty() => format!(" {} saved ", app.icons.save),
        Some(_) => format!(" {} ", app.icons.save),
        None => " ○ ".to_string(),
    };
    Line::from(vec![
        Span::styled(
            format!(" {} {now} ", app.icons.clock),
            Style::default().bg(Color::DarkGray).fg(Color::Gray),
        ),
        Span::raw(" "),
        Span::styled(
            saved,
            Style::default().bg(Color::DarkGray).fg(Color::LightGreen),
        ),
        Span::raw(" "),
        Span::styled(" ? help ", app.theme.hint),
    ])
}

fn right_segments_width(app: &App) -> u16 {
    let clock = format!(" {} 00:00 ", app.icons.clock).width();
    let saved = format!(" {} saved ", app.icons.save).width();
    let help = " ? help ".width();
    clock
        .saturating_add(saved)
        .saturating_add(help)
        .saturating_add(2) as u16
}
