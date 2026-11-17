//! `run_code_block` command body — the Tauri side of
//! `outl_actions::exec::run_code_block`.
//!
//! All the real work — walking the outline, resolving the page's `.md`
//! path, calling `outl_exec::run_block_at_index`, building the
//! Serde-friendly outcome — happens in `outl-actions` so every client
//! shares the same flow. This body owns only:
//!
//! - argument parsing (`String` → `NodeId`)
//! - the `AppHost` lookup (workspace + storage_root + hlc + registry)
//! - composing the refreshed [`PageView`] into the wire reply
//!
//! **Every command here is `async`.** The claim this module used to
//! carry — that Tauri serves synchronous commands from a worker pool —
//! is false: a `#[tauri::command]` without `async fn` is registered
//! `ExecutionContext::Blocking` and runs on the IPC thread, which under
//! `wry` is the UI main thread. Running a user's code block, or building
//! a `WorkspaceIndex` from disk, there stalls every other `invoke` and on
//! iOS trips the 10s scene-update watchdog. All three now clone what they
//! need out of the host and run the blocking half on the pool
//! ([`crate::offload`]).
//!
//! That moves the *stall*, not the mutex: `run_code_block` and
//! `run_auto_run_blocks` still hold the workspace lock for the whole
//! execution, because `outl_exec` writes the result subblock through it.
//! The win is that an edit now waits on a lock instead of on a thread
//! that cannot even deliver the keystroke.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use outl_actions::{
    page_meta as page_meta_action, read_page_outline_with_workspace,
    run_code_block as action_run_code_block, ExecOutputDto, RunCodeBlockOutcome,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use outl_exec::{extract_fence, run_block_at_index, RuntimeRegistry};
use outl_md::lang;
use parking_lot::Mutex;
use serde::Serialize;
use tracing::warn;

use crate::helpers::{build_page_view, parse_node_id};
use crate::host::AppHost;
use crate::offload;
use crate::state::PageView;

/// Wire reply for `run_code_block`. Adds the [`PageView`] to the shared
/// [`RunCodeBlockOutcome`] so the frontend re-renders in a single
/// round-trip.
#[derive(Debug, Clone, Serialize)]
pub struct RunCodeBlockReply {
    /// The detected language (`"python"`, `"lisp"`, …).
    pub language: String,
    /// Successful execution payload, or `None` when the runtime bailed
    /// before producing output.
    pub result_ok: Option<ExecOutputDto>,
    /// Infrastructure / runtime-not-found message, when applicable.
    pub error: Option<String>,
    /// Refreshed view of the page so the frontend re-renders without a
    /// follow-up `open_page_by_slug` round-trip — `outl-exec` already
    /// wrote the result subblock and reconciled with the op log.
    pub view: PageView,
}

/// Everything an off-thread `exec` body needs from the host.
///
/// Cloned out of the borrowed `AppHost` **before** the `await`, because a
/// blocking task has to be `'static` and `AppHost` hands out borrows.
/// Cloning the [`HlcGenerator`] is the subtle one, and it is safe: its
/// state is an `Arc<Mutex<_>>`, so a clone is the *same* generator — not
/// a second one issuing duplicate `(time, counter, actor)` triples on one
/// actor id, which is op identity and would make `Workspace::apply` dedup
/// one of the two away.
struct ExecHost {
    workspace: Arc<Mutex<Option<Workspace>>>,
    root: PathBuf,
    hlc: HlcGenerator,
    registry: Arc<RuntimeRegistry>,
}

impl ExecHost {
    fn of<S: AppHost>(state: &S) -> Result<Self, String> {
        Ok(Self {
            workspace: state.workspace_arc(),
            root: state.storage_root()?,
            hlc: state.hlc().clone(),
            registry: state.exec_registry(),
        })
    }

    /// The page as the client renders it, read under a brief lock.
    fn view(&self, page: NodeId) -> Result<PageView, String> {
        offload::locked(&self.workspace, |ws| {
            build_page_view(ws, &self.root, page).map_err(|e| e.to_string())
        })
    }
}

pub async fn run_code_block<S: AppHost>(
    state: &S,
    page_id: String,
    block_id: String,
) -> Result<RunCodeBlockReply, String> {
    let host = ExecHost::of(state)?;
    let page = parse_node_id(&page_id)?;
    let block = parse_node_id(&block_id)?;

    offload::run("run_code_block", move || {
        offload::locked_mut(&host.workspace, |ws| {
            let RunCodeBlockOutcome {
                language,
                result_ok,
                error,
            } = action_run_code_block(ws, &host.hlc, &host.root, &host.registry, page, block)
                .map_err(|e| e.to_string())?;

            if let Some(msg) = error.as_ref() {
                warn!("run_code_block runtime error: {msg}");
            }

            let view = build_page_view(ws, &host.root, page).map_err(|e| e.to_string())?;
            Ok(RunCodeBlockReply {
                language,
                result_ok,
                error,
                view,
            })
        })
    })
    .await
}

/// Wire reply for `run_auto_run_blocks`.
#[derive(Debug, Clone, Serialize)]
pub struct AutoRunReply {
    /// How many blocks were executed.
    pub ran: usize,
    /// Refreshed view after running query blocks.
    pub view: PageView,
}

/// Resolve a batch of block-ref handles (`blk-XXXXXX`) to their source
/// content. One reply serves both surfaces the handle can appear in:
/// `((blk-…))` inline refs render `text`; `!((blk-…))` embeds render
/// `text` plus the `children` subtree, mirroring the TUI.
/// Wire reply for `resolve_embeds`.
#[derive(Debug, Clone, Serialize)]
pub struct EmbedContent {
    pub handle: String,
    pub text: String,
    pub page_slug: String,
    /// `"todo"`, `"doing"`, `"done"`, or `None` when the block is not a task.
    pub status: Option<String>,
    /// The source block's subtree, projected with inline tokens so an
    /// embed surface can expand it exactly as the source page would.
    /// Empty for a leaf block. Ids are transient (see
    /// [`outl_actions::project_parsed_subtree`]) — the subtree is a
    /// read-only, re-resolved-on-navigation view, never a mutation target.
    pub children: Vec<outl_actions::OutlineNode>,
}

/// Sweep the current page for blocks whose fence language has
/// `auto_run() == true` (query blocks) and execute them. Returns the
/// refreshed [`PageView`].
pub async fn run_auto_run_blocks<S: AppHost>(
    state: &S,
    page_id: String,
) -> Result<AutoRunReply, String> {
    let host = ExecHost::of(state)?;
    let page = parse_node_id(&page_id)?;
    offload::run("run_auto_run_blocks", move || {
        auto_run_on_page(&host, page, &page_id)
    })
    .await
}

/// The blocking half of [`run_auto_run_blocks`], split out so the async
/// entry point stays the three lines that move host handles onto the
/// pool. Holds the workspace lock twice, briefly for the refreshed view
/// and for the whole sweep in between — the sweep writes each block's
/// result through the tree, so it cannot be any shorter than the
/// execution it drives.
fn auto_run_on_page(host: &ExecHost, page: NodeId, page_id: &str) -> Result<AutoRunReply, String> {
    let registry = &host.registry;
    let root = &host.root;

    let auto_run_langs: Vec<String> = registry
        .languages()
        .filter(|lang| registry.get(lang).map(|r| r.auto_run()).unwrap_or(false))
        .map(String::from)
        .collect();

    // Does any auto-run runtime actually read `ExecContext::index`?
    // Only `query` does today; the answer gates the derivation below.
    let needs_index = auto_run_langs
        .iter()
        .filter_map(|lang| registry.get(lang))
        .any(|r| r.needs_workspace_index());

    if auto_run_langs.is_empty() {
        return Ok(AutoRunReply {
            ran: 0,
            view: host.view(page)?,
        });
    }

    let ran = offload::locked_mut(&host.workspace, |ws| {
        let meta =
            page_meta_action(ws, page).ok_or_else(|| format!("page not found: {page_id}"))?;
        let outline =
            read_page_outline_with_workspace(root, &meta, ws).map_err(|e| e.to_string())?;

        let mut targets: Vec<usize> = Vec::new();
        let mut cursor = 0usize;
        collect_auto_run_flat_indices(&outline.nodes, &auto_run_langs, &mut cursor, &mut targets);

        let md_path = outl_actions::journal::page_md_path(root, &meta);
        let hlc = &host.hlc;
        // Build once for the whole sweep, not per block: a runtime
        // handed no index builds its own, and this loop runs every
        // auto-run block on the page. Skipped entirely when no target's
        // runtime reads one, so a page of `auto-run::` python blocks
        // does not pay for a facility only `query` uses.
        //
        // The **disk** build, not `outl_actions::index::derive`, and
        // deliberately so. This runs on every page load and holds the
        // workspace mutex for the whole command, while deriving reads
        // `block_text` for every node: the lazy-boot materialization
        // that froze the app in #179, and the reason
        // `build_backlink_index_from_disk` exists beside its
        // from-workspace twin. What the derivation buys is real, but not
        // worth a stall on the boot path. The short-lived readers that
        // hold no UI (CLI, MCP) get it instead.
        let index = (needs_index && !targets.is_empty())
            .then(|| outl_md::index::WorkspaceIndex::build(root));
        let mut count = 0usize;
        for idx in targets {
            if let Err(e) = run_block_at_index(
                ws,
                hlc,
                &md_path,
                idx,
                registry,
                Some(&root.join(".outl").join("orphans.log")),
                index.as_ref(),
            ) {
                warn!("auto-run block {idx} failed: {e}");
            } else {
                count += 1;
            }
        }
        Ok(count)
    })?;

    Ok(AutoRunReply {
        ran,
        view: host.view(page)?,
    })
}

/// How deep an embed's subtree is projected into the reply. Mirror of
/// the frontend's `EmbeddedSubtree` `MAX_DEPTH` (and the TUI's
/// `EMBED_MAX_DEPTH`): the client never renders past this, so shipping
/// deeper levels is pure IPC weight. Keep the three in step.
const EMBED_SUBTREE_MAX_DEPTH: usize = 4;

/// Batch-resolve embed handles to their source content.
///
/// Async: it builds a `WorkspaceIndex` from disk, parsing every `.md` +
/// sidecar in the workspace, and the desktop fires it on every
/// navigation to resolve the page's `((refs))` ([`crate::offload`]). It
/// takes no workspace lock at all — the index is read straight off the
/// projection — so this one is pure disk work on the pool.
pub async fn resolve_embeds<S: AppHost>(
    state: &S,
    handles: Vec<String>,
) -> Result<HashMap<String, EmbedContent>, String> {
    let root = state.storage_root()?;
    offload::run("resolve_embeds", move || {
        let index = outl_md::index::WorkspaceIndex::build(&root);
        let mut result = HashMap::new();
        for handle in &handles {
            if let Some(entry) = index.resolve_block_ref(handle) {
                let (status, text) = split_todo_prefix(&entry.text);
                let mut children = outl_actions::project_parsed_subtree(&entry.children);
                truncate_children_depth(&mut children, EMBED_SUBTREE_MAX_DEPTH);
                result.insert(
                    handle.clone(),
                    EmbedContent {
                        handle: entry.ref_handle.clone(),
                        text,
                        page_slug: entry.source_slug.clone(),
                        status,
                        children,
                    },
                );
            }
        }
        Ok(result)
    })
    .await
}

/// Wire reply for `resolve_page_embeds` — the page-level twin of
/// [`EmbedContent`], for the `![[page]]` token (issue #190).
#[derive(Debug, Clone, Serialize)]
pub struct PageEmbedContent {
    /// Slug the name resolved to.
    pub slug: String,
    /// The page's display title (its `title::`, or the slug when unset).
    pub title: String,
    /// The page's top-level blocks, projected with inline tokens and
    /// capped at `EMBED_SUBTREE_MAX_DEPTH` — the same projection and
    /// the same cap as [`EmbedContent::children`], so an embed renders
    /// what the source page renders. Ids are transient (see
    /// [`outl_actions::project_parsed_subtree`]): the expansion is a
    /// read-only view re-resolved on navigation, never a mutation
    /// target.
    pub blocks: Vec<outl_actions::OutlineNode>,
}

/// Batch-resolve `![[page]]` names to their target page's title and
/// top-level blocks.
///
/// Async for [`resolve_embeds`]'s reasons, which are the same ones: it
/// builds a `WorkspaceIndex` from disk, parsing every `.md` + sidecar in
/// the workspace, and a client fires it on every navigation
/// ([`crate::offload`]). It takes no workspace lock either — the index
/// and the page both come straight off the projection.
///
/// **Keyed by the name as written in the token**, not by slug: the
/// renderer holds `token.value`, the same way it holds the handle for
/// [`resolve_embeds`]. A name that resolves to no page is omitted,
/// exactly like an unresolvable block handle, and the client falls back
/// to showing the raw token.
pub async fn resolve_page_embeds<S: AppHost>(
    state: &S,
    names: Vec<String>,
) -> Result<HashMap<String, PageEmbedContent>, String> {
    let root = state.storage_root()?;
    offload::run("resolve_page_embeds", move || {
        let index = outl_md::index::WorkspaceIndex::build(&root);
        let mut result = HashMap::new();
        for name in &names {
            let Some(entry) = page_for_name(&index, name) else {
                continue;
            };
            let Some(blocks) = page_top_level_blocks(&entry.path) else {
                continue;
            };
            result.insert(
                name.clone(),
                PageEmbedContent {
                    slug: entry.slug.clone(),
                    title: entry.title.clone(),
                    blocks,
                },
            );
        }
        Ok(result)
    })
    .await
}

/// Resolve a `![[name]]` target to an indexed page — read-only, and
/// never creating one.
///
/// The ladder is `outl_actions::resolve::resolve_or_create_by_name`'s,
/// minus its last rung: a name with no page behind it is left out of the
/// reply, because rendering a block must never write to the op log.
///
/// 1. literal slug, for a name the user already typed as one;
/// 2. `slugify(name)`, which is what `[[Ideas]]` and `[[os/linux]]`
///    land on — the slug is lowercased and `/`-folded;
/// 3. `title::`, then `alias::`. [`outl_md::index::WorkspaceIndex::by_title`]
///    owns that precedence (a page titled `NYC` keeps `![[NYC]]` from a
///    page merely aliased `NYC`), which is why this asks it instead of
///    scanning the page list.
///
/// The one case the by-name ladder reaches and this does not is a
/// case-differing name on a page whose slug no longer derives from its
/// title; that costs a `list_all` over every page's text there, and the
/// embed shows its raw token here rather than paying it on every
/// navigation.
fn page_for_name<'a>(
    index: &'a outl_md::index::WorkspaceIndex,
    name: &str,
) -> Option<&'a outl_md::index::PageEntry> {
    if let Some(entry) = index.by_slug(name) {
        return Some(entry);
    }
    let slug = outl_md::slug::slugify(name);
    if slug != name {
        if let Some(entry) = index.by_slug(&slug) {
            return Some(entry);
        }
    }
    index.by_title(name)
}

/// The page's top-level blocks, read off its `.md` and projected with
/// inline tokens.
///
/// A second read of one file the index already parsed, which is the
/// cheap half of the trade: the index keeps page *metadata*, so the
/// alternative is reconstructing an AST out of its block entries, whose
/// properties are case-folded for the query DSL and whose population
/// path skips a block whose sidecar hash went stale.
///
/// `None` for a file that cannot be read — the page is then omitted from
/// the reply, same as an unresolvable name. Nothing here writes, so
/// "cannot show it right now" is the honest answer; an empty outline
/// would claim the page is empty.
fn page_top_level_blocks(md_path: &Path) -> Option<Vec<outl_actions::OutlineNode>> {
    let text = std::fs::read_to_string(md_path).ok()?;
    let parsed = outl_md::parse::parse(&text);
    let mut blocks = outl_actions::project_parsed_subtree(&parsed.blocks);
    truncate_children_depth(&mut blocks, EMBED_SUBTREE_MAX_DEPTH);
    Some(blocks)
}

/// Drop subtree levels the client won't render (beyond
/// [`EMBED_SUBTREE_MAX_DEPTH`]). `nodes` is level 1; once `depth_left`
/// hits 1, the current level keeps its text but loses its children.
fn truncate_children_depth(nodes: &mut [outl_actions::OutlineNode], depth_left: usize) {
    if depth_left <= 1 {
        for node in nodes {
            node.children.clear();
        }
        return;
    }
    for node in nodes {
        truncate_children_depth(&mut node.children, depth_left - 1);
    }
}

/// Split `"TODO body"` / `"DOING body"` / `"DONE body"` into
/// `(Some("todo"|"doing"|"done"), "body")`, reusing the canonical
/// `outl_actions::split_todo` so the marker vocabulary stays
/// consistent — a new state reaches the GUI without a change here.
fn split_todo_prefix(raw: &str) -> (Option<String>, String) {
    let (state, body) = outl_actions::split_todo(raw);
    let status = state.map(|s| s.as_str().to_lowercase());
    (status, body.into())
}

fn collect_auto_run_flat_indices(
    blocks: &[outl_actions::OutlineNode],
    auto_run_langs: &[String],
    cursor: &mut usize,
    out: &mut Vec<usize>,
) {
    for b in blocks {
        if let Some(parts) = extract_fence(&b.text) {
            let canonical = lang::canonical(&parts.language).unwrap_or(&parts.language);
            if auto_run_langs.iter().any(|l| l == canonical) {
                out.push(*cursor);
            }
        }
        *cursor += 1;
        collect_auto_run_flat_indices(&b.children, auto_run_langs, cursor, out);
    }
}
