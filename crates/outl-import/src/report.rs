//! Import report — the fidelity contract with the user.
//!
//! Every translation, fallback, and drop the pipeline performs is
//! counted here. The report doubles as the acceptance test for a
//! migration: run `--dry-run --json` against a real backup and the
//! numbers say exactly what would survive.

use serde::Serialize;
use std::collections::BTreeMap;

/// How many individual warnings [`ImportReport::print_human`] spells
/// out before summarising the rest. `--json` always carries all of them.
const WARNINGS_SHOWN: usize = 20;

/// Summary of what an import produced (or would produce, on dry-run).
#[derive(Debug, Default, Serialize)]
pub struct ImportReport {
    /// Adapter id (`roam`, `logseq`, `obsidian`).
    pub source: String,
    /// Non-journal pages written.
    pub pages: usize,
    /// Journal pages written.
    pub journals: usize,
    /// Total outline blocks emitted.
    pub blocks: usize,
    /// `((uid))` refs resolved to a real `((blk-XXXXXX))` handle.
    pub refs_resolved: usize,
    /// Refs whose target page couldn't be handle-mapped — degraded to
    /// a `[[Page Title]]` link.
    pub refs_page_fallback: usize,
    /// Refs whose UID matches nothing in the source, kept as the
    /// source wrote them, `((uid))`, still greppable.
    pub refs_unresolved: usize,
    /// `{{embed}}` of a block resolved to `!((blk-XXXXXX))`.
    pub embeds_resolved: usize,
    /// `{{embed}}` of a whole page — degraded to `[[Page]]` until
    /// page embeds land (issue #190).
    pub embeds_page_fallback: usize,
    /// `#[[Multi Word]]` tags rewritten to `[[Multi Word]]` page refs.
    pub tags_multiword_to_page_ref: usize,
    /// Source task-state counts (`TODO`, `DONE`, Logseq's `DOING`, …).
    pub tasks: BTreeMap<String, usize>,
    /// Task markers found *mid-block* (`did {{[[DONE]]}} the thing`).
    /// A block carries one task state, and it comes from the marker at
    /// its head — so a mid-text marker keeps its literal `TODO`/`DONE`
    /// text but the block is **not** a task: it won't answer `outl
    /// query --kind=task`. Counted here because that is real loss.
    pub tasks_midtext_literal: usize,
    /// Components preserved verbatim, counted per kind
    /// (`query`, `table`, `kanban`, `latex`, …).
    pub components_verbatim: BTreeMap<String, usize>,
    /// Source queries translated to a ` ```query ` fence.
    pub queries_translated: usize,
    /// Roam `{{[[pdf]]: url}}` components rewritten to a `[name](url)`
    /// file link, outl's form for a file.
    pub pdfs_linked: usize,
    /// Org-style `DEADLINE:`/`SCHEDULED:` timestamps converted to
    /// `[[YYYY-MM-DD]]` date links (issue #63 model). Logseq only: org
    /// syntax is Logseq's, and Roam keeps a stamp as the text it is.
    pub org_dates_converted: usize,
    /// Roam `{{[[table]]}}` blocks folded into one pipe-table block.
    pub tables_converted: usize,
    /// Cell blocks a table fold turned into rows of its table block.
    /// Another legitimate reducer of the emitted block count.
    pub blocks_folded_into_tables: usize,
    /// Blocks that stopped being blocks because their content was
    /// promoted to page properties (a leading pure-attribute block).
    /// A legitimate reducer of the emitted block count — tracked so the
    /// input-vs-output reconciliation closes instead of false-alarming.
    pub blocks_lifted_to_props: usize,
    /// Source pages that merged into one destination file (two source
    /// pages naming the same journal date). Another legitimate reducer
    /// of the emitted page count.
    pub pages_merged: usize,
    /// Pages present in the source, counted straight off the parsed
    /// source structure — the denominator of the reconciliation.
    /// `0` when the adapter doesn't report source counts.
    pub source_pages: usize,
    /// Blocks present in the source, counted recursively off the parsed
    /// source structure — the denominator of the reconciliation.
    pub source_blocks: usize,
    /// Destination pages whose sidecar was readable after the import —
    /// proof the file reached `reconcile_md` and its blocks are in the
    /// destination's op log.
    pub landed_pages: usize,
    /// Blocks that already lived under a page this import rewrote and
    /// were moved to the trash so the page holds exactly what the
    /// source has (`--force`, or a trashed page being re-imported).
    pub replaced_blocks: usize,
    /// Page-level properties removed from those pages for the same
    /// reason; the new `.md` sets the ones the source still carries.
    pub replaced_props: usize,
    /// Blocks confirmed in the destination's op log, summed off the
    /// sidecars `reconcile_md` stamps straight from the materialized
    /// tree. This is the only counter downstream of the write: a page
    /// that failed to write, failed to reconcile, or lost blocks in the
    /// 3-level matcher is missing from it.
    pub landed_blocks: usize,
    /// Whether [`Self::landed_pages`] / [`Self::landed_blocks`] were
    /// measured at all. `false` on a dry-run, which writes nothing —
    /// there the counts genuinely stop at render, and the report says so
    /// instead of implying the content is in a workspace.
    pub landing_measured: bool,
    /// Input-vs-output reconciliation, filled by
    /// [`ImportReport::finalize`] once the pipeline is done.
    pub reconciliation: Option<Reconciliation>,
    /// Page-level `key:: value` properties carried over.
    pub props_pages: usize,
    /// Block-level `key:: value` properties carried over.
    pub props_blocks: usize,
    /// Blocks whose collapsed/folded state landed as `Op::SetCollapsed`.
    pub collapsed_applied: usize,
    /// Blocks and pages whose create/edit timestamps were dropped
    /// (default policy; re-run with `--preserve-timestamps` to keep
    /// them).
    pub timestamps_dropped: usize,
    /// Page-name collisions after slugify — disambiguated with a
    /// numeric suffix.
    pub slug_collisions: usize,
    /// Source-only metadata stripped on the way in (`id::` lines,
    /// `#+` directives, `:LOGBOOK:` drawers, dropped frontmatter
    /// keys, `logseq.*` properties).
    pub artifacts_stripped: usize,
    /// Referenced files pulled into `assets/` (local copy or remote
    /// download), content-addressed.
    pub assets_copied: usize,
    /// Referenced files that could not be pulled (missing local file,
    /// failed download, over the size cap) — the original link is kept
    /// verbatim so nothing dangles silently.
    pub assets_missing: usize,
    /// Total bytes of the assets pulled in.
    pub assets_bytes: u64,
    /// Remote images left as the source wrote them (`![alt](url)`):
    /// not downloaded, see `ImportOptions::download_remote_assets`.
    pub assets_remote_kept: usize,
    /// Source files skipped entirely, with the reason.
    pub skipped: Vec<SkippedFile>,
    /// Non-fatal fidelity warnings, with location.
    pub warnings: Vec<ImportWarning>,
}

/// A source file the import refused or had no mapping for.
#[derive(Debug, Serialize)]
pub struct SkippedFile {
    /// Source-relative path.
    pub path: String,
    /// Human-readable reason.
    pub reason: String,
    /// Source blocks that went down with it (the whole subtree).
    pub blocks_dropped: usize,
}

/// Input-vs-output reconciliation: what the source held, what the
/// pipeline emitted, and every legitimate reason the two differ.
///
/// Without it the report only counts what the pipeline *knows* it
/// produced — a block lost in the parse shows up in neither the
/// numerator nor the denominator, and the numbers look perfect.
#[derive(Debug, Serialize)]
pub struct Reconciliation {
    /// Pages in the source.
    pub source_pages: usize,
    /// Destination files written (`pages` + `journals`).
    pub emitted_pages: usize,
    /// Source pages folded into an already-emitted destination file.
    pub pages_merged: usize,
    /// Source pages refused outright (see [`ImportReport::skipped`]).
    pub pages_skipped: usize,
    /// `source − emitted − merged − skipped`: pages that vanished with
    /// no reported reason. Non-zero means the pipeline lost content.
    pub pages_unaccounted: usize,
    /// Blocks in the source (recursive).
    pub source_blocks: usize,
    /// Outline blocks emitted.
    pub emitted_blocks: usize,
    /// Blocks promoted to page properties instead of staying blocks.
    pub blocks_lifted_to_props: usize,
    /// Table cell blocks folded into their table's block.
    pub blocks_folded_into_tables: usize,
    /// Blocks that belonged to a skipped page.
    pub blocks_skipped: usize,
    /// `source − emitted − lifted − folded − skipped`: blocks that vanished with
    /// no reported reason. Non-zero means the pipeline lost content.
    pub blocks_unaccounted: usize,
    /// Blocks confirmed in the destination's op log after the import.
    /// Always `0` when [`Self::landing_measured`] is `false`.
    pub landed_blocks: usize,
    /// `emitted − landed`: blocks the renderer produced that never
    /// reached the op log (write failure, reconcile failure, a block the
    /// matcher dropped). Always `0` when the landing wasn't measured.
    pub blocks_not_landed: usize,
    /// Whether the landing numbers mean anything. `false` on a dry-run:
    /// nothing was written, so every count above stops at render.
    pub landing_measured: bool,
    /// True when every source page and block is accounted for **and**,
    /// on a real import, every emitted block is confirmed in the
    /// destination's op log. On a dry-run it only means "parser and
    /// renderer agree" — nothing was written to check against.
    pub balanced: bool,
}

/// One non-fatal fidelity warning.
#[derive(Debug, Serialize)]
pub struct ImportWarning {
    /// Page (title or date) the warning belongs to.
    pub page: String,
    /// What happened.
    pub detail: String,
}

impl ImportReport {
    /// Fresh report for the given adapter id.
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            ..Self::default()
        }
    }

    /// Record a non-fatal fidelity warning.
    pub fn warn(&mut self, page: &str, detail: impl Into<String>) {
        self.warnings.push(ImportWarning {
            page: page.to_string(),
            detail: detail.into(),
        });
    }

    /// Record a source file/page the import refused, together with how
    /// many source blocks went down with it.
    pub fn skip(&mut self, path: impl Into<String>, reason: impl Into<String>, blocks: usize) {
        self.skipped.push(SkippedFile {
            path: path.into(),
            reason: reason.into(),
            blocks_dropped: blocks,
        });
    }

    /// Close the books: compare what the source held against what the
    /// pipeline emitted, and what the pipeline emitted against what
    /// actually reached the destination's op log. Records the result in
    /// [`Self::reconciliation`].
    ///
    /// No-op for an adapter that doesn't report source counts (both
    /// `source_pages` and `source_blocks` still zero) — a fabricated
    /// "0/0, balanced" would be worse than saying nothing.
    pub fn finalize(&mut self) {
        if self.source_pages == 0 && self.source_blocks == 0 {
            return;
        }
        let pages_skipped = self.skipped.len();
        let blocks_skipped: usize = self.skipped.iter().map(|s| s.blocks_dropped).sum();
        let emitted_pages = self.pages + self.journals;
        let pages_accounted = emitted_pages + self.pages_merged + pages_skipped;
        let blocks_accounted = self.blocks
            + self.blocks_lifted_to_props
            + self.blocks_folded_into_tables
            + blocks_skipped;
        // Downstream half of the contract: the counters above stop at
        // render, so they can't see a page that failed to write, failed
        // to reconcile, or lost blocks in the matcher. Only measured on
        // a real import — a dry-run has no op log to check against.
        let blocks_not_landed = if self.landing_measured {
            self.blocks.saturating_sub(self.landed_blocks)
        } else {
            0
        };
        self.reconciliation = Some(Reconciliation {
            landed_blocks: self.landed_blocks,
            blocks_not_landed,
            landing_measured: self.landing_measured,
            source_pages: self.source_pages,
            emitted_pages,
            pages_merged: self.pages_merged,
            pages_skipped,
            pages_unaccounted: self.source_pages.saturating_sub(pages_accounted),
            source_blocks: self.source_blocks,
            emitted_blocks: self.blocks,
            blocks_lifted_to_props: self.blocks_lifted_to_props,
            blocks_folded_into_tables: self.blocks_folded_into_tables,
            blocks_skipped,
            blocks_unaccounted: self.source_blocks.saturating_sub(blocks_accounted),
            balanced: pages_accounted == self.source_pages
                && blocks_accounted == self.source_blocks
                && blocks_not_landed == 0,
        });
    }

    /// Bump a per-kind component counter.
    pub fn count_component(&mut self, kind: &str) {
        *self
            .components_verbatim
            .entry(kind.to_string())
            .or_insert(0) += 1;
    }

    /// Bump a per-state task counter.
    pub fn count_task(&mut self, state: &str) {
        *self.tasks.entry(state.to_string()).or_insert(0) += 1;
    }

    /// Pretty-print the report for humans.
    pub fn print_human(&self) {
        println!();
        println!("Import summary ({}):", self.source);
        println!("  pages:               {}", self.pages);
        println!("  journals:            {}", self.journals);
        println!("  blocks:              {}", self.blocks);
        println!(
            "  block refs:          {} resolved, {} page-fallback, {} unresolved",
            self.refs_resolved, self.refs_page_fallback, self.refs_unresolved
        );
        println!(
            "  embeds:              {} resolved, {} page-fallback",
            self.embeds_resolved, self.embeds_page_fallback
        );
        if !self.tasks.is_empty() {
            let states: Vec<String> = self
                .tasks
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            println!("  tasks:               {}", states.join(", "));
        }
        if self.tasks_midtext_literal > 0 {
            println!(
                "  mid-block tasks:     {} TODO/DONE markers became literal text (task state not preserved)",
                self.tasks_midtext_literal
            );
        }
        println!(
            "  properties:          {} page-level, {} block-level",
            self.props_pages, self.props_blocks
        );
        if self.queries_translated > 0 {
            println!("  queries translated:  {}", self.queries_translated);
        }
        if self.pdfs_linked > 0 {
            println!(
                "  pdf embeds:          {} became file links",
                self.pdfs_linked
            );
        }
        if !self.components_verbatim.is_empty() {
            let kinds: Vec<String> = self
                .components_verbatim
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect();
            println!("  components verbatim: {}", kinds.join(", "));
        }
        if self.tags_multiword_to_page_ref > 0 {
            println!(
                "  multi-word tags:     {} became [[page refs]]",
                self.tags_multiword_to_page_ref
            );
        }
        if self.tables_converted > 0 {
            println!(
                "  tables:              {} became pipe tables",
                self.tables_converted
            );
        }
        if self.org_dates_converted > 0 {
            println!(
                "  org dates:           {} became [[date]] links",
                self.org_dates_converted
            );
        }
        if self.collapsed_applied > 0 {
            println!("  collapsed blocks:    {}", self.collapsed_applied);
        }
        if self.timestamps_dropped > 0 {
            println!(
                "  timestamps dropped:  {} (re-run with --preserve-timestamps to keep)",
                self.timestamps_dropped
            );
        }
        if self.slug_collisions > 0 {
            println!("  slug collisions:     {}", self.slug_collisions);
        }
        if self.artifacts_stripped > 0 {
            println!(
                "  artifacts stripped:  {} (id:: lines, #+ directives, LOGBOOK drawers, dropped frontmatter keys)",
                self.artifacts_stripped
            );
        }
        if self.assets_copied > 0 {
            println!(
                "  assets copied:       {} ({} KiB)",
                self.assets_copied,
                self.assets_bytes / 1024
            );
        }
        if self.assets_remote_kept > 0 {
            println!(
                "  remote images kept:  {} (link left as written; --download-remote-assets pulls them)",
                self.assets_remote_kept
            );
        }
        if self.assets_missing > 0 {
            println!(
                "  assets missing:      {} (original link kept — file not found / download failed / too large)",
                self.assets_missing
            );
        }
        for s in &self.skipped {
            println!("  skipped: {} ({})", s.path, s.reason);
        }
        if self.replaced_blocks > 0 || self.replaced_props > 0 {
            println!(
                "  replaced:            {} blocks (moved to the trash), {} page properties (previous content of re-imported pages)",
                self.replaced_blocks, self.replaced_props
            );
        }
        self.print_reconciliation();
        self.print_warnings();
    }

    /// Print the warning list, capped, and say out loud how much was
    /// hidden. A 66k-block import can emit thousands of warnings; a
    /// silent cut at 20 makes the rest look like it doesn't exist.
    fn print_warnings(&self) {
        let total = self.warnings.len();
        if total == 0 {
            return;
        }
        let shown = total.min(WARNINGS_SHOWN);
        if total > shown {
            println!("  warnings:            {total} ({shown} shown below)");
        } else {
            println!("  warnings:            {total}");
        }
        for w in self.warnings.iter().take(shown) {
            println!("    [{}] {}", w.page, w.detail);
        }
        if total > shown {
            println!(
                "    … {} more warning(s) hidden — re-run the same command with `--json` to \
                 read all {total}",
                total - shown
            );
        }
    }

    /// Print the input-vs-output reconciliation, loudly when it doesn't
    /// close. Silent when the adapter reports no source counts.
    fn print_reconciliation(&self) {
        let Some(r) = &self.reconciliation else {
            return;
        };
        println!("  reconciliation:");
        println!(
            "    pages:             {}/{} emitted ({} merged, {} skipped)",
            r.emitted_pages, r.source_pages, r.pages_merged, r.pages_skipped
        );
        println!(
            "    blocks:            {}/{} emitted ({} lifted to page props, {} in skipped pages)",
            r.emitted_blocks, r.source_blocks, r.blocks_lifted_to_props, r.blocks_skipped
        );
        if r.blocks_folded_into_tables > 0 {
            println!(
                "                       {} table cells folded into their table block",
                r.blocks_folded_into_tables
            );
        }
        if r.landing_measured {
            println!(
                "    in the op log:     {}/{} emitted blocks confirmed on disk after reconcile",
                r.landed_blocks, r.emitted_blocks
            );
        } else {
            println!(
                "    in the op log:     not measured — a dry-run writes nothing, so every count \
                 above stops at render"
            );
        }
        if r.balanced {
            return;
        }
        if r.pages_unaccounted > 0 || r.blocks_unaccounted > 0 {
            println!();
            println!("  !!  UNACCOUNTED CONTENT — the import does not add up  !!");
            println!(
                "      {} page(s) and {} block(s) from the source are unaccounted for.",
                r.pages_unaccounted, r.blocks_unaccounted
            );
            println!(
                "      They were not emitted, merged, lifted, or skipped — this is silent loss."
            );
            println!("      Re-run with --json and open an issue with the numbers.");
            println!();
        }
        if r.blocks_not_landed > 0 {
            println!();
            println!("  !!  CONTENT NEVER REACHED THE OP LOG  !!");
            println!(
                "      {} of {} emitted block(s) are not in the destination's op log.",
                r.blocks_not_landed, r.emitted_blocks
            );
            println!(
                "      A page failed to write, failed to reconcile, or lost blocks in matching \
                 — the warnings above name which."
            );
            println!("      Re-run with --json for the per-page detail.");
            println!();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An adapter that reports no source counts gets no reconciliation —
    /// a fabricated "0/0, balanced" would be worse than saying nothing.
    #[test]
    fn finalize_is_a_no_op_without_source_counts() {
        let mut r = ImportReport::new("logseq");
        r.pages = 3;
        r.blocks = 40;
        r.finalize();
        assert!(r.reconciliation.is_none());
    }

    /// Blocks that vanished with no reported reason are exactly what the
    /// reconciliation exists to surface.
    #[test]
    fn unexplained_loss_breaks_the_balance() {
        let mut r = ImportReport::new("roam");
        r.source_pages = 2;
        r.source_blocks = 10;
        r.pages = 2;
        r.blocks = 7; // three blocks went missing on the way out
        r.finalize();
        let rec = r.reconciliation.expect("source counts present");
        assert_eq!(rec.blocks_unaccounted, 3);
        assert_eq!(rec.pages_unaccounted, 0);
        assert!(!rec.balanced);
    }

    /// Named reducers close the books instead of raising a false alarm.
    #[test]
    fn named_reducers_keep_the_books_closed() {
        let mut r = ImportReport::new("roam");
        r.source_pages = 4;
        r.source_blocks = 10;
        r.pages = 1;
        r.journals = 1;
        r.pages_merged = 1;
        r.skip("backup.json#page[0]", "page title is empty", 3);
        r.blocks = 6;
        r.blocks_lifted_to_props = 1;
        r.finalize();
        let rec = r.reconciliation.expect("source counts present");
        assert_eq!(rec.pages_skipped, 1);
        assert_eq!(rec.blocks_skipped, 3);
        assert_eq!(rec.blocks_unaccounted, 0);
        assert!(rec.balanced, "{rec:?}");
    }

    /// Parse → render adding up says nothing about the write. Blocks the
    /// renderer produced that never reached the op log have to break the
    /// balance too, or `balanced: true` keeps promising something it
    /// never measured.
    #[test]
    fn blocks_that_never_landed_break_the_balance() {
        let mut r = ImportReport::new("roam");
        r.source_pages = 1;
        r.source_blocks = 10;
        r.pages = 1;
        r.blocks = 10; // the renderer emitted everything…
        r.landing_measured = true;
        r.landed_blocks = 7; // …but only 7 are in the op log
        r.finalize();
        let rec = r.reconciliation.expect("source counts present");
        assert_eq!(rec.blocks_unaccounted, 0, "parse → render is clean");
        assert_eq!(rec.blocks_not_landed, 3);
        assert!(!rec.balanced, "{rec:?}");
    }

    /// A dry-run writes nothing, so it must report the landing as
    /// unmeasured instead of as zero-loss — and its balance stays about
    /// what it can actually check.
    #[test]
    fn an_unmeasured_landing_never_invents_a_gap() {
        let mut r = ImportReport::new("roam");
        r.source_pages = 1;
        r.source_blocks = 10;
        r.pages = 1;
        r.blocks = 10;
        r.finalize();
        let rec = r.reconciliation.expect("source counts present");
        assert!(!rec.landing_measured);
        assert_eq!(rec.landed_blocks, 0);
        assert_eq!(rec.blocks_not_landed, 0, "no measurement, no accusation");
        assert!(rec.balanced, "{rec:?}");
    }
}
