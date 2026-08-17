# CLAUDE.md — outl-md

The boundary between **what the user sees** (clean markdown) and **what the core processes** (op log with stable IDs).

If this crate misroutes a block during matching, the user perceives "outl deleted my work" — even if the op log still has it.
Treat matching with the same paranoia as the CRDT.

> The **canonical reuse index** for the whole workspace is the [Shared primitives catalog](../../docs/shared-primitives.md) — index plus three parts ([core](../../docs/primitives-core.md), [markdown](../../docs/primitives-markdown.md), [actions](../../docs/primitives-actions.md)), mirrored in condensed form at [`.github/copilot-instructions.md`](../../.github/copilot-instructions.md) §5.1.
> This crate's rows live mostly in the [markdown pipeline](../../docs/primitives-markdown.md) part.
> The detailed list below describes this crate's responsibilities; the catalog is the "intent → use this" cross-crate index you should grep first when adding any helper.

## What this crate owns

- Parse `.md` (clean, no IDs) → outline AST.
  Parser is **permissive at every depth, not just depth 0**:
  lines that don't match the outl dialect (e.g. a leading `# heading`, a stray paragraph, an HTML snippet) are preserved verbatim as a recovered block.
  A sibling when found at depth 0, a child of the block above it when found indented with no open continuation to absorb it.
  Every recovery is recorded in `ParsedPage.warnings: Vec<ParseWarning>` (kind `UnrecognizedBlockMarker`).
  Nothing is silently dropped, at any depth; surfaces show the warning list so the user can clean the file at their pace.
  Multi-line bodies (including `> ` blockquote continuation lines, `TODO ` / `DOING ` / `DONE ` continuations, and free-text continuations) land verbatim in `OutlineNode.text` separated by `\n`;
  the prefix on each continuation line is preserved by the same "trim leading indent, append to text" path so blockquote bodies round-trip cleanly as CommonMark.
  **Blank lines and indentation inside a block's text now round-trip.**
  A whitespace-only line indented deeper than the block (what `render::write_block_text` emits for a blank line mid-continuation) folds into `text`; only a genuinely empty line closes continuation.
  A continuation line's own indentation survives via the private `strip_indent_levels` helper, which strips only the levels the renderer added.
  An over-indented line the grammar still can't place is recovered as a **child block** at its written depth (warning, not a silent drop).
  Getting any of these three wrong is the issue #210 producer — measured at 41 pages / 387 lines on a real workspace, and 0 after the fix.
- Render outline AST → `.md` (clean, no IDs).
  Each line in `OutlineNode.text` after the first is emitted at `indent + 1`; the renderer **does not invent** prefixes on continuation lines — whatever the user (or the parser) put in `text` round-trips as-is.
  Block-kind markers (`TODO `, `DOING `, `DONE `, `> `) are owned by `outl-actions` (`todo.rs`, `quote.rs`); this crate only preserves them verbatim, which is why `DOING ` needed no parser change.
  The lone exception is `outline_ops::count_todos` (the progress chip's `(done, total)`): `DOING` counts toward the total, never toward `done`.
  It reads both spellings and unwraps one `"> "` first, so it counts what the TUI draws; `split_todo` does not (see `outl-actions/CLAUDE.md`).
- Read/write `.outl` sidecar (JSON, sibling file) — current version `2`, reads v1 transparently (handles backfilled on load; missing `text` stays empty and level 2 skips that block).
  **Additive fields ride at the same version — see [Sidecar versioning](#sidecar-versioning-both-directions) before touching `SIDECAR_VERSION`.**
  The sidecar is **structural metadata only** (id, line, indent, content hash, ref handle, last-synced text).
  State that must converge between devices (fold flags, pinned, etc.) goes through the op log in `outl-core`, never here.
- The 3-level matching algorithm (external edit → reconstruct IDs).
  `matching.rs` owns the level walk; `similarity.rs` (crate-private) owns level-2 scoring and the global-confidence assignment that decides which new block keeps which old id.
- Diff (old AST + new AST + old sidecar blocks) → minimal sequence of `Op`s, preserving `ref_handle` verbatim on level-1/2 matches.
  The old-handle lookup is O(N) overall (HashMap by id), not O(N²) — never reintroduce a linear scan per new block.
  **`diff_to_ops` only emits structural ops (Create / Move / SetProp); a second pass inside `reconcile_md::sync_block_text` walks the AST + new sidecar in lockstep and emits one `Op::Edit` per block whose text differs from the workspace.
  Skipping that pass silently zeroes text across devices — local stays fine, peer replays an empty tree, iCloud ships the empty `.md` back.**
  **Page-root id derivation.**
  When a `.md` has **no sidecar**, `reconcile_md` seeds the page/journal-root id with `NodeId::from_slug(file_stem)`, **never** a fresh `NodeId::new()`.
  A page root's identity is its slug.
  Minting a time-based ULID here split a day's journal across two competing roots.
  That happened when the same `journals/YYYY-MM-DD.md` was reconciled on a device that had no `.outl` yet (external editor, peer that shipped only the `.md`, crash before the sidecar landed).
  `ensure_page_root_in_tree` then writes that same slug into `page-slug`, so the id and the property stay in agreement.
- **`remind`** — the `remind::` block-property grammar (`3pm every 1h until DONE`) → `RemindRule`.
  **Syntax only.** *When* a rule actually fires is `outl_actions::reminders` — one owner for the schedule math, wrapped by every client and OS bridge.
  Permissive like the rest of the parser: an unreadable rule yields `rule: None` plus `ParseWarningKind::Remind*` records, and the property stays on disk verbatim so the user can fix the typo in place.
  `parse` validates a `remind::` property line as it reads it, so the warnings carry the exact source line.
  User-facing spec: [`docs/reminders.md`](../../docs/reminders.md).
- **`outline_ops`** — pure `Vec<OutlineNode>` AST helpers (`flat_count`, `path_for_index`, `insert_sibling_after/before`, `indent_at_path`, `outdent_at_path`, `delete_at_path`, `move_up_at_path`, `move_down_at_path`, …).
  `insert_sibling_after_with_text` is the same insert as `insert_sibling_after` but seeds the new block's text — the TUI's in-flight block-split on Enter mid-text (issue #184).
  They operate on an in-flight AST that hasn't been parsed back into a workspace yet, so they sit in `outl-md` (UI-agnostic, no `Workspace`) rather than in `outl-actions`.
  The TUI re-exports them through a one-line shim at `outl-tui/src/outline_ops.rs`; the mobile client consumes them directly.
  **Insert helpers clamp**:
  `insert_sibling_after/before` clamp the computed position to `siblings.len()` instead of panicking when a caller passes a path the live tree no longer satisfies,
  (typical case: page parsed to zero blocks because its content didn't start with a `- ` marker, but the TUI's `selected` cursor defaulted to `[0]`).
  Falling back to "append at the end" is the right shape — the user's intent ("create a new block") is satisfied, no data is lost.
- **Emoji catalog** (`emoji.rs`) — GitHub gemoji catalog (backed by the [`emojis`] crate).
  `shortcode_to_unicode("tada") → Some("🎉")` is the one-way resolver every renderer uses;
  `search(query, limit) → Vec<EmojiHit>` powers the `:shortcode:` autocomplete shared by TUI / mobile / desktop through the `outl_emoji_search` Tauri command,
  and **iterates every alias** (`emoji.shortcodes()`, not `shortcode()`) so the autocomplete returns the same set the parser accepts (`:+1:` and `:thumbsup:` both surface).
  `is_valid_shortcode_char(c)` is the char-level alphabet check — exported so consumers walking buffers char-by-char (`try_emoji`, TUI's `detect_trigger`) avoid allocating a 1-char `String` per keystroke.
  The parser only tokenizes `:foo:` when `shortcode_to_unicode` finds `foo`, so unknown input (`:notarealemoji:`, `meeting at 14:00`) stays plain.
  **Never retro-translate `glyph → shortcode`** — multiple shortcodes can alias the same codepoint (`:+1:` and `:thumbsup:` both → 👍) so the disk form would become lossy.
- **Inline tokenization** (`inline.rs` + its sibling slices) — `**bold**`, `~~strike~~`, `==highlight==`, `[[refs]]`, `#tags`, `((blk-XXXXXX))`, `!((blk-XXXXXX))`, `![alt](url)`, `:shortcode:`.
  `inline.rs` owns the scan (`tokenize` / `tokenize_owned`), the `match_one` precedence table, and `inline_to_source` (the inverse that re-emits the source).
  One matcher family per sibling module: `emphasis.rs` (delimiter pairs), `reference.rs` (`[[name]]`, `((blk-…))`, `!((blk-…))`, `#tag`, `is_valid_block_handle`), `link.rs` (`[text](url)`, `![alt](url)`), `shortcode.rs` (`:emoji:`).
  The token vocabulary lives in `token.rs` and prose flattening (`plain_text`) in `plain.rs`.
  All of them are private modules re-exported through `inline`, so every `outl_md::inline::…` path stays stable.
  `==highlight==` (Roam's `^^highlight^^` on import) rejects a space adjacent to either marker so a spaced `==` operator stays plain — unlike `~~strike~~`.
  `![alt](url)` (`InlineTok::Image` / owned `InlineToken::Image { alt, href }`) is the image / embedded-asset token.
  `try_image` runs right after `try_embed` in `match_one` so the leading `!` is never stranded before `try_md_link` claims the bare `[`.
  Alt may be empty (`![](url)`); the url must be non-empty; `![[…]]` (Obsidian wiki-embed) does not match and is left to `wikilink::convert_image_links`.
  Clients decide `<img>` vs file chip by inspecting the url extension (`wikilink::is_image_target`).
- **Cursor introspection** (`cursor.rs`) — "what token sits under the caret?", re-exported through `inline` so `outl_md::inline::{…}` paths still resolve.
  `ref_at_cursor` resolves to a navigable `RefTarget::{Page, Journal, Tag, Block}`;
  `link_at_cursor` resolves a markdown link `[text](url)` under the caret (anchor OR url) and returns its URL — the building block for a client that opens links externally (the TUI's `g x`, issue #183);
  `byte_index_for_char` converts a char index (cursor column) to a byte offset.
  **UI-agnostic.**
  TUI, future Tauri GUI, and mobile clients all consume the same `InlineTok` / `RefTarget` types and map them to their own primitives (`Span`, HTML, `AttributedString`, `AnnotatedString`).
  Two forms:
  - `InlineTok<'a>` + `tokenize` — borrowed, zero-copy.
    Use inside Rust where the source string outlives the tokens.
  - `InlineToken` (owned) + `tokenize_owned` — Serde-friendly, suitable for wire payloads.
    `outl-actions` attaches the result to `OutlineNode.tokens` so mobile renders without a TS tokenizer.
    Adding a variant to `InlineTok` requires adding the matching variant to `InlineToken` plus the conversion in `InlineToken::from_borrowed` in the same change.
  **Underscore emphasis rule (CommonMark):** `_` does not open or close emphasis when it appears inside a word (no surrounding whitespace/punctuation on both sides).
  `chamados_chat`, `inc_lag1`, `prod.ml_atendimento` stay literal.
  `*` is not subject to this restriction — it works mid-word.
  Enforced in `try_italic_under` / `try_bold_under` via the `closing_underscore` helper.
  `inline.rs` was split by responsibility to stay under the file-size guard, so new code has one obvious home.
  A token variant goes in `token.rs` (both forms, same change); a new matcher goes in the sibling module for its family, plus its slot in `match_one`; a caret-resolution helper goes in `cursor.rs`.
- **External frontmatter** (`frontmatter.rs`) — metadata extraction for markdown authored by other tools.
  `split_frontmatter` splits the leading `---` fence off a `.md` body (CRLF-safe, honours the `...` end marker; no closing fence → whole file stays body).
  `parse_frontmatter(yaml, drop_keys) → Frontmatter { title, props, dropped }` flattens the YAML into `key:: value` properties: `title` lifted, `tags` normalized to `#name`, caller-supplied drop-list, values verbatim.
  Date normalization is caller policy because the flexible date parser lives in `outl-actions`, which depends on this crate.
  `extract_leading_h1` lifts a leading `# H1` line into a title (first non-blank line only).
  Consumed by the CLI importers (Obsidian today); source-specific key policy stays with the caller.
- **External wiki-link rewriting** (`wikilink.rs`) — `rewrite_wikilinks` / `clean_wikilink_target` collapse `[[Note|alias]]` / `[[Note#heading]]` / `[[Note^block-id]]` / `[[folder/Note]]` to canonical `[[Note]]`;
  `convert_image_links` / `is_image_target` turn image wiki-links and embeds (`![[img.png]]`, `[[a/b.jpeg|cap]]`) into standard CommonMark links with the folder path preserved.
  Pure text → text; no vault layout or routing policy.
- **Tag predicate** (`tag.rs`) — `text_contains_tag(text, tag)`: boundary-correct "does this text mention `#tag`?" built on the tokenizer.
  `#tag-longer` / `#tagged` never match `tag`; a `#tag` inside a `` `code` `` span is not a tag.
  Consumers must use this instead of `text.contains("#tag")` (the substring form is the false-positive bug this module deleted from the CLI).
- **Block index** (`block_index.rs`) — `NodeId → BlockEntry`, `ref_handle → NodeId`, `NodeId → [BlockReference]` (reverse refs), `(slug, dfs_path) → NodeId` for location lookup.
  Population is two-pass (`collect_page_blocks` then `collect_page_refs`) so reverse edges survive arbitrary page-load order during the initial build.
  Lookups are O(1).
- **Workspace index** (`index.rs`) — page-level (`slug → PageEntry`) plus block-level (re-exports the `BlockIndex` API).
  **Does not carry backlinks.**
  Backlinks live in `outl_actions::backlinks` / `outl_actions::backlinks_index` so every client computes them straight from the `Workspace` — an earlier parallel cache on this index hid self-references on one surface while the other showed them.
  Public surface includes `resolve_block_ref(handle)`, `block_by_id`, `block_at_location(slug, &[usize])`, `block_refs_to(id)`, `iter_blocks`, `block_count`, `search_block_text(query, limit)`.
  `block_index()` borrows the inner `BlockIndex` so a consumer that already holds a `WorkspaceIndex` can reuse its primitives through one value.
  `block_at_location` is the O(1) replacement for scanning `iter_blocks()` to find the entry for a known `(page, dfs_path)`, e.g. when the TUI translates a keyboard chord onto a specific block.
  `PageEntry` carries the page-level metadata every UI surface reads (`slug`, `title`, `icon`, `is_journal`, `pinned`, **`page_type`**);
  `pages_by_type(t)` filters pages by their `type::` property (case-insensitive), powering the `@` mention autocomplete that lists `type:: person` pages.
- **Slugify** (`slug.rs`) — `[[Avelino]]` → `pages/avelino.md`.
  The user-facing name is preserved verbatim in the page's `title::` property.
- **`derive_ref_handle(NodeId) -> String`** (`sidecar.rs`) — deterministic: `blk-` + last 6 chars of the ULID's Crockford base32, lowercased.
  Same input always yields the same handle so two devices agree on what `((blk-XXXXXX))` means.
  On a collision inside a single workspace, the **second** block to land gets its handle lazily expanded one character at a time (drawing from the same ULID tail) until unique — both the winner and the loser stay independently resolvable.
  The sidecar still records the deterministic 6-char form; the expanded handle lives in `BlockEntry.ref_handle` in memory and in the workspace handle map.
- **`BlockEntry.text_fold: String`** — lowercased cache of the block's `text`, populated at index build.
  Powers `search_block_text` without allocating per keystroke.
  Public field, but consumers must not build `BlockEntry` by hand — go through the index population path so `text_fold` stays consistent with `text`.
- **Asset links** (`asset.rs`) — the pure text/hash primitives behind `[name](assets/<hash>.<ext>)`.
  `ASSETS_DIR` (`"assets"`), `hash_bytes(bytes) -> String` (hex SHA-256, used as the on-disk filename stem so identical uploads dedupe to one file), `asset_rel_path(hash, ext) -> String`.
  `is_asset_link(url) -> bool` distinguishes a workspace-relative `assets/…` / `./assets/…` / `/assets/…` link from an external scheme.
  Re-exported at the crate root as `outl_md::{asset_rel_path, hash_bytes, is_asset_link, ASSETS_DIR}`.
  This module never touches the filesystem — the copy-into-workspace step lives in `outl_actions::asset` (`import_asset`, `resolve_asset_path`), and cross-device transfer lives in `outl-sync-iroh`.

- **Crash-safe file I/O** (`atomic.rs`) — `write_atomic` (tmp + fsync + rename, then fsync of the parent directory so the rename itself survives a power loss) and its read counterpart **`read_for_rewrite`**.
  `read_for_rewrite` treats a missing file as empty (a page that doesn't exist yet is legitimately an empty AST) and **propagates every other I/O error**.
  Any read-parse-mutate-render-write path must use it.
  `fs::read_to_string(p).unwrap_or_default()` turns a transient `EIO` — or an iCloud placeholder whose bytes haven't been downloaded — into an empty AST that is then rendered over the real page.
  The sidecar is rebuilt to match, so the hashes agree and no later scan can detect the loss.
  That was a P0 (`outl-actions::journal::apply::mutate_page_md`, `outl-actions::outline`, the TUI's page load).

## What this crate does NOT own

- The op log → `outl-core`
- File watching / debounce → `outl-cli`
- Reconcile TUI → `outl-tui`
- Network sync → `outl-sync-iroh` (P2P via iroh, default transport; file/iCloud opt-in)

## The 3-level matching algorithm

When an external save lands on `pages/foo.md`:

1. **Parse** new `.md` → AST without IDs.
2. **Load** `foo.outl` (sibling of `foo.md`) → AST with old IDs and content hashes.
3. **Match** new ↔ old blocks at 3 confidence levels:

| Level | Confidence | Criteria | Action |
|-------|-----------|----------|--------|
| 1 | High | `content_hash` exact match, same parent (by hash) or identical structure | Preserve ID, emit `Move` if position changed |
| 1.5 | High | Equal block counts + same DFS index + same indent + **same parent** | Preserve ID, emit `Edit` (+ `Move` if needed) |
| 2 | Medium | Normalized Levenshtein similarity > 80% against `SidecarBlock::text`, **and** DFS index within ±2 (unconditional) | Preserve ID, emit `Edit` (+ `Move` if needed), log warning |
| 3 | Low / no match | Falls through | New ULID for new block; old block becomes `Delete` (`Move` to `TRASH_ROOT`); record in `.outl/orphans.log` |

**Hard rule:** a block that drops to level 3 must appear in `orphans.log` before being deleted.
**Silent deletion is a P0 bug.**

**Second hard rule: level 3 says *what* to delete, never *how much*.**
One orphan and five thousand come back in the same `Vec<NodeId>`.
So a `.md` that arrived truncated — an iCloud placeholder whose bytes never downloaded, a half-flushed write, a parser that stopped reading the dialect halfway — empties a page as quietly as deleting one bullet.
Any caller that turns orphans into `Move(node, TRASH_ROOT)` goes through **`matching::guard::match_blocks_guarded`**, not the raw `match_blocks`.
Three properties it is built to have, which are also the review checklist for changing it:

- **It cannot lose half a page.**
  `match_blocks` is pure, so refusing after it ran is refusing before anything exists to apply.
- **It is never silent.**
  The refusal is `Err(MatchGuardError::BulkDelete { volume, trip })`, not a shortened orphan list.
  Quietly dropping the deletions leaves the blocks in the tree and out of the `.md`, which is the divergence the reconcile exists to close.
- **It has a way out.**
  `OrphanGuard::Disabled` is what a caller wires to the user saying "yes, I meant that" — today `outl reconcile --allow-bulk-delete`.
  Reachable only from an explicit act; a retry is not consent (root `CLAUDE.md` invariant 9 and RFC 0211 name a guard with no escape hatch as its own defect class).

The defaults, and why each number is what it is:

| Constant | Value | Why |
|---|---|---|
| `MAX_ORPHANED_BLOCKS` | 500 | No hand edit removes five hundred blocks from one page in one save. An unattended import legitimately might, and that is exactly the caller that should have to say so out loud. |
| `MAX_ORPHANED_RATIO` | 0.75 | Deliberately high. Deleting a section is ordinary editing and costs well under half a page, while a truncated read takes essentially all of it. RFC 0210 already recorded what a guard that fires on real edits costs: it gets disabled, and then it guards nothing. |
| `RATIO_FLOOR_BLOCKS` | 20 | A ratio is meaningless on a four-block scratch note. Under the floor only the absolute arm applies, which cannot fire that low — small pages are unguarded on purpose: small blast radius, high false-positive cost. |

**Level 1.5 compares parents, not just indents.**
Indent is depth, not identity: two blocks at the same depth can live in different subtrees, and matching on indent alone handed one subtree's id — plus its `((blk-…))` handle — to a block in another.
Parents are compared through the ids matching already resolved (DFS preorder guarantees a parent is resolved before its children), so an unresolved parent counts as disagreement — the conservative answer.
A rejection here isn't fatal: level 2 can still recover the id on similarity, and it warns when it does so across parents.

**Level 2 exists for one specific save**: the user edits the `.md` outside outl and, in a single write, rewords one block *and* adds or removes another.
The counts disagree, level 1.5 is out of play, and before level 2 every reworded block minted a fresh ULID while the old id went to the trash with every reference to it dangling.
That is the *common* external edit, not an exotic one.
Implementation notes:

- Similarity runs against `SidecarBlock::text` via `strsim::normalized_levenshtein`.
  An entry with no recorded text — written before the field existed, or by a peer binary that doesn't know it — doesn't fire level 2, so behaviour degrades to exactly what shipped before, never worse.
  **The gate is the empty string, never the version number** (see below).
- A length-ratio pre-filter (`min_len / max_len <= 0.8` ⇒ reject) skips the O(n·m) DP for pairs that could not have cleared the threshold anyway.
  It's exact, so it can never discard a real match.
- Blocks over 4096 chars are skipped: at that size it's a pasted document, not a reworded sentence.
- The ±2 DFS window is **unconditional** — parent agreement is not an alternative gate.
  It used to be skipped whenever the parents agreed, but `parents_agree(None, None)` is `true`, so every pair of *root* blocks agreed and the window never fired on a journal page.
  `same_parent` today only selects which of the two warnings is logged.
- **Assignment is by global confidence, never by document order** (`src/similarity.rs`).
  Every in-window pair is scored first, then resolved from the highest score down, and the runner-up margin is **two-sided**: a winner must beat the best other claim on its new block *and* the best other claim on its old entry.
  Walking `0..flat.len()` and taking each new block's first above-threshold candidate let a freshly typed block steal the id — and the `ref_handle` — of the block it merely resembles, purely by sitting at a lower index.
  The real owner then fell to level 3 with a fresh ULID, and because the old id *was* consumed, `orphans` came back empty and nothing reached `orphans.log`.
  Pinned by `tests/similarity_contention.rs`.
- The match is `MatchLevel::Medium`, which `diff_to_ops` already treats like level 1 for id **and** `ref_handle` preservation (invariant 6).

## Sidecar format

Current version: `2`.
Full spec in [`docs/markdown-format.md`](../../docs/markdown-format.md#the-outl-sidecar).

```json
{
  "version": 2,
  "page_id": "01HXY8KJZQ9T8M7VN3P2R6S4A0",
  "last_synced_hash": "sha256:...",
  "last_synced_at": "2026-05-24T11:22:00-03:00",
  "blocks": [
    {
      "id": "01HXY8KJZQ9T8M7VN3P2R6S4A1",
      "line": 1,
      "indent": 0,
      "content_hash": "sha256:...",
      "ref_handle": "blk-r6s4a1",
      "text": "decide the storage backend"
    }
  ]
}
```

Build entries through **`SidecarBlock::from_text(id, line, indent, text)`** so hash, handle, and stored text always describe the same revision.
Only a caller preserving a previous (expanded) handle should build the literal and override `ref_handle`.

- `content_hash` = SHA-256 of the **block's textual content** (not children).
- `ref_handle` = short user-typeable handle for `((blk-XXXXXX))`. v1 sidecars (no field) load fine — the handle is backfilled in memory via `derive_ref_handle`.
  The next write persists the current version.
  On collision, expansion may produce a 7+ char form (see `derive_ref_handle` above).
- `text` = the block's content **as of the last sync**, verbatim and untruncated — the "before" side of level-2 matching.
  Optional and additive: a payload without the field loads fine with an empty string and level 2 skips those blocks.
  There is nothing to backfill it from, since the whole value of the field is holding the text as it was *before* the `.md` changed.
  **Trade-off, decided deliberately:** this duplicates the `.md` body inside the sidecar.
  A truncated prefix (or prefix + length) would be smaller, but it makes two blocks sharing a long opening look identical.
  A level-2 false positive then hands one block's id *and its `ref_handle`* to a different block — the exact corruption matching exists to prevent.
  The sidecar is a rebuildable cache sitting next to the file it describes, so the cost is disk, and the alternative's cost is user trust.

### Sidecar versioning (both directions)

`version` answers one question for a reader that did not write the file: *can I still trust the fields I know?*
Canonical spec: [`docs/markdown-format.md` → Sidecar versioning](../../docs/markdown-format.md#sidecar-versioning).
The short version, because getting it wrong is a P0:

- **Backward** (new binary, old payload) — always supported down to `MIN_READABLE_SIDECAR_VERSION`.
  Never drop a read path.
- **Forward** (already-shipped binary, new payload) — every released binary rejects `version > its own SIDECAR_VERSION`, and you cannot patch the copies already on users' machines.
  An unreadable sidecar used to look exactly like a missing one downstream: no old blocks → every block at level 3 → fresh ULID each while the old ids stayed in the tree.
  One boot of a stale device on a shared iCloud folder duplicated the page and rotated every `((blk-…))` handle, and the newer binary did the same in reverse next boot.
  TestFlight lag and closed laptops mean the fleet is *always* mixed.

Therefore:

1. **Adding an optional field does NOT bump `SIDECAR_VERSION`.**
   `#[serde(default)]`, and detect the feature by **field presence, never by version number**.
   `pipeline_version` and `text` are both additive and both live at version `2`.
2. **Bump only when an older reader would _misread_ the file** — an existing field changes meaning, changes encoding, or disappears.
   Then `UnsupportedVersion` on the old side is the *desired* outcome.
3. A bump is a coordinated release with a migration note and an `outl doctor` path, not a patch.

`reconcile_md` **propagates** `UnsupportedVersion` rather than treating the page as sidecar-less — a newer peer's file is not corruption, and rebuilding over it is what turns a version mismatch into duplicated blocks.
Unparseable JSON still rebuilds (never block on a corrupt sidecar).
Pinned by `tests/mixed_version_sidecar.rs`, which models the shipped v2 binary explicitly and runs it against the current one over the same files.

**The withheld `last_synced_hash` (invariant 8) is not free on old peers, and no value is.**
An empty hash makes a shipped binary re-read the page with its own parser; a real one authorises it to render the tree straight over the `.md`.
Its two gates are complementary, so nothing disarms both, and moving the signal into `version` arms the duplication loop above.
Measured numbers, the refuted "the guard never re-arms" claim, and four regression tests live in the `tests/mixed_version_sidecar.rs` module doc.
Open in [issue #210](https://github.com/outlmd/outl/issues/210).

**Sidecar is not a sync surface.**
UI state that must converge between devices — fold flags, pinned, selection, anything user-meaningful — goes through the op log (`outl-core`), not here.
iCloud / Syncthing sync the sidecar file as one blob with last-write-wins semantics, so two devices flipping different fields in the same window lose data.
The op log gives each actor its own jsonl, lets the FS sync per-file without conflict, and reconverges through the CRDT.
See the root `CLAUDE.md` invariant 7.
- Sidecar lives next to the `.md` as `pages/<slug>.outl` (no leading dot).
  Replicated between devices alongside the `.md`.
  Don't gitignore by default.
  The dotfile form (`.foo.outl`) was abandoned because iCloud Documents skips dotted paths during cross-device sync.
- **Stale entries are skipped during index build.**
  When a sidecar block's `content_hash` no longer matches the corresponding block in the `.md`, that entry is left out of the workspace index instead of polluting it with a wrong subtree.
  The block reappears in the index after the next reconcile updates the sidecar.

## Outl markdown dialect

> **What it costs to add a token to this dialect:** [RFC 0008](../../docs/rfcs/0008-markdown-dialect-and-sidecar-tokens.md).

```markdown
title:: example
status:: active
tags:: #project

- top level block
  priority:: high
  - child block with [[page reference]]
  - child block with ((blk-r6s4a1))
  - expanded inline: !((blk-r6s4a1))
- another top level
```

- `key:: value` lines at top of file = page properties (frontmatter outliner-style).
- `key:: value` lines nested as children of a block = block properties.
- `[[name]]` = page reference (bidirectional link).
- `[[2026-05-24]]` = journal reference (renders as date).
- `#tag` = tag (page reference with classification semantics).
- `((blk-XXXXXX))` = inline block reference (renders as the source block's text).
- `!((blk-XXXXXX))` = block embed (renders source block expanded with subtree).
- `{{query: ...}}` = inline query token (legacy; parsed as opaque text; the ` ```query ` code block is the supported path — see `docs/query.md`).

**No `id::`, no UUID, no HTML comments** — IDs go in the sidecar only.

## Files

```
src/
├── lib.rs
├── parse.rs        # md → AST (no IDs): the grammar + the block-list reader
├── ast.rs          # OutlineNode, ParsedPage, ParseWarning(Kind) — re-exported by parse
├── property.rs     # `key:: value` line + the page-property header run (private mod)
├── fence.rs        # fenced code: literal capture while the outline grammar is suspended
├── render.rs       # AST → md (clean)
├── sidecar.rs      # read/write .outl JSON, derive_ref_handle, content_hash
├── matching.rs     # 3-level matching algorithm
├── matching/
│   └── guard.rs    # match_blocks_guarded — volume guard over level-3 orphans (OrphanGuard, OrphanVolume)
├── similarity.rs   # level-2 scoring + global-confidence assignment (private to the crate)
├── unlogged.rs     # content_lines_missing_from, sidecar_can_answer — "does the op log know this line"
├── diff.rs         # AST diff → Op sequence (takes old_blocks to preserve ref_handle)
├── inline.rs       # the scan: tokenize / tokenize_owned, match_one precedence, inline_to_source
├── token.rs        # InlineTok (Plain/Bold/.../BlockRef/Embed/Emoji), owned InlineToken, RefTarget
├── emphasis.rs     # `**`/`__`/`*`/`_`/`~~`/`==`/backtick matchers + the intra-word `_` rule
├── reference.rs    # [[name]], ((blk-…)), !((blk-…)), #tag matchers + is_valid_block_handle
├── link.rs         # [text](url) / ![alt](url) matchers (try_md_link reused by cursor)
├── shortcode.rs    # :emoji: matcher — shape gate + emoji-catalog gate
├── plain.rs        # plain_text — flatten tokens to the prose a human reads
├── cursor.rs       # ref_at_cursor, link_at_cursor, byte_index_for_char (re-exported via inline)
├── emoji.rs        # shortcode_to_unicode, search, is_valid_shortcode, EmojiHit
├── frontmatter.rs  # split_frontmatter, parse_frontmatter, extract_leading_h1 (external md metadata)
├── wikilink.rs     # rewrite_wikilinks, clean_wikilink_target, convert_image_links, is_image_target
├── lang.rs         # canonical(fence) — alias table shared by outl-exec + frontend syntax highlighter
├── index.rs        # WorkspaceIndex — page-level + block-level facade
├── block_index.rs  # BlockEntry, BlockReference, BlockIndex (id ↔ handle ↔ reverse refs)
├── reconcile.rs    # high-level reconcile_md (parse → match → diff → apply)
├── slug.rs         # slugify page names
├── tag.rs          # text_contains_tag — boundary-correct #tag predicate over the tokenizer
├── view.rs         # render helpers consumed by UIs
├── asset.rs        # ASSETS_DIR, hash_bytes, asset_rel_path, is_asset_link (pure; no filesystem)
└── atomic.rs       # crash-safe write_atomic + its read counterpart read_for_rewrite

tests/
├── roundtrip.rs              # render(parse(md)) == md (property test)
├── external_edit.rs          # light external edit preserves IDs
├── edit_and_delete.rs        # end-to-end: one save rewords a block AND adds/removes another (level 2)
├── duplicate_block.rs        # Ctrl+D in vscode → first keeps ID, second gets new
├── identical_blocks_swap.rs  # two identical blocks change parents
├── heavy_edit.rs             # >20% content change → level 2 warning
├── similarity_contention.rs  # two new blocks claim one old entry: confidence decides, not index order
├── mixed_version_sidecar.rs  # shipped v2 binary + current one over one folder: no dup, no handle rotation
└── multiline_block_roundtrip.rs  # render → parse roundtrip for multi-line/blank-line/indented block text (issue #210 producer)

benches/
└── block_index.rs            # resolve / search_block_text on 100k blocks
```

## Bench harness

`cargo bench -p outl-md --bench block_index` measures the cost the `((blk-XXXXXX))` path adds to the index.
Today's numbers (M-series laptop):

- `resolve(handle)` — ~17 ns at 100k indexed blocks.
  O(1) HashMap hit.
- `search_block_text(query, limit)` — ~12 ms at 100k blocks (linear scan with case-fold + position scoring).
  Suitable for the autocomplete popup the TUI uses today; future fzf-style scoring can drop in behind the same signature.

## The corpus gate

`tests/corpus_gate.rs` runs three properties over `tests/corpus/`, and it exists because of a measured fact about this crate:

> Across issue #210 — the original defect and all four regressions introduced while fixing it — **every** one was found by running code over 2,827 real `.md` files, and **none** by the unit suite, which was green (237 tests here) at every one of those moments.

The unit tests pin shapes once someone knows them. The corpus is for the shapes nobody thought of: odd indentation, tabs, Roam leftovers, bullets inside fences, unicode separators pasted from PDFs.

The three properties, in the order they were learned:

1. **No line is lost** across `parse → render`.
2. **`render → parse` is a fixpoint** — not merely lossless.
   A document that changes shape on every save mutates the user's file forever and emits an `Op::Edit` per reconcile, which is worse than the bug it replaced: that one at least converged.
3. **The unlogged-content check does not cry wolf.**
   A page the log fully accounts for must not be reported, because that verdict withholds `last_synced_hash` and refuses re-projection — it freezes a page that has nothing wrong with it.

Property 3 did not exist until the defect it catches had already shipped, inside the same PR that named false positives as the worst failure mode available.

**Maintenance rule, the whole of it: when a `.md` bug is found in the wild, its shape becomes a file in `tests/corpus/`.**
Reduce it to the smallest input that reproduces, and do not clean it up — the ugliness is the point.

## Invariants

1. **Roundtrip stability.** `render(parse(md))` produces a semantically identical `.md` (same tree, properties, content; whitespace may normalize).
2. **No silent block loss.**
   A block falling to level 3 is always in `orphans.log`.
3. **Sidecar is JSON-valid.**
   Always.
   If you can't write valid JSON, you fail.
4. **Sidecar `version` field always present.**
   Future migrations.
5. **`content_hash`** is `sha256(block.content_text())` consistently.
   Same hash function across read and write.
6. **`ref_handle` is preserved across level-1 and level-2 matches.**
   `diff_to_ops` reads it from the previous sidecar's block list and reuses it verbatim,
   so a `((blk-XXXXXX))` already written in another `.md` keeps resolving even if the handle was once expanded past the default 6-char tail.
7. **`derive_ref_handle` is deterministic** — same `NodeId` in, same handle out.
   Two devices building the sidecar independently must agree on what `((blk-XXXXXX))` means.
8. **`last_synced_hash` may only be advanced over content the same call emitted ops for.**
   Writing that hash is a claim — "the op log holds what is in this file" — and every consumer downstream believes it.
   `reconcile_md` is the one place that both reads a `.md` and rewrites its sidecar, so it is the one place that can make the claim falsely.
   Stamp the hash over a block that produced no op and the page becomes hash-faithful with unlogged content.
   That reads as a merely *stale* projection to `apply_page_md_with_sidecar_if_stale`, and gets deleted on the next page open.
   Measured cost on a real workspace: 233 pages, 1,426 lines ([RFC 0210](../../docs/rfcs/0210-md-content-outside-op-log.md), root `CLAUDE.md` invariant 8).
   If you cannot emit an op for something you read, **do not advance the hash**.
   Leave the page looking dirty so the reconcile runs again: a page that reconciles twice is a nuisance, a page that lies about its own state is a data-loss bug.

   **Enforced, not just followed.**
   `reconcile_md` asks `unlogged::content_lines_missing_from` before writing the sidecar and, when it returns anything, writes an empty `last_synced_hash` instead of the real one so the page is looked at again next pass.
   `ReconcileReport.unlogged_lines` carries the count for callers to surface.
   `unlogged.rs` lives here (not `outl-actions`) so the producer can ask the same question its consumers ask; `outl_actions::content_lines_missing_from` / `sidecar_can_answer` are re-exports.
   Pinned by `tests/multiline_block_roundtrip.rs` (`the_hash_is_still_advanced_for_every_shape_a_real_workspace_holds` and siblings).
   The bulk-delete half has the same shape: `reconcile_md_with_guard` refuses the whole pass (`ReconcileError::BulkDelete`) instead of trashing an oversized orphan list — see "Second hard rule" under the matching algorithm.

## Things to never do here

- ❌ Write IDs into the `.md` file (use sidecar)
- ❌ Delete a block in matching without recording in `orphans.log` first
- ❌ Advance `last_synced_hash` over content this call did not emit ops for (invariant 8)
- ❌ Match on similarity > 80% across **different parents** without warning
- ❌ Skip the property test in `roundtrip.rs`
- ❌ Use a different hash function in sidecar read vs write
- ❌ Drop an older sidecar version's read path when adding a new one (always backward read)
- ❌ Bump `SIDECAR_VERSION` for a field that is merely **additive**.
  Every already-shipped binary refuses the file, and downstream a refused sidecar reads as a missing one: fresh ULID per block, every ref handle rotated, duplicates on both sides of the sync.
  Use `#[serde(default)]` + presence-based detection instead
- ❌ Gate a feature on the sidecar version number when the field's presence already answers the question
- ❌ Build a `SidecarBlock` literal when `SidecarBlock::from_text` would do — that's how `content_hash` and `text` end up describing different revisions
- ❌ Block on a corrupt sidecar — fall back to "regenerate from op log" via `outl doctor`

## Reuse-first

This crate owns the **shared parsing and view primitives** every client needs (`char_to_line_col` / `line_col_to_char`, `block_to_rows`, `tokenize`, `slugify`, `derive_ref_handle`, …).
Clients should *wrap* these, not re-derive them.

When you add a primitive, pair it:
`char_to_line_col` already existed;
the recent `line_col_to_char` addition made the pair complete so `outl-tui::EditBuffer::move_up` / `move_down` could be 3-line wrappers instead of duplicating the line/column scan.
**Inverses, encoders/decoders, and parser/renderer pairs always ship together** so the next consumer doesn't have to re-derive half of one.

If you find a client (`outl-tui`, `outl-mobile`, `outl-actions`) hand-rolling something that's already here, move the call to your API and delete the duplicate.
The root [`CLAUDE.md`](../../CLAUDE.md#reuse-first) documents this at the workspace level, in full at [`docs/contributing.md`](../../docs/contributing.md#reuse-first-no-parallel-implementations).

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-md -- -D warnings`
3. `cargo test -p outl-md`
4. `/roundtrip` to invoke the markdown-roundtrip-tester agent
5. Manual smoke: create a fixture md, render it back, diff
