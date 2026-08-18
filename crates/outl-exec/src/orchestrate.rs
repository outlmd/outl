//! End-to-end "run this block": single function every UI calls.
//!
//! Flow:
//!
//! 1. Parse the page (`.md` on disk → AST).
//! 2. Walk the AST to find the block at `flat_index` (DFS preorder).
//! 3. Extract `(language, body)` from its fence text.
//! 4. Look up the runtime, call `execute`.
//! 5. Render the output as a `> **result:**` subblock.
//! 6. Upsert that subblock under the code block.
//! 7. Re-render the AST back to `.md`, atomic-write, reconcile into
//!    the op log.
//!
//! The function is sync today — runtimes are sync, the I/O is sync,
//! the TUI calls it from its event loop. When we add long-running
//! runtimes (compile-then-run Rust, streaming output) the boundary
//! becomes an async wrapper; the orchestration stays the same.

use std::path::Path;
use std::time::Duration;

use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use outl_md::index::WorkspaceIndex;
use outl_md::parse::{parse, OutlineNode};
use outl_md::reconcile::reconcile_md;
use outl_md::render::render;

use crate::language::extract_fence;
use crate::registry::RuntimeRegistry;
use crate::result_block::{
    render_result_body, result_source_hash, source_hash, upsert_result_child,
    upsert_result_child_with_hash, upsert_result_embeds, RESULT_MARKER,
};
use crate::runtime::{ExecContext, ExecError, ExecOutput, OutputFormat};

/// Default per-run timeout. UIs can override by building an
/// [`ExecContext`] manually and going around this helper.
/// Wall-clock budget for a single fence execution.
///
/// iOS gets a tighter 2-second budget: the UI is fully blocked during
/// execution (sync call from the Tauri command) and a longer wait
/// makes the app feel hung on a touch device. The narrative also
/// helps with App Review — a bounded, sub-second-typical timeout is
/// easier to defend under Guideline 2.5.2 than "user can queue
/// arbitrary workloads". Desktop / TUI keep 5s where the user has
/// keyboard interrupt and a real terminal mental model.
#[cfg(target_os = "ios")]
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
/// Wall-clock budget for a single fence execution (non-iOS targets).
///
/// See the iOS-specific entry above for the rationale on the
/// per-platform split.
#[cfg(not(target_os = "ios"))]
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// Errors that can happen *around* execution — before we ever reach
/// the runtime, or while persisting its output.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// `flat_index` doesn't point at any block in the page.
    #[error("no block at flat index {0}")]
    BlockNotFound(usize),
    /// The block exists but its text isn't a fenced code block.
    #[error("block is not a fenced code block")]
    NotACodeBlock,
    /// Fence has no language tag (` ``` ` with nothing after).
    #[error("code block has no language tag (e.g. ```lisp)")]
    MissingLanguage,
    /// No runtime registered for the requested language.
    #[error("no runtime registered for language `{0}`")]
    UnknownLanguage(String),
    /// Failed reading the `.md` from disk.
    #[error("read {path}: {source}")]
    Read {
        /// Path we tried to read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Failed writing the `.md` back.
    #[error("write {path}: {source}")]
    Write {
        /// Path we tried to write.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Reconciling the new AST into the op log failed.
    #[error("reconcile: {0}")]
    Reconcile(#[from] outl_md::reconcile::ReconcileError),
}

/// What a successful run hands back to the caller (the UI). Enough to
/// show a status-line message without re-reading the page.
///
/// Not `Clone` — `ExecError::Io` wraps `std::io::Error` which itself is
/// not `Clone`. UIs consume the report once.
#[derive(Debug)]
pub struct RunReport {
    /// Language tag that was executed.
    pub language: String,
    /// Outcome of the run — `Ok` when the runtime returned (even with
    /// non-zero exit), `Err` for infrastructure failures.
    pub result: Result<ExecOutput, ExecError>,
}

/// Run the code block at `flat_index` inside `md_path` through the
/// registry's matching runtime.
///
/// `flat_index` is the DFS-preorder position of the block in the page.
/// That's what TUI selection already tracks (`App.selected`) and what
/// `path_for_index` returns — same coordinate system.
///
/// `index` is a workspace index the caller already holds. The `query`
/// runtime needs one to answer at all; given `None` it builds a fresh
/// one off disk **per fence**, so a page carrying several ` ```query `
/// blocks re-reads and re-parses the whole workspace once for each of
/// them, on every page load, because `query` auto-runs. Pass what a
/// resident client already keeps (the TUI's `App::index`); pass `None`
/// only when there genuinely is none.
#[allow(clippy::too_many_arguments)]
pub fn run_block_at_index(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    flat_index: usize,
    registry: &RuntimeRegistry,
    orphans_log: Option<&Path>,
    index: Option<&WorkspaceIndex>,
) -> Result<RunReport, RunError> {
    // 1. Load and parse.
    let text = std::fs::read_to_string(md_path).map_err(|source| RunError::Read {
        path: md_path.display().to_string(),
        source,
    })?;
    let mut page = parse(&text);

    // 2. Find the block.
    let block = block_at_flat_index_mut(&mut page.blocks, flat_index)
        .ok_or(RunError::BlockNotFound(flat_index))?;

    // 3. Pull (language, body) out.
    let parts = extract_fence(&block.text).ok_or(RunError::NotACodeBlock)?;
    if parts.language.is_empty() {
        return Err(RunError::MissingLanguage);
    }
    let language = parts.language.clone();
    let body = parts.body;

    // 4. Look up the runtime.
    let runtime = registry
        .get(&language)
        .ok_or_else(|| RunError::UnknownLanguage(language.clone()))?;

    // 5. Execute.
    let ctx = ExecContext {
        workspace_root: workspace
            .root
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
        stdin: None,
        timeout: DEFAULT_TIMEOUT,
        mem_limit: None,
        index,
    };
    let result = runtime.execute(&body, &ctx);

    // 6. Render result, upsert (without source hash — manual `gx` is
    // always meant to refresh).
    match result.as_ref() {
        Ok(o) if o.format == OutputFormat::Embeds => {
            let embeds: Vec<&str> = o.stdout.lines().filter(|l| !l.is_empty()).collect();
            let header = format!("{RESULT_MARKER} ({} blocks)", embeds.len());
            upsert_result_embeds(block, header, &embeds);
        }
        _ => {
            let body = render_result_body(result.as_ref());
            upsert_result_child(block, body);
        }
    }

    // 7. Persist + reconcile.
    let rendered = render(&page);
    outl_md::write_atomic(md_path, rendered.as_bytes()).map_err(|source| RunError::Write {
        path: md_path.display().to_string(),
        source,
    })?;
    reconcile_md(workspace, hlc, md_path, orphans_log)?;

    Ok(RunReport { language, result })
}

/// Cache-aware variant of [`run_block_at_index`].
///
/// Used by the TUI's auto-run loop: a block with `auto-run::` set
/// runs **only when its source has changed** since the last execution.
/// "Changed" is decided by comparing SHA-256 of the fence body against
/// the `source-hash::` property stamped on the result subblock.
///
/// Returns:
/// - `Ok(Some(report))` — the block ran. Caller can update status.
/// - `Ok(None)` — cache hit, nothing happened.
/// - `Err(_)` — orchestration failure (same surface as `run_block_at_index`).
#[allow(clippy::too_many_arguments)]
pub fn run_block_at_index_if_source_changed(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    flat_index: usize,
    registry: &RuntimeRegistry,
    orphans_log: Option<&Path>,
    index: Option<&WorkspaceIndex>,
) -> Result<Option<RunReport>, RunError> {
    let text = std::fs::read_to_string(md_path).map_err(|source| RunError::Read {
        path: md_path.display().to_string(),
        source,
    })?;
    let mut page = parse(&text);

    let block = block_at_flat_index_mut(&mut page.blocks, flat_index)
        .ok_or(RunError::BlockNotFound(flat_index))?;
    let parts = extract_fence(&block.text).ok_or(RunError::NotACodeBlock)?;
    if parts.language.is_empty() {
        return Err(RunError::MissingLanguage);
    }
    let language = parts.language.clone();
    let body = parts.body;
    let want_hash = source_hash(&body);

    // Cache check: if the result subblock already records this exact
    // source hash, there's nothing to do.
    if result_source_hash(block)
        .map(|s| s == want_hash)
        .unwrap_or(false)
    {
        return Ok(None);
    }

    let runtime = registry
        .get(&language)
        .ok_or_else(|| RunError::UnknownLanguage(language.clone()))?;
    let ctx = ExecContext {
        workspace_root: workspace
            .root
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default()),
        stdin: None,
        timeout: DEFAULT_TIMEOUT,
        mem_limit: None,
        index,
    };
    let result = runtime.execute(&body, &ctx);

    match result.as_ref() {
        Ok(o) if o.format == OutputFormat::Embeds => {
            let embeds: Vec<&str> = o.stdout.lines().filter(|l| !l.is_empty()).collect();
            let header = format!("{RESULT_MARKER} ({} blocks)", embeds.len());
            upsert_result_embeds(block, header, &embeds);
        }
        _ => {
            let body_md = render_result_body(result.as_ref());
            upsert_result_child_with_hash(block, body_md, &want_hash);
        }
    }

    let rendered = render(&page);
    outl_md::write_atomic(md_path, rendered.as_bytes()).map_err(|source| RunError::Write {
        path: md_path.display().to_string(),
        source,
    })?;
    reconcile_md(workspace, hlc, md_path, orphans_log)?;

    Ok(Some(RunReport { language, result }))
}

/// DFS-preorder traversal returning the block at `target` flat index.
///
/// Lives here (small and private) so the crate doesn't need to depend
/// on `outl-tui::outline_ops` — keeps the dep graph one-way.
fn block_at_flat_index_mut(blocks: &mut [OutlineNode], target: usize) -> Option<&mut OutlineNode> {
    fn walk<'a>(
        nodes: &'a mut [OutlineNode],
        target: usize,
        counter: &mut usize,
    ) -> Option<&'a mut OutlineNode> {
        for node in nodes {
            if *counter == target {
                return Some(node);
            }
            *counter += 1;
            if let Some(hit) = walk(&mut node.children, target, counter) {
                return Some(hit);
            }
        }
        None
    }
    walk(blocks, target, &mut 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use outl_md::parse::ParsedPage;

    fn page_with_blocks(blocks: Vec<OutlineNode>) -> ParsedPage {
        ParsedPage {
            properties: Vec::new(),
            blocks,
            warnings: Vec::new(),
        }
    }

    fn leaf(text: &str) -> OutlineNode {
        OutlineNode {
            text: text.into(),
            properties: Vec::new(),
            children: Vec::new(),
        }
    }

    #[test]
    fn flat_index_zero_returns_first_block() {
        let mut p = page_with_blocks(vec![leaf("a"), leaf("b")]);
        let n = block_at_flat_index_mut(&mut p.blocks, 0).unwrap();
        assert_eq!(n.text, "a");
    }

    #[test]
    fn flat_index_descends_into_children() {
        // Tree:
        //   a (0)
        //     a1 (1)
        //     a2 (2)
        //   b (3)
        let mut p = page_with_blocks(vec![
            OutlineNode {
                text: "a".into(),
                properties: vec![],
                children: vec![leaf("a1"), leaf("a2")],
            },
            leaf("b"),
        ]);
        assert_eq!(
            block_at_flat_index_mut(&mut p.blocks, 1).unwrap().text,
            "a1"
        );
        assert_eq!(
            block_at_flat_index_mut(&mut p.blocks, 2).unwrap().text,
            "a2"
        );
        assert_eq!(block_at_flat_index_mut(&mut p.blocks, 3).unwrap().text, "b");
    }

    #[test]
    fn flat_index_past_end_returns_none() {
        let mut p = page_with_blocks(vec![leaf("a")]);
        assert!(block_at_flat_index_mut(&mut p.blocks, 99).is_none());
    }
}
