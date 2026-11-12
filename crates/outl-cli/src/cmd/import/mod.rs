//! `outl import` — bring an existing graph in from Roam, Logseq, or
//! Obsidian.
//!
//! Every source routes through the adapter-based [`outl_import`]
//! crate: source → typed IR → outl markdown with placeholder refs →
//! sidecar-backed resolution into real `((blk-XXXXXX))` handles and
//! `Op::SetCollapsed` flags. This module is pure glue: argument
//! handling, workspace bootstrap, report printing.
//!
//! `outl import auto <src> <dst>` picks the adapter from the source's
//! shape (Roam = JSON file, Logseq DB = EDN graph export, Logseq =
//! graph dir, Obsidian = vault with `.obsidian/`).
//!
//! Re-import safety (is this destination safe to write into, and is a
//! half-finished import resumable) lives in [`guard`].

mod guard;

use anyhow::{Context, Result};
use guard::{
    clear_marker, guard_foreign_destination, guard_workspace, read_marker, recovery_hint,
    write_marker,
};
use outl_import::adapters::logseq_db::{db_graph_dir_message, is_db_graph_dir};
use outl_import::adapters::{LogseqAdapter, LogseqDbAdapter, ObsidianAdapter, RoamAdapter};
use outl_import::SourceAdapter;
use std::path::Path;

/// Flags shared by every import source.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImportFlags {
    /// Parse + report only, write nothing.
    pub dry_run: bool,
    /// Print the report as JSON.
    pub json: bool,
    /// Keep source create/edit timestamps as block properties.
    pub preserve_timestamps: bool,
    /// Skip pulling referenced files into `assets/` — keep original links.
    pub no_assets: bool,
    /// Also download remote images into `assets/` (off: keep the URL).
    pub download_remote_assets: bool,
    /// Import into a destination that already holds content, overwriting
    /// it. Off by default — see [`guard::guard_workspace`].
    pub force: bool,
}

/// Dispatch on the source format chosen by the user.
pub fn run(source: &str, src: &Path, dst: &Path, flags: ImportFlags) -> Result<()> {
    let adapter: &dyn SourceAdapter = match source {
        "roam" => &RoamAdapter,
        "logseq" => &LogseqAdapter,
        "logseq-db" => &LogseqDbAdapter,
        "obsidian" => &ObsidianAdapter,
        "auto" => auto_detect(src)?,
        other => {
            anyhow::bail!(
                "unknown import source: {other} (expected: roam, logseq, logseq-db, obsidian, auto)"
            )
        }
    };
    run_adapter(adapter, src, dst, flags)
}

/// Pick the adapter from the source's on-disk shape.
///
/// Obsidian is checked before Logseq on purpose: `.obsidian/` is the
/// unambiguous marker, while Logseq's heuristic (`pages/` +
/// `journals/`) also matches an Obsidian vault that happens to use
/// those folder names — and routing such a vault through the Logseq
/// outline parser silently drops its non-outline prose.
fn auto_detect(src: &Path) -> Result<&'static dyn SourceAdapter> {
    if RoamAdapter::detect(src) {
        return Ok(&RoamAdapter);
    }
    if LogseqDbAdapter::detect(src) {
        return Ok(&LogseqDbAdapter);
    }
    if ObsidianAdapter::detect(src) {
        return Ok(&ObsidianAdapter);
    }
    // A DB graph directory matches no file layout: without this it would
    // fall through to the generic message, or (via `logseq`) import nothing.
    if is_db_graph_dir(src) {
        anyhow::bail!("{}", db_graph_dir_message(src));
    }
    if LogseqAdapter::detect(src) {
        return Ok(&LogseqAdapter);
    }
    anyhow::bail!(
        "could not auto-detect the source format of {} — pass it explicitly \
         (roam = JSON backup file, logseq = graph directory, logseq-db = EDN graph export, \
         obsidian = vault directory)",
        src.display()
    )
}

/// Live single-line progress on stderr (`\r`-repainted). TTY-only —
/// piped/CI runs stay silent — and throttled so a 66k-block import
/// doesn't spend its time writing terminal escapes. Purely cosmetic:
/// events come from `outl_import::ImportProgress` and dropping them
/// never affects the import.
struct ProgressLine {
    tty: bool,
    start: std::time::Instant,
    last_paint: std::time::Instant,
}

impl ProgressLine {
    fn new() -> Self {
        use std::io::IsTerminal;
        let now = std::time::Instant::now();
        Self {
            tty: std::io::stderr().is_terminal(),
            start: now,
            last_paint: now - std::time::Duration::from_secs(1),
        }
    }

    fn paint(&mut self, ev: outl_import::ImportProgress<'_>) {
        use outl_import::ImportProgress as P;
        if !self.tty {
            return;
        }
        // Phase transitions always paint; per-page ticks are throttled.
        let throttled = matches!(
            ev,
            P::Writing { .. } | P::Reconciling { .. } | P::Resolving { .. }
        );
        if throttled && self.last_paint.elapsed() < std::time::Duration::from_millis(80) {
            return;
        }
        self.last_paint = std::time::Instant::now();

        let line = match ev {
            P::Parsing => "parsing source…".to_string(),
            P::Rendered { pages } => format!("rendered {pages} pages"),
            P::Writing { done, total } => {
                format!("{} writing pages {done}/{total}", bar(done, total))
            }
            P::Reconciling { done, total, page } => {
                let mut name: String = page.chars().take(32).collect();
                if name.len() < page.len() {
                    name.push('…');
                }
                format!(
                    "{} reconciling {done}/{total} · {name} · {}",
                    bar(done, total),
                    elapsed(self.start)
                )
            }
            P::Resolving { done, total } => {
                format!(
                    "{} resolving refs {done}/{total} · {}",
                    bar(done, total),
                    elapsed(self.start)
                )
            }
            P::Finishing => format!("finishing… · {}", elapsed(self.start)),
        };
        eprint!("\r\x1b[K{line}");
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }

    /// Clear the status line and leave a final elapsed stamp.
    fn finish(&mut self) {
        if self.tty {
            eprintln!("\r\x1b[Kdone in {}", elapsed(self.start));
        }
    }
}

/// 20-slot ASCII progress bar with percentage.
fn bar(done: usize, total: usize) -> String {
    let pct = (done * 100).checked_div(total).unwrap_or(100);
    let filled = pct / 5;
    format!(
        "[{}{}] {pct:>3}%",
        "#".repeat(filled),
        "-".repeat(20 - filled)
    )
}

/// Compact elapsed time (`45s`, `3m12s`).
fn elapsed(start: std::time::Instant) -> String {
    let s = start.elapsed().as_secs();
    if s < 60 {
        format!("{s}s")
    } else {
        format!("{}m{:02}s", s / 60, s % 60)
    }
}

/// Run one adapter against the destination workspace.
fn run_adapter(
    adapter: &dyn SourceAdapter,
    src: &Path,
    dst: &Path,
    flags: ImportFlags,
) -> Result<()> {
    let opts = outl_import::ImportOptions {
        preserve_timestamps: flags.preserve_timestamps,
        import_assets: !flags.no_assets,
        download_remote_assets: flags.download_remote_assets,
        max_bytes: outl_config::load().assets.max_bytes,
    };

    let report = if flags.dry_run {
        outl_import::dry_run(adapter, src, &opts)
            .with_context(|| format!("{} dry-run from {}", adapter.id(), src.display()))?
    } else {
        let dst = dst.to_path_buf();
        guard_foreign_destination(&dst, flags.force)?;
        if !dst.exists() {
            crate::cmd::init::run(&dst, "global", false)?;
        }
        let paths = crate::workspace_layout::Paths::at(dst.clone());
        // Read before opening: `ws::open` itself writes nothing to the
        // marker, but keeping the read next to the guard makes the
        // "resume" decision one place.
        let unfinished = read_marker(&paths.dot_outl);
        let mut ctx = crate::ws::open(&dst)
            .map_err(|e| anyhow::anyhow!("opening workspace: {} ({})", e.message, e.code))?;
        guard_workspace(&dst, &ctx.workspace, flags.force, unfinished.as_ref())?;
        write_marker(&paths.dot_outl, adapter.id(), src);
        let dest = outl_import::ImportDest {
            workspace: &mut ctx.workspace,
            hlc: &ctx.hlc,
            root: paths.root.clone(),
            pages_dir: paths.pages.clone(),
            journals_dir: paths.journals.clone(),
            orphans: paths.orphans.clone(),
        };
        let mut progress = ProgressLine::new();
        let out = outl_import::run_import_with_progress(adapter, src, dest, &opts, &mut |ev| {
            progress.paint(ev)
        })
        .with_context(|| format!("{} import from {}", adapter.id(), src.display()));
        progress.finish();
        match out {
            Ok(report) => {
                clear_marker(&paths.dot_outl);
                report
            }
            // Leave the marker in place: it's what makes the retry
            // possible without `--force`.
            Err(e) => return Err(e.context(recovery_hint(&dst, &paths.dot_outl))),
        }
    };

    if flags.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        report.print_human();
        if !flags.dry_run {
            println!();
            println!(
                "Next: run `outl --workspace {}` to open the imported workspace.",
                dst.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: an Obsidian vault that also uses `pages/` +
    /// `journals/` folder names matches Logseq's heuristic too —
    /// `.obsidian/` must win or the Logseq outline parser silently
    /// drops the vault's non-outline prose.
    #[test]
    fn auto_detect_prefers_obsidian_marker_over_logseq_heuristic() {
        let dir = tempfile::tempdir().expect("tempdir");
        for sub in [".obsidian", "pages", "journals"] {
            std::fs::create_dir_all(dir.path().join(sub)).expect("mkdir");
        }
        let adapter = auto_detect(dir.path()).expect("detects");
        assert_eq!(adapter.id(), "obsidian");
    }

    /// A Logseq DB graph directory (db.sqlite, no pages/ + journals/)
    /// used to import as 0 pages with no error. Every entry point must
    /// now say how to export it instead.
    #[test]
    fn a_logseq_db_graph_directory_is_explained_by_every_entry_point() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("db.sqlite"), b"SQLite format 3\0").expect("write");
        let dst = dir.path().join("dst");
        let flags = ImportFlags {
            dry_run: true,
            ..ImportFlags::default()
        };
        for source in ["auto", "logseq", "logseq-db"] {
            let err =
                run(source, dir.path(), &dst, flags).expect_err("a DB graph directory is refused");
            let msg = format!("{err:#}");
            assert!(msg.contains("Export EDN file"), "{source}: {msg}");
            assert!(msg.contains("outl import logseq-db"), "{source}: {msg}");
        }
    }

    /// `auto` routes a datoms EDN export to the DB adapter, and a
    /// page-level EDN snippet to the generic "pass it explicitly" error.
    #[test]
    fn auto_detect_picks_logseq_db_for_a_datoms_export_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let good = dir.path().join("graph.edn");
        std::fs::write(
            &good,
            "{:logseq.db.sqlite.export/graph-format :datoms, :datoms []}",
        )
        .expect("write");
        assert_eq!(auto_detect(&good).expect("detects").id(), "logseq-db");

        let page = dir.path().join("page.edn");
        std::fs::write(&page, "{:logseq.db.sqlite.export/export-type :page}").expect("write");
        assert!(auto_detect(&page).is_err());
    }
}
