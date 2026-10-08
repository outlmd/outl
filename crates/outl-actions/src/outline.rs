//! UI-friendly projection of a page, built either from the workspace
//! tree (materialised op log) or from the `.md` file on disk.
//!
//! Both paths produce the same [`OutlineNode`] shape so the mobile
//! frontend doesn't care which source was used. In v0 the mobile and
//! TUI clients build the outline from `.md` + sidecar; the
//! [`project_outline`] variant stays around for tools that need to
//! materialise straight from the op log (e.g. doctor, debug dumps).

use std::path::Path;
use std::str::FromStr;

use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

/// Render a property value as the user-facing string the markdown
/// pipeline already stores. `Text` is the only variant emitted today
/// by `outl-md::diff` (see `crates/outl-md/src/diff.rs`); the other
/// variants are surfaced for forward-compat with the future query DSL
/// but should never appear in v0 workspaces.
pub(crate) fn prop_value_to_string(v: &PropValue) -> String {
    match v {
        PropValue::Text(s) | PropValue::PageRef(s) | PropValue::Tag(s) => s.clone(),
        PropValue::List(items) => items
            .iter()
            .map(prop_value_to_string)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// Enumerate every DFS path *inside* an [`OutlineNode`] (treated as
/// a self-contained subtree). The first entry is always `vec![]`
/// (the root itself); subsequent entries descend into children in
/// order.
///
/// Used by the TUI's inline backlinks panel so `j`/`k` can step
/// through a referencing block and its descendants without rebuilding
/// the index. Lives here (rather than in `outl-md`) so any future
/// client that consumes [`Backlink::source_block`][crate::Backlink::source_block]
/// can navigate its subtree with the same helper.
pub fn flatten_subtree_paths(root: &OutlineNode) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    out.push(stack.clone());
    walk_subtree(root, &mut stack, &mut out);
    out
}

fn walk_subtree(node: &OutlineNode, stack: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
    for (i, child) in node.children.iter().enumerate() {
        stack.push(i);
        out.push(stack.clone());
        walk_subtree(child, stack, out);
        stack.pop();
    }
}

/// Resolve a block's [`NodeId`] to its flat DFS index inside an
/// outline forest.
///
/// The ordering matches what `outl_exec::run_block_at_index` expects:
/// it parses the page's `.md`, walks `ParsedPage.blocks` in DFS, and
/// addresses the target block by its position in that walk. The
/// outline projected from the workspace tree (via [`project_outline`])
/// preserves the same order, so the two stay in sync as long as the
/// `.md` and the op log are reconciled (they always are after a
/// mutation through `outl-actions`).
///
/// Returns `None` when the id isn't in the outline (foreign page,
/// stale call, deleted block). Callers should surface that as a soft
/// error rather than panic — it's the canonical "outline drifted, try
/// again" signal.
///
/// Used by `outl_actions::exec::run_code_block` and by the Tauri
/// adapter shims in mobile + desktop that translate a block-id click
/// into a runtime invocation.
pub fn flat_index_for_block(outline: &[OutlineNode], target: NodeId) -> Option<usize> {
    let target_str = target.to_string();
    fn walk(nodes: &[OutlineNode], target: &str, counter: &mut usize) -> Option<usize> {
        for n in nodes {
            if n.id == target {
                return Some(*counter);
            }
            *counter += 1;
            if let Some(hit) = walk(&n.children, target, counter) {
                return Some(hit);
            }
        }
        None
    }
    let mut counter = 0usize;
    walk(outline, &target_str, &mut counter)
}

/// Build a single [`OutlineNode`] for `node` straight from the
/// workspace, including its subtree and properties.
///
/// Same shape as one element of [`project_outline`] — used by the
/// backlinks builder so each backlink carries the *source block* with
/// its children and properties, instead of forcing the caller to
/// reach back into the workspace per backlink.
///
/// Materialises `node`'s whole subtree; `project_outline_node_shallow`
/// is the cheaper answer when only the block itself is wanted.
pub fn project_outline_node(workspace: &Workspace, node: NodeId) -> OutlineNode {
    let index = crate::tree::children_index(workspace, node);
    project_node(workspace, node, &index)
}
use outl_md::parse::OutlineNode as ParsedOutlineNode;
use outl_md::sidecar::SidecarBlock;
use serde::Serialize;

use crate::error::ActionError;
use crate::journal::page_md_path;
use crate::page::PageMeta;
use crate::todo::{split_todo, TodoState};

/// A node in the outline as seen by the UI.
///
/// `text` is the block body **without** the TODO/DONE prefix (if any).
/// The prefix lives in [`Self::todo`].
#[derive(Debug, Clone, Serialize)]
pub struct OutlineNode {
    /// Stable block identifier, stringified.
    pub id: String,
    /// Block body without the TODO/DONE prefix.
    pub text: String,
    /// `None` for a plain bullet, `Some(Todo)` / `Some(Done)` otherwise.
    #[serde(serialize_with = "serialize_todo_state")]
    pub todo: Option<TodoState>,
    /// Whether the block is rendered collapsed (children hidden) in
    /// the outline. Overlaid from the workspace via
    /// [`Op::SetCollapsed`][outl_core::op::Op::SetCollapsed]; the op
    /// log is the source of truth. Clients SHOULD still send
    /// `children` so the renderer can show a "(N hidden)" hint
    /// without a second round trip.
    ///
    /// **Use `read_page_view_with_workspace` to populate this.** The
    /// bare [`read_page_view`] has no workspace in scope and leaves
    /// every entry at `false`.
    ///
    /// Mutated via [`crate::collapsed::set_block_collapsed`] /
    /// [`crate::collapsed::toggle_block_collapsed`], which generate
    /// `Op::SetCollapsed` and apply it through `Workspace::apply` —
    /// never via the sidecar.
    pub collapsed: bool,
    /// `(key, value)` properties attached to this block, in
    /// **alphabetical-by-key order**.
    ///
    /// Both producer paths normalise to this order so a backlink
    /// rendering of a block (workspace-driven) and the outline of
    /// the page that owns it (disk-driven) show properties in the
    /// same sequence. The workspace path has no authoring order to
    /// preserve (properties live in a `HashMap` keyed on
    /// `(NodeId, key)`); the disk path used to keep parse-order but
    /// now sorts on `outline_from_parsed` so the two surfaces never
    /// disagree visually.
    ///
    /// Shape mirrors [`outl_md::parse::OutlineNode::properties`].
    /// Populated from [`outl_core::tree::Tree::properties_of`] when
    /// the workspace is in scope, and from the parsed `.md` otherwise.
    pub properties: Vec<(String, String)>,
    /// Pre-tokenized inline markdown for `text` (no TODO/DONE prefix).
    ///
    /// The backend runs `outl_md::tokenize_owned` here so every client
    /// can render the block without keeping its own inline tokenizer
    /// in sync with the Rust canonical one. Mobile renders these
    /// straight into JSX; the TUI can ignore the field and keep using
    /// borrowed [`outl_md::InlineTok`] on `text` directly when it
    /// already has the string in scope.
    pub tokens: Vec<outl_md::InlineToken>,
    /// The block read as a table, when its **whole** text is one.
    ///
    /// Same bargain as [`Self::tokens`], one construct up: the backend
    /// decides what the grid is so no client splits pipes for itself
    /// and renders a cell's `[[ref]]` as literal text. A client that
    /// has this field draws a `<table>`; one that doesn't falls back
    /// to `tokens` and shows the rows verbatim, which is what every
    /// client did before tables were modelled.
    ///
    /// Omitted from the wire for an ordinary block (almost all of
    /// them), so the field costs nothing until a table exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub table: Option<outl_md::TableView>,
    /// Children, in their fractional-index order.
    pub children: Vec<OutlineNode>,
}

impl OutlineNode {
    /// The two fields that are **functions of the block's text** —
    /// inline tokens and the tokenized table reading — derived in one
    /// call.
    ///
    /// Together on purpose. Four producers build this type, and one
    /// that set `tokens` and forgot `table` would ship a block that
    /// renders as a grid on the page and as a wall of pipes in a
    /// backlink. Call this instead of `outl_md::tokenize_owned`
    /// directly.
    fn rendered(text: &str) -> (Vec<outl_md::InlineToken>, Option<outl_md::TableView>) {
        (outl_md::tokenize_owned(text), outl_md::tokenize_table(text))
    }
}

fn serialize_todo_state<S>(state: &Option<TodoState>, ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match state {
        None => ser.serialize_none(),
        Some(s) => ser.serialize_str(s.as_str()),
    }
}

/// A pre-built `parent -> children (in fractional order)` map.
///
/// Re-exported from [`crate::tree`], which owns both the type and the
/// sibling order its entries carry. The name stays reachable here
/// because a full-workspace walk is not a private concern:
/// [`crate::index::derive`] and
/// [`crate::backlinks_index::build_backlink_index`] both need one.
pub use crate::tree::ChildrenIndex;

/// Walk the workspace tree starting from `parent` and return the
/// outline below it. `NodeId::root()` is the usual starting point.
///
/// Builds one children index scoped to `parent`'s subtree and projects
/// through it, where this used to call [`crate::tree::children_of`] per
/// visited node — `O(nodes²)`, 7.2 s on a 64k-node workspace.
pub fn project_outline(workspace: &Workspace, parent: NodeId) -> Vec<OutlineNode> {
    let index = crate::tree::children_index(workspace, parent);
    project_children(workspace, parent, &index)
}

/// Project the outline below `parent`, resolving each level through
/// `index` in `O(children)`.
///
/// There is deliberately **no** fallback to [`crate::tree::children_of`]
/// here. The `Option<&ChildrenIndex>` this used to take defaulted to
/// the scanning path and every caller took the default — the doc
/// comment admitted it was "quadratic for a full-workspace walk" and
/// nothing made a caller notice.
fn project_children(
    workspace: &Workspace,
    parent: NodeId,
    index: &ChildrenIndex,
) -> Vec<OutlineNode> {
    index
        .get(&parent)
        .map(|kids| {
            kids.iter()
                .map(|&child| project_node(workspace, child, index))
                .collect()
        })
        .unwrap_or_default()
}

/// Build one [`OutlineNode`] for `node` — body, todo, properties,
/// tokens, and its subtree. Shared core behind [`project_outline`],
/// [`project_outline_node`], and [`project_outline_node_indexed`]; the
/// only difference between them is how children are resolved (`index`).
/// Project a single block as a **leaf** — body, todo, properties,
/// tokens — with an **empty** `children`.
///
/// The backlinks index uses this instead of [`project_outline_node`] so
/// building the index never descends (and tokenizes, and reads
/// properties of) every descendant of every referencing block in the
/// workspace. That full-subtree materialization, run while holding the
/// workspace lock, is what froze input on a large workspace. Clients
/// render a backlink row from the leaf's `tokens`; nothing on the
/// backlinks path needs the subtree.
pub(crate) fn project_outline_node_shallow(workspace: &Workspace, node: NodeId) -> OutlineNode {
    let raw = workspace.block_text(node).unwrap_or_default();
    let (todo, body) = split_todo(&raw);
    let mut properties: Vec<(String, String)> = workspace
        .tree()
        .properties_of(node)
        .map(|(k, v)| (k.to_string(), prop_value_to_string(v)))
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));
    let (tokens, table) = OutlineNode::rendered(body);
    OutlineNode {
        id: node.to_string(),
        text: body.to_string(),
        todo,
        collapsed: workspace.tree().is_collapsed(node),
        properties,
        tokens,
        table,
        children: Vec::new(),
    }
}

fn project_node(workspace: &Workspace, node: NodeId, index: &ChildrenIndex) -> OutlineNode {
    let raw = workspace.block_text(node).unwrap_or_default();
    let (todo, body) = split_todo(&raw);
    let mut properties: Vec<(String, String)> = workspace
        .tree()
        .properties_of(node)
        .map(|(k, v)| (k.to_string(), prop_value_to_string(v)))
        .collect();
    properties.sort_by(|a, b| a.0.cmp(&b.0));
    let (tokens, table) = OutlineNode::rendered(body);
    OutlineNode {
        id: node.to_string(),
        text: body.to_string(),
        todo,
        collapsed: workspace.tree().is_collapsed(node),
        properties,
        tokens,
        table,
        children: project_children(workspace, node, index),
    }
}

/// Read the page's `.md`, parse it, attach `NodeId`s from the sidecar,
/// and return the outline.
///
/// This is the **canonical UI path** in v0. The `.md` is the source
/// the user sees in Files.app / iCloud / vim; rendering anything else
/// would let the on-disk view drift from what the app shows.
///
/// Sidecar resolution accepts both the modern `<name>.outl` location
/// and the legacy `.<name>.outl` location and migrates the latter on
/// first read (see [`outl_md::resolve_sidecar_path`]). A missing
/// sidecar is not fatal — the outline returns block ids derived from
/// position so the UI can still render, but those ids are not stable
/// across processes and callers should run a reconcile before mutating.
pub fn read_page_view(root: &Path, meta: &PageMeta) -> Result<Vec<OutlineNode>, ActionError> {
    read_page_outline(root, meta).map(|po| po.nodes)
}

/// Same as [`read_page_view`] but also surfaces the parser warnings
/// emitted while reading the page's `.md` (see `outl_md::ParseWarning`).
///
/// Use this when a UI surface is going to render a banner /
/// status-line hint for "this file has lines that don't match the
/// outl dialect". The bare [`read_page_view`] discards them.
pub fn read_page_outline(root: &Path, meta: &PageMeta) -> Result<PageOutline, ActionError> {
    let md_path = page_md_path(root, meta);
    // A page whose `.md` isn't on disk yet legitimately reads as empty;
    // an unreadable one does not. Rendering an empty outline for a page
    // that *does* have content invites the user to retype it, and the
    // next commit writes that emptiness back. See `read_for_rewrite`.
    let md_text = outl_md::read_for_rewrite(&md_path)?;
    let parsed = outl_md::parse::parse(&md_text);
    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    let sidecar = outl_md::sidecar::read(&sidecar_path).ok();

    let mut nodes = Vec::with_capacity(parsed.blocks.len());
    let mut iter = sidecar
        .as_ref()
        .map(|sc| SidecarBlockCursor::Some(sc.blocks.iter()))
        .unwrap_or(SidecarBlockCursor::None);
    for block in &parsed.blocks {
        nodes.push(outline_from_parsed(block, &mut iter));
    }
    Ok(PageOutline {
        nodes,
        warnings: parsed.warnings,
    })
}

/// Same as [`read_page_view`] but overlays the workspace's
/// `Op::SetCollapsed` state so each [`OutlineNode`] reports the
/// authoritative `collapsed` flag. UI clients (TUI, mobile) **must**
/// use this variant — the bare `read_page_view` leaves `collapsed`
/// at `false` because it has no op log in scope.
pub fn read_page_view_with_workspace(
    root: &Path,
    meta: &PageMeta,
    workspace: &Workspace,
) -> Result<Vec<OutlineNode>, ActionError> {
    read_page_outline_with_workspace(root, meta, workspace).map(|po| po.nodes)
}

/// Workspace-aware variant of [`read_page_outline`]. Use this when a
/// client needs both the authoritative collapsed flags **and** the
/// parser warnings (every modern client does — mobile, desktop, TUI).
pub fn read_page_outline_with_workspace(
    root: &Path,
    meta: &PageMeta,
    workspace: &Workspace,
) -> Result<PageOutline, ActionError> {
    let mut outline = read_page_outline(root, meta)?;
    overlay_collapsed(&mut outline.nodes, workspace);
    Ok(outline)
}

/// Outline of a page bundled with the parser warnings produced while
/// reading its `.md`.
///
/// `warnings` is empty for a clean file in the outl dialect. When
/// non-empty, the UI is expected to render an actionable hint per
/// entry (line number + first chars of the raw text). Surfaces that
/// don't care can ignore the field; the legacy
/// [`read_page_view`] / [`read_page_view_with_workspace`] paths
/// return `Vec<OutlineNode>` and silently drop them for back-compat.
#[derive(Debug, Clone, Serialize)]
pub struct PageOutline {
    /// Outline nodes (same shape as the legacy `Vec<OutlineNode>`).
    pub nodes: Vec<OutlineNode>,
    /// Non-fatal parser recoveries — empty when the `.md` is clean.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<outl_md::parse::ParseWarning>,
}

/// Walk `nodes` in place, setting `collapsed` from
/// `workspace.tree().is_collapsed(id)` for every node whose id parses
/// as a valid ULID. Transient ids (the ones minted by
/// `outline_from_parsed` when the sidecar is missing or shorter than
/// the AST) parse fine but the workspace has never seen them, so
/// `is_collapsed` returns the default `false` — same result as
/// leaving the field untouched, which keeps the contract "every
/// node with no op log presence renders expanded".
fn overlay_collapsed(nodes: &mut [OutlineNode], workspace: &Workspace) {
    for node in nodes {
        if let Ok(ulid) = ulid::Ulid::from_str(&node.id) {
            let id = NodeId(ulid);
            node.collapsed = workspace.tree().is_collapsed(id);
        }
        overlay_collapsed(&mut node.children, workspace);
    }
}

enum SidecarBlockCursor<'a> {
    Some(std::slice::Iter<'a, SidecarBlock>),
    None,
}

impl<'a> SidecarBlockCursor<'a> {
    fn next(&mut self) -> Option<&'a SidecarBlock> {
        match self {
            SidecarBlockCursor::Some(it) => it.next(),
            SidecarBlockCursor::None => None,
        }
    }
}

/// Project a parsed subtree (bare `.md` AST, no sidecar) into wire
/// [`OutlineNode`]s with inline tokens attached.
///
/// Ids are **transient** — freshly minted per call, not the blocks'
/// stable ULIDs — because the parsed AST carries no ids (they live in
/// the sidecar). This suits read-only surfaces that re-resolve on
/// navigation rather than hold identity: the embed subtree
/// (`!((blk-XXXXXX))` expansion) is the caller, mirroring the TUI's
/// visual-only child expansion. Do **not** use it where a client needs
/// a stable id to mutate a block.
pub fn project_parsed_subtree(children: &[ParsedOutlineNode]) -> Vec<OutlineNode> {
    let mut cursor = SidecarBlockCursor::None;
    children
        .iter()
        .map(|child| outline_from_parsed(child, &mut cursor))
        .collect()
}

fn outline_from_parsed(
    block: &ParsedOutlineNode,
    iter: &mut SidecarBlockCursor<'_>,
) -> OutlineNode {
    let entry = iter.next();
    // When the sidecar is absent or shorter than the parsed AST, mint a
    // fresh transient NodeId per block. Returning an empty string would
    // give every fallback block the same id, which breaks keyed
    // rendering on the frontend (Solid for-each, React lists). The id is
    // unstable across renders by design — clients are expected to call
    // back into the workspace once `reconcile_md` has populated the
    // sidecar.
    let id = entry
        .map(|b| b.id.to_string())
        .unwrap_or_else(|| outl_core::id::NodeId::new().to_string());
    let (todo, body) = split_todo(&block.text);
    let children = block
        .children
        .iter()
        .map(|child| outline_from_parsed(child, iter))
        .collect();
    // Sort alphabetically so this disk-driven path matches what
    // `project_outline` (workspace-driven) produces. The two surfaces
    // would otherwise disagree on the order properties show up in,
    // visible when a block renders both inside its own page (parse
    // order) and as a backlink elsewhere (workspace order). See the
    // `OutlineNode.properties` doc-comment.
    let mut properties = block.properties.clone();
    properties.sort_by(|a, b| a.0.cmp(&b.0));
    let (tokens, table) = OutlineNode::rendered(body);
    // `collapsed` is overlaid by the caller using the workspace as the
    // source of truth (`Op::SetCollapsed` lives in the op log). The
    // bare `read_page_view` path leaves it `false`; the workspace-
    // aware `read_page_view_with_workspace` patches it.
    OutlineNode {
        id,
        text: body.to_string(),
        todo,
        collapsed: false,
        properties,
        tokens,
        table,
        children,
    }
}

#[cfg(test)]
mod tests;
