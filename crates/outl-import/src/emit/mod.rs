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
use unicode_normalization::UnicodeNormalization;

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
        if let Err(e) = discard_stale_page(&mut dest, p, report) {
            report.warn(
                &p.title,
                format!("could not discard the previous content: {e}"),
            );
            continue;
        }
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

/// Make an emitted page replace, not merge into, what the workspace
/// already holds under its slug.
///
/// `reconcile_md` on a `.md` with no sidecar derives the page id from
/// the slug and starts with no known blocks, so every child the page
/// node already has in the tree survives next to the freshly created
/// ones — and a page node sitting in the trash is pulled back out with
/// all of its old children. That suits ordinary reconcile; an import
/// overwrites the pages it emits. With a sidecar beside the `.md` the
/// matcher already trashes what the source dropped, so this is a no-op
/// there (and on a resumed import, whose finished pages have sidecars).
///
/// The old children go to the trash through the op log, each recorded
/// in `orphans.log` first (crate rule: logged before moved).
fn discard_stale_page(
    dest: &mut ImportDest<'_>,
    page: &render::RenderedPage,
    report: &mut ImportReport,
) -> Result<(), Box<dyn std::error::Error>> {
    use outl_core::id::NodeId;
    use std::io::Write;

    if outl_md::sidecar::sidecar_path_for(&page.path).exists() {
        return Ok(());
    }
    let stem = page
        .path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let page_id = NodeId::from_slug(&stem.nfc().collect::<String>());
    let children: Vec<NodeId> = outl_actions::tree::children_of(dest.workspace, page_id)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    // Page-level properties the importer does not own. Structural keys
    // (`page-slug`, `page-kind`, `page-source`, `page-frontmatter`) and
    // `title` stay: reconcile sets or clears them from the new `.md`.
    let stale_props: Vec<String> = dest
        .workspace
        .tree()
        .properties_of(page_id)
        .map(|(k, _)| k.to_string())
        .filter(|k| {
            !outl_actions::tree::is_page_model_key(k) && k != outl_actions::page::TITLE_KEY
        })
        .collect();
    if children.is_empty() && stale_props.is_empty() {
        return Ok(());
    }
    let mut discarded = 0usize;
    for c in &children {
        discarded += 1 + descendant_count(dest.workspace, *c);
    }
    if let Some(dir) = dest.orphans.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&dest.orphans)?;
    let now = chrono::Local::now().to_rfc3339();
    for c in &children {
        writeln!(log, "{now}\tmd={}\tid={c}\thash=?", page.path.display())?;
    }
    log.flush()?;
    for c in children {
        outl_actions::block::delete(dest.workspace, dest.hlc, c)?;
    }
    for key in &stale_props {
        outl_actions::page::set_property(dest.workspace, dest.hlc, page_id, key, None)?;
    }
    report.replaced_blocks += discarded;
    report.replaced_props += stale_props.len();
    Ok(())
}

fn descendant_count(ws: &outl_core::workspace::Workspace, node: outl_core::id::NodeId) -> usize {
    outl_actions::tree::children_of(ws, node)
        .into_iter()
        .map(|(id, _)| 1 + descendant_count(ws, id))
        .sum()
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
