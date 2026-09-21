//! Embed expansion — the read-only subtree an outline block draws
//! below itself when its text is a single `!((blk-XXXXXX))` token.
//!
//! Split out of `view::outline` because it answers a question of its
//! own (what does *borrowed* content look like), and the outline
//! module is the one every other view concern already grew into.

use crate::state::App;
use crate::view::inline::render_pretty_block_text;
use crate::view::wrap::push_wrapped;
use outl_md::inline::{tokenize, InlineTok};
use outl_md::parse::OutlineNode;
use ratatui::text::{Line, Span};

/// Maximum AST nesting depth we'll render inside a single embed
/// expansion. Caps the size of the visual block we draw under one
/// `!((blk-XXXXXX))` — a deeply nested source subtree gets truncated
/// instead of flooding the outline.
///
/// **Not a cycle protector.** Embed-of-embed (a source block whose
/// own text is another `!((blk-Y))`) is rendered inline with the `↳ `
/// marker by `render_pretty_block_text`; it is *not* recursively
/// expanded here. So an `A → B → A` cycle never enters this recursion
/// and the cap doesn't need to defend against it. If recursive embed
/// expansion ever lands, add a `visited: &HashSet<&str>` argument and
/// short-circuit when the current handle is already in the set.
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
        let content = render_pretty_block_text(&child.text, &app.theme, &app.index);
        push_wrapped(guides, head, content, text_width, None, out);
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
mod tests {
    use super::*;
    #[test]
    fn embed_only_handle_detects_bare_token() {
        assert_eq!(embed_only_handle("!((blk-r6s4a1))"), Some("blk-r6s4a1"));
    }

    #[test]
    fn embed_only_handle_ignores_surrounding_whitespace() {
        assert_eq!(embed_only_handle("  !((blk-r6s4a1))  "), Some("blk-r6s4a1"));
    }

    #[test]
    fn embed_only_handle_rejects_mixed_text() {
        assert_eq!(embed_only_handle("see !((blk-r6s4a1)) context"), None);
    }

    #[test]
    fn embed_only_handle_rejects_inline_ref() {
        // `((blk-X))` (no leading `!`) is a ref, not an embed —
        // must not trigger expansion.
        assert_eq!(embed_only_handle("((blk-r6s4a1))"), None);
    }

    #[test]
    fn embed_only_handle_rejects_two_embeds_on_one_block() {
        // Two embeds in the same block is ambiguous (which one expands
        // first?) — for now keeps the rule strict: exactly one token,
        // surrounded by whitespace.
        assert_eq!(embed_only_handle("!((blk-aaaaaa)) !((blk-bbbbbb))"), None);
    }
}
