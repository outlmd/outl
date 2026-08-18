//! Block-level index — the lookup machinery behind `((blk-XXXXXX))`
//! inline references and `!((blk-XXXXXX))` embeds.
//!
//! Lives next to [`crate::index`] rather than inside it so neither
//! module owns more than one responsibility. The page-level index
//! (`slug → PageEntry`, backlinks) stays in `index.rs`; this file owns:
//!
//! - `NodeId → BlockEntry` (the canonical "where does this block live
//!   and what does it contain").
//! - `ref_handle → NodeId` (resolution path for `((blk-XXXXXX))`).
//! - `NodeId → [BlockReference]` (reverse: who cites a given block).
//! - `(slug, dfs_path) → NodeId` (reverse location lookup for the
//!   TUI's `yr` chord and `/refer` commands, kept O(1) per keystroke).
//! - `slug → Vec<NodeId>` (per-page block list so `forget_page` runs
//!   O(blocks_in_page) instead of O(workspace_blocks)).
//!
//! Population happens in [`BlockIndex::collect_page`], invoked once per
//! `.md` (whether on full build or after a single-page save). Lookups
//! are pure HashMap reads so they stay O(1) regardless of workspace
//! size — the contract that bench #12 validates.
//!
//! ## Two population paths, one shape
//!
//! Blocks reach this index from either side of the `.md` ↔ op-log
//! boundary, and the pair is deliberate:
//!
//! - **From disk** ([`BlockIndex::collect_page`] and its two-pass
//!   siblings) — an AST parsed out of a `.md` plus the sidecar that
//!   supplies the ids. Used when the caller has no `Workspace` in
//!   scope, and when the question being asked is genuinely about the
//!   file (`outl doctor` comparing disk against tree).
//! - **From the tree** ([`BlockIndex::collect_page_blocks_from_tree`]
//!   plus [`BlockIndex::collect_refs_from_indexed`]) — an [`IdentifiedNode`] forest projected from the op
//!   log, which already carries its own ids. Used by every caller that
//!   holds a `Workspace`, which is nearly all of them.
//!
//! The tree path is not merely faster (no walkdir, no comrak, no
//! sidecar JSON): it also cannot hit the disk path's **positional
//! skip**. `walk_blocks` pairs the AST with `sidecar_blocks[cursor]`
//! and drops any block whose `content_hash` disagrees, so one block
//! typed before reconcile ran desynchronises the cursor and silently
//! removes the rest of the page from the index. A node projected from
//! the tree carries its id by construction, so there is nothing to
//! disagree with and nothing to drop.
//!
//! Keep the two paths behaviourally identical in everything else —
//! handle assignment, collision expansion, DFS path numbering and
//! reverse-ref collection all run through the same helpers, so a fix
//! to one is a fix to both.
//!
//! ## Handle collision handling
//!
//! The 6-char tail of a ULID has ~1B values, so collisions are
//! astronomically rare but not impossible. On insert, if the base
//! handle is already taken by a *different* id, the new block's
//! handle is lazily expanded one character at a time until it's
//! unique within the workspace. Both the surviving entry and the
//! expanded loser are resolvable via their (now distinct) handles;
//! `outl doctor` surfaces the expansion so the user can rerun
//! reconcile to persist the expanded handle in the sidecar.

use crate::inline::{tokenize, InlineTok};
use crate::parse::OutlineNode;
use crate::sidecar::{self, content_hash, SidecarBlock, REF_HANDLE_PREFIX, REF_HANDLE_TAIL_LEN};
use outl_core::id::NodeId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// One indexed block. Carries enough context that
/// `WorkspaceIndex::resolve_block_ref` (see `crate::index`) can return
/// it directly — no follow-up disk read needed for the common path.
///
/// `children` is a clone of the block's subtree (same shape
/// `outl_actions::Backlink::source_block` carries for backlinks).
/// The cost is bounded: one clone per indexed block, not one per
/// reference. For an embed surface, the consumer renders `text` +
/// `children` exactly as the source page would.
#[derive(Debug, Clone)]
pub struct BlockEntry {
    /// Block's stable ULID.
    pub id: NodeId,
    /// Short ref handle (`blk-XXXXXX`). May be 7+ characters when a
    /// collision forced lazy expansion at index time.
    pub ref_handle: String,
    /// Slug of the page hosting the block.
    pub source_slug: String,
    /// Filesystem path of the hosting `.md`.
    pub source_path: PathBuf,
    /// DFS path inside the source page's AST.
    pub source_block_path: Vec<usize>,
    /// Block text at index time. Used as the inline-resolved text
    /// when a `((blk-XXXXXX))` is rendered.
    pub text: String,
    /// Lowercased copy of `text`. Cached so
    /// [`BlockIndex::search_text`] doesn't reallocate per block on
    /// every autocomplete keystroke.
    pub text_fold: String,
    /// Cloned subtree under this block — used by embed surfaces.
    pub children: Vec<OutlineNode>,
}

/// A block projected straight from the op-log tree: the outline shape
/// a renderer needs, plus the stable id that the disk path has to go
/// to the sidecar for.
///
/// This is the input type of the tree-side population path
/// ([`BlockIndex::collect_page_blocks_from_tree`]). It exists because
/// [`OutlineNode`] deliberately carries no id — it is the shape of a
/// *parsed `.md`*, where ids live in the sidecar and nowhere else
/// (root `CLAUDE.md` invariant 2). A projection of the tree has the
/// opposite problem: the id is the one thing it is certain of.
///
/// Producers live in `outl-actions`, which owns the tree walk
/// (`outl_actions::index::project_identified`). This crate only
/// consumes the shape, so nothing here needs a `Workspace` — the
/// dependency arrow keeps pointing the one way it always has.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IdentifiedNode {
    /// Stable id of the block, straight from the tree.
    pub id: NodeId,
    /// Block content, same convention as [`OutlineNode::text`]
    /// (markdown inline, no `- ` prefix, no property lines).
    pub text: String,
    /// Properties attached to this block.
    pub properties: Vec<(String, String)>,
    /// Children, depth-first — same order the `.md` renders them in.
    pub children: Vec<IdentifiedNode>,
}

impl IdentifiedNode {
    /// Drop the ids, yielding the plain AST shape that
    /// [`BlockEntry::children`] and the renderer both take.
    ///
    /// Recursive, and it clones: one clone per indexed block, which is
    /// the same bound the disk path already pays (`b.children.clone()`
    /// in `walk_blocks`).
    pub fn to_outline(&self) -> OutlineNode {
        OutlineNode {
            text: self.text.clone(),
            properties: self.properties.clone(),
            children: self.children.iter().map(Self::to_outline).collect(),
        }
    }
}

/// One reverse edge: somebody cites the block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockReference {
    /// Slug of the citing page.
    pub source_slug: String,
    /// DFS path of the citing block inside its page's AST.
    pub source_block_path: Vec<usize>,
}

/// Container for the block-level maps.
///
/// Lives behind [`crate::index::WorkspaceIndex`] so consumers see one
/// index, not two.
#[derive(Debug, Default, Clone)]
pub struct BlockIndex {
    blocks: HashMap<NodeId, BlockEntry>,
    handle_to_block: HashMap<String, NodeId>,
    block_refs: HashMap<NodeId, Vec<BlockReference>>,
    /// `slug → ids of blocks contributed by that page`. Lets
    /// [`Self::forget_page`] iterate only the page's blocks instead of
    /// scanning the whole workspace.
    pages: HashMap<String, Vec<NodeId>>,
    /// `(slug, dfs_path) → NodeId`. Lets the TUI resolve "the block at
    /// my cursor" in O(1) (powers `yr` / `/refer` / `/refer-embed`).
    location_to_block: HashMap<(String, Vec<usize>), NodeId>,
}

impl BlockIndex {
    /// Look up a block by its short ref handle (`blk-XXXXXX`).
    ///
    /// O(1). Returns `None` for unknown handles, including orphaned
    /// ones (block deleted but a `.md` still cites it) — that's the
    /// signal `outl doctor` uses to flag the workspace.
    pub fn resolve(&self, handle: &str) -> Option<&BlockEntry> {
        let id = self.handle_to_block.get(handle)?;
        self.blocks.get(id)
    }

    /// Look up a block by its `NodeId`. Used by the embed render path
    /// once the handle has already been resolved.
    pub fn get(&self, id: NodeId) -> Option<&BlockEntry> {
        self.blocks.get(&id)
    }

    /// Look up a block by its location `(slug, dfs_path)` — O(1).
    ///
    /// Used by `yank_current_ref` / `yank_current_embed` so the
    /// keyboard chord stays snappy regardless of workspace size.
    pub fn at_location(&self, slug: &str, path: &[usize]) -> Option<&BlockEntry> {
        // Tuple ownership is fine here: HashMap keys are owned so we
        // must clone on lookup. The TUI calls this once per chord
        // press, not per render frame.
        let key = (slug.to_string(), path.to_vec());
        let id = self.location_to_block.get(&key)?;
        self.blocks.get(id)
    }

    /// Reverse refs: every block that cites `id` via `((blk-XXXXXX))`.
    pub fn refs_to(&self, id: NodeId) -> &[BlockReference] {
        self.block_refs
            .get(&id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Total indexed blocks. Used by tests and bench #12.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Iterate every indexed block in unspecified order. Used by
    /// autocomplete (`((` popup) to fuzzy-match on `text`.
    pub fn iter_blocks(&self) -> impl Iterator<Item = &BlockEntry> {
        self.blocks.values()
    }

    /// Find blocks whose text contains `query` (case-insensitive),
    /// sorted by relevance heuristics, capped at `limit`.
    ///
    /// Scoring is deliberately simple:
    ///   1. Prefer matches earlier in the string (prefix > middle).
    ///   2. Tiebreak by shorter text (more specific blocks rank
    ///      higher than long ones containing the query incidentally).
    ///   3. Final tiebreak: NodeId (lexicographic, ULID-sortable) so
    ///      autocomplete order stays deterministic across rebuilds.
    ///
    /// Uses the precomputed [`BlockEntry::text_fold`] so per-keystroke
    /// cost stays O(blocks). The bench in `#12` measures the upper
    /// bound; a fzf-style scorer can drop in later behind this
    /// signature without affecting callers.
    pub fn search_text(&self, query: &str, limit: usize) -> Vec<&BlockEntry> {
        if query.is_empty() {
            return Vec::new();
        }
        let needle = query.to_lowercase();
        let mut hits: Vec<(&BlockEntry, usize)> = self
            .blocks
            .values()
            .filter_map(|b| b.text_fold.find(&needle).map(|pos| (b, pos)))
            .collect();
        hits.sort_by(|(a, ap), (b, bp)| {
            ap.cmp(bp)
                .then_with(|| a.text.len().cmp(&b.text.len()))
                .then_with(|| a.id.cmp(&b.id))
        });
        hits.truncate(limit);
        hits.into_iter().map(|(b, _)| b).collect()
    }

    /// Drop every entry contributed by `slug`. Used before
    /// re-collecting a single page after a save.
    ///
    /// O(blocks_in_page) thanks to the `pages` secondary index — does
    /// not scan the whole workspace.
    pub fn forget_page(&mut self, slug: &str) {
        let victims = self.pages.remove(slug).unwrap_or_default();
        for id in &victims {
            if let Some(entry) = self.blocks.remove(id) {
                // Bug fix: only drop the handle entry if it points at
                // *this* block. In a collision, the surviving block
                // owns the base handle; removing it would unresolve
                // refs to a block we never owned in the first place.
                if self.handle_to_block.get(&entry.ref_handle) == Some(id) {
                    self.handle_to_block.remove(&entry.ref_handle);
                }
                self.location_to_block
                    .remove(&(entry.source_slug.clone(), entry.source_block_path.clone()));
            }
        }
        for list in self.block_refs.values_mut() {
            list.retain(|r| r.source_slug != slug);
        }
        self.block_refs.retain(|_, v| !v.is_empty());
    }

    /// One-shot page indexing — populates blocks, handles **and**
    /// reverse refs in a single call.
    ///
    /// Safe to use after the initial build has finished: every cited
    /// handle that exists somewhere in the workspace is already in
    /// `handle_to_block`, so reverse-edge resolution works on the
    /// first walk. During the initial build, where pages are loaded
    /// in arbitrary order, use the two-pass variants
    /// ([`collect_page_blocks`](Self::collect_page_blocks) +
    /// [`collect_page_refs`](Self::collect_page_refs)) so a citing
    /// page processed before its target still records its edge.
    pub fn collect_page(
        &mut self,
        source_slug: &str,
        source_path: &Path,
        blocks: &[OutlineNode],
        sidecar_blocks: &[SidecarBlock],
    ) {
        self.collect_page_blocks(source_slug, source_path, blocks, sidecar_blocks);
        self.collect_page_refs(source_slug, blocks, sidecar_blocks);
    }

    /// Pass 1 of the two-pass build: register every block of a page
    /// (id, handle, text, subtree) without touching reverse refs.
    pub fn collect_page_blocks(
        &mut self,
        source_slug: &str,
        source_path: &Path,
        blocks: &[OutlineNode],
        sidecar_blocks: &[SidecarBlock],
    ) {
        let mut cursor = 0usize;
        let mut path_stack: Vec<usize> = Vec::new();
        self.walk_blocks(
            blocks,
            sidecar_blocks,
            &mut cursor,
            &mut path_stack,
            source_slug,
            source_path,
        );
    }

    /// Pass 2 of the two-pass build: scan every block's text for
    /// `((blk-XXXXXX))` / `!((blk-XXXXXX))` and record the reverse
    /// edge. Assumes [`collect_page_blocks`](Self::collect_page_blocks)
    /// has already run for **every** page in the workspace —
    /// otherwise edges to pages processed later would be missed.
    pub fn collect_page_refs(
        &mut self,
        source_slug: &str,
        blocks: &[OutlineNode],
        sidecar_blocks: &[SidecarBlock],
    ) {
        let mut cursor = 0usize;
        let mut path_stack: Vec<usize> = Vec::new();
        self.walk_refs(
            blocks,
            sidecar_blocks,
            &mut cursor,
            &mut path_stack,
            source_slug,
        );
    }

    fn walk_blocks(
        &mut self,
        blocks: &[OutlineNode],
        sidecar_blocks: &[SidecarBlock],
        cursor: &mut usize,
        path_stack: &mut Vec<usize>,
        source_slug: &str,
        source_path: &Path,
    ) {
        for (i, b) in blocks.iter().enumerate() {
            path_stack.push(i);
            if let Some(sb) = sidecar_blocks.get(*cursor) {
                // Defensive: AST and sidecar must agree on this block
                // (content_hash is the canonical equality check). On
                // mismatch the sidecar is stale relative to the AST —
                // a brand-new block typed in-editor before reconcile
                // ran is the common cause. Skip the index entry; the
                // next reconcile writes a fresh sidecar with the new
                // block and the next build picks it up.
                if sb.content_hash == content_hash(&b.text) {
                    let base_handle = if sb.ref_handle.is_empty() {
                        sidecar::derive_ref_handle(sb.id)
                    } else {
                        sb.ref_handle.clone()
                    };

                    // Insert the BlockEntry first (with a placeholder
                    // handle) so `assign_handle` can patch a displaced
                    // owner's `ref_handle` even when that displaced
                    // owner is the same block being inserted right
                    // now (re-index path).
                    let text = b.text.clone();
                    let text_fold = text.to_lowercase();
                    self.blocks.insert(
                        sb.id,
                        BlockEntry {
                            id: sb.id,
                            ref_handle: base_handle.clone(),
                            source_slug: source_slug.to_string(),
                            source_path: source_path.to_path_buf(),
                            source_block_path: path_stack.clone(),
                            text,
                            text_fold,
                            children: b.children.clone(),
                        },
                    );
                    let final_handle = self.assign_handle(sb.id, base_handle);
                    if let Some(entry) = self.blocks.get_mut(&sb.id) {
                        entry.ref_handle = final_handle;
                    }
                    self.pages
                        .entry(source_slug.to_string())
                        .or_default()
                        .push(sb.id);
                    self.location_to_block
                        .insert((source_slug.to_string(), path_stack.clone()), sb.id);
                }
            }
            *cursor += 1;
            self.walk_blocks(
                &b.children,
                sidecar_blocks,
                cursor,
                path_stack,
                source_slug,
                source_path,
            );
            path_stack.pop();
        }
    }

    /// Pass 1 from the tree: register every block (id, handle, text,
    /// subtree) without touching reverse refs.
    ///
    /// Unlike [`collect_page_blocks`](Self::collect_page_blocks) this
    /// **cannot skip a block**. There is no sidecar to agree or
    /// disagree with — the id arrives on the node itself — so a page
    /// edited outside outl before reconcile ran is indexed in full
    /// rather than truncated at the first hash mismatch.
    pub fn collect_page_blocks_from_tree(
        &mut self,
        source_slug: &str,
        source_path: &Path,
        blocks: &[IdentifiedNode],
    ) {
        let mut path_stack: Vec<usize> = Vec::new();
        self.walk_blocks_tree(blocks, &mut path_stack, source_slug, source_path);
    }

    /// Pass 2 from the tree: record every `((blk-XXXXXX))` /
    /// `!((blk-XXXXXX))` reverse edge, workspace-wide.
    ///
    /// Reads the blocks **already in the index** rather than a caller's
    /// forest, so the caller does not have to keep one alive across
    /// both passes. That mattered: `derive` used to buffer the whole
    /// projected forest for this, holding a second copy of every
    /// block's subtree beside the copies the index itself stores.
    ///
    /// Call once, after
    /// [`collect_page_blocks_from_tree`](Self::collect_page_blocks_from_tree)
    /// has run for **every** page — a citation can point at a page
    /// walked later, and only then is every handle known.
    ///
    /// Idempotent: it rebuilds the reverse map from scratch, so calling
    /// it twice cannot double an edge.
    pub fn collect_refs_from_indexed(&mut self) {
        self.block_refs.clear();
        // Resolve against `handle_to_block` while iterating `blocks`,
        // then write — two disjoint borrows of `self` cannot overlap.
        let mut edges: Vec<(NodeId, BlockReference)> = Vec::new();
        for entry in self.blocks.values() {
            for tok in tokenize(&entry.text) {
                let cited = match tok {
                    InlineTok::BlockRef { handle } | InlineTok::Embed { handle } => handle,
                    _ => continue,
                };
                if let Some(&target) = self.handle_to_block.get(cited) {
                    edges.push((
                        target,
                        BlockReference {
                            source_slug: entry.source_slug.clone(),
                            source_block_path: entry.source_block_path.clone(),
                        },
                    ));
                }
            }
        }
        for (target, edge) in edges {
            self.block_refs.entry(target).or_default().push(edge);
        }
    }

    fn walk_blocks_tree(
        &mut self,
        blocks: &[IdentifiedNode],
        path_stack: &mut Vec<usize>,
        source_slug: &str,
        source_path: &Path,
    ) {
        for (i, b) in blocks.iter().enumerate() {
            path_stack.push(i);
            let base_handle = sidecar::derive_ref_handle(b.id);
            let text = b.text.clone();
            let text_fold = text.to_lowercase();
            // Insert before assigning the handle for the same reason
            // the disk path does: `assign_handle` may rewrite a
            // displaced owner's `ref_handle`, and that owner can be
            // this very entry on a re-index.
            self.blocks.insert(
                b.id,
                BlockEntry {
                    id: b.id,
                    ref_handle: base_handle.clone(),
                    source_slug: source_slug.to_string(),
                    source_path: source_path.to_path_buf(),
                    source_block_path: path_stack.clone(),
                    text,
                    text_fold,
                    children: b.children.iter().map(IdentifiedNode::to_outline).collect(),
                },
            );
            let final_handle = self.assign_handle(b.id, base_handle);
            if let Some(entry) = self.blocks.get_mut(&b.id) {
                entry.ref_handle = final_handle;
            }
            self.pages
                .entry(source_slug.to_string())
                .or_default()
                .push(b.id);
            self.location_to_block
                .insert((source_slug.to_string(), path_stack.clone()), b.id);

            self.walk_blocks_tree(&b.children, path_stack, source_slug, source_path);
            path_stack.pop();
        }
    }

    /// Assign a final handle to `id` and update `handle_to_block`.
    ///
    /// Determinism: when `base_handle` is already taken by a different
    /// block, the **smaller** `NodeId` (ULIDs sort lexicographically
    /// by creation time) keeps the base handle and the bigger one is
    /// expanded. Same outcome regardless of which device — or which
    /// workspace traversal order — first observed the collision.
    ///
    /// If `id` itself displaces the current owner, that owner's
    /// `BlockEntry.ref_handle` is rewritten in place so the new
    /// expanded form propagates everywhere it's read from.
    fn assign_handle(&mut self, id: NodeId, base_handle: String) -> String {
        match self.handle_to_block.get(&base_handle).copied() {
            Some(owner) if owner == id => base_handle,
            None => {
                self.handle_to_block.insert(base_handle.clone(), id);
                base_handle
            }
            Some(owner) if id < owner => {
                // `id` wins the base; current owner is dethroned.
                let owner_expanded = self.next_unused_expansion(owner, &base_handle);
                if let Some(entry) = self.blocks.get_mut(&owner) {
                    entry.ref_handle = owner_expanded.clone();
                }
                self.handle_to_block.remove(&base_handle);
                self.handle_to_block.insert(owner_expanded, owner);
                self.handle_to_block.insert(base_handle.clone(), id);
                base_handle
            }
            Some(_) => {
                // `id` is bigger; existing owner keeps the base.
                let expanded = self.next_unused_expansion(id, &base_handle);
                self.handle_to_block.insert(expanded.clone(), id);
                expanded
            }
        }
    }

    /// Smallest expanded handle (tail length > [`REF_HANDLE_TAIL_LEN`])
    /// for `id` that isn't already owned by a different block. Used as
    /// the displaced / losing side of a collision in [`assign_handle`].
    fn next_unused_expansion(&self, id: NodeId, _base: &str) -> String {
        let ulid_str = id.to_string();
        let total = ulid_str.chars().count();
        for tail_len in (REF_HANDLE_TAIL_LEN + 1)..=total {
            let chars_taken: String = ulid_str
                .chars()
                .skip(total - tail_len)
                .collect::<String>()
                .to_lowercase();
            let candidate = format!("{REF_HANDLE_PREFIX}{chars_taken}");
            match self.handle_to_block.get(&candidate) {
                Some(&owner) if owner == id => return candidate,
                None => return candidate,
                Some(_) => continue,
            }
        }
        // Fallback: the whole ULID. Only reachable if every prefix
        // length collides — pragmatically impossible for distinct ULIDs.
        format!("{REF_HANDLE_PREFIX}{}", ulid_str.to_lowercase())
    }

    fn walk_refs(
        &mut self,
        blocks: &[OutlineNode],
        sidecar_blocks: &[SidecarBlock],
        cursor: &mut usize,
        path_stack: &mut Vec<usize>,
        source_slug: &str,
    ) {
        for (i, b) in blocks.iter().enumerate() {
            path_stack.push(i);
            if sidecar_blocks.get(*cursor).is_some() {
                for tok in tokenize(&b.text) {
                    let cited = match tok {
                        InlineTok::BlockRef { handle } | InlineTok::Embed { handle } => handle,
                        _ => continue,
                    };
                    if let Some(target_id) = self.handle_to_block.get(cited).copied() {
                        self.block_refs
                            .entry(target_id)
                            .or_default()
                            .push(BlockReference {
                                source_slug: source_slug.to_string(),
                                source_block_path: path_stack.clone(),
                            });
                    }
                }
            }
            *cursor += 1;
            self.walk_refs(&b.children, sidecar_blocks, cursor, path_stack, source_slug);
            path_stack.pop();
        }
    }
}
