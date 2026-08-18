//! Workspace-wide derived index.
//!
//! Walks the `pages/` and `journals/` directories, parses each `.md`,
//! and builds in-memory maps the TUI / GUI / mobile can query without
//! re-walking the filesystem:
//!
//! - `slug → PageEntry` (filename without `.md`).
//! - `title → slug` (the `title::` property; falls back to slug).
//! - block-level lookup (`((blk-XXXXXX))` → block, reverse refs, etc).
//!
//! **Backlinks live in `outl_actions::backlinks`**, not here. Both the
//! TUI and the mobile client compute them straight from the
//! `Workspace` so policy (self-refs, dedup) never drifts between
//! surfaces. The earlier parallel cache on this index was the bug
//! that hid self-references on the TUI panel while the mobile path
//! showed them.
//!
//! Rebuild on demand: this is cheap for hundreds of pages, expensive
//! for thousands. The TUI calls `rebuild()` at startup and on a debounce
//! after writes.

use crate::block_index::{BlockEntry, BlockIndex, BlockReference, IdentifiedNode};
use crate::parse::parse;
use crate::sidecar::{self, Sidecar};
use outl_core::id::NodeId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One entry in the index — the data we want to know about a page
/// without re-reading its file.
#[derive(Debug, Clone)]
pub struct PageEntry {
    /// Filesystem path, e.g. `pages/avelino.md`.
    pub path: PathBuf,
    /// Slug (filename without extension).
    pub slug: String,
    /// User-visible title (from `title::` property, or `slug` if unset).
    pub title: String,
    /// Optional decoration from the `icon::` property — usually a
    /// single emoji or short string the UI prepends to the title.
    /// `None` when the page has no `icon::` set.
    pub icon: Option<String>,
    /// Whether the file lives in `journals/`.
    pub is_journal: bool,
    /// `pinned:: true` page-level property. Surfaces that ship a
    /// sidebar (TUI, future Tauri) list pinned pages prominently so
    /// frequently-touched notes are a single click away.
    pub pinned: bool,
    /// `type::` page-level property, lowercased+trimmed. `None` when
    /// unset. Powers `pages_by_type` — the `@` mention autocomplete
    /// filters on `Some("person")` to surface only people pages.
    pub page_type: Option<String>,
}

/// Full workspace index.
#[derive(Debug, Default, Clone)]
pub struct WorkspaceIndex {
    pages: HashMap<String, PageEntry>,
    title_to_slug: HashMap<String, String>,
    /// Block-level index — owns the `((blk-XXXXXX))` lookup machinery.
    /// Kept private so the public surface stays a single `WorkspaceIndex`.
    blocks: BlockIndex,
}

impl WorkspaceIndex {
    /// Walk `pages/` and `journals/` under `workspace_root`, parse every
    /// `.md`, and return the populated index. Files that fail to parse
    /// are skipped with no error — the index is best-effort.
    ///
    /// Block-level indexing runs in two passes (register every block
    /// first, then collect reverse references) so a page B citing a
    /// block of page A still wins an edge even when B is walked first.
    pub fn build(workspace_root: &Path) -> Self {
        let mut idx = WorkspaceIndex::default();
        // Buffer of (slug, parsed AST, sidecar). Populated alongside
        // the `pages` map; consumed below for the block-index pass 2.
        let mut parsed_pages: Vec<(String, crate::parse::ParsedPage, Option<Sidecar>)> =
            Vec::with_capacity(64);

        for (dir, is_journal) in [
            (workspace_root.join("pages"), false),
            (workspace_root.join("journals"), true),
        ] {
            if !dir.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&dir).max_depth(1) {
                let Ok(entry) = entry else {
                    continue;
                };
                if !entry.file_type().is_file() {
                    continue;
                }
                let path = entry.path();
                if path.extension().and_then(|x| x.to_str()) != Some("md") {
                    continue;
                }
                if path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
                {
                    continue;
                }
                let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(text) = std::fs::read_to_string(path) else {
                    continue;
                };
                let parsed = parse(&text);
                let title = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "title")
                    .map(|(_, v)| v.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| slug.to_string());
                let icon = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "icon")
                    .map(|(_, v)| v.trim().to_string())
                    .filter(|s| !s.is_empty());
                let pinned = parsed
                    .properties
                    .iter()
                    .any(|(k, v)| k == "pinned" && is_truthy(v));
                let page_type = parsed
                    .properties
                    .iter()
                    .find(|(k, _)| k == "type")
                    .map(|(_, v)| v.trim().to_lowercase())
                    .filter(|s| !s.is_empty());

                idx.pages.insert(
                    slug.to_string(),
                    PageEntry {
                        path: path.to_path_buf(),
                        slug: slug.to_string(),
                        title: title.clone(),
                        icon,
                        is_journal,
                        pinned,
                        page_type,
                    },
                );
                idx.title_to_slug.insert(title.clone(), slug.to_string());

                // Block-level indexing pass 1: register every block
                // (id, handle, text, subtree) without recording
                // reverse refs yet. Pass 2 below scans citations
                // once all handles are known.
                let cached_sidecar = read_sidecar_best_effort(path);
                if let Some(sc) = &cached_sidecar {
                    idx.blocks
                        .collect_page_blocks(slug, path, &parsed.blocks, &sc.blocks);
                }

                parsed_pages.push((slug.to_string(), parsed, cached_sidecar));
            }
        }

        // Pass 2 of block indexing: now that every handle is in
        // `handle_to_block`, scan each page's blocks for
        // `((blk-XXXXXX))` and record the reverse edges.
        for (slug, parsed, cached_sidecar) in &parsed_pages {
            if let Some(sc) = cached_sidecar {
                idx.blocks
                    .collect_page_refs(slug, &parsed.blocks, &sc.blocks);
            }
        }

        idx
    }

    /// Insert (or replace) a page entry and its `title → slug` alias.
    ///
    /// The building block of the **tree-derived** build path: a caller
    /// holding a `Workspace` reads page metadata straight off the page
    /// root's properties and hands the result here, instead of this
    /// module walking `pages/` and parsing every `.md` to recover the
    /// same four values. See `outl_actions::index::derive`.
    ///
    /// Replaces any previous alias for this slug so a renamed title
    /// can't leave a stale entry shadowing the new one — the same rule
    /// [`patch_page`](Self::patch_page) follows.
    pub fn insert_page(&mut self, entry: PageEntry) {
        let slug = entry.slug.clone();
        let title = entry.title.clone();
        self.title_to_slug.retain(|_, s| s != &slug);
        self.pages.insert(slug.clone(), entry);
        self.title_to_slug.insert(title, slug);
    }

    /// Pass 1 of the tree-derived block build: register every block of
    /// a page. Forwards to
    /// [`BlockIndex::collect_page_blocks_from_tree`].
    pub fn collect_page_blocks_from_tree(
        &mut self,
        slug: &str,
        path: &Path,
        blocks: &[IdentifiedNode],
    ) {
        self.blocks
            .collect_page_blocks_from_tree(slug, path, blocks);
    }

    /// Pass 2 of the tree-derived block build: record reverse refs for
    /// every block already indexed. Forwards to
    /// [`BlockIndex::collect_refs_from_indexed`] — call once, after
    /// every page's blocks are in.
    pub fn collect_refs_from_indexed(&mut self) {
        self.blocks.collect_refs_from_indexed();
    }

    /// Look up a page by its slug.
    pub fn by_slug(&self, slug: &str) -> Option<&PageEntry> {
        self.pages.get(slug)
    }

    /// Look up a page by its `title::` (or slug fallback). Title match
    /// is case-sensitive — use `pages_by_title_prefix` for autocomplete.
    pub fn by_title(&self, title: &str) -> Option<&PageEntry> {
        let slug = self.title_to_slug.get(title)?;
        self.pages.get(slug)
    }

    /// Iterate every page entry in unspecified order.
    pub fn pages(&self) -> impl Iterator<Item = &PageEntry> {
        self.pages.values()
    }

    /// Number of pages indexed.
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Re-index a single page in place — replacement for the global
    /// scan when only one page changed.
    ///
    /// Refreshes the `PageEntry` metadata (title/icon/pinned) and
    /// rebuilds the block-level entries for this slug. `is_journal`
    /// is inferred from the parent directory of `path`
    /// (`journals/...` vs anything else).
    ///
    /// Roughly `O(blocks_in_this_page)` — orders of magnitude cheaper
    /// than `build`, which walks every `.md` in the workspace. Use
    /// this on every page save so the index stays fresh without
    /// paying for a full rescan.
    pub fn patch_page(&mut self, path: &Path, page: &crate::parse::ParsedPage) {
        let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
            return;
        };
        let slug = slug.to_string();
        let is_journal = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            == Some("journals");

        let title = page
            .properties
            .iter()
            .find(|(k, _)| k == "title")
            .map(|(_, v)| v.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slug.clone());
        let icon = page
            .properties
            .iter()
            .find(|(k, _)| k == "icon")
            .map(|(_, v)| v.trim().to_string())
            .filter(|s| !s.is_empty());
        let pinned = page
            .properties
            .iter()
            .any(|(k, v)| k == "pinned" && is_truthy(v));
        let page_type = page
            .properties
            .iter()
            .find(|(k, _)| k == "type")
            .map(|(_, v)| v.trim().to_lowercase())
            .filter(|s| !s.is_empty());

        // Forget the page's previous `title -> slug` mapping in case
        // the title changed (otherwise a stale alias would shadow the
        // new one for `by_title` lookups).
        self.title_to_slug.retain(|_, s| s != &slug);

        let entry = PageEntry {
            path: path.to_path_buf(),
            slug: slug.clone(),
            title: title.clone(),
            icon,
            is_journal,
            pinned,
            page_type,
        };
        self.pages.insert(slug.clone(), entry);
        self.title_to_slug.insert(title, slug.clone());

        // Block-level re-index: drop everything this page used to
        // contribute, then re-collect from the fresh AST + current
        // sidecar.
        self.blocks.forget_page(&slug);
        if let Some(sc) = read_sidecar_best_effort(path) {
            self.blocks
                .collect_page(&slug, path, &page.blocks, &sc.blocks);
        }
    }

    /// Drop a page from the index entirely. Use when a `.md` is
    /// deleted on disk. Removes the `PageEntry`, its title alias,
    /// and every block the page contributed to the block index.
    pub fn remove_page(&mut self, slug: &str) {
        self.pages.remove(slug);
        self.title_to_slug.retain(|_, s| s != slug);
        self.blocks.forget_page(slug);
    }

    /// Resolve `((blk-XXXXXX))` to the indexed block, if known.
    ///
    /// `O(1)`. Returns `None` for handles that don't match any
    /// indexed block — orphan references are surfaced this way for
    /// the doctor pass to flag.
    pub fn resolve_block_ref(&self, handle: &str) -> Option<&BlockEntry> {
        self.blocks.resolve(handle)
    }

    /// Look up a block by its `NodeId`.
    pub fn block_by_id(&self, id: NodeId) -> Option<&BlockEntry> {
        self.blocks.get(id)
    }

    /// Blocks that cite `id` via `((blk-XXXXXX))`. May be empty.
    pub fn block_refs_to(&self, id: NodeId) -> &[BlockReference] {
        self.blocks.refs_to(id)
    }

    /// Iterate every indexed block in unspecified order. Used by
    /// the TUI's `((` autocomplete to fuzzy-match on block text.
    pub fn iter_blocks(&self) -> impl Iterator<Item = &BlockEntry> {
        self.blocks.iter_blocks()
    }

    /// Search indexed blocks by text substring (case-insensitive).
    ///
    /// Powers the `((` autocomplete in any UI surface (TUI today,
    /// Tauri / mobile later). Returns up to `limit` `BlockEntry`s
    /// ranked by match position then text length.
    pub fn search_block_text(&self, query: &str, limit: usize) -> Vec<&BlockEntry> {
        self.blocks.search_text(query, limit)
    }

    /// Borrow the inner block-level index directly.
    ///
    /// Lets a consumer that already builds a `WorkspaceIndex` reuse the
    /// `BlockIndex` primitives (`iter_blocks`, `search_text`) through one
    /// value instead of the forwarding shims above — e.g. the desktop's
    /// `((` block-ref autocomplete, whose selection logic is unit-tested
    /// against a `BlockIndex` built in-memory.
    pub fn block_index(&self) -> &BlockIndex {
        &self.blocks
    }

    /// Find the block at `(slug, dfs_path)` in O(1).
    ///
    /// Backs `yr` / `/refer` / `/refer-embed` so the TUI doesn't
    /// scan `iter_blocks()` linearly per chord press.
    pub fn block_at_location(&self, slug: &str, path: &[usize]) -> Option<&BlockEntry> {
        self.blocks.at_location(slug, path)
    }

    /// Total indexed blocks. Tests and the future bench (#12) read
    /// this to sanity-check workspace coverage.
    pub fn block_count(&self) -> usize {
        self.blocks.block_count()
    }

    /// Titles starting with `prefix` (case-insensitive), best-effort
    /// for autocomplete. Returns at most `limit` results sorted by
    /// title length (shorter first).
    pub fn pages_by_title_prefix(&self, prefix: &str, limit: usize) -> Vec<&PageEntry> {
        let needle = prefix.to_lowercase();
        let mut hits: Vec<&PageEntry> = self
            .pages
            .values()
            .filter(|p| p.title.to_lowercase().starts_with(&needle))
            .collect();
        hits.sort_by_key(|p| (p.title.len(), p.title.clone()));
        hits.truncate(limit);
        hits
    }

    /// Pages whose `type::` property equals `t` (case-insensitive
    /// comparison; `t` is matched against the already lowercased
    /// `page_type` stored on each [`PageEntry`]).
    ///
    /// Powers the `@` mention autocomplete (with `t == "person"`).
    /// The catalog of accepted `type::` values is a UX decision the
    /// caller owns — this method just filters; no other normalization
    /// (no plural aliasing, no synonyms).
    pub fn pages_by_type<'a>(&'a self, t: &'a str) -> impl Iterator<Item = &'a PageEntry> + 'a {
        let needle = t.to_lowercase();
        self.pages
            .values()
            .filter(move |p| p.page_type.as_deref() == Some(needle.as_str()))
    }
}

/// Read the sidecar paired with `md_path`, swallowing I/O and parse
/// errors so the index build (a best-effort pass) cannot abort over a
/// single bad file.
fn read_sidecar_best_effort(md_path: &Path) -> Option<Sidecar> {
    let p = sidecar::sidecar_path_for(md_path);
    sidecar::read(&p).ok()
}

/// Loose truthy check for boolean-ish property values (`pinned::`,
/// `archived::`, etc). Accepts `true`, `yes`, `1`, `on` (case
/// insensitive). Empty string is treated as falsy so a stray
/// `pinned:: ` doesn't flip a page into the pinned list.
fn is_truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "true" | "yes" | "1" | "on"
    )
}

// Integration-level tests for WorkspaceIndex live in
// `tests/workspace_index.rs`. They exercise only the public API surface
// so the same suite would catch a regression introduced by a UI client
// (TUI, future Tauri, mobile).
