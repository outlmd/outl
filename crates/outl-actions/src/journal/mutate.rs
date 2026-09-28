//! The `.md`-as-source-of-truth rewrite path.
//!
//! [`mutate_page_md`] is the odd one out in this module tree: it does
//! not render the workspace tree, it reads the `.md`, mutates the parsed
//! AST, renders that back, and rebuilds the sidecar by content-hash
//! matching so unchanged blocks keep their `NodeId`.
//!
//! It lives beside the projection writers because it writes the same two
//! files and has to agree with them about the sidecar's shape, and
//! because it asks `super::guard`'s absent-markdown question for the
//! same reason they do: an unguarded absence here recreates the page as
//! a single block, which is the id mapping gone.

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_md::sidecar::{content_hash, file_hash, Sidecar, SidecarBlock};

use super::guard::guard_absent_markdown;
use super::paths::page_md_path;
use crate::error::ActionError;
use crate::page::PageMeta;

/// Apply a pure-AST mutation to a page's `.md`, then rewrite both the
/// `.md` and its sidecar.
///
/// **This is the path mobile mutations should take.** The workspace
/// op log isn't on the hot edit path here — we read the `.md` as the
/// source of truth, mutate the parsed AST, render it back, and rebuild
/// the sidecar by content-hash-matching the new blocks against the
/// previous sidecar so unchanged blocks keep their `NodeId`. Anything
/// the closure inserts gets a fresh ULID. Peers reading the resulting
/// `.md` + `.outl` see consistent ids.
///
/// The closure receives a map `NodeId -> block_path` derived from the
/// sidecar so callers can translate the ids the frontend passes in
/// (e.g. "create after block ABC") into the path-based mutations that
/// [`outl_md::outline_ops`] expects.
pub fn mutate_page_md<F>(root: &Path, meta: &PageMeta, mutation: F) -> Result<PathBuf, ActionError>
where
    F: FnOnce(
        &mut outl_md::parse::ParsedPage,
        &std::collections::HashMap<NodeId, Vec<usize>>,
    ) -> Result<(), ActionError>,
{
    use std::collections::HashMap;

    let md_path = page_md_path(root, meta);
    // NOT `unwrap_or_default()`: this function renders the parsed AST
    // straight back over `md_path`, so a read that fails for any reason
    // other than "the page doesn't exist yet" would replace the page
    // with an empty one — and rebuild the sidecar to agree, hiding it
    // from every later consistency scan. See `read_for_rewrite`.
    let md_text = outl_md::read_for_rewrite(&md_path)?;

    let sidecar_path = outl_md::resolve_sidecar_path(&md_path);
    // `read_for_rewrite` answers "absent" with `Ok("")`, which is only
    // legitimate for a page that does not exist yet. Everything else
    // that presents as absent is checked here, before the empty parse
    // becomes a write.
    if md_text.is_empty() {
        guard_absent_markdown(&md_path, &sidecar_path)?;
    }

    let mut parsed = outl_md::parse::parse(&md_text);

    // NOT `.ok()`: the same "unreadable reads as empty" bug the `.md`
    // was fixed for, one line down and worse. With `old_sidecar = None`
    // every block content-hash lookup misses, `build_sidecar_from_ast`
    // mints a **fresh ULID per block**, and the rewritten sidecar
    // replaces the id ↔ text mapping wholesale: every `((blk-…))`
    // pointing into this page stops resolving, and the next
    // `reconcile_md` sees N unknown ids and trashes the tree's blocks.
    // A `.md` is recoverable from the op log; that mapping is not.
    let old_sidecar = match outl_md::sidecar::read(&sidecar_path) {
        Ok(sidecar) => Some(sidecar),
        // Genuinely absent — a page this device has never projected.
        // The only case where minting ids is correct.
        Err(outl_md::sidecar::SidecarError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            None
        }
        Err(e) => return Err(e.into()),
    };

    // Build NodeId -> block_path map from the AST + sidecar (DFS
    // preorder lines up between the two).
    let mut id_to_path: HashMap<NodeId, Vec<usize>> = HashMap::new();
    if let Some(sc) = &old_sidecar {
        let mut iter = sc.blocks.iter();
        build_id_path_map(&parsed.blocks, &mut Vec::new(), &mut iter, &mut id_to_path);
    }

    mutation(&mut parsed, &id_to_path)?;

    let new_md = outl_md::render::render(&parsed);
    outl_md::write_atomic(&md_path, new_md.as_bytes())?;

    let page_id_ulid = ulid::Ulid::from_string(&meta.id)
        .map_err(|e| ActionError::NotInTree(format!("invalid page id {}: {e}", meta.id)))?;
    let page_id = NodeId(page_id_ulid);
    let new_sidecar = build_sidecar_from_ast(&parsed, old_sidecar.as_ref(), &new_md, page_id);
    outl_md::sidecar::write(&sidecar_path, &new_sidecar)?;

    Ok(md_path)
}
fn build_id_path_map<'a>(
    blocks: &[outl_md::parse::OutlineNode],
    current_path: &mut Vec<usize>,
    sidecar_iter: &mut std::slice::Iter<'a, SidecarBlock>,
    out: &mut std::collections::HashMap<NodeId, Vec<usize>>,
) {
    for (i, block) in blocks.iter().enumerate() {
        current_path.push(i);
        if let Some(sc) = sidecar_iter.next() {
            out.insert(sc.id, current_path.clone());
        }
        build_id_path_map(&block.children, current_path, sidecar_iter, out);
        current_path.pop();
    }
}

fn build_sidecar_from_ast(
    parsed: &outl_md::parse::ParsedPage,
    old_sidecar: Option<&Sidecar>,
    md: &str,
    page_id: NodeId,
) -> Sidecar {
    use std::collections::HashSet;
    let mut used: HashSet<NodeId> = HashSet::new();
    let mut blocks: Vec<SidecarBlock> = Vec::new();
    let mut line = 1usize;
    walk_ast_for_sidecar(
        &parsed.blocks,
        0,
        old_sidecar,
        &mut used,
        &mut line,
        &mut blocks,
    );
    Sidecar {
        version: outl_md::sidecar::SIDECAR_VERSION,
        page_id,
        last_synced_hash: file_hash(md),
        last_synced_at: chrono::Local::now().fixed_offset(),
        blocks,
        // Built from a parsed `.md` + workspace tree — both sources
        // already carry the page properties consistently, so this
        // sidecar represents a fully-propagated state.
        pipeline_version: outl_md::sidecar::CURRENT_PIPELINE_VERSION,
    }
}

fn walk_ast_for_sidecar(
    blocks: &[outl_md::parse::OutlineNode],
    indent: u32,
    old_sidecar: Option<&Sidecar>,
    used: &mut std::collections::HashSet<NodeId>,
    line: &mut usize,
    out: &mut Vec<SidecarBlock>,
) {
    for block in blocks {
        let hash = content_hash(&block.text);
        let id = old_sidecar
            .and_then(|sc| {
                sc.blocks
                    .iter()
                    .find(|b| b.content_hash == hash && !used.contains(&b.id))
                    .map(|b| b.id)
            })
            .unwrap_or_else(|| {
                // No content-hash match: this is a freshly inserted
                // block, so allocate a new random id.
                NodeId::new()
            });
        used.insert(id);
        out.push(SidecarBlock::from_text(id, *line, indent, &block.text));
        *line += 1;
        walk_ast_for_sidecar(&block.children, indent + 1, old_sidecar, used, line, out);
    }
}
