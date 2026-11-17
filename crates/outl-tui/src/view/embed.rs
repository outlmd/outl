//! Embed expansion — the read-only subtree an outline block draws
//! below itself when its text is a single embed token, in either
//! spelling: `!((blk-XXXXXX))` (one block and its subtree) or
//! `![[page]]` (a whole page's outline, issue #190).
//!
//! Split out of `view::outline` because it answers a question of its
//! own (what does *borrowed* content look like), and the outline
//! module is the one every other view concern already grew into. Both
//! of the page embed's inline renders live here too, rather than in
//! `view::inline` beside their sibling arms, so one module owns every
//! shape borrowed content takes — and because `inline.rs` sits near
//! the file-size ratchet while this one does not.

use crate::state::App;
use crate::theme::Theme;
use crate::view::inline::render_pretty_block_text;
use crate::view::wrap::push_wrapped;
use outl_md::index::WorkspaceIndex;
use outl_md::inline::{tokenize, InlineTok};
use outl_md::parse::OutlineNode;
use ratatui::text::{Line, Span};

/// Maximum AST nesting depth we'll render inside a single embed
/// expansion. Caps the size of the visual block we draw under one
/// `!((blk-XXXXXX))` or one `![[page]]` — a deeply nested source
/// subtree gets truncated instead of flooding the outline.
///
/// **Not a cycle protector**, and both spellings stay out of the
/// recursion for the same reason. An embed nested *inside* an
/// expansion is rendered inline — `render_pretty_block_text` draws a
/// block embed as the `↳ ` marker plus the source block's text, and a
/// page embed as the `↳ ` marker plus the target page's title — and is
/// never recursively expanded here. So an `A → B → A` cycle, at the
/// block level or the page level, never enters this recursion and the
/// cap doesn't need to defend against it. [`emit_embedded_children`]
/// walks concrete `OutlineNode` data and re-resolves nothing, so the
/// cap and the source tree's own depth are its only bounds.
///
/// If recursive embed expansion ever lands, add a `visited:
/// &HashSet<&str>` argument and short-circuit when the current handle
/// (or page slug) is already in the set.
const EMBED_MAX_DEPTH: u32 = 4;

/// Return the handle if `text` is a single `!((blk-XXXXXX))` token
/// surrounded only by whitespace; `None` otherwise.
///
/// Mixed content (`prelude !((blk-X)) postlude`) keeps the inline
/// `↳ <text>` render — we only expand when the user clearly meant
/// the whole block to *be* the embed.
pub(crate) fn embed_only_handle(text: &str) -> Option<&str> {
    let mut handle: Option<&str> = None;
    for tok in tokenize(text.trim()) {
        match tok {
            InlineTok::Plain(s) if s.trim().is_empty() => continue,
            InlineTok::Embed { handle: h } if handle.is_none() => handle = Some(h),
            _ => return None,
        }
    }
    handle
}

/// Return the page name if `text` is a single `![[page]]` token
/// surrounded only by whitespace; `None` otherwise.
///
/// The page-level twin of [`embed_only_handle`], and strict for the
/// same reason: mixed content (`see ![[notes]] first`) keeps the
/// inline `↳ <title>` chip, because only a block the user made *be*
/// the embed reads as a request to expand a whole page under it.
///
/// The two predicates are mutually exclusive — a text that is exactly
/// one `Embed` token cannot also be exactly one `PageEmbed` token — so
/// a caller asks them in either order and gets one answer at most.
pub(crate) fn page_embed_only_name(text: &str) -> Option<&str> {
    let mut name: Option<&str> = None;
    for tok in tokenize(text.trim()) {
        match tok {
            InlineTok::Plain(s) if s.trim().is_empty() => continue,
            InlineTok::PageEmbed { name: n } if name.is_none() => name = Some(n),
            _ => return None,
        }
    }
    name
}

/// The inline chip a `![[page]]` token draws inside a line of prose:
/// the target page's `icon::` (when it has one), the `↳ ` marker the
/// expansion rows below carry, then the page's **title**.
///
/// A name that resolves to no page keeps the literal `![[name]]`
/// dimmed — exactly what an orphan `!((blk-XXXXXX))` handle does, and
/// for the same reason: a silently blank row tells the user nothing
/// and leaves `outl doctor` nothing to point at.
///
/// Resolution is [`WorkspaceIndex::by_title`], the same lookup the
/// `[[ref]]` arm uses for its icon. A page embed *is* a page
/// reference, so a second opinion about what a name resolves to would
/// be drift between the chip and the link beside it.
pub(crate) fn page_embed_chip(
    name: &str,
    theme: &Theme,
    index: &WorkspaceIndex,
) -> Vec<Span<'static>> {
    let Some(page) = index.by_title(name) else {
        return vec![Span::styled(format!("![[{name}]]"), theme.dim)];
    };
    let mut out = Vec::new();
    if let Some(icon) = page.icon.as_deref() {
        out.push(Span::styled(format!("{icon} "), theme.dim));
    }
    out.push(Span::styled("↳ ".to_string(), theme.dim));
    out.push(Span::styled(page.title.clone(), theme.ref_link));
    out
}

/// The cursor-bearing render of a `![[page]]` token: the raw source
/// with its delimiters dimmed, so the visible cursor's columns stay
/// 1:1 with the source bytes while the user edits the token itself.
/// Same shape the `!((blk-XXXXXX))` arm uses.
pub(crate) fn page_embed_raw(name: &str, theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::styled("![[".to_string(), theme.dim),
        Span::styled(name.to_string(), theme.ref_link),
        Span::styled("]]".to_string(), theme.dim),
    ]
}

/// The target page's top-level blocks, in document order — what a
/// `![[page]]` expansion draws underneath its carrying block.
///
/// Empty when the name resolves to no page, or to a page the index has
/// no blocks for (never reconciled, or indexed from a `.md` whose
/// sidecar was stale). Empty is also what an unresolvable
/// `!((blk-XXXXXX))` expands to, so the carrying row's chip is the one
/// place either failure is reported.
///
/// `properties` are deliberately dropped rather than carried over:
/// [`outl_md::block_index::BlockEntry`] stores them **lowercased** for
/// the query DSL's `prop:` filter, so handing them to a renderer would
/// print a key the user never typed. Nothing reads them here —
/// [`emit_embedded_children`] draws text and children only, as it does
/// for the block embed.
pub(crate) fn page_embed_roots(index: &WorkspaceIndex, name: &str) -> Vec<OutlineNode> {
    let Some(page) = index.by_title(name) else {
        return Vec::new();
    };
    let mut roots = Vec::new();
    // `block_at_location` is the O(1) `(slug, dfs_path) → block` lookup;
    // walking `[0]`, `[1]`, … until it misses is how a page's top-level
    // run ends, since the index exposes no ordered per-page list.
    let mut i = 0usize;
    while let Some(entry) = index.block_at_location(&page.slug, &[i]) {
        roots.push(OutlineNode {
            text: entry.text.clone(),
            properties: Vec::new(),
            children: entry.children.clone(),
        });
        i += 1;
    }
    roots
}

/// Expand whatever embed a block's text *is*, as read-only rows under
/// it. Emits nothing when the text is not a bare embed token in either
/// spelling, which is the overwhelmingly common case.
///
/// One entry point rather than two `if let`s at the call site, because
/// "does this block borrow content, and what" is one question with two
/// answers and this module is its owner. The outline module only needs
/// to know *where* the rows go.
///
/// - `!((blk-XXXXXX))` → the source block's **children** (the block's
///   own text is already on the carrying row, as the inline chip).
/// - `![[page]]` → the target page's **top-level blocks** (the page's
///   title is on the carrying row, same way).
///
/// Both start at depth 1 and share [`EMBED_MAX_DEPTH`]: the carrying
/// row is depth 0 in each case, so the two geometries line up and a
/// reader cannot tell from the indentation which spelling produced
/// the rows.
pub(crate) fn emit_block_embed(
    text: &str,
    outer_indent: u32,
    app: &App,
    out: &mut Vec<Line<'static>>,
    text_width: u16,
) {
    if let Some(handle) = embed_only_handle(text) {
        // An unresolvable handle expands to nothing; the carrying row's
        // chip already shows the literal, so nothing is swallowed.
        if let Some(entry) = app.index.resolve_block_ref(handle) {
            emit_embedded_children(&entry.children, outer_indent, 1, app, out, text_width);
        }
    } else if let Some(name) = page_embed_only_name(text) {
        let roots = page_embed_roots(&app.index, name);
        emit_embedded_children(&roots, outer_indent, 1, app, out, text_width);
    }
}

/// Emit a source block's subtree underneath the embedding block.
///
/// Each row gets the same `↳ ` prefix the embed's first row carries
/// so the whole expansion reads as one cohesive block visually. Two
/// indent layers are stacked per row:
///
/// 1. `│ ` per ancestor indent of the carrying block (matches the
///    outline's own indent guides so the embed sits under the right
///    parent at a glance);
/// 2. two spaces per embed-subtree depth, so a child of the embed's
///    root lands visually under the root's `↳ ` instead of next to
///    it (otherwise the reader can't tell whether the row is a
///    sibling of the carrying block or a child of the source).
///
/// Depth-capped at [`EMBED_MAX_DEPTH`] so an embed cycle can't run
/// forever.
pub(crate) fn emit_embedded_children(
    children: &[OutlineNode],
    outer_indent: u32,
    depth: u32,
    app: &App,
    out: &mut Vec<Line<'static>>,
    text_width: u16,
) {
    if depth > EMBED_MAX_DEPTH {
        return;
    }
    for child in children {
        // `guides` repeat on every wrapped row; the `↳ ` marker lives in
        // `head` so it only appears once and continuations re-indent
        // under the embedded text (same split `emit_block_lines` uses).
        let mut guides: Vec<Span<'static>> = Vec::new();
        // 1. Outline indent guides (mirrors what `emit_block_lines`
        //    draws for a regular block at the same depth in the doc).
        for _ in 0..outer_indent {
            guides.push(Span::styled("│ ", app.theme.dim));
        }
        // 2. Embed-internal indent so children land **below the source
        //    root's text**, not alongside its `↳ `. The carrying
        //    block's first row reads `- ↳ <root-text>`: bullet + space
        //    + `↳` + space = four cells before the root text starts.
        //    A child needs to clear those four cells plus one more
        //    embed-indent step (two cells) before its own `↳ `, then
        //    another two per nested level. `(depth + 1) * 2` spaces
        //    keeps the geometry: depth 1 → 4 spaces, depth 2 → 6, etc.
        for _ in 0..(depth + 1) {
            guides.push(Span::raw("  "));
        }
        let head = vec![Span::styled("↳ ", app.theme.dim)];
        // An embedded table reads as a grid, like it does in the
        // outline. This is the one block-text path that does not go
        // through `emit_block_lines`, so it asks for the grid itself; a
        // table is multi-line, so each row becomes its own `↳` row.
        if let Some(grid) = crate::view::table::aligned_grid(&child.text) {
            for line in grid.lines() {
                let row = crate::view::table::render_row(line, &app.theme, &app.index, &app.icons);
                push_wrapped(guides.clone(), head.clone(), row, text_width, None, out);
            }
        } else {
            let content = render_pretty_block_text(&child.text, &app.theme, &app.index, &app.icons);
            push_wrapped(guides, head, content, text_width, None, out);
        }
        emit_embedded_children(
            &child.children,
            outer_indent,
            depth + 1,
            app,
            out,
            text_width,
        );
    }
}

#[cfg(test)]
mod tests;
