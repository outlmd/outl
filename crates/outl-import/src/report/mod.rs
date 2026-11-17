//! Import report — the fidelity contract with the user.
//!
//! Two concerns, one per module: **what is counted** (here — the
//! struct, the counters, and `finalize`'s input-vs-output balance) and
//! **how it is shown** ([`print`] — the human summary). They split
//! because the second one grew past the point where a reader looking
//! for "which reducer closes the books" had to scroll through
//! `println!`s to find it.
//!
//! Every translation, fallback, and drop the pipeline performs is
//! counted here. The report doubles as the acceptance test for a
//! migration: run `--dry-run --json` against a real backup and the
//! numbers say exactly what would survive.

use serde::Serialize;
use std::collections::BTreeMap;

/// How many individual warnings [`ImportReport::print_human`] spells
/// out before summarising the rest. `--json` always carries all of them.
mod print;
#[cfg(test)]
mod tests;

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
    /// `{{embed}}` of a whole page translated to `![[Page]]`, outl's own
    /// page embed (issue #190).
    ///
    /// Not a loss counter. It used to be (`embeds_page_fallback`, a
    /// `[[Page]]` link plus a per-occurrence warning) because outl had no
    /// page embed to translate into; it has one now, so the construct
    /// survives and this only says how many arrived.
    pub embeds_page: usize,
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
    /// Template pages the import *added*, which no source page
    /// corresponds to: a Logseq block template is a block, an outl
    /// template is a page, so the lift copies one into the other
    /// (issue #563).
    ///
    /// The mirror image of [`Self::pages_merged`]: it goes **up**
    /// rather than down, so `finalize` subtracts it from the emitted
    /// count instead of adding it to the source's. Without that the
    /// reconciliation would read every lifted template as a page that
    /// appeared out of nowhere.
    pub pages_added_as_templates: usize,
    /// Blocks copied into those template pages — same accounting as
    /// [`Self::pages_added_as_templates`], one level down.
    pub blocks_copied_into_templates: usize,
    /// Pages present in the source, counted straight off the parsed
    /// source structure — the denominator of the reconciliation.
    /// `0` when the adapter doesn't report source counts.
    pub source_pages: usize,
    /// Blocks present in the source, counted recursively off the parsed
    /// source structure — the denominator of the reconciliation.
    pub source_blocks: usize,
    /// Whether [`Self::source_blocks`] is a measurement.
    ///
    /// A free-markdown source (an Obsidian vault) has no block
    /// structure before outl's own parser runs: the **page** is its
    /// unit of content. Reporting `0` as its block denominator would
    /// read as "every block vanished", so such an adapter reconciles
    /// pages only and leaves this `false`. An adapter that does count
    /// sets it, and a graph that genuinely holds no blocks then
    /// reconciles as `0/0` instead of going unmeasured.
    pub source_blocks_measured: bool,
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
    /// Pages whose `.md` the import rewrote at the end so the bytes on
    /// disk are exactly what the op log projects.
    ///
    /// Not a loss and not printed in the human summary: the content is
    /// identical either way, and the count only says how many files the
    /// writer spelled differently from the projection. It is in `--json`
    /// because a sudden jump here means the two drifted apart again.
    pub pages_normalized: usize,
    /// Blocks made from the free text a Logseq page carries before its
    /// first bullet (Logseq's "pre-block"). Those lines used to be
    /// dropped one by one, silently.
    pub blocks_from_pre_text: usize,
    /// `((uid))` refs found inside a **property value**, which never
    /// reaches the inline tokenizer. Counted because they used to be
    /// written out verbatim and stay inert text.
    pub refs_in_prop_values: usize,
    /// `#+BEGIN_SRC` / `#+BEGIN_QUOTE` org blocks translated to a
    /// markdown fence or `> ` lines.
    pub org_blocks_converted: usize,
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
    /// Pages the import added that no source page corresponds to
    /// (Logseq block templates lifted into outl template pages).
    pub pages_added_as_templates: usize,
    /// Blocks copied into those added pages.
    pub blocks_copied_into_templates: usize,
    /// Source pages refused outright (see [`ImportReport::skipped`]).
    pub pages_skipped: usize,
    /// `source − emitted − merged − skipped`: pages that vanished with
    /// no reported reason. Non-zero means the pipeline lost content.
    pub pages_unaccounted: usize,
    /// Blocks in the source (recursive). Only meaningful when
    /// [`Self::source_blocks_measured`] is `true`.
    pub source_blocks: usize,
    /// Whether the source's block count was measured at all.
    pub source_blocks_measured: bool,
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

    /// Record the source's own counts — the reconciliation's
    /// denominators, counted off the parsed source before any pipeline
    /// decision can hide a loss from both sides.
    ///
    /// Calling this is what marks the block denominator as *measured*,
    /// so an adapter that counts zero blocks in an empty graph is told
    /// apart from one that cannot count blocks at all (see
    /// [`Self::source_blocks_measured`]). A free-markdown adapter calls
    /// [`Self::count_source_pages`] instead.
    pub fn count_source(&mut self, pages: usize, blocks: usize) {
        self.source_pages += pages;
        self.source_blocks += blocks;
        self.source_blocks_measured = true;
    }

    /// Record source pages for an adapter whose source has no block
    /// structure to count (free markdown — the page is the unit).
    pub fn count_source_pages(&mut self, pages: usize) {
        self.source_pages += pages;
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
        if self.source_pages == 0 && self.source_blocks == 0 && !self.source_blocks_measured {
            return;
        }
        let pages_skipped = self.skipped.len();
        let blocks_skipped: usize = self.skipped.iter().map(|s| s.blocks_dropped).sum();
        let emitted_pages = self.pages + self.journals;
        // Pages and blocks the import *added* (a Logseq block template
        // copied into an outl template page) have no source counterpart,
        // so they come off the emitted side before the comparison. Left
        // in, every lift would read as a page that appeared out of
        // nowhere and the books would stop closing.
        let pages_accounted = emitted_pages.saturating_sub(self.pages_added_as_templates)
            + self.pages_merged
            + pages_skipped;
        let blocks_accounted = self
            .blocks
            .saturating_sub(self.blocks_copied_into_templates)
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
        // An unmeasured block denominator cannot accuse anyone: the
        // page half of the balance still has to close, and so does the
        // landing, which is measured downstream of the write either way.
        let blocks_balanced =
            !self.source_blocks_measured || blocks_accounted == self.source_blocks;
        let blocks_unaccounted = if self.source_blocks_measured {
            self.source_blocks.saturating_sub(blocks_accounted)
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
            pages_added_as_templates: self.pages_added_as_templates,
            blocks_copied_into_templates: self.blocks_copied_into_templates,
            pages_skipped,
            pages_unaccounted: self.source_pages.saturating_sub(pages_accounted),
            source_blocks: self.source_blocks,
            source_blocks_measured: self.source_blocks_measured,
            emitted_blocks: self.blocks,
            blocks_lifted_to_props: self.blocks_lifted_to_props,
            blocks_folded_into_tables: self.blocks_folded_into_tables,
            blocks_skipped,
            blocks_unaccounted,
            balanced: pages_accounted == self.source_pages
                && blocks_balanced
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
}
