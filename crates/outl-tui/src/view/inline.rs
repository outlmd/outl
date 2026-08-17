//! Inline (span-level) markdown rendering.
//!
//! Two flavours — `render_markdown_inline` strips delimiters and
//! prepends ref icons (read-only blocks), `highlight_inline` keeps the
//! markdown source visible with dim delimiters (cursor-bearing blocks
//! so column-to-byte alignment stays 1:1).

use crate::theme::Theme;
use outl_actions::TodoState;
use outl_md::inline::{inline_to_source, tokenize, InlineTok};
use ratatui::text::Span;

/// Strip an optional `"> "` blockquote prefix off a block's text.
/// Returns `(quoted, body)` — same shape as [`outl_actions::split_quote`].
/// The wrapper exists so other modules in this crate stay decoupled
/// from `outl-actions` and so a future render-time tweak (e.g. handling
/// `">> "` later) lands in one place.
pub(crate) fn split_quote_prefix(text: &str) -> (bool, &str) {
    outl_actions::quote::split_quote(text)
}

/// Strip TODO/DONE and `"> "` blockquote prefixes off a block's text
/// in **either order**. Returns `(todo_state, quoted, body)`.
///
/// Why both orders: the TUI works on the raw text from disk where the
/// user can type `"TODO > foo"` or `"> TODO foo"` — same intent, two
/// authoring shapes. The mobile / desktop frontends don't need this
/// because the backend (`outline.rs::project_outline`) already calls
/// `split_todo` before serialising, so the DTO's `block.text` only
/// ever carries the quote marker. The TUI sees the raw form.
///
/// At most one TODO/DONE and one quote marker are recognised — a
/// nested `">> "` keeps the inner `> ` inside the body, matching the
/// "no nested quotes in v1" policy from the canonical helper.
pub(crate) fn split_block_prefixes(text: &str) -> (Option<TodoState>, bool, &str) {
    let mut todo: Option<TodoState> = None;
    let mut quoted = false;
    let mut current = text;
    // Two passes are enough — at most one of each marker is recognised.
    for _ in 0..2 {
        if !quoted {
            let (q, rest) = split_quote_prefix(current);
            if q {
                quoted = true;
                current = rest;
                continue;
            }
        }
        if todo.is_none() {
            let (t, rest) = outl_actions::split_todo(current);
            if t.is_some() {
                todo = t;
                current = rest;
                continue;
            }
        }
        break;
    }
    (todo, quoted, current)
}

/// Render a block's body the way the outline renders it in read-only
/// mode: strip and visualize a `TODO`/`DONE` prefix, then pass the
/// remainder through [`render_markdown_inline`].
///
/// Used by the outline (for single-line bullets) **and** by the embed
/// expansion (which needs the same affordance — a `TODO` inside an
/// embedded block should still render as `☐` so the reader sees
/// state, not the raw word).
pub(crate) fn render_pretty_block_text(
    text: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
) -> Vec<Span<'static>> {
    render_pretty_block_text_impl(text, theme, index, true)
}

/// Internal variant that controls whether `InlineTok::Embed` tokens
/// inside `text` are expanded again. Set `expand_embed = false` when
/// the caller is *already* rendering an embed's body — otherwise
/// `A` embedding `B` while `B` embeds `A` (or even `A` itself) would
/// recurse forever and blow the stack.
fn render_pretty_block_text_impl(
    text: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
    expand_embed: bool,
) -> Vec<Span<'static>> {
    // Strip TODO/DONE and quote markers in either order so the user
    // can type `"> TODO foo"` or `"TODO > foo"` — same intent, two
    // authoring shapes. The two affordances stack: a quoted TODO
    // renders as `│ ☐ foo`. Body keeps its full colour palette —
    // dimming refs / tags / bold would erase their affordance, and
    // the `│` bar is already cue enough that "this is a quote".
    let (todo_state, quoted, body) = split_block_prefixes(text);
    let mut out: Vec<Span<'static>> = Vec::new();
    if quoted {
        // Left bar + a space, dimmed. The `│` is one column wide; the
        // trailing space gives the body breathing room without taking
        // a second cell from the marker.
        out.push(Span::styled("│ ", theme.dim));
    }
    match todo_state {
        // Both open states share `todo_open`'s colour on purpose: a
        // started task is unfinished work, and the glyph already
        // carries the distinction. A third palette entry would mean a
        // new key in every theme preset for something one character
        // says.
        Some(state @ (TodoState::Todo | TodoState::Doing)) => {
            let glyph = if state == TodoState::Doing {
                "◐ "
            } else {
                "☐ "
            };
            out.push(Span::styled(glyph, theme.todo_open));
            out.extend(render_markdown_inline_impl(
                body,
                theme,
                index,
                expand_embed,
            ));
        }
        Some(TodoState::Done) => {
            out.push(Span::styled("☑ ", theme.todo_done));
            for sp in render_markdown_inline_impl(body, theme, index, expand_embed) {
                out.push(Span::styled(
                    sp.content.into_owned(),
                    sp.style.patch(theme.todo_done_body),
                ));
            }
        }
        None => out.extend(render_markdown_inline_impl(
            body,
            theme,
            index,
            expand_embed,
        )),
    }
    out
}

/// Render with markdown stripped — bold/italic/code/strike applied as
/// styles, `[[ref]]` / `#tag` / `[text](url)` shown without their
/// delimiters. Used when the block is read-only (not selected, not in
/// Insert mode).
///
/// Looks up `[[ref]]` / `#tag` targets in `index` to prepend the
/// page's `icon::` when one is set. The icon is *display-only* — the
/// underlying `.md` keeps the plain `[[Title]]` / `#tag` text.
///
/// `highlight_inline` (the raw, cursor-bearing render) deliberately
/// does *not* take this path — adding a non-source glyph would
/// break column-to-byte alignment for the visible cursor.
pub(crate) fn render_markdown_inline(
    text: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
) -> Vec<Span<'static>> {
    render_markdown_inline_impl(text, theme, index, true)
}

/// Internal variant: when `expand_embed = false`, `InlineTok::Embed`
/// renders as its raw `!((handle))` form (dim) instead of recursing
/// into the source block. The Embed arm passes `false` to its own
/// recursive call so an A → B → A cycle terminates after a single
/// expansion. Doctor still surfaces the citation in either direction.
fn render_markdown_inline_impl(
    text: &str,
    theme: &Theme,
    index: &outl_md::index::WorkspaceIndex,
    expand_embed: bool,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for tok in tokenize(text) {
        match tok {
            InlineTok::Plain(s) => out.push(Span::raw(s.to_string())),
            InlineTok::PageRef { name } => {
                if let Some(icon) = index.by_title(name).and_then(|p| p.icon.as_deref()) {
                    out.push(Span::styled(format!("{icon} "), theme.dim));
                }
                out.push(Span::styled(name.to_string(), theme.ref_link));
            }
            InlineTok::Tag { name } => {
                if let Some(icon) = index.by_slug(name).and_then(|p| p.icon.as_deref()) {
                    out.push(Span::styled(format!("{icon} "), theme.dim));
                }
                out.push(Span::styled(format!("#{name}"), theme.tag_link));
            }
            InlineTok::Bold { inner } => {
                out.push(Span::styled(inline_to_source(&inner), theme.bold))
            }
            InlineTok::Italic { inner, .. } => {
                out.push(Span::styled(inline_to_source(&inner), theme.italic))
            }
            InlineTok::Strike { inner } => {
                out.push(Span::styled(inline_to_source(&inner), theme.strike))
            }
            InlineTok::Highlight { inner } => {
                out.push(Span::styled(inline_to_source(&inner), theme.highlight))
            }
            InlineTok::Code { inner } => out.push(Span::styled(inner.to_string(), theme.code)),
            InlineTok::Link { text, .. } => out.push(Span::styled(text.to_string(), theme.md_link)),
            InlineTok::Image { alt, url } => {
                // A terminal can't paint pixels, so surface a placeholder:
                // an image glyph for image extensions, a generic file
                // glyph otherwise, followed by the alt text (or the
                // filename when alt is empty). The glyph decision reuses
                // `outl_md::wikilink::is_image_target` so the extension
                // list stays owned by one place.
                let glyph = if outl_md::wikilink::is_image_target(url) {
                    "🖼"
                } else {
                    "📄"
                };
                let label = if alt.is_empty() {
                    url.rsplit('/').next().unwrap_or(url)
                } else {
                    alt
                };
                out.push(Span::styled(format!("{glyph} {label}"), theme.md_link));
            }
            InlineTok::BlockRef { handle } => {
                // Resolve the handle to the source block's text and
                // surface it inline, Roam-style. Citing-block readers
                // see content, not a UUID-ish handle.
                //
                // Unresolved handles (orphan reference: source block
                // deleted or never indexed) render dimmed with the
                // handle visible so `outl doctor` (#10) has something
                // to point at and the user understands what broke.
                match index.resolve_block_ref(handle) {
                    Some(entry) => {
                        // Source page icon — same affordance as PageRef.
                        if let Some(icon) = index
                            .by_slug(&entry.source_slug)
                            .and_then(|p| p.icon.as_deref())
                        {
                            out.push(Span::styled(format!("{icon} "), theme.dim));
                        }
                        out.push(Span::styled(entry.text.clone(), theme.ref_link));
                    }
                    None => {
                        out.push(Span::styled(format!("(({handle}))"), theme.dim));
                    }
                }
            }
            InlineTok::Emoji { shortcode } => {
                // Pretty render: just the glyph. Defensive fall-back
                // to the literal `:shortcode:` if the catalog misses
                // (parser pre-validates, so this is dead code in
                // practice — but the day a peer ships an unknown
                // shortcode the user should see it, not an empty span).
                match outl_md::emoji::shortcode_to_unicode(shortcode) {
                    Some(glyph) => out.push(Span::raw(glyph.to_string())),
                    None => out.push(Span::raw(format!(":{shortcode}:"))),
                }
            }
            InlineTok::Embed { handle } => {
                // Cycle / recursion guard: when this call is itself
                // rendering an embedded block's body, treat the inner
                // embed as raw text. That breaks `A → B → A` cycles
                // and bounds rendering work at one level of expansion.
                if !expand_embed {
                    out.push(Span::styled(format!("!(({handle}))"), theme.dim));
                    continue;
                }
                // Inline read-only render: `↳ ` prefix marks "this row
                // belongs to an embed", and the source block's text is
                // pushed through `render_pretty_block_text_impl` (with
                // `expand_embed = false`) so TODO / DONE prefixes,
                // `[[refs]]`, `#tags`, bold etc. render but a nested
                // `!((blk-Y))` inside `entry.text` doesn't recurse.
                // The subtree below this row (rendered by
                // `emit_embedded_children`) uses the same `↳ ` prefix
                // so the whole embed reads as one visual block.
                match index.resolve_block_ref(handle) {
                    Some(entry) => {
                        if let Some(icon) = index
                            .by_slug(&entry.source_slug)
                            .and_then(|p| p.icon.as_deref())
                        {
                            out.push(Span::styled(format!("{icon} "), theme.dim));
                        }
                        out.push(Span::styled("↳ ".to_string(), theme.dim));
                        // `expand_embed = false`: stop recursive embed
                        // expansion at this depth so a nested embed in
                        // `entry.text` shows up raw and doesn't cycle.
                        out.extend(render_pretty_block_text_impl(
                            &entry.text,
                            theme,
                            index,
                            false,
                        ));
                    }
                    None => {
                        out.push(Span::styled(format!("!(({handle}))"), theme.dim));
                    }
                }
            }
        }
    }
    out
}

/// Render with markdown markers visible (dimmed) and inner text styled.
/// Used when the block is selected in Normal mode (so the visible cursor
/// columns match the underlying source bytes) or in Insert mode. The
/// delimiters themselves use a dim style so the formatting markers
/// don't distract.
///
/// Quote handling: we detect a leading `"> "` and **style** those two
/// characters as dim — we never move them. Cursor columns continue to
/// match source bytes 1:1 (the same constraint the rest of this
/// function obeys). The reader still sees the body styled dim so the
/// "this is a quote" affordance is present even on the cursor-bearing
/// row; the chrome (left `│` bar) lives in the pretty render only,
/// since drawing it here would push every column by 2.
pub(crate) fn highlight_inline(text: &str, theme: &Theme) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let dim = theme.dim;

    // Detect + emit the quote prefix verbatim (no column shift), then
    // tokenize the body. Treating it inside the per-token loop below
    // would require special-casing the very first Plain token's leading
    // characters — easier to handle once up-front.
    // We only dim the `"> "` prefix; the body keeps its colours so
    // refs / tags / bold inside a quoted block stay legible on the
    // selected / editing row (the chrome `│` bar lives in the pretty
    // render only).
    let body = match text.strip_prefix(outl_actions::quote::QUOTE_PREFIX) {
        Some(rest) => {
            out.push(Span::styled(
                outl_actions::quote::QUOTE_PREFIX.to_string(),
                dim,
            ));
            rest
        }
        None => text,
    };

    for tok in tokenize(body) {
        match tok {
            InlineTok::Plain(s) => out.push(Span::raw(s.to_string())),
            InlineTok::PageRef { name } => {
                out.push(Span::styled(format!("[[{name}]]"), theme.ref_link))
            }
            InlineTok::Tag { name } => out.push(Span::styled(format!("#{name}"), theme.tag_link)),
            InlineTok::Bold { inner } => {
                out.push(Span::styled("**".to_string(), dim));
                out.push(Span::styled(inline_to_source(&inner), theme.bold));
                out.push(Span::styled("**".to_string(), dim));
            }
            InlineTok::Italic { inner, marker } => {
                let m = marker.to_string();
                out.push(Span::styled(m.clone(), dim));
                out.push(Span::styled(inline_to_source(&inner), theme.italic));
                out.push(Span::styled(m, dim));
            }
            InlineTok::Strike { inner } => {
                out.push(Span::styled("~~".to_string(), dim));
                out.push(Span::styled(inline_to_source(&inner), theme.strike));
                out.push(Span::styled("~~".to_string(), dim));
            }
            InlineTok::Highlight { inner } => {
                out.push(Span::styled("==".to_string(), dim));
                out.push(Span::styled(inline_to_source(&inner), theme.highlight));
                out.push(Span::styled("==".to_string(), dim));
            }
            InlineTok::Code { inner } => {
                out.push(Span::styled("`".to_string(), dim));
                out.push(Span::styled(inner.to_string(), theme.code));
                out.push(Span::styled("`".to_string(), dim));
            }
            InlineTok::Link { text, url } => {
                out.push(Span::styled("[".to_string(), dim));
                out.push(Span::styled(text.to_string(), theme.md_link));
                out.push(Span::styled(format!("]({url})"), dim));
            }
            InlineTok::Image { alt, url } => {
                // Cursor-bearing render: emit the raw source verbatim
                // (mirroring the Link arm with the leading `!`) so the
                // visible cursor columns stay 1:1 with the source bytes.
                // No placeholder glyph here — that lives in the pretty
                // render only.
                out.push(Span::styled("![".to_string(), dim));
                out.push(Span::styled(alt.to_string(), theme.md_link));
                out.push(Span::styled(format!("]({url})"), dim));
            }
            InlineTok::BlockRef { handle } => {
                // Cursor-bearing render keeps the `((...))` delimiters
                // dimmed so column-to-byte alignment for the visible
                // cursor stays 1:1 with the source bytes.
                out.push(Span::styled("((".to_string(), dim));
                out.push(Span::styled(handle.to_string(), theme.ref_link));
                out.push(Span::styled("))".to_string(), dim));
            }
            InlineTok::Embed { handle } => {
                // Cursor-bearing render: full raw source so column
                // accounting stays exact while the user is editing
                // the embed token itself.
                out.push(Span::styled("!((".to_string(), dim));
                out.push(Span::styled(handle.to_string(), theme.ref_link));
                out.push(Span::styled("))".to_string(), dim));
            }
            InlineTok::Emoji { shortcode } => {
                // Highlight (cursor-bearing) render keeps the source
                // shape intact: `:` (dim) + shortcode (raw) + `:` (dim).
                // Rendering only the glyph here would shift cursor
                // columns by `len(:shortcode:) - 1` and break the
                // 1:1 char-to-column mapping the rest of this function
                // relies on.
                out.push(Span::styled(":".to_string(), dim));
                out.push(Span::raw(shortcode.to_string()));
                out.push(Span::styled(":".to_string(), dim));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::default_theme;

    fn empty_index() -> outl_md::index::WorkspaceIndex {
        outl_md::index::WorkspaceIndex::default()
    }

    /// A quoted bullet must lead with the `│ ` bar before any other
    /// content. The TODO checkbox slots in after the bar so a quoted
    /// open task reads as `│ ☐ body`.
    #[test]
    fn pretty_render_emits_quote_bar_first() {
        let theme = default_theme();
        let idx = empty_index();
        let spans = render_pretty_block_text("> hello", &theme, &idx);
        assert!(
            spans
                .first()
                .map(|s| s.content.starts_with('│'))
                .unwrap_or(false),
            "expected leading │ bar, got {spans:#?}",
        );
    }

    #[test]
    fn pretty_render_composes_quote_and_todo() {
        let theme = default_theme();
        let idx = empty_index();
        let spans = render_pretty_block_text("> TODO ship it", &theme, &idx);
        // First span: quote bar. Second: TODO checkbox.
        assert!(spans
            .first()
            .map(|s| s.content.starts_with('│'))
            .unwrap_or(false));
        assert!(
            spans.iter().any(|s| s.content.contains('☐')),
            "expected ☐ checkbox span somewhere after the bar, got {spans:#?}",
        );
    }

    /// `TODO > body` must paint both checkbox and quote bar — the
    /// user can author the prefixes in either order. Regression for
    /// the screenshot where the TUI rendered `☐ > foo` literally
    /// instead of `│ ☐ foo` when TODO came before quote.
    #[test]
    fn pretty_render_accepts_todo_before_quote() {
        let theme = default_theme();
        let idx = empty_index();
        let spans = render_pretty_block_text("TODO > ship it", &theme, &idx);
        assert!(spans
            .first()
            .map(|s| s.content.starts_with('│'))
            .unwrap_or(false));
        assert!(
            spans.iter().any(|s| s.content.contains('☐')),
            "expected ☐ checkbox span, got {spans:#?}",
        );
        // The literal `> ` must NOT survive to the body — split_block_prefixes
        // ate both markers, the inline tokenizer sees `ship it` only.
        assert!(
            !spans.iter().any(|s| s.content.contains('>')),
            "expected no literal `>` in the body, got {spans:#?}",
        );
    }

    #[test]
    fn split_block_prefixes_recognises_both_orders() {
        let todo = Some(TodoState::Todo);
        let doing = Some(TodoState::Doing);
        let done = Some(TodoState::Done);
        assert_eq!(split_block_prefixes("> TODO foo"), (todo, true, "foo"));
        assert_eq!(split_block_prefixes("TODO > foo"), (todo, true, "foo"));
        assert_eq!(split_block_prefixes("DOING > foo"), (doing, true, "foo"));
        assert_eq!(split_block_prefixes("> DOING foo"), (doing, true, "foo"));
        assert_eq!(split_block_prefixes("DONE > foo"), (done, true, "foo"));
        assert_eq!(split_block_prefixes("> DONE foo"), (done, true, "foo"));
        assert_eq!(split_block_prefixes("> foo"), (None, true, "foo"));
        assert_eq!(split_block_prefixes("TODO foo"), (todo, false, "foo"));
        assert_eq!(split_block_prefixes("DOING foo"), (doing, false, "foo"));
        // The checkbox spelling reads the same (issue #230).
        assert_eq!(split_block_prefixes("> [ ] foo"), (todo, true, "foo"));
        assert_eq!(split_block_prefixes("[x] foo"), (done, false, "foo"));
        assert_eq!(split_block_prefixes("foo"), (None, false, "foo"));
        // Nested `>>` keeps inner `>` in body — no double-strip.
        assert_eq!(split_block_prefixes("> > foo"), (None, true, "> foo"));
    }

    /// An image embed renders as a placeholder in pretty mode (the
    /// terminal can't paint pixels) but survives verbatim in the
    /// cursor-bearing raw render so column-to-byte alignment holds.
    #[test]
    fn image_pretty_shows_glyph_raw_keeps_source() {
        let theme = default_theme();
        let idx = empty_index();

        // Image extension → 🖼 placeholder in the pretty render.
        let pretty = render_pretty_block_text("![cover](assets/x.png)", &theme, &idx);
        assert!(
            pretty.iter().any(|s| s.content.contains('🖼')),
            "expected 🖼 placeholder for an image asset, got {pretty:#?}",
        );

        // Non-image extension → 📄 generic-file placeholder.
        let pretty_doc = render_pretty_block_text("![doc](assets/x.pdf)", &theme, &idx);
        assert!(
            pretty_doc.iter().any(|s| s.content.contains('📄')),
            "expected 📄 placeholder for a non-image asset, got {pretty_doc:#?}",
        );

        // Cursor-bearing render keeps the raw `![alt](url)` delimiters so
        // the visible cursor stays aligned with the source bytes.
        let raw = highlight_inline("![cover](assets/x.png)", &theme);
        let joined: String = raw.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(
            joined, "![cover](assets/x.png)",
            "raw render must reproduce the source verbatim, got {joined:?}",
        );
    }

    /// Plain (non-quoted) text must not get a leading bar.
    #[test]
    fn pretty_render_skips_bar_for_plain_text() {
        let theme = default_theme();
        let idx = empty_index();
        let spans = render_pretty_block_text("plain body", &theme, &idx);
        assert!(
            !spans
                .first()
                .map(|s| s.content.starts_with('│'))
                .unwrap_or(false),
            "plain block must not lead with the quote bar, got {spans:#?}",
        );
    }
}
