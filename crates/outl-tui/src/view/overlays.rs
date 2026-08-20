//! Modal overlays: quick switcher, search, slash menu, command bar,
//! error popup, help popup, and the inline autocomplete dropdown.
//!
//! Each function takes the full frame `Rect` and centers / anchors its
//! own popup inside. `render_app` in the parent module dispatches based
//! on `app.overlay`.

use crate::actions::plugins::value_to_input;
use crate::state::{
    App, AutocompleteKind, AutocompleteState, CommandState, ErrorState, PluginSettingsState,
    QuickSwitchState, RemindersState, SearchState, SlashState, SwitchKind, TemplatePickerState,
};
use crate::theme::Theme;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Tabs};

pub(crate) fn render_autocomplete(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    ac: &AutocompleteState,
) {
    let height = (ac.candidates.len() as u16 + 2).min(10);
    if height < 3 {
        return;
    }
    let width = 36u16.min(full.width.saturating_sub(4));
    // Bottom-right anchor so it doesn't fight with the outline.
    let area = Rect {
        x: full.x + full.width.saturating_sub(width + 2),
        y: full.y + full.height.saturating_sub(height + 2),
        width,
        height,
    };
    f.render_widget(Clear, area);
    let title = match ac.kind {
        AutocompleteKind::PageRef => format!("[[{}]]", ac.query),
        AutocompleteKind::Tag => format!("#{}", ac.query),
        AutocompleteKind::BlockRef => format!("(({}))", ac.query),
        AutocompleteKind::SlashCommand => format!("/{}", ac.query),
        AutocompleteKind::Mention => format!("@{}", ac.query),
        AutocompleteKind::Emoji => format!(":{}", ac.query),
    };
    let items: Vec<ListItem<'_>> = ac
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let style = if i == ac.selected {
                app.theme.list_selected
            } else {
                Style::default()
            };
            // Decorate the candidate row according to its kind. For
            // pages/tags we prepend the page's `icon::` (display-only);
            // for slash commands we append a dim description so the
            // popup doubles as in-context help.
            match ac.kind {
                AutocompleteKind::PageRef | AutocompleteKind::Tag | AutocompleteKind::Mention => {
                    // Both PageRef and Mention list candidates by
                    // **title** (`by_title`); Tag lists by slug.
                    let icon = match ac.kind {
                        AutocompleteKind::PageRef | AutocompleteKind::Mention => {
                            app.index.by_title(c).and_then(|p| p.icon.clone())
                        }
                        AutocompleteKind::Tag => app.index.by_slug(c).and_then(|p| p.icon.clone()),
                        _ => None,
                    };
                    let label = match icon {
                        Some(ic) => format!("{ic} {c}"),
                        None => c.clone(),
                    };
                    ListItem::new(Line::from(Span::styled(label, style)))
                }
                AutocompleteKind::BlockRef => {
                    // `c` is the handle. Resolve to the block's text
                    // for display — that's what the user is hunting
                    // for; the raw handle would be unreadable.
                    let text = app
                        .index
                        .resolve_block_ref(c)
                        .map(|b| b.text.clone())
                        .unwrap_or_else(|| c.clone());
                    ListItem::new(Line::from(vec![
                        Span::styled(text, style),
                        Span::styled(format!("  {c}"), app.theme.dim),
                    ]))
                }
                AutocompleteKind::SlashCommand => {
                    let cmd = app.command_registry.get(c);
                    let description = cmd.as_ref().map(|cmd| cmd.description()).unwrap_or("");
                    let needs_args = cmd.as_ref().map(|cmd| cmd.needs_args()).unwrap_or(false);
                    let suffix = if needs_args { " …" } else { "" };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{c}{suffix}  "), style),
                        Span::styled(description.to_string(), app.theme.dim),
                    ]))
                }
                AutocompleteKind::Emoji => {
                    // `c` is the shortcode. Resolve to the glyph for
                    // the leading column; the canonical shortcode stays
                    // in the right column as the affordance the user
                    // is searching by.
                    let glyph = outl_md::emoji::shortcode_to_unicode(c).unwrap_or("");
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{glyph}  "), style),
                        Span::styled(format!(":{c}:"), app.theme.dim),
                    ]))
                }
            }
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(title, app.theme.help_title)),
        )
        .style(app.theme.popup_style());
    f.render_widget(list, area);
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

pub(crate) fn render_quick_switch(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    qs: &QuickSwitchState,
) {
    // Wider overlay (80%) so the preview pane has room to show real
    // outline context, not 5-char truncations.
    let area = centered_rect(full, 80, 70);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" › ", app.theme.help_title),
        Span::raw(qs.query.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled("Quick Switcher", app.theme.help_title)),
    )
    .style(app.theme.popup_style());
    f.render_widget(input, outer[0]);

    // Telescope-style split: list on the left, preview on the right.
    // The preview re-reads the highlighted page from disk per frame —
    // cheap for a single page, and avoids leaking a page cache into
    // App state for a feature that's open for ~5 seconds at a time.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(outer[1]);

    let items: Vec<ListItem<'_>> = qs
        .candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let icon = match c.kind {
                SwitchKind::Page => "📄 ",
                SwitchKind::Journal => "📅 ",
            };
            let style = if i == qs.selected {
                app.theme.list_selected
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![
                Span::raw(icon),
                Span::styled(c.label.clone(), style),
            ]))
        })
        .collect();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(format!("{} matches  ↑↓ Enter Esc", qs.candidates.len())),
        )
        .style(app.theme.popup_style());
    f.render_widget(list, cols[0]);

    render_preview_pane(f, cols[1], app, qs);
}

/// Right-hand preview pane for the quick switcher. Shows the first
/// ~N blocks of the highlighted candidate, or a placeholder when the
/// candidate isn't indexed yet (cold-start race) / has no body.
fn render_preview_pane(f: &mut ratatui::Frame<'_>, area: Rect, app: &App, qs: &QuickSwitchState) {
    let Some(candidate) = qs.candidates.get(qs.selected) else {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  (type to search)",
            app.theme.dim,
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(" preview ", app.theme.help_title)),
        )
        .style(app.theme.popup_style());
        f.render_widget(empty, area);
        return;
    };

    // One-slot cache: the renderer is called on every poll tick, but
    // the underlying file changes only when the user touches j/k (or
    // edits a page elsewhere). Re-read only when the cached key
    // doesn't match the current candidate.
    let path = app.index.by_slug(&candidate.key).map(|e| e.path.clone());
    let cached_text: Option<String> = {
        let mut slot = qs.preview_cache.borrow_mut();
        let hit = matches!(slot.as_ref(), Some((k, _)) if k == &candidate.key);
        if !hit {
            *slot = path
                .as_deref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|text| (candidate.key.clone(), text));
        }
        slot.as_ref().map(|(_, text)| text.clone())
    };

    let body_lines: Vec<Line<'static>> = match (path.is_some(), cached_text) {
        (true, Some(text)) => preview_lines(&text, area.height.saturating_sub(2) as usize, app),
        (true, None) => vec![Line::from(Span::styled(
            "  (couldn't read file)",
            app.theme.dim,
        ))],
        (false, _) => vec![Line::from(Span::styled(
            "  (not yet indexed)",
            app.theme.dim,
        ))],
    };

    let title_prefix = match candidate.kind {
        SwitchKind::Page => "📄 ",
        SwitchKind::Journal => "📅 ",
    };
    let preview = Paragraph::new(body_lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(
                    format!(" {title_prefix}{} ", candidate.label),
                    app.theme.help_title,
                )),
        )
        .style(app.theme.popup_style())
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(preview, area);
}

/// Cheap markdown → preview lines. Doesn't reuse the outline renderer
/// (we don't want cursors, TODO checkboxes, etc.) — just enough to
/// give the user a sense of what they're about to open.
fn preview_lines(text: &str, max: usize, app: &App) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    for line in text.lines() {
        if out.len() >= max {
            break;
        }
        let trimmed_start = line.trim_start();
        if trimmed_start.is_empty() {
            out.push(Line::raw(""));
            continue;
        }
        if let Some(rest) = trimmed_start.strip_prefix("- ") {
            let indent_chars = line.len() - trimmed_start.len();
            let indent = " ".repeat(indent_chars);
            out.push(Line::from(vec![
                Span::raw(indent),
                Span::styled("• ", app.theme.bullet),
                Span::raw(rest.to_string()),
            ]));
        } else if trimmed_start.contains("::") {
            // property line — show dimmer to de-emphasize.
            out.push(Line::from(Span::styled(line.to_string(), app.theme.dim)));
        } else {
            out.push(Line::raw(line.to_string()));
        }
    }
    if out.is_empty() {
        out.push(Line::from(Span::styled("  (empty page)", app.theme.dim)));
    }
    out
}

pub(crate) fn render_search_overlay(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    s: &SearchState,
) {
    let area = centered_rect(full, 75, 70);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" / ", app.theme.help_title),
        Span::raw(s.query.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled("Search", app.theme.help_title)),
    )
    .style(app.theme.popup_style());
    f.render_widget(input, outer[0]);

    let lines: Vec<Line<'_>> = s
        .hits
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let style = if i == s.selected {
                app.theme.list_selected
            } else {
                Style::default()
            };
            let icon_prefix = h
                .page_icon
                .as_deref()
                .map(|i| format!("{i} "))
                .unwrap_or_default();
            Line::from(vec![
                Span::styled(format!(" {icon_prefix}{} · ", h.page_label), app.theme.dim),
                Span::styled(h.snippet.clone(), style),
            ])
        })
        .collect();
    let list = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(format!(
                    "{} hits  ↑↓ navigate · Enter jump · Esc cancel",
                    s.hits.len()
                )),
        )
        .style(app.theme.popup_style())
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(list, outer[1]);
}

pub(crate) fn render_slash_overlay(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    s: &SlashState,
) {
    let area = centered_rect(full, 65, 65);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" / ", app.theme.help_title),
        Span::raw(s.query.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled(" Command palette ", app.theme.help_title)),
    )
    .style(app.theme.popup_style());
    f.render_widget(input, outer[0]);

    // Paint commands in the canonical visual order (category-first, buckets
    // sorted by `category_order`, within each bucket the order that
    // `s.candidates` already carries — score-desc under a filter, registry
    // order when the query is empty).  `visual_order` is the single source
    // of truth consumed by both this renderer and keyboard navigation, so
    // the two can never disagree about "which row is highlighted".
    let order = visual_order(&s.candidates);

    let mut lines: Vec<Line<'static>> = Vec::new();
    // Track which visual row the highlighted command landed on so
    // we can scroll the Paragraph to keep it in view when category
    // headers + blank rows push it past the visible height.
    let mut highlighted_row: Option<usize> = None;
    let mut prev_cat: Option<&'static str> = None;
    for orig_idx in &order {
        let c = &s.candidates[*orig_idx];
        let cat = category_for(&c.name);
        // Emit a section header whenever the category changes.
        if prev_cat != Some(cat) {
            if prev_cat.is_some() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::from(Span::styled(
                format!(" {} {} ", category_icon(cat), cat),
                app.theme.help_title,
            )));
            prev_cat = Some(cat);
        }
        if *orig_idx == s.selected {
            highlighted_row = Some(lines.len());
        }
        let style = if *orig_idx == s.selected {
            app.theme.list_selected
        } else {
            Style::default()
        };
        let suffix = if c.needs_args { " …" } else { "" };
        lines.push(Line::from(vec![
            Span::styled(
                format!("   {}  {}{suffix}  ", command_icon(&c.name), c.name),
                style,
            ),
            Span::styled(c.description.to_string(), app.theme.dim),
        ]));
    }
    if prev_cat.is_some() {
        lines.push(Line::raw(""));
    }

    // Viewport height inside the bordered block.
    let inner_h = outer[1].height.saturating_sub(2) as usize;
    let total = lines.len();
    // Auto-scroll: when the highlighted row would render past the
    // bottom of the viewport, push the scroll offset just enough to
    // bring it back into view. Clamps so the bottom of the list
    // doesn't scroll past the last row.
    let scroll: u16 = match highlighted_row {
        Some(row) if inner_h > 0 && row >= inner_h => {
            let max = total.saturating_sub(inner_h);
            ((row + 1).saturating_sub(inner_h)).min(max) as u16
        }
        _ => 0,
    };

    let list = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(format!(" {} commands · ↑↓ Enter Esc ", s.candidates.len())),
        )
        .style(app.theme.popup_style())
        .scroll((scroll, 0));
    f.render_widget(list, outer[1]);
}

/// Bucket a command name into a coarse category. Names follow loose
/// prefix conventions (`date-*`, `time-*`, `iso-*`, `week-*`, …) so
/// we can group without each command having to declare its category.
fn category_for(name: &str) -> &'static str {
    let n = name;
    if n.starts_with("date")
        || n.starts_with("time")
        || n.starts_with("iso")
        || n.starts_with("week")
        || n == "stamp"
        || n == "dt"
        || n == "dtm"
        || n == "dy"
    {
        "Dates & time"
    } else if n == "search" || n == "find" {
        "Search"
    } else if n == "theme" || n == "set" || n == "config" {
        "Settings"
    } else if n == "open" || n == "switch" || n == "quit" || n == "q" {
        "Navigation"
    } else {
        "Actions"
    }
}

/// Canonical sort order — Actions first (most common), Dates last
/// (long list, scrolls off).
fn category_order(cat: &str) -> u8 {
    match cat {
        "Actions" => 0,
        "Navigation" => 1,
        "Search" => 2,
        "Settings" => 3,
        "Dates & time" => 4,
        _ => 5,
    }
}

/// Return the canonical sort order for the category a command name
/// belongs to. Used by tests to assert that navigation walks commands
/// in non-decreasing category order.
#[cfg(test)]
pub(crate) fn category_order_for(name: &str) -> u8 {
    category_order(category_for(name))
}

/// Flatten `candidates` into the exact visual order the palette paints:
/// buckets grouped by category, buckets sorted by [`category_order`],
/// the relative order of commands *within* each bucket preserved (callers
/// are responsible for putting the highest-relevance commands first inside
/// their bucket — `refresh_slash` does this via score-desc sort).
///
/// Returns indices into `candidates` in paint order.
/// This is the **single source of truth** for "visual order": both the
/// renderer and keyboard navigation consume it so they can never diverge.
pub(crate) fn visual_order(candidates: &[crate::state::SlashCommand]) -> Vec<usize> {
    // Collect (category, original_index) pairs, grouping by category while
    // preserving within-category order.
    let mut buckets: Vec<(&'static str, Vec<usize>)> = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let cat = category_for(&c.name);
        if let Some(b) = buckets.iter_mut().find(|(k, _)| *k == cat) {
            b.1.push(i);
        } else {
            buckets.push((cat, vec![i]));
        }
    }
    buckets.sort_by_key(|(k, _)| category_order(k));
    buckets.into_iter().flat_map(|(_, idxs)| idxs).collect()
}

fn category_icon(cat: &str) -> &'static str {
    match cat {
        "Actions" => "⚡",
        "Navigation" => "↪",
        "Search" => "🔎",
        "Settings" => "⚙",
        "Dates & time" => "📅",
        _ => "•",
    }
}

/// Per-command leading glyph. Falls back to a dot for anything we
/// haven't curated.
fn command_icon(name: &str) -> &'static str {
    match name {
        "run" => "▶",
        "prop" => "≡",
        "search" | "find" => "🔎",
        "theme" => "🎨",
        "open" | "switch" => "↪",
        "quit" | "q" => "✕",
        n if n.starts_with("date") || n == "dt" || n == "dy" || n == "dtm" => "📅",
        n if n.starts_with("time") => "🕐",
        n if n.starts_with("iso") => "🔢",
        n if n.starts_with("week") => "📆",
        "stamp" => "🕒",
        _ => "·",
    }
}

pub(crate) fn render_template_picker(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    tp: &TemplatePickerState,
) {
    let area = centered_rect(full, 60, 50);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let input = Paragraph::new(Line::from(vec![
        Span::styled(" 📋 ", app.theme.help_title),
        Span::raw(tp.query.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled(" Templates ", app.theme.help_title)),
    )
    .style(app.theme.popup_style());
    f.render_widget(input, outer[0]);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (vis_i, &data_i) in tp.filtered.iter().enumerate() {
        let Some(tpl) = tp.all.get(data_i) else {
            continue;
        };
        let icon = if tpl.params.is_empty() { "📄" } else { "⚡" };
        let label = format!(" {icon} {:<20} {}", tpl.name, tpl.slug);
        if vis_i == tp.selected {
            lines.push(Line::from(vec![Span::styled(label, app.theme.help_title)]));
        } else {
            lines.push(Line::from(vec![Span::raw(label)]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " No templates found. Add `template:: name` to a page.",
            app.theme.help_title,
        )]));
    }

    let list = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .style(app.theme.popup_style()),
        )
        .scroll(if tp.selected > 5 {
            (0, (tp.selected - 5) as u16)
        } else {
            (0, 0)
        });
    f.render_widget(list, outer[1]);
}

/// Reminders overlay: every `remind::` in the workspace, soonest
/// first, with the next fire in a right-hand column.
///
/// A finished rule (DONE, expired, out of `max`) shows `—` instead of
/// a time and sorts to the bottom — the scan already ordered it that
/// way, so this renderer stays a pure projection.
pub(crate) fn render_reminders(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    state: &RemindersState,
) {
    let area = centered_rect(full, 72, 60);
    f.render_widget(Clear, area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (i, r) in state.all.iter().enumerate() {
        let when = match r.next_fire {
            Some(t) => t.format("%a %d %b %H:%M").to_string(),
            None => "—".to_string(),
        };
        let snoozed = if r.snoozed_until_ms.is_some() {
            " 💤"
        } else {
            ""
        };
        let label = format!(
            " {:<18} {:<34} {}{}",
            when,
            truncate(&r.text, 34),
            r.rule_text,
            snoozed
        );
        if i == state.selected {
            lines.push(Line::from(vec![Span::styled(label, app.theme.help_title)]));
        } else if r.done {
            lines.push(Line::from(vec![Span::styled(label, app.theme.dim)]));
        } else {
            lines.push(Line::from(vec![Span::raw(label)]));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " No reminders. Press `g r` on a block to add one.",
            app.theme.help_title,
        )]));
    }

    let list = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(
                    " Reminders — Enter open · s 1h · t tomorrow · w next week · Esc close ",
                    app.theme.help_title,
                ))
                .style(app.theme.popup_style()),
        )
        .scroll(if state.selected > 5 {
            (0, (state.selected - 5) as u16)
        } else {
            (0, 0)
        });
    f.render_widget(list, area);
}

/// Clip `s` to `max` display columns, appending `…` when it had to cut.
/// Char-based (not byte-based) so a multi-byte block title can't panic
/// the renderer mid-frame.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

pub(crate) fn render_plugin_settings(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    ps: &PluginSettingsState,
) {
    let area = centered_rect(full, 72, 62);
    f.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(2)])
        .split(area);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut row = 0usize; // flat index, aligned with ps.rows / ps.selected
    for entry in &ps.entries {
        lines.push(Line::from(vec![Span::styled(
            format!(" {} ", entry.plugin_id),
            app.theme.help_title,
        )]));
        for field in &entry.fields {
            let selected = row == ps.selected;
            let editing = selected && ps.editing.is_some();
            let kind = format!("{:?}", field.kind).to_lowercase();

            let state = if editing {
                let buf = ps.editing.as_deref().unwrap_or("");
                let shown = if field.secret {
                    "•".repeat(buf.chars().count())
                } else {
                    buf.to_string()
                };
                format!("{shown}▏")
            } else if field.secret {
                if field.is_set {
                    "set".to_string()
                } else {
                    "not set".to_string()
                }
            } else {
                match &field.value {
                    Some(v) => value_to_input(v),
                    None => match &field.default {
                        Some(d) => format!("({})", value_to_input(d)),
                        None => "(unset)".to_string(),
                    },
                }
            };

            let prefix = if selected { "❯ " } else { "  " };
            let label = format!("{prefix}{}  [{kind}]  {state}", field.key);
            let style = if selected {
                app.theme.help_title
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![Span::styled(label, style)]));
        }
        lines.push(Line::from(Span::raw("")));
        row += entry.fields.len();
    }

    let scroll = if ps.selected > 8 {
        (ps.selected.saturating_sub(8)) as u16
    } else {
        0
    };
    let list = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border)
                .title(Span::styled(" Plugin settings ", app.theme.help_title))
                .style(app.theme.popup_style()),
        )
        .scroll((scroll, 0));
    f.render_widget(list, outer[0]);

    let help = match (&ps.message, ps.editing.is_some()) {
        (Some(msg), _) => msg.clone(),
        (None, true) => "Enter save · Esc cancel".to_string(),
        (None, false) => "↑↓ move · Enter edit/toggle · Esc close".to_string(),
    };
    let footer =
        Paragraph::new(Line::from(Span::raw(format!(" {help}")))).style(app.theme.popup_style());
    f.render_widget(footer, outer[1]);
}

pub(crate) fn render_error_overlay(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    err: &ErrorState,
) {
    // doesn't draw a giant empty modal.
    let body_lines = err.body.lines().count().max(1) as u16;
    let popup_w = (full.width as f32 * 0.8) as u16;
    let popup_h = (body_lines + 4).min((full.height as f32 * 0.7) as u16);
    let x = (full.width.saturating_sub(popup_w)) / 2;
    let y = (full.height.saturating_sub(popup_h)) / 2;
    let area = Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    };
    f.render_widget(Clear, area);

    let lines: Vec<Line<'_>> = err
        .body
        .lines()
        .map(|l| Line::from(Span::raw(l.to_string())))
        .collect();

    let title = format!(" ✕ {} · press any key to dismiss ", err.title);
    let widget = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.status_message)
                .title(Span::styled(title, app.theme.status_message)),
        )
        .style(app.theme.popup_style())
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(widget, area);
}

pub(crate) fn render_command_bar(
    f: &mut ratatui::Frame<'_>,
    full: Rect,
    app: &App,
    c: &CommandState,
) {
    let h = 3u16;
    let area = Rect {
        x: full.x,
        y: full.y + full.height.saturating_sub(h),
        width: full.width,
        height: h,
    };
    f.render_widget(Clear, area);
    let line = Line::from(vec![
        Span::styled(" : ", app.theme.help_title),
        Span::raw(c.buffer.clone()),
        Span::styled("▏", app.theme.cursor_caret),
    ]);
    let bar = Paragraph::new(line)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(app.theme.border),
        )
        .style(app.theme.popup_style());
    f.render_widget(bar, area);
}

/// Tab titles for the help popup, in the order they appear. The
/// `App.help_tab` index points into this slice (saturating, so an
/// out-of-range value clamps to the last tab).
pub(crate) const HELP_TABS: &[&str] =
    &["Normal", "Insert", "Visual", "Sidebar", "Overlays", "Dates"];

pub(crate) fn render_help_popup(f: &mut ratatui::Frame<'_>, full: Rect, app: &App) {
    let popup_w = (full.width as f32 * 0.7) as u16;
    let popup_h = 28u16.min(full.height.saturating_sub(2));
    let x = (full.width.saturating_sub(popup_w)) / 2;
    let y = (full.height.saturating_sub(popup_h)) / 2;
    let area = Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    };
    f.render_widget(Clear, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3)])
        .split(area);

    let tab = app.help_tab.min(HELP_TABS.len() - 1);
    let tabs = Tabs::new(
        HELP_TABS
            .iter()
            .map(|t| Line::from(format!(" {t} ")))
            .collect::<Vec<_>>(),
    )
    .select(tab)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(app.theme.border)
            .title(Span::styled(
                " Help · h/l tabs · j/k scroll · PgUp/PgDn page · g/G top/end · ? close ",
                app.theme.help_title,
            )),
    )
    .style(
        app.theme
            .popup_style()
            .fg(app.theme.dim.fg.unwrap_or(ratatui::style::Color::Gray)),
    )
    .highlight_style(app.theme.list_selected)
    .divider(Span::styled("│", app.theme.dim));
    f.render_widget(tabs, chunks[0]);

    let body = help_tab_body(tab, &app.theme);
    let body_len = body.len() as u16;
    // Inner height = block area minus the 2 border rows.
    let inner_h = chunks[1].height.saturating_sub(2);
    // Clamp the requested scroll against the actual body so `G` /
    // PgDn don't park the user past the end of the content.
    let max_scroll = body_len.saturating_sub(inner_h);
    let scroll = app.help_scroll.min(max_scroll);

    // Title carries a scroll indicator when the content overflows —
    // gives the user a visual cue that there's more below / above.
    let title = if body_len > inner_h {
        format!(" {} · ↕ {}/{} ", HELP_TABS[tab], scroll + 1, max_scroll + 1)
    } else {
        format!(" {} ", HELP_TABS[tab])
    };
    let popup = Paragraph::new(body)
        .block(
            Block::default()
                .borders(Borders::ALL)
                // Highlight the border so it reads as "this owns focus"
                // — the dim outline border behind would otherwise
                // suggest the popup is informational.
                .border_style(app.theme.heading)
                .title(Span::styled(title, app.theme.help_title)),
        )
        .style(app.theme.popup_style())
        .scroll((scroll, 0))
        .wrap(ratatui::widgets::Wrap { trim: false });
    f.render_widget(popup, chunks[1]);
}

/// The help popup's rows for one tab.
///
/// Takes the `Theme` rather than the whole `App`: styling was all it
/// ever needed, and a bare theme lets a test assert the text without
/// constructing a workspace. The rows are hand-curated (grouped and
/// worded for the terminal) rather than generated from
/// `outl-shortcuts`, so a chord added to the catalog does NOT appear
/// here on its own. The guard test at the bottom of this file is what
/// keeps the reminder chords from silently dropping out again.
fn help_tab_body(tab: usize, theme: &Theme) -> Vec<Line<'static>> {
    match HELP_TABS.get(tab).copied().unwrap_or("Normal") {
        "Normal" => vec![
            Line::from(Span::styled("Editing", theme.help_title)),
            Line::from("  i           edit current block"),
            Line::from("  I           edit, cursor at start of block"),
            Line::from("  o / O       new block below / above"),
            Line::from("  Tab / S-Tab indent / outdent"),
            Line::from("  K / J       move block up / down (Alt+↑/↓ too)"),
            Line::from("  dd          delete block"),
            Line::from("  yy / p / P  yank · paste after · paste before"),
            Line::from("  Ctrl+T      cycle TODO / DOING / DONE / none"),
            Line::from("  c           fold / unfold the selected block"),
            Line::from("              (▼ expanded · ▶ collapsed · synced via op log)"),
            Line::from("  u / Ctrl+R  undo / redo"),
            Line::from("  g P         toggle pinned:: on this page (chord)"),
            Line::from(""),
            Line::from(Span::styled("Navigation", theme.help_title)),
            Line::from("  j/k ↑↓      move between blocks"),
            Line::from("  PgDn/PgUp   one viewport"),
            Line::from("  Ctrl+D / U  half-page"),
            Line::from("  g g / G     first / last block"),
            Line::from("  h/l ←→      cursor inside the current block"),
            Line::from("  w / b       next / previous word"),
            Line::from("  0 / $       start / end of block"),
            Line::from("  Enter       open [[ref]] / #tag / journal under cursor"),
            Line::from(""),
            Line::from(Span::styled("Journal & workspace", theme.help_title)),
            Line::from("  t           today's journal"),
            Line::from("  [ / ]       previous / next journal"),
            Line::from("  g j         jump to today"),
            Line::from("  g x         run code block under cursor (also `:run`)"),
            Line::from("  Ctrl+S      force save"),
            Line::from("  Ctrl+L      reload workspace from disk"),
            Line::from("  B           toggle inline backlinks"),
            Line::from("  \\           toggle left sidebar (opens with focus on Pinned)"),
            Line::from("  q q         quit (chord)"),
            Line::from(""),
            Line::from(Span::styled("Properties (key:: value)", theme.help_title)),
            Line::from("  g p         open the property editor for this block"),
            Line::from("              j/k move · Enter edit · o add · dd delete"),
            Line::from("              p switches between block and page properties"),
            Line::from("              Tab completes the key from the workspace"),
            Line::from("              [[ picks a page in the value, # a tag"),
            Line::from("  :prop <key> <value>        set / clear on the block"),
            Line::from("  :prop-page <key> <value>   set / clear on the page"),
            Line::from(""),
            Line::from(Span::styled("Reminders", theme.help_title)),
            Line::from("  g r         add remind:: to this block"),
            Line::from("  g R         nag me (now every 1h until DONE)"),
            Line::from("  g n         open the reminders list"),
            Line::from("  g s         snooze this block 1h (every device)"),
            Line::from("  :prop remind <rule>   edit or clear the rule"),
        ],
        "Insert" => vec![
            Line::from(Span::styled("Commit / cancel", theme.help_title)),
            Line::from("  Esc         commit (write buffer → AST → disk)"),
            Line::from("  Enter       commit + new block below"),
            Line::from(""),
            Line::from(Span::styled("Block ops (stay in Insert)", theme.help_title)),
            Line::from("  Tab / S-Tab indent / outdent"),
            Line::from("  Ctrl+T      cycle TODO / DOING / DONE / none"),
            Line::from(""),
            Line::from(Span::styled("Text editing", theme.help_title)),
            Line::from("  chars       insert at cursor"),
            Line::from("  Backspace   delete previous (deletes block if empty)"),
            Line::from("  arrows/home/end   move cursor"),
            Line::from("  ( [ {       auto-pair with matching close"),
            Line::from(""),
            Line::from(Span::styled("Autocomplete", theme.help_title)),
            Line::from("  [[          page-ref picker"),
            Line::from("  #           tag picker"),
            Line::from("  /           slash command picker"),
        ],
        "Visual" => vec![
            Line::from(Span::styled("Selection", theme.help_title)),
            Line::from("  V           enter Visual (Normal mode)"),
            Line::from("  j / k       extend selection"),
            Line::from("  Esc         cancel"),
            Line::from(""),
            Line::from(Span::styled("Batch ops on the range", theme.help_title)),
            Line::from("  d / x       delete selected blocks"),
            Line::from("  y           yank selected blocks"),
            Line::from("  Tab / S-Tab indent / outdent the range"),
        ],
        "Sidebar" => vec![
            Line::from(Span::styled("Open / close", theme.help_title)),
            Line::from("  \\           toggle sidebar (opens with focus on Pinned)"),
            Line::from("  Esc         return focus to the outline (sidebar stays open)"),
            Line::from(""),
            Line::from(Span::styled("Inside the sidebar", theme.help_title)),
            Line::from("  j / k ↑↓    move between items in the focused section"),
            Line::from("  g / G       first / last item"),
            Line::from("  Tab / S-Tab cycle sections (Pinned → Recent → Calendar)"),
            Line::from("  Enter       open the highlighted page or journal"),
            Line::from(""),
            Line::from(Span::styled("Sections", theme.help_title)),
            Line::from("  📅 Calendar  current month — journals marked with ●"),
            Line::from("  ⭐ Pinned    pages with `pinned:: true` property"),
            Line::from("              (toggle with `g P` chord in Normal, or `/pin`)"),
            Line::from("  🕘 Recent    pages opened this session (LRU, cap 20)"),
        ],
        "Overlays" => vec![
            Line::from(Span::styled("Open", theme.help_title)),
            Line::from("  Ctrl+P      quick switcher (pages + journals, with preview)"),
            Line::from("  /           slash command menu (Notion-style)"),
            Line::from("  :           vim-style palette (same registry as /)"),
            Line::from("  ?           toggle this help"),
            Line::from(""),
            Line::from(Span::styled("Inside an overlay", theme.help_title)),
            Line::from("  ↑↓ j k      navigate candidates"),
            Line::from("  Enter       accept / run / open"),
            Line::from("  Esc         dismiss"),
            Line::from(""),
            Line::from(Span::styled("Search hits", theme.help_title)),
            Line::from("  n / N       next / previous hit (after `/` is closed)"),
        ],
        "Dates" => vec![
            Line::from(Span::styled("Insert-mode slash commands", theme.help_title)),
            Line::from("  /date-today          [[YYYY-MM-DD]]  (also /dt, /dtm, /dy)"),
            Line::from("  /date-next-monday    next Monday's journal ref"),
            Line::from("                       (one alias per weekday)"),
            Line::from("  /date +3d            offset: +Nd, -Nw, +Nm  or absolute YYYY-MM-DD"),
            Line::from("  /time-now            HH:MM, plain (no brackets)"),
            Line::from("  /datetime-now        [[YYYY-MM-DD]] HH:MM  (alias /stamp)"),
            Line::from("  /iso-date-today      YYYY-MM-DD, no brackets (for `due::` etc)"),
            Line::from("  /week-num            #YYYY-Www  (ISO week as a tag)"),
            Line::from(""),
            Line::from(Span::styled(format!("theme: {}", theme.name), theme.dim)),
        ],
        _ => vec![Line::from("  (no content for this tab)")],
    }
}

#[cfg(test)]
mod help_coverage_tests {
    //! The help popup is a hand-curated list, not generated from
    //! `outl-shortcuts`. That buys a layout worded for the terminal
    //! and costs the guarantee that a new chord shows up on its own:
    //! `g r` / `g R` / `g n` / `g s` shipped working and undiscoverable
    //! because nobody edited this file.
    //!
    //! These pin the reminder rows. The chord *spelling* is guarded
    //! separately against the catalog in `input/normal.rs`, so between
    //! the two a re-spelled or a dropped chord fails a test.

    use super::help_tab_body;
    use crate::theme::default_theme;

    fn normal_help() -> String {
        let theme = default_theme();
        help_tab_body(0, &theme)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_reminder_chord_is_listed() {
        let help = normal_help();
        for chord in ["g r", "g R", "g n", "g s"] {
            assert!(
                help.contains(chord),
                "`{chord}` works but isn't in the help; the popup is \
                 hand-written, so adding a chord means editing it too"
            );
        }
    }

    #[test]
    fn the_property_editor_and_its_keys_are_listed() {
        // `g p` used to mean "pin"; the re-spelling only pays off if
        // both the new chord and the moved one are findable here.
        // And an overlay whose inner keys aren't documented is a
        // list the user can open and not operate.
        let help = normal_help();
        for chord in ["g p", "g P", "dd delete", "Tab completes"] {
            assert!(
                help.contains(chord),
                "`{chord}` works but isn't in the help; the popup is \
                 hand-written, so adding a chord means editing it too"
            );
        }
    }

    #[test]
    fn the_help_says_how_to_edit_a_rule() {
        // `g r` writes a starter rule the user almost always tunes.
        // Without this line the only way to find `:prop` is the toast
        // that scrolls away.
        assert!(normal_help().contains(":prop remind"));
    }

    #[test]
    fn the_first_tab_is_the_one_these_chords_live_on() {
        // `normal_help()` hardcodes tab 0; if the tab order ever
        // changes these assertions would silently test the wrong list.
        assert_eq!(super::HELP_TABS.first().copied(), Some("Normal"));
    }
}
