//! The human-readable import summary.
//!
//! Separate from the counters because the two change for different
//! reasons: a new counter is a fidelity decision, a new line here is a
//! wording one. The rule that matters in this file is that **a number
//! the user cannot interpret is worse than no number** — `8/8 emitted`
//! with two lifted template pages, or `blocks: 0` after importing a
//! whole vault, both read as something going wrong when nothing did.

use super::ImportReport;

/// How many individual warnings [`ImportReport::print_human`] spells
/// out before summarising the rest. `--json` always carries all of them.
const WARNINGS_SHOWN: usize = 20;

impl ImportReport {
    /// Pretty-print the report for humans.
    pub fn print_human(&self) {
        println!();
        println!("Import summary ({}):", self.source);
        println!("  pages:               {}", self.pages);
        println!("  journals:            {}", self.journals);
        // A free-markdown source has no block structure before outl's
        // parser runs, so the renderer emits text and counts no blocks.
        // Printing `blocks: 0` after importing someone's whole vault
        // reads as "none of my content arrived"; the sidecars written
        // from the materialized tree know the real number.
        if self.blocks == 0 && self.landed_blocks > 0 {
            println!(
                "  blocks:              {} (counted in the op log — this source is free \
                 markdown, so the blocks are the ones outl's own parser made)",
                self.landed_blocks
            );
        } else {
            println!("  blocks:              {}", self.blocks);
        }
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
        if self.pages_added_as_templates > 0 {
            println!(
                "  templates lifted:    {} block template(s) became template pages ({} blocks copied)",
                self.pages_added_as_templates, self.blocks_copied_into_templates
            );
        }
        if self.org_blocks_converted > 0 {
            println!(
                "  org blocks:          {} #+BEGIN_ blocks became fences / quotes",
                self.org_blocks_converted
            );
        }
        if self.blocks_from_pre_text > 0 {
            println!(
                "  pre-bullet text:     {} page(s) had text before the first bullet, kept as a block",
                self.blocks_from_pre_text
            );
        }
        if self.refs_in_prop_values > 0 {
            println!(
                "  refs in properties:  {} resolved inside property values",
                self.refs_in_prop_values
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
        let mut page_notes = vec![
            format!("{} merged", r.pages_merged),
            format!("{} skipped", r.pages_skipped),
        ];
        if r.pages_added_as_templates > 0 {
            // Without this the line reads as a coincidence: the emitted
            // count includes pages the import added, so `8/8` with two
            // lifted templates means six source pages arrived.
            page_notes.push(format!(
                "{} added as template pages",
                r.pages_added_as_templates
            ));
        }
        println!(
            "    pages:             {}/{} emitted ({})",
            r.emitted_pages,
            r.source_pages,
            page_notes.join(", ")
        );
        if r.source_blocks_measured {
            let mut block_notes = vec![
                format!("{} lifted to page props", r.blocks_lifted_to_props),
                format!("{} in skipped pages", r.blocks_skipped),
            ];
            if r.blocks_copied_into_templates > 0 {
                block_notes.push(format!(
                    "{} copied into template pages",
                    r.blocks_copied_into_templates
                ));
            }
            println!(
                "    blocks:            {}/{} emitted ({})",
                r.emitted_blocks,
                r.source_blocks,
                block_notes.join(", ")
            );
        } else {
            println!(
                "    blocks:            not compared — this source is free markdown, where the \
                 page is the unit of content, so there is no source block count to compare \
                 the op log against"
            );
        }
        if r.blocks_folded_into_tables > 0 {
            println!(
                "                       {} table cells folded into their table block",
                r.blocks_folded_into_tables
            );
        }
        if r.landing_measured && r.emitted_blocks == 0 {
            println!(
                "    in the op log:     {} block(s) confirmed on disk after reconcile",
                r.landed_blocks
            );
        } else if r.landing_measured {
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
