//! Outline rendering — turn a `ParsedPage` (current view) into a
//! flat `Vec<Line>` for ratatui, with selection / cursor / TODO
//! decoration.

use crate::outline_ops::path_for_index;
use crate::state::{App, Focus, Mode};
use crate::theme::Theme;
use crate::view::embed::{embed_only_handle, emit_embedded_children};
use crate::view::inline::{highlight_inline, render_markdown_inline, render_pretty_block_text};
use crate::view::wrap::push_wrapped;
use outl_md::inline::byte_index_for_char;
use outl_md::parse::{OutlineNode, ParsedPage};
use outl_md::view::{block_to_rows, BlockRowKind};
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::view::row_chrome::{push_body_indent, push_property_row, FoldMarker, AUTO_RUN_GLYPH};

/// Render the outline into a flat list of `Line`s for ratatui, and
/// report the visual line index where the *selected* block's bullet
/// row landed. The caller uses that index to keep the selection
/// inside the scrolled viewport.
pub(crate) fn render_outline(
    p: &ParsedPage,
    app: &App,
    text_width: u16,
) -> (Vec<Line<'static>>, Option<usize>, Vec<(usize, usize)>) {
    let mut out = Vec::new();
    for (k, v) in &p.properties {
        out.push(Line::from(vec![
            Span::styled(format!("{k}:: "), app.theme.property_key),
            Span::styled(v.clone(), app.theme.property_value),
        ]));
    }
    if !p.properties.is_empty() && !p.blocks.is_empty() {
        out.push(Line::from(""));
    }
    let mut cursor = 0usize;
    let mut selected_line: Option<usize> = None;
    // `block_starts` records `(first visual line, flat index)` for each
    // block as it's emitted, in DFS order with ascending start lines, so
    // a mouse click can resolve a screen row back to the block it landed
    // on (see `App::block_at_visual_line`).
    let mut block_starts: Vec<(usize, usize)> = Vec::new();
    // Zoom (Roam/Workflowy): when the user has zoomed into a block, draw
    // only that block's subtree. We render the single root node instead
    // of every top-level block; `cursor` still counts from 0 in whole-
    // page DFS order (advanced through the skipped prefix first) so
    // `selected` / `id_by_flat` / `block_starts` keep their whole-page
    // indices — the zoom is a render window, not a re-indexing.
    if let Some((root, root_index)) = app.zoom_root_node() {
        cursor = root_index;
        render_block(
            root,
            0,
            &mut cursor,
            app,
            &mut out,
            &mut selected_line,
            &mut block_starts,
            text_width,
        );
    } else {
        for block in &p.blocks {
            render_block(
                block,
                0,
                &mut cursor,
                app,
                &mut out,
                &mut selected_line,
                &mut block_starts,
                text_width,
            );
        }
    }
    (out, selected_line, block_starts)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_block(
    b: &OutlineNode,
    indent: u32,
    cursor: &mut usize,
    app: &App,
    out: &mut Vec<Line<'static>>,
    selected_line: &mut Option<usize>,
    block_starts: &mut Vec<(usize, usize)>,
    text_width: u16,
) {
    // This block's first visual row is wherever `out` is right now —
    // record it against the flat index before emitting any line.
    block_starts.push((out.len(), *cursor));
    // Outline only owns selection/cursor decoration when focus lives
    // here. With `Focus::Backlink`, the bullet/caret belong to the
    // backlinks section — drawing them on the outline too would leave
    // a "ghost cursor" on the last outline block (the value `selected`
    // still happens to point at).
    let focused_on_outline = matches!(app.focus, Focus::Outline);
    let is_selected = focused_on_outline && *cursor == app.selected;
    // Record the visual line where this block's bullet row begins so
    // the caller can scroll the viewport to keep it visible.
    if is_selected && selected_line.is_none() {
        *selected_line = Some(out.len());
    }
    let in_visual_range = focused_on_outline
        && app
            .visual_range()
            .is_some_and(|(lo, hi)| *cursor >= lo && *cursor <= hi);
    let editing_here = focused_on_outline
        && matches!(&app.mode, Mode::Insert { block_path, .. }
            if path_for_index(&app.page.blocks, *cursor).as_deref() == Some(block_path.as_slice()));

    let bullet_style = if is_selected || in_visual_range {
        app.theme.selected_bullet
    } else {
        app.theme.bullet
    };

    // The block's stable id (needed both for the collapsed lookup
    // below and the content-transformer cache lookup in the `mode`
    // decision). `None` on a sidecar gap — no id, no cached transform.
    let block_id = app.id_by_flat.get(*cursor).copied();

    // Determine which text and cursor position to render. Four cases:
    //   1. Editing here       → buffer with caret cursor (raw fence).
    //   2. Selected in Normal → block text with block-style cursor (raw).
    //   3. Plugin-transformed → cached transformer output (read-only).
    //   4. Anything else      → block text, no cursor, pretty render.
    //
    // The cursor cases (1, 2) always win over a cached transform: a
    // block under the cursor shows its real fence source so the user
    // edits what they see. Only a read-only block swaps in the
    // transformer output.
    let mode = if editing_here {
        if let Mode::Insert { buffer, .. } = &app.mode {
            RenderMode::Editing {
                text: buffer.as_string(),
                cursor_char: buffer.cursor,
            }
        } else {
            unreachable!("editing_here matched but mode isn't Insert")
        }
    } else if is_selected && matches!(app.mode, Mode::Normal) {
        RenderMode::NormalCursor {
            text: b.text.clone(),
            cursor_char: app.cursor_col,
        }
    } else if let Some(content) = block_id.and_then(|id| app.transform_cache.get(&id)) {
        RenderMode::Transformed {
            content: content.clone(),
        }
    } else {
        RenderMode::Pretty {
            text: b.text.clone(),
        }
    };

    // Fold indicator for the bullet row.
    //   - `▼ ` when the block has children and is expanded
    //   - `▶ ` when it has children and is collapsed
    //   - `  ` (two spaces) when it has no children — keeps column
    //     alignment with the other two cases so the bullet column
    //     never jitters across blocks on the same indent.
    let is_collapsed = block_id
        .map(|id| app.collapsed.contains(&id))
        .unwrap_or(false);
    let has_children = !b.children.is_empty();
    let fold_marker = match (has_children, is_collapsed) {
        (false, _) => FoldMarker::None,
        (true, false) => FoldMarker::Expanded,
        (true, true) => FoldMarker::Collapsed,
    };

    let has_auto_run = b.properties.iter().any(|(k, _)| k == "auto-run");
    emit_block_lines(
        indent,
        bullet_style,
        &mode,
        has_auto_run,
        fold_marker,
        app,
        out,
        text_width,
    );

    for (k, v) in &b.properties {
        push_property_row(indent, k, v, has_auto_run, app, out, text_width);
    }

    // Expand `!((blk-XXXXXX))` embeds as a read-only subtree under
    // the carrying block. Triggered when:
    //   - the block's text resolves to a single Embed token
    //     (mixed prose keeps the inline `↳ <text>` render);
    //   - the handle resolves through the workspace index.
    // The expanded rows are visual-only — they don't move `cursor`
    // (the flat index used for navigation), so `j` / `k` cross them
    // in one step instead of paging through borrowed content. The
    // carrying block's own row keeps whatever render `mode` chose
    // (raw with cursor, raw with caret, or pretty) so column-to-byte
    // alignment is never broken by the expansion.
    if let Some(handle) = embed_only_handle(&b.text) {
        if let Some(entry) = app.index.resolve_block_ref(handle) {
            // `outer_indent` matches the carrying block's own indent so
            // the `│ ` guides line up with the outline's normal indent
            // pattern. Embed-internal nesting comes from `depth`.
            emit_embedded_children(&entry.children, indent, 1, app, out, text_width);
        }
    }

    *cursor += 1;
    if is_collapsed {
        // Children are hidden — but the flat cursor still has to
        // skip past them because `App.selected` and friends index
        // the full DFS preorder (collapsed or not). Without this
        // bump, selection bookkeeping for blocks *below* the
        // collapsed subtree would shift up by `flat_count(children)`.
        *cursor += outl_md::outline_ops::flat_count(&b.children);
    } else {
        for child in &b.children {
            render_block(
                child,
                indent + 1,
                cursor,
                app,
                out,
                selected_line,
                block_starts,
                text_width,
            );
        }
    }
}

/// Where the cursor sits on a block being rendered, and what style
/// the renderer should use for it. The UI-agnostic decomposition
/// lives in [`outl_md::view`]; this enum carries the *TUI-flavored*
/// detail of "caret vs block cursor".
pub(crate) enum RenderMode {
    /// Insert mode — show the live buffer with the caret on
    /// `cursor_char`. Markdown is rendered raw so columns match bytes.
    Editing { text: String, cursor_char: usize },
    /// Normal mode on the selected block — show a vim-style block
    /// cursor on the character under `cursor_char`. Raw render.
    NormalCursor { text: String, cursor_char: usize },
    /// Anything else — markdown is rendered prettily; no cursor.
    Pretty { text: String },
    /// A read-only block whose code fence a plugin content-transformer
    /// turned into text/markdown. `content` replaces the raw fence in
    /// the outline; the bullet stays so the block is still anchored.
    /// Never carries a cursor — the cursor cases render the raw fence so
    /// the user edits the real source. Multi-line `content` becomes a
    /// bullet row plus continuation rows.
    Transformed { content: String },
}

/// Emit one or more ratatui [`Line`]s for a block's text.
///
/// Decomposition into visual rows (bullet vs continuation vs code
/// fence marker vs code fence body) is delegated to
/// [`outl_md::view::block_to_rows`] so the Tauri GUI and mobile
/// clients use the same classification. This function is the
/// TUI-specific mapping: each [`outl_md::view::BlockRow`] becomes a
/// `Line` of `Span`s using the active theme.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_block_lines(
    indent: u32,
    bullet_style: Style,
    mode: &RenderMode,
    has_auto_run: bool,
    fold: FoldMarker,
    app: &App,
    out: &mut Vec<Line<'static>>,
    text_width: u16,
) {
    let (text, cursor_char, cursor_style) = match mode {
        RenderMode::Editing { text, cursor_char } => {
            (text.as_str(), Some(*cursor_char), Some(CursorStyle::Caret))
        }
        RenderMode::NormalCursor { text, cursor_char } => {
            (text.as_str(), Some(*cursor_char), Some(CursorStyle::Block))
        }
        RenderMode::Pretty { text } => (text.as_str(), None, None),
        // Transformer output renders as pretty markdown — same styling
        // path as `Pretty`, just sourced from the cached `content`
        // instead of the raw fence text.
        RenderMode::Transformed { content } => (content.as_str(), None, None),
    };
    let pretty = matches!(
        mode,
        RenderMode::Pretty { .. } | RenderMode::Transformed { .. }
    );
    let rows = block_to_rows(text, indent, cursor_char);

    // TODO/DONE checkbox decoration only fits on single-line bullets
    // (multi-line ones would have the icon floating above body text).
    let single_line_pretty = pretty && rows.len() == 1;

    for row in &rows {
        // The line is built in three parts so word-wrap can keep the
        // prefix on the first visual row and re-indent continuations:
        //   - `guides`  : the `│ ` indent rails (repeated on every wrap row)
        //   - `head`    : fold marker + bullet (first wrap row only)
        //   - `content` : the styled block text that may wrap
        let mut guides: Vec<Span<'static>> = Vec::new();
        for _ in 0..row.indent {
            guides.push(Span::styled("│ ", app.theme.dim));
        }
        let mut head: Vec<Span<'static>> = Vec::new();
        match row.kind {
            BlockRowKind::Bullet => {
                // Fold indicator goes first — two-cell slot whether
                // the marker is visible or not. Keeps the bullet `-`
                // column stable across siblings (leaf next to a
                // parent must line up).
                match fold {
                    FoldMarker::None => head.push(Span::raw("  ")),
                    FoldMarker::Expanded => head.push(Span::styled("▼ ", app.theme.dim)),
                    FoldMarker::Collapsed => head.push(Span::styled("▶ ", app.theme.hint)),
                }
                // Blocks with `auto-run::` get a ⚡ before the bullet
                // so the user can see at a glance which cells re-run
                // themselves on page open.
                if has_auto_run {
                    head.push(Span::styled(AUTO_RUN_GLYPH, app.theme.hint));
                }
                head.push(Span::styled("- ", bullet_style));
            }
            BlockRowKind::Continuation
            | BlockRowKind::CodeFenceMarker
            | BlockRowKind::CodeFenceBody => {
                push_body_indent(&mut head, has_auto_run);
            }
        }

        let mut content: Vec<Span<'static>> = Vec::new();
        // If the cursor is on this row we always go raw — we want
        // bytes to line up with what the user typed, regardless of
        // fence state.
        let mut cursor_cell: Option<Style> = None;
        if let (Some(col), Some(style)) = (row.cursor_col, cursor_style) {
            cursor_cell = Some(emit_row_with_cursor(
                row.text,
                col,
                style,
                &app.theme,
                &mut content,
            ));
        } else {
            // A bullet row whose text opens a code fence (`` ```lisp ``)
            // is *both* a bullet and a fence marker — style the text
            // dimly so the fence reads visually like the rest of the
            // code block while keeping the `- ` glyph emitted above.
            let bullet_is_fence_opener = matches!(row.kind, BlockRowKind::Bullet)
                && row.text.trim_start().starts_with("```");
            match row.kind {
                _ if pretty && bullet_is_fence_opener => {
                    content.push(Span::styled(row.text.to_string(), app.theme.dim));
                }
                BlockRowKind::CodeFenceMarker if pretty => {
                    content.push(Span::styled(row.text.to_string(), app.theme.dim));
                }
                BlockRowKind::CodeFenceBody if pretty => {
                    content.push(Span::styled(row.text.to_string(), app.theme.code));
                }
                BlockRowKind::Bullet if single_line_pretty => {
                    // Single owner for the bullet's pretty render: it
                    // strips TODO/DONE + `"> "` markers in either
                    // order, paints the `│ ` quote bar and the
                    // `☐`/`☑` checkbox, then tokenises the body. Same
                    // function the embed expansion uses, so the
                    // chrome stays in lockstep between bullet and
                    // embed root.
                    content.extend(render_pretty_block_text(row.text, &app.theme, &app.index));
                }
                _ => content.extend(render_markdown_inline(row.text, &app.theme, &app.index)),
            }
        }

        // Cursor rows wrap too (#99). The cursor is already baked into
        // `content` as a styled span by `emit_row_with_cursor` *before*
        // we wrap, so reflowing the spans just carries the cursor onto
        // its wrapped visual row — the char offset was already consumed
        // turning it into a span, there's nothing left to desync. A
        // `text_width` of 0 (headless render) is still the "don't wrap"
        // sentinel for every row, cursor or not.
        push_wrapped(guides, head, content, text_width, cursor_cell, out);
    }
}

/// Draw one row with the cursor highlighted at `col` (a char index
/// into `text`). Splits the row in three: left of cursor, the char
/// the cursor lands on (or a `▏` if past-end), right of cursor.
///
/// Returns the [`Style`] it painted the cursor cell with, which
/// [`push_wrapped`] needs to tell that cell apart from a discardable
/// separator when the row wraps on a space (#320).
fn emit_row_with_cursor(
    text: &str,
    col: usize,
    style: CursorStyle,
    theme: &Theme,
    spans: &mut Vec<Span<'static>>,
) -> Style {
    let byte = byte_index_for_char(text, col);
    let (left, right) = text.split_at(byte);
    spans.extend(highlight_inline(left, theme));
    // Neither cursor adds a cell: each paints the character it lands
    // on. Past the end there is no character to paint, so both fall
    // back to a `▏` — appended after the last cell it has nothing to
    // its right to shift.
    let (on_char, past_end) = match style {
        CursorStyle::Caret => (theme.cursor_caret_on_char(), theme.cursor_caret),
        CursorStyle::Block => (theme.cursor_block, theme.cursor_block),
    };
    let mut right_chars = right.chars();
    match right_chars.next() {
        Some(ch) => {
            spans.push(Span::styled(ch.to_string(), on_char));
            let rest: String = right_chars.collect();
            spans.extend(highlight_inline(&rest, theme));
        }
        None => {
            spans.push(Span::styled("▏", past_end));
            return past_end;
        }
    }
    on_char
}

/// Cursor visual style: `Caret` is Insert mode (see
/// `Theme::cursor_caret_on_char`),
/// `Block` the inverted single-char box on the selected block in
/// Normal mode.
#[derive(Debug, Clone, Copy)]
enum CursorStyle {
    Caret,
    Block,
}

#[cfg(test)]
mod tests;
