//! Concurrent edits the CRDT merged, kept so the sync panel can show them
//! and offer a way back. RFC 0362, "Trust you can see".
//!
//! Detection runs at reload, with the old and the fresh tree in hand. A
//! block counts as merged when a newly arrived `Edit` from another actor
//! is *older* than an `Edit` this device had already made to the same
//! block: we wrote without having seen it, so the two raced and Yrs
//! merged them. A peer edit authored later than ours, without having
//! seen ours, is also a race but looks sequential from here; without
//! per-op causal context it cannot be told apart, and it is not listed.
//!
//! The log is device-local view state (root invariant 7 does not apply:
//! nothing here must converge), newest first, capped at [`MERGE_LOG_CAP`],
//! and persisted to `<root>/.outl/state/merges-<actor>.json` so it
//! survives a restart. The file is named by this device's actor so two
//! devices sharing a synced folder never overwrite each other's log. Undo is
//! an ordinary `edit_text` back to this device's text from before the
//! merge, so it is new ops, syncs, and `Cmd+Z` can step it back.
//!
//! Shared by every client: desktop and mobile record at reload through
//! `outl-tauri-shared`, the terminal from its own reload.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::Op;
use outl_core::workspace::Workspace;
use serde::{Deserialize, Serialize};

/// Entries kept per workspace.
pub const MERGE_LOG_CAP: usize = 50;

/// One merged concurrent edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedEditDto {
    /// Stable id for undo: the block id plus the arriving op's HLC.
    pub id: String,
    /// Page holding the block.
    pub page_id: String,
    /// That page's title, for the list.
    pub page_title: String,
    /// The merged block.
    pub block_id: String,
    /// This device's text just before the merge.
    pub ours: String,
    /// The block's text after the merge.
    pub merged: String,
    /// Actor (op-log id, not a peer id) whose edit arrived.
    pub peer_actor: String,
    /// Wall clock of the arriving edit.
    pub at_ms: u64,
}

type Book = HashMap<PathBuf, Vec<MergedEditDto>>;

fn book() -> &'static Mutex<Book> {
    static BOOK: OnceLock<Mutex<Book>> = OnceLock::new();
    BOOK.get_or_init(Mutex::default)
}

/// `<root>/.outl/state/merges-<actor>.json`.
pub fn state_path(root: &Path, actor: ActorId) -> PathBuf {
    root.join(".outl")
        .join("state")
        .join(format!("merges-{}.json", actor.0))
}

fn load(root: &Path, actor: ActorId) -> Vec<MergedEditDto> {
    let mut log: Vec<MergedEditDto> = std::fs::read(state_path(root, actor))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    log.truncate(MERGE_LOG_CAP);
    log
}

fn save(root: &Path, actor: ActorId, log: &[MergedEditDto]) {
    let path = state_path(root, actor);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().expect("state dir"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(log)?)?;
        std::fs::rename(&tmp, &path)
    };
    if let Err(e) = write() {
        tracing::warn!("merges: could not persist {}: {e}", path.display());
    }
}

/// The log for `root`, read from disk the first time this process asks.
fn log_for<'a>(book: &'a mut Book, root: &Path, actor: ActorId) -> &'a mut Vec<MergedEditDto> {
    book.entry(root.to_path_buf())
        .or_insert_with(|| load(root, actor))
}

/// Blocks a reload merged concurrently, per the rule in the module doc.
pub fn detect(old: &Workspace, fresh: &Workspace, local: ActorId) -> Vec<MergedEditDto> {
    let seen: HashSet<Hlc> = old.log().iter().map(|op| op.ts).collect();
    let mut our_last: HashMap<NodeId, Hlc> = HashMap::new();
    for op in old.log().iter().filter(|op| op.actor == local) {
        if let Op::Edit { node, .. } = op.op {
            let slot = our_last.entry(node).or_insert(op.ts);
            *slot = (*slot).max(op.ts);
        }
    }
    let mut out = Vec::new();
    let mut listed = HashSet::new();
    for op in fresh.log().iter().filter(|op| !seen.contains(&op.ts)) {
        let Op::Edit { node, .. } = op.op else {
            continue;
        };
        if op.actor == local || listed.contains(&node) {
            continue;
        }
        if our_last.get(&node).is_none_or(|ours| *ours <= op.ts) {
            continue;
        }
        let ours = old.block_text(node).unwrap_or_default();
        let merged = fresh.block_text(node).unwrap_or_default();
        if ours == merged {
            continue;
        }
        let Some(page) = crate::enclosing_page_id(fresh, node) else {
            continue;
        };
        let page_title = crate::page_meta(fresh, page)
            .map(|m| m.title)
            .unwrap_or_default();
        listed.insert(node);
        out.push(MergedEditDto {
            id: format!("{node}@{}.{}", op.ts.physical_ms, op.ts.logical),
            page_id: page.to_string(),
            page_title,
            block_id: node.to_string(),
            ours,
            merged,
            peer_actor: op.actor.0.to_string(),
            at_ms: op.ts.physical_ms,
        });
    }
    out
}

/// Prepend `found` to the workspace's log, newest first, capped, and
/// persist it. An entry already listed (same id) is not added twice.
pub fn record(root: &Path, actor: ActorId, mut found: Vec<MergedEditDto>) {
    if found.is_empty() {
        return;
    }
    let Ok(mut book) = book().lock() else { return };
    let log = log_for(&mut book, root, actor);
    found.retain(|f| !log.iter().any(|e| e.id == f.id));
    found.append(log);
    // Sorted after the merge, not before. Sorting only the arriving
    // batch and prepending it ordered the list by *detection*, so a
    // peer edit that synced late sat above entries newer than itself —
    // while the DTO shows `at_ms`, and this doc says newest first. It
    // also decided the cap: the truncation dropped whatever happened to
    // be last rather than the oldest entry.
    found.sort_by_key(|b| std::cmp::Reverse(b.at_ms));
    found.truncate(MERGE_LOG_CAP);
    *log = found;
    save(root, actor, log);
}

/// The listed entries for `root`, newest first.
pub fn list(root: &Path, actor: ActorId) -> Vec<MergedEditDto> {
    match book().lock() {
        Ok(mut book) => log_for(&mut book, root, actor).clone(),
        Err(_) => Vec::new(),
    }
}

/// Remove one entry by id and persist; `None` when it is not listed.
pub fn take(root: &Path, actor: ActorId, id: &str) -> Option<MergedEditDto> {
    let mut book = book().lock().ok()?;
    let log = log_for(&mut book, root, actor);
    let at = log.iter().position(|e| e.id == id)?;
    let entry = log.remove(at);
    save(root, actor, log);
    Some(entry)
}

#[cfg(test)]
mod tests;
