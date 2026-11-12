//! Emission — shared by every adapter.
//!
//! `render` (pass A) turns the IR into markdown with placeholders;
//! `resolve` (pass B) maps source UIDs to real handles through the
//! stamped sidecars and applies the rewrite through the op log.

mod assets;
mod merge_names;
pub mod render;
mod resolve;

use crate::adapter::ImportError;
use crate::ir::ImportGraph;
use crate::progress::{ImportProgress, ProgressSink};
use crate::report::ImportReport;
use crate::{ImportDest, ImportOptions};
use std::path::Path;

/// Full pipeline: render → write → reconcile → resolve.
pub fn run(
    graph: &ImportGraph,
    mut dest: ImportDest<'_>,
    opts: &ImportOptions,
    report: &mut ImportReport,
    sink: ProgressSink<'_>,
) -> Result<(), ImportError> {
    let (mut pages, uid_map) =
        render::render_graph(graph, &dest.pages_dir, &dest.journals_dir, opts, report);
    // Resolve assets (copy / download) and rewrite every asset
    // placeholder BEFORE anything hits disk — the ref/embed resolve pass
    // that runs after reconcile must never see an asset placeholder.
    assets::apply(&mut pages, graph, &dest.root, opts, report);
    sink(ImportProgress::Rendered { pages: pages.len() });

    // Pass A: write every file, then reconcile so sidecars are
    // stamped with NodeIds + ref handles.
    for (i, p) in pages.iter().enumerate() {
        if let Some(dir) = p.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| ImportError::io(dir, e))?;
        }
        outl_md::write_atomic(&p.path, p.text.as_bytes())
            .map_err(|e| ImportError::io(&p.path, e))?;
        sink(ImportProgress::Writing {
            done: i + 1,
            total: pages.len(),
        });
    }
    for (i, p) in pages.iter().enumerate() {
        // `i + 1` = "processing page i+1 of N" (same convention as
        // `Writing`), so the bar moves from the first page instead of
        // sitting at 0% while a large page reconciles.
        sink(ImportProgress::Reconciling {
            done: i + 1,
            total: pages.len(),
            page: &p.stem,
        });
        if let Err(e) =
            outl_md::reconcile_md(dest.workspace, dest.hlc, &p.path, Some(&dest.orphans))
        {
            report.warn(&p.title, format!("reconcile failed: {e}"));
        }
    }

    // Pass B: placeholders → real handles, collapsed flags → ops.
    resolve::apply(&mut dest, &pages, &uid_map, report, sink)?;

    measure_landing(&pages, report);
    Ok(())
}

/// Count what actually reached the destination's op log.
///
/// Every other counter in the report is bumped in `render`, in memory,
/// **before** a byte hits disk — so a write failure, a reconcile
/// failure, or a block the 3-level matcher dropped is invisible to all
/// of them. The sidecar is the one artefact written *from* the
/// materialized tree by `reconcile_md`, so summing its block entries
/// measures the blocks that are genuinely in the op log.
///
/// A missing or unreadable sidecar is a page whose content is **not**
/// confirmed; it is warned and left out of the total rather than
/// assumed present.
fn measure_landing(pages: &[render::RenderedPage], report: &mut ImportReport) {
    for page in pages {
        match outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&page.path)) {
            Ok(sc) => {
                report.landed_pages += 1;
                report.landed_blocks += sc.blocks.len();
            }
            Err(e) => report.warn(
                &page.title,
                format!(
                    "no readable sidecar after reconcile ({e}) — this page's blocks are NOT \
                     confirmed in the op log"
                ),
            ),
        }
    }
    report.landing_measured = true;
}

/// Dry-run: render in memory and simulate the resolve pass so the
/// report carries the same ref/embed numbers a real import would.
pub fn dry_run(graph: &ImportGraph, opts: &ImportOptions, report: &mut ImportReport) {
    let (pages, uid_map) = render::render_graph(
        graph,
        Path::new("pages"),
        Path::new("journals"),
        opts,
        report,
    );
    for page in &pages {
        // Scan the whole page text (not just placeholder blocks) so
        // markers the parser lifts into property values count too —
        // matching what the real run's file-fallback sweep reaches.
        if page.has_markers {
            let _ = resolve::rewrite_placeholders(&page.text, &mut |uid, is_embed| {
                if uid_map.contains_key(uid) {
                    if is_embed {
                        report.embeds_resolved += 1;
                    } else {
                        report.refs_resolved += 1;
                    }
                } else {
                    report.refs_unresolved += 1;
                }
                String::new()
            });
        }
        report.collapsed_applied += page.collapsed.len();
    }
    // Asymmetry (documented, like `refs_page_fallback`): a dry-run does
    // no asset IO, so it can't tell a copyable file from a missing one.
    // It counts every referenced asset as "would copy" — the optimistic
    // estimate — rather than downloading or stat'ing anything.
    if opts.import_assets {
        for asset in &graph.assets {
            if assets::pulls(asset, opts) {
                report.assets_copied += 1;
            } else {
                report.assets_remote_kept += 1;
            }
        }
    }
}
