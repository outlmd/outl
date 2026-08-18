//! Pre-computed backlinks index.
//!
//! [`crate::backlinks::backlinks_for_page`] walks the **whole
//! workspace** on every call (`O(blocks)`, materializing each source
//! block's subtree), so a client that recomputes it per page-open pays
//! that scan every time. This module builds the same result **once**
//! into an inverted index (`target key -> referencing blocks`) so a
//! page's backlinks become an `O(refs-of-the-page)` lookup, and the
//! expensive walk can run on a background thread while the UI stays
//! responsive.
//!
//! **One owner for "what does this block mention".** The rule that
//! decides whether a block references a page (`mentions_of`) and the
//! rule that decides which keys a page looks itself up under
//! (`keys_for_page`) live *here only*.
//! [`crate::backlinks::backlinks_for_page`] /
//! [`crate::backlinks::backlinks_for_target`] are thin lookups on top of
//! a freshly built index, so the "what counts as a mention" logic can
//! never fork between the on-demand path and the indexed path — the
//! divergence that once made self-references visible on one client and
//! not another (see the `backlinks` module doc).
//!
//! The index is a **projection** derived from the tree, never an `Op`
//! and never persisted: rebuild it from the workspace whenever the tree
//! changes, the same way the `.md` files and `WorkspaceIndex` are
//! rebuilt.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use tracing::warn;

use crate::backlinks::{extract_refs, Backlink, BacklinkCrumb};
use crate::journal::page_md_path;
use crate::outline::{project_outline_node_shallow, read_page_outline, ChildrenIndex, OutlineNode};
use crate::page::{page_meta, PageMeta};
use crate::todo::split_todo;

/// A key a block can be indexed under, mirroring the four channels the
/// old `TargetMatcher` matched on.
///
/// `Ref` is a literal `[[X]]` target (matched verbatim, like the old
/// `needle`); `Tag` is a `#tag` reduced to its slug form (so `#Avelino`
/// and page `avelino` meet); `Call` / `Provenance` are the two
/// template channels (a ` ```call:<name> ` fence and a
/// `from-template:: <slug>` property).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TargetKey {
    /// Literal `[[X]]` target.
    Ref(String),
    /// `#tag` reduced via `slugify`.
    Tag(String),
    /// ` ```call:<name> ` fence invocation name.
    Call(String),
    /// `from-template:: <slug>` provenance slug.
    Provenance(String),
}

/// One indexed referencing block: the backlink itself plus the keys it
/// is indexed under (so it can be removed from `by_key` on re-index).
/// Its page slug lives in `by_page`, not here.
#[derive(Debug, Clone)]
struct IndexedEntry {
    backlink: Backlink,
    keys: Vec<TargetKey>,
}

/// Inverted backlinks index.
///
/// `by_key` maps each `TargetKey` to the ids of the blocks that mention
/// it (`O(refs)` lookup). `by_page` maps a page slug to the ids of the
/// referencing blocks that live on it, so
/// [`reindex_page_from_disk`][Self::reindex_page_from_disk] can drop +
/// rebuild a single edited page's entries without rescanning the
/// workspace — the incremental update that keeps editing cheap.
/// A block mentioning a page several ways (`[[avelino]] #avelino`) has
/// one `IndexedEntry` reachable from several keys; deduping by id
/// collapses it.
#[derive(Debug, Clone, Default)]
pub struct BacklinkIndex {
    by_block: HashMap<String, IndexedEntry>,
    by_key: HashMap<TargetKey, HashSet<String>>,
    by_page: HashMap<String, HashSet<String>>,
}

impl BacklinkIndex {
    /// Backlinks for `meta`'s page: every block that mentions it, deduped
    /// by block and returned in DFS order (within a page). Display order
    /// is still the caller's job via [`crate::sort_backlinks`].
    pub fn for_page(&self, workspace: &Workspace, meta: &PageMeta) -> Vec<Backlink> {
        self.collect_by_keys(&keys_for_page(workspace, meta))
    }

    /// Backlinks for a raw target string (a page's slug or title),
    /// matching the literal `[[target]]` and `#tag`-slug channels — the
    /// indexed equivalent of [`crate::backlinks_for_target`].
    pub fn for_target(&self, target: &str) -> Vec<Backlink> {
        self.collect_by_keys(&[
            TargetKey::Ref(target.to_string()),
            TargetKey::Tag(outl_md::slug::slugify(target)),
        ])
    }

    /// Number of backlinks for `meta`'s page **without cloning** the
    /// list — for count-only callers like a footer chip.
    pub fn count_for_page(&self, workspace: &Workspace, meta: &PageMeta) -> usize {
        self.hit_ids(&keys_for_page(workspace, meta)).len()
    }

    /// Total number of referencing blocks across the whole workspace.
    pub fn len(&self) -> usize {
        self.by_block.len()
    }

    /// Whether the workspace has no backlinks at all.
    pub fn is_empty(&self) -> bool {
        self.by_block.is_empty()
    }

    /// Re-index a single page from its `.md` on disk: drop the page's
    /// current entries and re-add them from the fresh projection.
    ///
    /// This is the **incremental** update a client runs after editing a
    /// page — `O(one page)` of disk I/O + parse, no `Workspace`, no full
    /// rescan (the mirror of [`outl_md::index::WorkspaceIndex::patch_page`]).
    /// Rebuilding the whole index from every `.md` on every commit is
    /// what made the TUI's Esc slow. The page's `.md` must already be
    /// projected (callers do this before calling).
    pub fn reindex_page_from_disk(&mut self, meta: &PageMeta, root: &Path) {
        self.remove_page(&meta.slug);
        let outline = match read_page_outline(root, meta) {
            Ok(o) => o,
            Err(e) => {
                warn!("backlink reindex: skipping page `{}`: {e}", meta.slug);
                return;
            }
        };
        let source_path = page_md_path(root, meta);
        let mut path: Vec<usize> = Vec::new();
        let mut ancestors: Vec<BacklinkCrumb> = Vec::new();
        walk_parsed(
            &outline.nodes,
            meta,
            &source_path,
            &mut path,
            &mut ancestors,
            self,
        );
    }

    /// Gather the backlinks hit by any of `keys`, deduped by block and
    /// sorted into DFS order (by `source_block_path`, so a page's blocks
    /// come back top-to-bottom before [`crate::sort_backlinks`] regroups).
    pub(crate) fn collect_by_keys(&self, keys: &[TargetKey]) -> Vec<Backlink> {
        let mut out: Vec<Backlink> = self
            .hit_ids(keys)
            .into_iter()
            .filter_map(|id| self.by_block.get(&id))
            .map(|e| e.backlink.clone())
            .collect();
        out.sort_by(|a, b| a.source_block_path.cmp(&b.source_block_path));
        out
    }

    /// Dedup'd block ids hit by any of `keys`.
    fn hit_ids(&self, keys: &[TargetKey]) -> Vec<String> {
        let mut ids: HashSet<&String> = HashSet::new();
        for k in keys {
            if let Some(set) = self.by_key.get(k) {
                ids.extend(set);
            }
        }
        ids.into_iter().cloned().collect()
    }

    /// Add one referencing block under each of its keys.
    fn insert(&mut self, backlink: Backlink, keys: Vec<TargetKey>) {
        let id = backlink.block_id.clone();
        let page_slug = backlink
            .source_page
            .as_ref()
            .map(|p| p.slug.clone())
            .unwrap_or_default();
        for k in &keys {
            self.by_key.entry(k.clone()).or_default().insert(id.clone());
        }
        self.by_page
            .entry(page_slug)
            .or_default()
            .insert(id.clone());
        self.by_block.insert(id, IndexedEntry { backlink, keys });
    }

    /// Remove every referencing block that lives on `page_slug`.
    fn remove_page(&mut self, page_slug: &str) {
        let Some(ids) = self.by_page.remove(page_slug) else {
            return;
        };
        for id in ids {
            let Some(entry) = self.by_block.remove(&id) else {
                continue;
            };
            for k in &entry.keys {
                if let Some(set) = self.by_key.get_mut(k) {
                    set.remove(&id);
                    if set.is_empty() {
                        self.by_key.remove(k);
                    }
                }
            }
        }
    }
}

/// Build the inverted index over the whole workspace in a single DFS
/// walk. This is the `O(blocks)` pass — run it on a background thread
/// and hand the finished index to the UI (see the module doc).
pub fn build_backlink_index(workspace: &Workspace, root: &Path) -> BacklinkIndex {
    let children = build_children_index(workspace);
    let mut index = BacklinkIndex::default();
    let Some(pages) = children.get(&NodeId::root()) else {
        return index;
    };
    for &page_id in pages {
        let Some(meta) = page_meta(workspace, page_id) else {
            continue;
        };
        let source_path = page_md_path(root, &meta);
        let mut path: Vec<usize> = Vec::new();
        let mut ancestors: Vec<BacklinkCrumb> = Vec::new();
        walk_page(
            workspace,
            page_id,
            &meta,
            &source_path,
            &mut path,
            &mut ancestors,
            &children,
            &mut index,
        );
    }
    index
}

/// Build the inverted index by reading each page's `.md` from disk,
/// **without touching the `Workspace`**.
///
/// This is the client-facing builder. Reading block text through
/// `Workspace::block_text` (what [`build_backlink_index`] does) forces
/// the whole workspace to materialize on a lazy-boot vault (#179) and
/// holds the workspace lock across the `O(blocks)` walk — together, the
/// "opening the journal / pressing Esc freezes" bug. The `.md` files are
/// the projection the user already sees on disk and carry every block's
/// text + properties + sidecar id, so the index can be built from them
/// on a background thread with no lock and no Yrs materialization.
///
/// `metas` is the page list (cheap to get via `list_pages`, which reads
/// only page roots, not block text); pass it in so this function needs
/// no `Workspace` at all and stays `Send` for a worker thread.
pub fn build_backlink_index_from_disk(metas: &[PageMeta], root: &Path) -> BacklinkIndex {
    let started = std::time::Instant::now();
    let mut index = BacklinkIndex::default();
    for meta in metas {
        let outline = match read_page_outline(root, meta) {
            Ok(o) => o,
            Err(e) => {
                // Skipping a page here means every `[[ref]]` it makes
                // disappears from the backlinks panel. A missing `.md`
                // is silent by design (nothing to index); anything else
                // is an unreadable page and the user deserves to know
                // why their links went quiet.
                warn!("backlink index: skipping page `{}`: {e}", meta.slug);
                continue;
            }
        };
        let source_path = page_md_path(root, meta);
        let mut path: Vec<usize> = Vec::new();
        let mut ancestors: Vec<BacklinkCrumb> = Vec::new();
        walk_parsed(
            &outline.nodes,
            meta,
            &source_path,
            &mut path,
            &mut ancestors,
            &mut index,
        );
    }
    tracing::debug!(
        pages = metas.len(),
        refs = index.len(),
        elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
        "build_backlink_index_from_disk (full workspace rescan)"
    );
    index
}

/// DFS the parsed `.md` outline of one page, adding every mentioning
/// block to `index`. The from-disk twin of [`walk_page`]: same shape,
/// but reads the leaf + `from-template::` value off the already-parsed
/// [`OutlineNode`] instead of the workspace tree.
fn walk_parsed(
    nodes: &[OutlineNode],
    meta: &PageMeta,
    source_path: &Path,
    path: &mut Vec<usize>,
    ancestors: &mut Vec<BacklinkCrumb>,
    index: &mut BacklinkIndex,
) {
    for (idx, node) in nodes.iter().enumerate() {
        path.push(idx);
        let from_template = node
            .properties
            .iter()
            .find(|(k, _)| k == crate::template::FROM_TEMPLATE_KEY)
            .map(|(_, v)| v.as_str());
        let keys = mentions_of(&node.text, from_template);
        if !keys.is_empty() {
            index.insert(
                Backlink {
                    block_id: node.id.clone(),
                    block_text: node.text.clone(),
                    todo: node.todo,
                    source_page: Some(meta.clone()),
                    source_block: shallow_parsed(node),
                    source_block_path: path.clone(),
                    ancestors: ancestors.clone(),
                    source_path: Some(source_path.to_path_buf()),
                },
                keys,
            );
        }
        ancestors.push(BacklinkCrumb {
            id: node.id.clone(),
            text: node.text.clone(),
        });
        walk_parsed(&node.children, meta, source_path, path, ancestors, index);
        ancestors.pop();
        path.pop();
    }
}

/// Copy a parsed outline node as a shallow leaf (no `children`), keeping
/// the already-parsed tokens/properties. Mirrors
/// `project_outline_node_shallow` for the from-disk path.
fn shallow_parsed(node: &OutlineNode) -> OutlineNode {
    OutlineNode {
        id: node.id.clone(),
        text: node.text.clone(),
        todo: node.todo,
        collapsed: node.collapsed,
        properties: node.properties.clone(),
        tokens: node.tokens.clone(),
        children: Vec::new(),
    }
}

/// Every key a block mentions — the single source of truth for "does
/// this block reference something".
///
/// `[[X]]` targets come through [`extract_refs`] (literal); `#tag`s go
/// through the real inline tokenizer and `slugify` (so a tag in a code
/// span doesn't count and `#avelino-foo` doesn't reduce to `avelino`);
/// the callable channel reads the fence invocation name from the text.
/// The `from-template::` provenance value is passed in by the caller —
/// the workspace build reads it off the tree, the from-disk build reads
/// it off the parsed `.md` block properties — so this one function stays
/// the sole owner of "what counts as a mention" regardless of source.
fn mentions_of(text: &str, from_template: Option<&str>) -> Vec<TargetKey> {
    let mut keys: Vec<TargetKey> = Vec::new();
    for r in extract_refs(text) {
        keys.push(TargetKey::Ref(r));
    }
    if text.contains('#') {
        for tok in outl_md::inline::tokenize(text) {
            if let outl_md::inline::InlineTok::Tag { name } = tok {
                keys.push(TargetKey::Tag(outl_md::slug::slugify(name)));
            }
        }
    }
    if let Some(name) = crate::template::call_target_name(text) {
        keys.push(TargetKey::Call(name));
    }
    if let Some(slug) = from_template {
        keys.push(TargetKey::Provenance(slug.to_string()));
    }
    keys
}

/// The keys a page looks itself up under — the lookup-side mirror of
/// [`mentions_of`], matching what `backlinks_for_page` used to scan for.
///
/// For each target string (slug, title, and the `@`-alias forms for a
/// person page) the page is found under both the literal `Ref` and the
/// `Tag` slug, exactly like the old `TargetMatcher::refs`. A template
/// page additionally looks itself up under its callable name and its
/// own slug (provenance).
fn keys_for_page(workspace: &Workspace, meta: &PageMeta) -> Vec<TargetKey> {
    let mut keys: Vec<TargetKey> = Vec::new();
    let mut add_target = |t: &str| {
        keys.push(TargetKey::Ref(t.to_string()));
        keys.push(TargetKey::Tag(outl_md::slug::slugify(t)));
    };
    add_target(&meta.slug);
    if meta.title != meta.slug {
        add_target(&meta.title);
    }
    if meta.page_type.as_deref() == Some(crate::person::PERSON_TYPE) {
        add_target(&format!("@{}", meta.slug));
        if meta.title != meta.slug {
            add_target(&format!("@{}", meta.title));
        }
    }
    if let Some(name) = template_name_of(workspace, meta) {
        keys.push(TargetKey::Call(name));
        keys.push(TargetKey::Provenance(meta.slug.clone()));
    }
    keys
}

/// The template invocation name of `meta`'s page, when it is a template
/// (has a non-empty `template::` property).
fn template_name_of(workspace: &Workspace, meta: &PageMeta) -> Option<String> {
    let id = crate::page::find_by_slug(workspace, &meta.slug)?;
    let name = crate::page::read_text_prop(workspace, id, crate::template::TEMPLATE_KEY)?;
    (!name.trim().is_empty()).then_some(name)
}

/// DFS a page's blocks, adding every mentioning block to `index` under
/// each of its keys. Mirrors the old `walk_inside_page`, but emits by
/// [`mentions_of`] instead of a single-target matcher.
#[allow(clippy::too_many_arguments)]
fn walk_page(
    workspace: &Workspace,
    parent: NodeId,
    meta: &PageMeta,
    source_path: &Path,
    path: &mut Vec<usize>,
    ancestors: &mut Vec<BacklinkCrumb>,
    children: &ChildrenIndex,
    index: &mut BacklinkIndex,
) {
    let Some(kids) = children.get(&parent) else {
        return;
    };
    for (idx, child_id) in kids.iter().copied().enumerate() {
        path.push(idx);
        let text = workspace.block_text(child_id).unwrap_or_default();
        let (todo, body) = split_todo(&text);
        let from_template =
            crate::page::read_text_prop(workspace, child_id, crate::template::FROM_TEMPLATE_KEY);
        let keys = mentions_of(&text, from_template.as_deref());
        if !keys.is_empty() {
            // Shallow leaf only — never materialize the subtree here.
            // Descending every referencing block's children (tokenize +
            // props per node) across the whole workspace, under the
            // workspace lock, is what froze input. Clients render the row
            // from `source_block.tokens`; the subtree isn't needed.
            let source_block = project_outline_node_shallow(workspace, child_id);
            index.insert(
                Backlink {
                    block_id: child_id.to_string(),
                    block_text: body.to_string(),
                    todo,
                    source_page: Some(meta.clone()),
                    source_block,
                    source_block_path: path.clone(),
                    ancestors: ancestors.clone(),
                    source_path: Some(source_path.to_path_buf()),
                },
                keys,
            );
        }
        ancestors.push(BacklinkCrumb {
            id: child_id.to_string(),
            text: body.to_string(),
        });
        walk_page(
            workspace,
            child_id,
            meta,
            source_path,
            path,
            ancestors,
            children,
            index,
        );
        ancestors.pop();
        path.pop();
    }
}

/// Build a `parent -> children (in fractional order)` map in one scan
/// over the workspace, so the DFS walk + subtree projection don't pay
/// [`crate::tree::children_of`]'s per-call `O(total-nodes)` rescan
/// (which made the whole pass `O(n²)`). Pages are the children of
/// [`NodeId::root`].
pub fn build_children_index(workspace: &Workspace) -> ChildrenIndex {
    let mut grouped: HashMap<NodeId, Vec<(NodeId, Fractional)>> = HashMap::new();
    for (id, parent, pos) in workspace.tree().iter_nodes() {
        grouped.entry(parent).or_default().push((id, pos.clone()));
    }
    grouped
        .into_iter()
        .map(|(parent, mut kids)| {
            kids.sort_by(|a, b| a.1.cmp(&b.1));
            (parent, kids.into_iter().map(|(id, _)| id).collect())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::page::{open_journal, open_or_create, PageKind};
    use chrono::NaiveDate;
    use outl_core::hlc::HlcGenerator;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    fn root() -> &'static Path {
        Path::new("/tmp/outl-test")
    }

    #[test]
    fn index_lookup_matches_slug_refs() {
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, avelino).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
        let m = append_block(&mut w, &hlc, Some(day), Some("ping [[avelino]]")).unwrap();

        let index = build_backlink_index(&w, root());
        let links = index.for_page(&w, &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, m.to_string());
    }

    #[test]
    fn one_index_serves_many_pages() {
        // The whole point: build once, look up each page cheaply.
        let (mut w, hlc) = ws();
        let a = open_or_create(&mut w, &hlc, "a", "a", PageKind::Page).unwrap();
        let b = open_or_create(&mut w, &hlc, "b", "b", PageKind::Page).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
        append_block(&mut w, &hlc, Some(day), Some("see [[a]]")).unwrap();
        append_block(&mut w, &hlc, Some(day), Some("and [[b]] too")).unwrap();

        let index = build_backlink_index(&w, root());
        assert_eq!(index.for_page(&w, &page_meta(&w, a).unwrap()).len(), 1);
        assert_eq!(index.for_page(&w, &page_meta(&w, b).unwrap()).len(), 1);
    }

    #[test]
    fn index_matches_the_on_demand_path() {
        // Equivalence net: the indexed lookup returns the same blocks
        // (as a set) the direct walk does, across ref + tag + title.
        let (mut w, hlc) = ws();
        let avelino = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, avelino).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
        append_block(&mut w, &hlc, Some(day), Some("[[avelino]] a")).unwrap();
        append_block(&mut w, &hlc, Some(day), Some("#Avelino b")).unwrap();
        append_block(&mut w, &hlc, Some(day), Some("[[Avelino]] c")).unwrap();
        append_block(&mut w, &hlc, Some(day), Some("nothing here")).unwrap();

        let direct = crate::backlinks::backlinks_for_page(&w, root(), &meta);
        let indexed = build_backlink_index(&w, root()).for_page(&w, &meta);

        let ids = |v: &[Backlink]| {
            let mut s: Vec<String> = v.iter().map(|b| b.block_id.clone()).collect();
            s.sort();
            s
        };
        assert_eq!(ids(&direct), ids(&indexed));
        assert_eq!(indexed.len(), 3);
    }

    #[test]
    fn dedups_block_matched_by_slug_and_title() {
        let (mut w, hlc) = ws();
        let p = open_or_create(&mut w, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
        let meta = page_meta(&w, p).unwrap();
        let day =
            open_journal(&mut w, &hlc, NaiveDate::from_ymd_opt(2026, 7, 20).unwrap()).unwrap();
        let m = append_block(&mut w, &hlc, Some(day), Some("[[avelino]] aka [[Avelino]]")).unwrap();

        let links = build_backlink_index(&w, root()).for_page(&w, &meta);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].block_id, m.to_string());
    }

    #[test]
    fn empty_workspace_has_empty_index() {
        let (w, _hlc) = ws();
        let index = build_backlink_index(&w, root());
        assert!(index.is_empty());
    }
}
