//! Inline backlinks section — rendered *below* the outline, separated
//! by a full-width `─` rule. Each source page contributes a header
//! followed by one row per referencing block **and that block's
//! children**, capped at `outl_actions::SUBTREE_MAX_DEPTH` levels.
//! `Backlink::source_block` carries no children by design (the #179
//! freeze fix; that field's doc owns why), so the children are read
//! from the index's side table through `App::backlink_subtree`, an
//! `Arc` bump per frame and never a deep copy.
//!
//! Selection of a backlink block (`Focus::Backlink { idx, node }`)
//! highlights the bullet the same way the outline does — same theme
//! `selected_bullet`, same cursor styles. The TUI scrolls so the
//! selection stays visible (handled by the caller in `view::render_main`).
//!
//! The list is produced by [`crate::actions::App::backlinks_for_current`]
//! — the single source for backlinks across the TUI, shared with the
//! mobile client through `outl_actions::backlinks_for_page`.

mod memo;

pub(crate) use memo::{Panel, PanelMemo};

use crate::state::{App, EditTarget, Focus, Mode};
use crate::view::inline::render_markdown_inline;
use crate::view::outline::{emit_block_lines, RenderMode};
use crate::view::row_chrome::{push_property_row, FoldMarker};
use outl_actions::{BacklinkCrumb, OutlineNode};
use outl_core::id::NodeId;
use ratatui::text::{Line, Span};

/// Do two ancestor trails name the same chain of blocks (by id)?
/// Drives breadcrumb collapse: consecutive references in the same
/// branch show the trail once. All-or-nothing on equality — mirrors
/// `sameCrumbTrail` in `@outl/shared/outline`.
fn same_trail(a: &[BacklinkCrumb], b: &[BacklinkCrumb]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.id == y.id)
}

/// The breadcrumb above a referencing block, as one dimmed row.
///
/// The crumbs render as markdown — a `[[ref]]` reads as its name and a
/// `#tag` keeps its colour — because the trail is the branch the
/// reference sits in, and that branch reads as prose on its own page.
/// Drawing it as source was the panel's one spot written in markup.
///
/// Dim is the **line** style, so `Span::raw` prose inherits it while a
/// ref or tag keeps the hue `render_markdown_inline` gave it: the row
/// stays quieter than the block below it without erasing what the
/// tokens are. Nothing here is navigable — `j`/`k` and `Enter` reach
/// the citing block, never its ancestors.
fn render_crumb_trail(ancestors: &[BacklinkCrumb], app: &App) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
    for (i, crumb) in ancestors.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" › ", app.theme.dim));
        }
        spans.extend(render_markdown_inline(
            &crumb.text,
            &app.theme,
            &app.index,
            &app.icons,
        ));
    }
    Line::from(spans).style(app.theme.dim)
}

/// Render the inline backlinks section.
///
/// What the backlinks section should say when it has no rows.
///
/// `None` means draw nothing at all. `Some(text)` means draw the
/// section with that line in place of the rows, because the reason
/// there are none is "not known yet" rather than "there are none".
///
/// The two were indistinguishable: the section returned nothing while
/// the background index was still building, so on a large workspace
/// "still loading" and "this page has no backlinks" looked identical
/// for seconds ([#448](https://github.com/outlmd/outl/issues/448)).
/// `App::has_pending_backlink_index` already answered the question and
/// no view asked it.
fn empty_section_notice(shown: bool, has_rows: bool, index_pending: bool) -> Option<&'static str> {
    if !shown || has_rows || !index_pending {
        return None;
    }
    Some(" Backlinks · indexing…")
}

/// Returns `(lines, selected_line)`. `selected_line` is the index in
/// the returned `Vec<Line>` of the bullet row of the focused backlink
/// block, or `None` when focus is on the outline. `inner_width` is the
/// drawable width of the outline panel (i.e. excluding the 2-col
/// border) so the separator rule spans the full visible width.
///
/// Empty result `(vec![], None)` when there are no backlinks for the
/// current view or the user has toggled the section off via `B`.
///
/// An owned copy of [`panel`]'s memoised lines; the frame itself reads
/// the shared ones.
#[cfg(test)]
pub(crate) fn render_backlinks_inline(
    app: &App,
    inner_width: u16,
) -> (Vec<Line<'static>>, Option<usize>) {
    let p = panel(app, inner_width);
    (p.lines.to_vec(), p.selected)
}

/// The panel's lines, rebuilt only when something they read changed.
///
/// Every keystroke redraws, and a hub page's panel is ~850 citing rows
/// plus ~3.5k subtree nodes: building it per frame re-tokenised all of
/// them while the user typed in the outline above (root invariant 15).
/// See [`memo`] for the key and for what it deliberately does not cover.
pub(crate) fn panel(app: &App, inner_width: u16) -> Panel {
    if !app.show_backlinks || !app.page_sections_visible() {
        return Panel::empty();
    }
    let backlinks = app.backlinks_for_current();
    if backlinks.is_empty() {
        return Panel::from_built(build_empty_panel(app, inner_width));
    }
    memo::cached_or_build(app, inner_width, &backlinks, || {
        build_panel(app, inner_width, &backlinks)
    })
}

/// [`render_backlinks_inline`] without the memo: the old full render,
/// the reference the memo's equivalence tests compare against.
#[cfg(test)]
pub(crate) fn render_backlinks_uncached(
    app: &App,
    inner_width: u16,
) -> (Vec<Line<'static>>, Option<usize>) {
    build_panel(app, inner_width, &app.backlinks_for_current())
}

/// The section for zero rows: nothing, or the "indexing…" notice.
fn build_empty_panel(app: &App, inner_width: u16) -> (Vec<Line<'static>>, Option<usize>) {
    // Zero rows has two causes, and they used to render the same.
    let Some(notice) =
        empty_section_notice(app.show_backlinks, false, app.has_pending_backlink_index())
    else {
        return (Vec::new(), None);
    };
    let rule = "─".repeat(inner_width.max(1) as usize);
    (
        vec![
            Line::from(""),
            Line::from(Span::styled(rule, app.theme.border)),
            Line::from(Span::styled(notice, app.theme.dim)),
        ],
        None,
    )
}

/// The uncached build: every row, every child. Kept separate so the
/// memo's equivalence test has the old full render to compare against.
fn build_panel(
    app: &App,
    inner_width: u16,
    backlinks: &[outl_actions::Backlink],
) -> (Vec<Line<'static>>, Option<usize>) {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut selected_line: Option<usize> = None;

    // Full-width rule and section header. The blank line above the
    // rule isolates the section visually from the outline.
    out.push(Line::from(""));
    let rule = "─".repeat(inner_width.max(1) as usize);
    out.push(Line::from(Span::styled(rule, app.theme.border)));
    // Header carries the direction indicator + the toggle hint (issue
    // #142): ↓ = newest on top (default), ↑ = oldest on top. `Ctrl+O`
    // flips it.
    let (arrow, order_label) = if app.backlinks_newest_first {
        ("↓", "newest")
    } else {
        ("↑", "oldest")
    };
    out.push(Line::from(vec![
        Span::styled(
            format!(" Backlinks · {} ref(s)  ", backlinks.len()),
            app.theme.heading,
        ),
        Span::styled(format!("{arrow} {order_label}"), app.theme.dim),
        Span::styled("  (^O)", app.theme.dim),
    ]));
    out.push(Line::from(""));

    let mut prev_source: Option<String> = None;
    for (idx, bl) in backlinks.iter().enumerate() {
        let source_slug = bl
            .source_page
            .as_ref()
            .map(|p| p.slug.as_str())
            .unwrap_or("");
        let source_title = bl
            .source_page
            .as_ref()
            .map(|p| p.title.as_str())
            .unwrap_or("");
        let source_icon = bl.source_page.as_ref().and_then(|p| p.icon.as_deref());
        // Header per source page. Multiple backlinks from the same
        // page collapse under one header.
        if prev_source.as_deref() != Some(source_slug) {
            if prev_source.is_some() {
                out.push(Line::from(""));
            }
            let header = match source_icon {
                Some(icon) => format!("{icon}  {source_title}"),
                None => format!("{}  {source_title}", app.icons.file),
            };
            out.push(Line::from(Span::styled(header, app.theme.heading)));
            prev_source = Some(source_slug.to_string());
        }

        // Breadcrumb of ancestor blocks as dimmed context, so a
        // reference buried in a nested outline reads with its branch.
        // Collapsed against the previous entry: consecutive references
        // in the same branch show the trail once (ancestor ids are
        // globally unique, so a trail never matches across pages).
        let prev = idx.checked_sub(1).and_then(|i| backlinks.get(i));
        let show_crumbs = !bl.ancestors.is_empty()
            && prev.is_none_or(|p| !same_trail(&p.ancestors, &bl.ancestors));
        if show_crumbs {
            out.push(render_crumb_trail(&bl.ancestors, app));
        }

        // The block the cursor names, when it is on this row: the
        // citing block, or the descendant `node` picked out of its
        // subtree. An id, because the subtree arrives by `Arc` and a
        // path would go stale on every refetch.
        let focus_id = match &app.focus {
            Focus::Backlink { idx: fidx, node } if *fidx == idx => {
                node.or_else(|| crate::actions::node_id(&bl.block_id))
            }
            _ => None,
        };

        render_backlink_node(
            &bl.source_block,
            0,
            focus_id,
            app,
            &mut out,
            &mut selected_line,
            inner_width,
        );
        // The children come off the index's side table, a refcount
        // bump per row: the cloned `Backlink` above is shallow on
        // purpose, and a deep copy here would run every frame.
        if let Some(children) = app.backlink_subtree(&bl.block_id) {
            render_backlink_children(
                &children,
                1,
                focus_id,
                app,
                &mut out,
                &mut selected_line,
                inner_width,
            );
        }
    }

    (out, selected_line)
}

/// Draw `nodes` and everything under them, `indent` levels in.
fn render_backlink_children(
    nodes: &[OutlineNode],
    indent: u32,
    focus_id: Option<NodeId>,
    app: &App,
    out: &mut Vec<Line<'static>>,
    selected_line: &mut Option<usize>,
    text_width: u16,
) {
    for node in nodes {
        render_backlink_node(node, indent, focus_id, app, out, selected_line, text_width);
        render_backlink_children(
            &node.children,
            indent + 1,
            focus_id,
            app,
            out,
            selected_line,
            text_width,
        );
    }
}

/// Emit one block of a backlink's mini-outline: highlighted when it is
/// the block the cursor names, and drawn as an in-place edit buffer
/// when it is the block being edited (cross-page Insert through
/// `EditTarget::SourcePage`).
fn render_backlink_node(
    node: &OutlineNode,
    indent: u32,
    focus_id: Option<NodeId>,
    app: &App,
    out: &mut Vec<Line<'static>>,
    selected_line: &mut Option<usize>,
    text_width: u16,
) {
    let is_focused = focus_id.is_some() && focus_id == crate::actions::node_id(&node.id);
    if is_focused && selected_line.is_none() {
        *selected_line = Some(out.len());
    }

    let bullet_style = if is_focused {
        app.theme.selected_bullet
    } else {
        app.theme.bullet
    };

    // Is this exact block the one the user is editing in place? One id
    // comparison, because `EditTarget::SourcePage` names the block
    // itself. It used to reconstruct an absolute path out of
    // `bl.source_block_path` + `current_path` and compare that against
    // `Mode::Insert.block_path` — a second coordinate system, and one
    // that answered "yes" for whichever block shared the path.
    let editing_here = is_focused
        && matches!(
            &app.mode,
            Mode::Insert {
                target: EditTarget::SourcePage { block, .. },
                ..
            } if Some(*block) == crate::actions::node_id(&node.id)
        );

    // Source block carries the body without the TODO/DONE prefix —
    // reattach it so the outline renderer shows the same checkbox
    // decoration the main outline uses on the source page.
    let raw_text = match node.todo {
        Some(state) => format!("{} {}", state.as_str(), node.text),
        None => node.text.clone(),
    };

    let mode = if editing_here {
        if let Mode::Insert { buffer, .. } = &app.mode {
            RenderMode::Editing {
                text: buffer.as_string(),
                cursor_char: buffer.cursor,
            }
        } else {
            unreachable!("editing_here matched but mode isn't Insert")
        }
    } else if is_focused && matches!(app.mode, Mode::Normal) {
        RenderMode::NormalCursor {
            text: raw_text.clone(),
            cursor_char: app.cursor_col,
        }
    } else {
        RenderMode::Pretty {
            text: raw_text.clone(),
        }
    };

    let has_auto_run = node.properties.iter().any(|(k, _)| k == "auto-run");
    // Backlinks render a *projection* of a source block — fold state
    // belongs to the source page's outline, not here. Always pass
    // `None` so the layout stays flush.
    emit_block_lines(
        indent,
        bullet_style,
        &mode,
        has_auto_run,
        FoldMarker::None,
        app,
        out,
        text_width,
    );

    for (k, v) in &node.properties {
        push_property_row(indent, k, v, has_auto_run, app, out, text_width);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod empty_section_tests;
