# Shared primitives — markdown pipeline

Everything between the `.md` bytes on disk and the op log: parsing, rendering, external-markdown coercion and ingest, reconcile / matching / diff, the `.outl` sidecar.
Plus what reads that layer back — in-flight outline AST helpers, indices and search, inline tokenizers, editor view helpers, and asset links.
If a primitive reads or writes markdown, it lives here.

Part of the **Shared primitives catalog** — the index of every part lives in [`shared-primitives.md`](shared-primitives.md).

**Before writing any helper, scan these tables first.**
Most "I need a small string transform / id helper / md coercion / tree walk" needs already have an owner here —
the cost of finding the existing one is a `grep`;
the cost of missing it shows up later as drift between two parallel implementations (the user is the one who hits the divergence).

For the reuse-first rule (why this matters, past drift incidents, what to do when a primitive doesn't exist yet), see [Contributing → Reuse-first](contributing.md#reuse-first-no-parallel-implementations).

---

## 1. Parse / render (outl-md::parse + render)

| Intent | Use this | File |
|---|---|---|
| Parse `.md` → outline AST (no IDs) | `outl_md::parse::parse` → `ParsedPage` (includes `warnings: Vec<ParseWarning>`) | `crates/outl-md/src/parse.rs` |
| Render outline AST → `.md` (clean, no IDs) | `outl_md::render::render` | `crates/outl-md/src/render.rs` |
| Non-fatal parser recovery records (heading instead of bullet, etc.) | `outl_md::ParseWarning` + `outl_md::ParseWarningKind` (re-exported from `parse`) | `crates/outl-md/src/ast.rs` |
| The outline AST node DTO (UI-friendly, no `Workspace` coupling) | `outl_md::OutlineNode` / `outl_actions::outline::OutlineNode` | `crates/outl-md/src/ast.rs` + `crates/outl-actions/src/outline.rs` |
| Read one `key:: value` line (page property or block property) — never hand-roll the `::` split, both positions share this reader | `outl_md::parse::parse_property_line` (re-exported from `property`) | `crates/outl-md/src/property.rs` |
| Project the workspace tree under a node into the UI DTO | `outl_actions::outline::project_outline` / `project_outline_node` | `crates/outl-actions/src/outline.rs` |
| Project a **parsed** subtree (`.md` AST, no sidecar) into wire `OutlineNode`s with `tokens` attached — ids are **transient** (fresh per call), for read-only surfaces that re-resolve on navigation (the `!((blk))` embed subtree expansion) | `outl_actions::outline::project_parsed_subtree` (re-exported `outl_actions::project_parsed_subtree`) | `crates/outl-actions/src/outline.rs` |
| Flatten an `OutlineNode` subtree to DFS paths (for selection / navigation) | `outl_actions::outline::flatten_subtree_paths` | `crates/outl-actions/src/outline.rs` |
| Read a page from disk + project to outline view in one call | `outl_actions::outline::read_page_view` / `read_page_view_with_workspace` | `crates/outl-actions/src/outline.rs` |
| Read a page **and** surface parser warnings (banner, doctor, status line) | `outl_actions::outline::read_page_outline` / `read_page_outline_with_workspace` → `PageOutline { nodes, warnings }` | `crates/outl-actions/src/outline.rs` |
| Slugify a user-visible page name into a filesystem-safe slug (lowercase, folds Latin diacritics `á` → `a`, non-alphanumerics collapse to `-`; empty input → `UNTITLED_SLUG`) — never hand-roll an ASCII-only copy | `outl_md::slug::slugify` / `UNTITLED_SLUG` | `crates/outl-md/src/slug.rs` |

---

## 2. External markdown coercion & ingest (outl-md::frontmatter + wikilink, outl-actions::paste + ingest)

| Intent | Use this | File |
|---|---|---|
| Coerce **external markdown** (line endings, indent unit 4→2, Roam/GitHub/Logseq tokens, long-form dates → ISO, strip `id::` with Crockford validation, strip unknown `{{…}}` / `^^…^^`) | `outl_actions::paste::normalize_external_syntax` | `crates/outl-actions/src/paste/normalize.rs` |
| Split a leading `---` YAML frontmatter fence off a `.md` body (CRLF-safe, honours `...` end marker, no-fence → verbatim body) | `outl_md::frontmatter::split_frontmatter` | `crates/outl-md/src/frontmatter.rs` |
| Parse a YAML frontmatter block into flat `key:: value` properties (`title` lifted, `tags` normalized to `#name`, caller-supplied drop-list; values verbatim — date normalization stays with the caller) | `outl_md::frontmatter::parse_frontmatter` → `Frontmatter` | `crates/outl-md/src/frontmatter.rs` |
| Lift a leading `# H1` line into a page title (first non-blank line only) | `outl_md::frontmatter::extract_leading_h1` | `crates/outl-md/src/frontmatter.rs` |
| Collapse external wiki-link variants (`[[Note\|alias]]`, `[[Note#h]]`, `[[Note^blk]]`, `[[folder/Note]]`) to canonical `[[Note]]` | `outl_md::wikilink::rewrite_wikilinks` (whole text) / `clean_wikilink_target` (one target) | `crates/outl-md/src/wikilink.rs` |
| Convert image wiki-links / embeds (`![[img.png]]`, `[[a/b.jpeg\|cap]]`) into CommonMark links, folder path preserved | `outl_md::wikilink::convert_image_links` (+ `is_image_target` predicate) | `crates/outl-md/src/wikilink.rs` |
| "Does this clipboard look like an outline?" classifier | `outl_actions::paste::looks_like_outline` | `crates/outl-actions/src/paste/mod.rs` |
| Convert clipboard markdown into outl ops grafted at a position | `outl_actions::paste::paste_markdown` → `PasteOutcome` (anchor described by `PasteAnchor`) | `crates/outl-actions/src/paste/mod.rs` |
| Paste raw text as a single block with no normalisation or outline parsing (the "without formatting" path) | `outl_actions::paste::paste_plain(workspace, hlc, anchor, raw)` → `PasteOutcome` | `crates/outl-actions/src/paste/mod.rs` |
| Serialize a block selection (+ subtrees) to clean outl markdown **for the clipboard** (the inverse of `paste_markdown` / `parse`) | `outl_actions::clipboard::copy_markdown` (workspace + `NodeId`s; GUI backends) / `copy_markdown_nodes` (already-projected `OutlineNode`s; the TUI's AST-first yank) | `crates/outl-actions/src/clipboard.rs` |

---

## 3. Reconcile & matching (outl-md::reconcile + matching + diff)

| Intent | Use this | File |
|---|---|---|
| Reconcile an existing `.md` against its sidecar (3-level matching → diff → min ops) | `outl_md::reconcile::reconcile_md` (no sidecar = fresh random id) / `reconcile_md_with_page_id` (pin id for first ingest) | `crates/outl-md/src/reconcile.rs` |
| Same reconcile, with an explicit orphan-volume policy — `reconcile_md` delegates here with `OrphanGuard::Enforced`; pass `OrphanGuard::Disabled` only from a confirmed user opt-in (`outl reconcile --allow-bulk-delete`) | `outl_md::reconcile::reconcile_md_with_guard` | `crates/outl-md/src/reconcile.rs` |
| Content lines a `.md` holds that **no block the op log knows** can account for — the single owner of "does the log know this line", asked by `reconcile_md` before advancing `last_synced_hash` (invariant 8), by `outl_actions::apply_page_md_with_sidecar_if_stale` before re-projecting, and by `outl doctor`'s read-only listing. Re-exported at `outl_actions::content_lines_missing_from` | `outl_md::unlogged::content_lines_missing_from` | `crates/outl-md/src/unlogged.rs` |
| Whether a sidecar's blocks can answer the question above at all — `false` for a pre-0.11 sidecar whose entries all carry `text: ""`; an empty verdict from a reference that cannot answer is not "nothing at risk". Re-exported at `outl_actions::sidecar_can_answer` | `outl_md::unlogged::sidecar_can_answer` | `crates/outl-md/src/unlogged.rs` |
| Reconcile every `.md` in a directory | `outl_md::reconcile::reconcile_dir` | `crates/outl-md/src/reconcile.rs` |
| Materialise a page root in the tree (`Op::Create` at root + `page-slug` / `page-kind` `SetProp`s, idempotent) without running the full reconcile pipeline | `outl_md::reconcile::ensure_page_root_in_tree` | `crates/outl-md/src/reconcile.rs` |
| Reconcile error / report types | `outl_md::ReconcileError` / `ReconcileReport` | `crates/outl-md/src/reconcile.rs` |
| 3-level matching algorithm (hash → positional fallback → similarity → orphan log); level 2 diffs `SidecarBlock::text` with normalized Levenshtein > 0.8 and preserves id + `ref_handle` | `outl_md::matching::match_blocks` → `Match` / `MatchLevel` | `crates/outl-md/src/matching.rs` |
| Same matching, with the **volume** of the resulting deletion checked before the caller can act on it. Use this wherever orphans become `Move(node, TRASH_ROOT)`: `match_blocks` alone treats 1 orphan and 5,000 identically, so a truncated `.md` (iCloud placeholder, half-flushed write) empties a page as quietly as deleting one bullet. Refusal is an `Err` over the whole pass — never a shortened orphan list | `outl_md::matching::guard::match_blocks_guarded` → `MatchGuardError` | `crates/outl-md/src/matching/guard.rs` |
| The thresholds that refusal uses, and the explicit opt-out a caller wires to a user-facing flag (`OrphanGuard::Disabled`, reached by `outl reconcile --allow-bulk-delete`). Defaults: `MAX_ORPHANED_BLOCKS` = 500 absolute, `MAX_ORPHANED_RATIO` = 0.75 of the page, ratio ignored under `RATIO_FLOOR_BLOCKS` = 20 known blocks | `outl_md::matching::guard::{OrphanGuard, OrphanVolume}` | `crates/outl-md/src/matching/guard.rs` |
| Diff old AST + new AST + old sidecar → minimum sequence of `Op`s | `outl_md::diff::diff_to_ops` → `DiffPlan` | `crates/outl-md/src/diff.rs` |
| Same diff, **plus** propagate page-level properties (`title::`, `type::`, `pinned::`, `icon::`, …) into the op log as `Op::SetProp` on the page root so the CRDT tree agrees with what's on disk (legacy `.md` files populated via fixtures / external editors get materialised here on the next reconcile) | `outl_md::diff::diff_to_ops_with_page_props` | `crates/outl-md/src/diff.rs` |
| Reconcile-pipeline version number stamped on every sidecar — orphan scanner re-runs `reconcile_md` when a sidecar's version is below this constant, so a binary that gains a new pipeline pass automatically rematerialises every legacy page on the next boot | `outl_md::sidecar::CURRENT_PIPELINE_VERSION` | `crates/outl-md/src/sidecar.rs` |

---

## 4. Sidecar (outl-md::sidecar + atomic)

| Intent | Use this | File |
|---|---|---|
| The full sidecar struct + per-block entries | `outl_md::Sidecar` / `SidecarBlock` | `crates/outl-md/src/sidecar.rs` |
| Construct a fresh sidecar for a new page | `outl_md::sidecar::Sidecar::new_for_page(page_id, &file_hash)` | `crates/outl-md/src/sidecar.rs` |
| Build one sidecar block entry — **the only way to build one** unless you're preserving an expanded `ref_handle`; keeps `content_hash`, `ref_handle` and the level-2 `text` derived from the same revision | `outl_md::sidecar::SidecarBlock::from_text(id, line, indent, text)` | `crates/outl-md/src/sidecar.rs` |
| Sidecar format version — currently `2`. **Feature-detect by field presence, never by version number**: an additive `#[serde(default)]` field (`text`, `pipeline_version`) does NOT bump it, because a reader that doesn't know the field still reads every field it does know correctly. Bump only when an existing field changes meaning or encoding — there, an older reader's `UnsupportedVersion` is the *desired* outcome. Bumping for an additive field is what makes an already-shipped binary reject the file, rebuild the sidecar from scratch, and duplicate every block | `outl_md::sidecar::SIDECAR_VERSION` / `MIN_READABLE_SIDECAR_VERSION` | `crates/outl-md/src/sidecar.rs` |
| Read / write sidecar (JSON, writes `SIDECAR_VERSION` = 2, backward-reads v1) | `outl_md::sidecar::read` / `write` | `crates/outl-md/src/sidecar.rs` |
| Sidecar path resolution for a `.md` | `outl_md::sidecar::sidecar_path_for` / `resolve_sidecar_path` | `crates/outl-md/src/sidecar.rs` |
| Derive `((blk-XXXXXX))` ref handle from `NodeId` (deterministic, collision-aware) | `outl_md::sidecar::derive_ref_handle` | `crates/outl-md/src/sidecar.rs` |
| Hash block / file content for sidecar (`content_hash` = single block; `file_hash` = whole `.md`) | `outl_md::sidecar::content_hash` / `file_hash` | `crates/outl-md/src/sidecar.rs` |
| Low-level crash-safe write (use the `journal::write_md_atomic` wrapper unless you have a reason) | `outl_md::atomic::write_atomic` | `crates/outl-md/src/atomic.rs` |
| Read a `.md` you are about to mutate and write back (missing → empty, every other I/O error propagates) | `outl_md::atomic::read_for_rewrite` | `crates/outl-md/src/atomic.rs` |
| Path of a workspace's orphan log — **every `reconcile_md` caller must pass this**, never `None`. The one owner: `outl_ws::layout::Paths::at` derives its `orphans` field from it rather than re-joining the path | `outl_actions::sync::orphans_log_path` / `SyncEngine::orphans_log` | `crates/outl-actions/src/sync.rs` |

---

## 5. In-flight outline AST helpers (outl-md::outline_ops)

These operate on `Vec<OutlineNode>` **before** the tree is rebuilt from the op log — typing into a buffer that hasn't been parsed back yet.
UI-agnostic; both TUI and mobile consume them.

| Intent | Use this | File |
|---|---|---|
| Flat count / TODO+DONE counts across an outline | `outline_ops::flat_count` / `count_todos` | `crates/outl-md/src/outline_ops.rs` |
| Convert flat index ↔ path / look up a node at a path | `outline_ops::path_for_index` / `index_for_path` / `node_at_path` / `node_at_path_mut` | `crates/outl-md/src/outline_ops.rs` |
| Count descendants under a path / grab a mutable siblings slice | `outline_ops::descendants_count_at_path` / `siblings_mut` | `crates/outl-md/src/outline_ops.rs` |
| Insert a sibling before / after a path | `outline_ops::insert_sibling_before` / `outline_ops::insert_sibling_after` | `crates/outl-md/src/outline_ops.rs` |
| Insert a sibling after a path, seeded with text (the TUI's in-flight block-split: tail of the split goes into the new sibling) | `outline_ops::insert_sibling_after_with_text` | `crates/outl-md/src/outline_ops.rs` |
| Indent / outdent / delete / move up / move down at a path | `outline_ops::indent_at_path` / `outdent_at_path` / `delete_at_path` / `move_up_at_path` / `move_down_at_path` | `crates/outl-md/src/outline_ops.rs` |

---

## 6. Indices and search (outl-md::index + block_index)

| Intent | Use this | File |
|---|---|---|
| Build / query the workspace-wide index (slug → page, backlinks, block lookups) | `outl_md::WorkspaceIndex::build` / `by_slug` / `by_title` / `pages` / `pages_by_title_prefix` / `pages_by_type` | `crates/outl-md/src/index.rs` |
| Populate that index from the **op-log tree** instead of from disk (id-carrying nodes, no sidecar, cannot drop a block on a stale hash) | `outl_md::IdentifiedNode` + `WorkspaceIndex::insert_page` / `collect_page_blocks_from_tree` (per page) / `collect_refs_from_indexed` (once, after every page is in) | `crates/outl-md/src/index.rs`, `block_index.rs` |
| Patch / remove a page in an existing index | `WorkspaceIndex::patch_page` / `remove_page` | `crates/outl-md/src/index.rs` |
| Resolve `((blk-XXXXXX))` to a block / look a block up by id or location | `WorkspaceIndex::resolve_block_ref` / `block_by_id` / `block_at_location` | `crates/outl-md/src/index.rs` |
| Reverse refs to a block / iterate / search | `WorkspaceIndex::block_refs_to` / `iter_blocks` / `search_block_text` / `block_count` / `block_index` (borrow the inner `BlockIndex`) | `crates/outl-md/src/index.rs` |
| Stand-alone block-level index (when you don't need the page facade) | `outl_md::BlockIndex` + `BlockEntry` + `BlockReference` | `crates/outl-md/src/block_index.rs` |
| `PageEntry` DTO returned by `WorkspaceIndex` lookups | `outl_md::PageEntry` | `crates/outl-md/src/index.rs` |

---

## 7. View helpers for editors (outl-md::view + inline)

| Intent | Use this | File |
|---|---|---|
| Char ↔ (line, col) on a buffer (both TUI and mobile editors share) | `outl_md::view::char_to_line_col` / `line_col_to_char` | `crates/outl-md/src/view.rs` |
| Project a block to renderable rows (with `BlockRowKind` discrimination) | `outl_md::view::block_to_rows` → `BlockRow` / `BlockRowKind` | `crates/outl-md/src/view.rs` |
| Tokenize inline markdown (`**bold**`, `[[refs]]`, `#tags`, `((blk-…))`, `!((blk-…))`) | `outl_md::inline::tokenize` → `InlineTok` | `crates/outl-md/src/inline.rs` |
| Tokenize inline markdown into an **owned, Serde-friendly** form for wire / DTO payloads (mobile renders these straight; no parallel TS tokenizer) | `outl_md::inline::tokenize_owned` → `InlineToken` | `crates/outl-md/src/inline.rs` |
| Reconstruct the source markdown from a `Vec<InlineTok>` (Bold / Italic / Strike now carry recursively-tokenized inners; use this when a surface wants the whole inner span as one styled string instead of dispatching per-variant) | `outl_md::inline::inline_to_source` | `crates/outl-md/src/inline.rs` |
| Resolve the ref under a caret position (`Page` / `Journal` / `Tag` / `Block`) | `outl_md::inline::ref_at_cursor` → `RefTarget` | `crates/outl-md/src/cursor.rs` |
| Resolve the markdown link `[text](url)` under a caret position (anchor OR url) — the URL a client opens externally (TUI `gx`) | `outl_md::inline::link_at_cursor` → `Option<&str>` | `crates/outl-md/src/cursor.rs` |
| Does a text mention `#tag` as a whole tag token? Boundary-correct via the tokenizer (`#tag-longer` / `#tagged` never match `tag`; `#tag` inside a code span is not a tag) — never use `text.contains("#tag")` | `outl_md::tag::text_contains_tag` | `crates/outl-md/src/tag.rs` |
| Validate a `((blk-XXXXXX))` handle string | `outl_md::inline::is_valid_block_handle` | `crates/outl-md/src/reference.rs` |
| Flatten a block's inline markup to the prose a human reads (notification bodies, a11y labels, plain-text export) — refs / tags keep their name, emphasis drops its markers, `((blk-…))` resolves to nothing | `outl_md::plain_text` (also `outl_md::inline::plain_text`) | `crates/outl-md/src/plain.rs` |
| Byte offset for a char index (UTF-8 safe) | `outl_md::inline::byte_index_for_char` | `crates/outl-md/src/cursor.rs` |
| Canonicalize a fence info-string (`rs` → `rust`, `js`/`javascript`/`node` → `js`, …) — single source of truth for both `outl-exec`'s runtime dispatch and the frontend syntax highlighter | `outl_md::lang::canonical`, `outl_md::lang::KNOWN_ALIASES` | `crates/outl-md/src/lang.rs` |
| Resolve a `:shortcode:` to its unicode glyph (one-way; never retro-translate glyph → shortcode, multiple shortcodes can alias the same codepoint) | `outl_md::emoji::shortcode_to_unicode` | `crates/outl-md/src/emoji.rs` |
| Validate the `[a-z0-9_+-]+` shape of an emoji shortcode (does **not** check the catalog — that's `shortcode_to_unicode`) | `outl_md::emoji::is_valid_shortcode` | `crates/outl-md/src/emoji.rs` |
| Validate **one char** of a shortcode (`[a-z0-9_+-]`) — use this when walking the buffer char-by-char (`try_emoji`, TUI's `detect_trigger`) so you don't allocate a 1-char `String` per keystroke just to call `is_valid_shortcode` | `outl_md::emoji::is_valid_shortcode_char` | `crates/outl-md/src/emoji.rs` |
| Search the GitHub gemoji catalog for shortcodes matching a query (exact → prefix → substring; shorter shortcodes win ties) — powers the `:emoji:` autocomplete in every client through one shared `outl_emoji_search` Tauri command | `outl_md::emoji::search` → `EmojiHit` | `crates/outl-md/src/emoji.rs` |

---

## 8. Asset links (outl-md::asset + outl-actions::asset)

Uploaded files (PDFs, images) referenced from a block as `[name](assets/<hash>.<ext>)`.
The bytes are not workspace state and never enter the op log; only the link does, as an ordinary `Op::Edit`.

| Intent | Use this | File |
|---|---|---|
| Directory name for uploaded assets, relative to the workspace root | `outl_md::ASSETS_DIR` | `crates/outl-md/src/asset.rs` |
| Content-hash an uploaded file's bytes (hex SHA-256, used as the on-disk filename stem so identical uploads dedupe to one file) | `outl_md::hash_bytes` | `crates/outl-md/src/asset.rs` |
| Build the workspace-relative link target for a hash + extension | `outl_md::asset_rel_path` | `crates/outl-md/src/asset.rs` |
| Does a link target point at a workspace asset rather than an external URL? | `outl_md::is_asset_link` | `crates/outl-md/src/asset.rs` |
| Is `name` a safe asset basename (`<hash>.<ext>`, no traversal)? The one owner of the anti-traversal check — the P2P transport validates every peer-sent name through it | `outl_md::is_safe_asset_name` | `crates/outl-md/src/asset.rs` |
| Copy an uploaded file into `<root>/assets/<hash>.<ext>` and return the ready-to-insert markdown link (content-addressed, atomic tmp+rename, size-capped by `[assets] max_bytes` from `outl-config`); `import_asset_bytes` is the same for already-in-memory bytes (a remote image downloaded during a Roam import) | `outl_actions::import_asset` / `outl_actions::import_asset_bytes` → `ImportedAsset` | `crates/outl-actions/src/asset.rs` |
| Resolve a `[name](assets/…)` link back to an on-disk path for "open outside outl" handlers (rejects traversal / external schemes via `ActionError::InvalidAssetPath`) | `outl_actions::resolve_asset_path` | `crates/outl-actions/src/asset.rs` |
