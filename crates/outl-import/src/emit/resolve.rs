//! Pass B — placeholder → `((blk-XXXXXX))`, through the op log.
//!
//! After pass A reconciles every written file, each page's sidecar
//! carries the freshly minted `NodeId`s and ref handles in
//! depth-first preorder — the same order the renderer counted. That
//! makes UID → handle a direct index lookup, guarded twice:
//!
//! - the sidecar's block count must match the renderer's count
//!   (a mismatch means the parser split/merged something the renderer
//!   didn't predict — the whole page degrades to `[[Title]]` links,
//!   counted and warned, never mis-wired);
//! - the target block's `content_hash` must match the rendered text
//!   before any edit lands.
//!
//! The rewrite itself goes through `outl_actions::edit_text` (op log
//! stays the source of truth) and the page is re-projected with
//! `apply_page_md_with_sidecar` — never by editing `.md` behind the
//! workspace's back, which would push blocks through the 3-level
//! matcher and could re-identify them.

use super::render::{RenderedPage, UidLoc, PLACEHOLDER_PREFIX};
use crate::adapter::ImportError;
use crate::report::ImportReport;
use crate::ImportDest;
use outl_md::sidecar::{self, content_hash, sidecar_path_for, Sidecar};
use std::collections::HashMap;

/// Rewrite every placeholder occurrence in `text`. `lookup` receives
/// `(uid, is_embed)` and returns the replacement for the `((…))` part
/// alone — the embed's leading `!` is user-visible outl syntax already
/// on disk and is never consumed.
///
/// Ref-vs-embed comes from the marker prefix itself
/// ([`PLACEHOLDER_PREFIX`] vs [`render::PLACEHOLDER_EMBED_PREFIX`]),
/// never from the surrounding text.
pub(super) fn rewrite_placeholders(
    text: &str,
    lookup: &mut dyn FnMut(&str, bool) -> String,
) -> String {
    let ref_open = format!("(({PLACEHOLDER_PREFIX}");
    let embed_open = format!("(({}", super::render::PLACEHOLDER_EMBED_PREFIX);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    loop {
        let next_ref = text[cursor..].find(&ref_open);
        let next_embed = text[cursor..].find(&embed_open);
        let (rel, open_len, is_embed) = match (next_ref, next_embed) {
            (Some(r), Some(e)) => {
                if e < r {
                    (e, embed_open.len(), true)
                } else {
                    (r, ref_open.len(), false)
                }
            }
            (None, Some(e)) => (e, embed_open.len(), true),
            (Some(r), None) => (r, ref_open.len(), false),
            (None, None) => break,
        };
        let start = cursor + rel;
        let after_open = start + open_len;
        let Some(close_rel) = text[after_open..].find("))") else {
            break; // Unbalanced — keep the rest verbatim.
        };
        let uid = &text[after_open..after_open + close_rel];
        out.push_str(&text[cursor..start]);
        let replacement = lookup(uid, is_embed);
        // A resolved REF whose preceding user text ends in `!` would
        // read as `!((blk-…))` — outl's *embed* syntax. A space is the
        // minimal faithful separator that keeps it a reference. An
        // unresolved ref goes back to its source text (`((uid))`), which
        // outl does not read as a handle, so it needs no separator.
        let handle_open = format!("(({}", outl_md::sidecar::REF_HANDLE_PREFIX);
        if !is_embed && out.ends_with('!') && replacement.starts_with(&handle_open) {
            out.push(' ');
        }
        out.push_str(&replacement);
        cursor = after_open + close_rel + 2;
    }
    out.push_str(&text[cursor..]);
    out
}

/// Apply pass B against the destination workspace.
pub(super) fn apply(
    dest: &mut ImportDest<'_>,
    pages: &[RenderedPage],
    uid_map: &HashMap<String, UidLoc>,
    report: &mut ImportReport,
    sink: crate::progress::ProgressSink<'_>,
) -> Result<(), ImportError> {
    // Which pages need their sidecar in memory: every ref target plus
    // every page that has placeholders or collapsed blocks.
    let mut needed: Vec<bool> = pages
        .iter()
        .map(|p| !p.placeholder_blocks.is_empty() || !p.collapsed.is_empty())
        .collect();
    for loc in uid_map.values() {
        needed[loc.page_idx] = true;
    }

    let sidecars: Vec<Option<Sidecar>> = pages
        .iter()
        .zip(&needed)
        .map(|(page, wanted)| {
            if !wanted {
                return None;
            }
            match sidecar::read(&sidecar_path_for(&page.path)) {
                Ok(sc) if sc.blocks.len() == page.block_count => Some(sc),
                Ok(sc) => {
                    report.warn(
                        &page.title,
                        format!(
                            "sidecar has {} blocks, renderer counted {} — refs into this page \
                             degrade to [[links]]",
                            sc.blocks.len(),
                            page.block_count
                        ),
                    );
                    None
                }
                Err(e) => {
                    report.warn(&page.title, format!("sidecar unreadable: {e}"));
                    None
                }
            }
        })
        .collect();

    let work_total = pages
        .iter()
        .filter(|p| !p.placeholder_blocks.is_empty() || !p.collapsed.is_empty())
        .count();
    let mut work_done = 0usize;
    for (idx, page) in pages.iter().enumerate() {
        if page.placeholder_blocks.is_empty() && page.collapsed.is_empty() {
            continue;
        }
        work_done += 1;
        sink(crate::progress::ImportProgress::Resolving {
            done: work_done,
            total: work_total,
        });
        let Some(sc) = &sidecars[idx] else {
            // The file-fallback sweep below degrades this page's
            // placeholders to `[[links]]`.
            continue;
        };

        let mut touched = false;
        for pb in &page.placeholder_blocks {
            let entry = &sc.blocks[pb.dfs_index];
            if entry.content_hash != content_hash(&pb.text) {
                report.warn(
                    &page.title,
                    format!(
                        "block #{} text diverged between render and reconcile — degraded via \
                         file fallback",
                        pb.dfs_index
                    ),
                );
                continue;
            }
            let new_text = rewrite_placeholders(&pb.text, &mut |uid, is_embed| {
                resolve_one(uid, is_embed, uid_map, &sidecars, report)
            });
            if new_text != pb.text {
                outl_actions::edit_text(dest.workspace, dest.hlc, entry.id, &new_text)
                    .map_err(|e| ImportError::Workspace(e.to_string()))?;
                touched = true;
            }
        }

        for &ci in &page.collapsed {
            outl_actions::set_block_collapsed(dest.workspace, dest.hlc, sc.blocks[ci].id, true)
                .map_err(|e| ImportError::Workspace(e.to_string()))?;
            report.collapsed_applied += 1;
        }

        if touched {
            outl_actions::journal::apply_page_md_with_sidecar(
                dest.workspace,
                &dest.root,
                sc.page_id,
            )
            .map_err(|e| ImportError::Workspace(e.to_string()))?;
        }
    }

    sink(crate::progress::ImportProgress::Finishing);

    // Final sweep: no placeholder marker survives on disk. Whatever the
    // op path couldn't rewrite — a page whose block count diverged from
    // the sidecar, an individual block whose text drifted, a marker the
    // parser lifted into a property value — is degraded in the file
    // itself (`[[Title]]`, or the source `((uid))` for a target the
    // export does not have) and reconciled back, the same external-edit
    // path a hand-edited `.md` takes. Nothing
    // references these blocks by handle (resolution into an unmappable
    // page already fell back), so a re-identification is harmless.
    for page in pages {
        if !page.has_markers {
            continue;
        }
        let text = match std::fs::read_to_string(&page.path) {
            Ok(t) => t,
            Err(e) => {
                report.warn(&page.title, format!("fallback sweep read failed: {e}"));
                continue;
            }
        };
        if !text.contains(PLACEHOLDER_PREFIX)
            && !text.contains(super::render::PLACEHOLDER_EMBED_PREFIX)
        {
            continue;
        }
        // The page *holding* the marker could not be wired block by
        // block (a marker in a property value, or a page whose block
        // count diverged), but the *target* usually can: its handle is
        // in its own sidecar. Only a target on an unmappable page
        // degrades to a `[[Title]]` link. Degrading every one of them
        // turned a `note:: ((uid))` property into a page link even when
        // the block it named was right there.
        let new_text = rewrite_placeholders(&text, &mut |uid, is_embed| {
            resolve_one(uid, is_embed, uid_map, &sidecars, report)
        });
        if let Err(e) = outl_md::write_atomic(&page.path, new_text.as_bytes()) {
            report.warn(&page.title, format!("fallback sweep write failed: {e}"));
            continue;
        }
        if let Err(e) =
            outl_md::reconcile_md(dest.workspace, dest.hlc, &page.path, Some(&dest.orphans))
        {
            report.warn(&page.title, format!("fallback sweep reconcile failed: {e}"));
        }
        report.warn(
            &page.title,
            "placeholders outside block text resolved via file fallback (unmappable targets \
             degrade to [[links]])",
        );
    }

    Ok(())
}

/// One placeholder → its final text.
fn resolve_one(
    uid: &str,
    is_embed: bool,
    uid_map: &HashMap<String, UidLoc>,
    sidecars: &[Option<Sidecar>],
    report: &mut ImportReport,
) -> String {
    match uid_map.get(uid) {
        Some(loc) => match &sidecars[loc.page_idx] {
            Some(sc) => {
                // The embed's leading `!` is already in the text —
                // both forms replace only the `((…))` part.
                let handle = &sc.blocks[loc.dfs_index].ref_handle;
                if is_embed {
                    report.embeds_resolved += 1;
                } else {
                    report.refs_resolved += 1;
                }
                format!("(({handle}))")
            }
            None => {
                report.refs_page_fallback += 1;
                format!("[[{}]]", loc.page_title)
            }
        },
        // The target is not in the export (deleted in the source, or
        // never part of it). Keep the source text as written, `((uid))`:
        // outl does not read a source uid as one of its handles, so it
        // stays inert text, and the uid is still there to search for.
        // An embed keeps its `!` (already in the text), so it reads
        // `!((uid))`.
        None => {
            report.refs_unresolved += 1;
            format!("(({uid}))")
        }
    }
}
