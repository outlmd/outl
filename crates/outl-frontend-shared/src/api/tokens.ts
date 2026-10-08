/**
 * The inline-markdown token vocabulary, split out of `types.ts`.
 *
 * Its own file because it is a **shared dependency rather than a page
 * DTO**: `api/table.ts` imports it, every renderer imports it, and
 * `types.ts` only happens to be where it was first written. Same rule
 * as the rest of `api/` — each shape mirrors a `serde`-serialized Rust
 * type, the mirror is hand-written on purpose, and this file is
 * registered in `outl-tauri-shared/tests/ts_parser`'s `MIRROR_FILES`,
 * so a variant added to `outl_md::InlineToken` and forgotten here fails
 * there rather than reaching a device as an unrendered token.
 *
 * `types.ts` re-exports it, so `import type { InlineToken } from
 * "../api/types"` still resolves.
 */

/**
 * Pre-tokenized inline markdown coming from the Rust backend
 * (`outl_md::tokenize_owned`). The renderer at
 * `@outl/shared/markdown::MarkdownInline` maps each variant to JSX.
 * There is no parallel TS tokenizer — `outl_md::inline::tokenize` is
 * the single source of truth for inline syntax across every client.
 * Adding a token in Rust means extending this union and the renderer
 * switch in the same change.
 */
export type InlineToken =
  | { kind: "plain"; value: string }
  // Bold / italic / strike carry their inner span as a re-tokenized
  // list so nested refs, tags, and block-refs render with their own
  // styling. `**[[avelino]]**` arrives as `Bold { inner: [Ref … ] }`
  // — the renderer wraps the inner tokens in the bold style.
  | { kind: "bold"; inner: InlineToken[] }
  | { kind: "italic"; inner: InlineToken[] }
  | { kind: "strike"; inner: InlineToken[] }
  // `==highlight==` — the on-disk form of Roam's `^^highlight^^`.
  | { kind: "highlight"; inner: InlineToken[] }
  | { kind: "code"; value: string }
  | { kind: "link"; value: string; href: string }
  // `![alt](href)` image / embedded asset. `href` is a workspace-relative
  // `assets/<hash>.<ext>` path or a remote URL. The renderer shows an
  // `<img>` for image extensions and a file chip for other kinds (pdf,
  // …). Mirrors `outl_md::InlineToken::Image { alt, href }`.
  | { kind: "image"; alt: string; href: string }
  | { kind: "ref"; value: string }
  | { kind: "tag"; value: string }
  | { kind: "blockref"; value: string }
  | { kind: "embed"; value: string }
  // `:shortcode:` — GitHub gemoji shortcode. `shortcode` is the disk
  // form (`"tada"`); `glyph` is the resolved unicode codepoint
  // (`"🎉"`). The renderer shows the glyph and surfaces the shortcode
  // for hover / `aria-label`. Mirrors `outl_md::InlineToken::Emoji`.
  | { kind: "emoji"; shortcode: string; glyph: string };
