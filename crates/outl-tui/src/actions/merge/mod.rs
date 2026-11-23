//! `/merge <page>`: fold the open page into another page.
//!
//! What a merge *is* (blocks, properties, aliases, the link rewrite, the
//! source going to the trash) belongs to `outl_actions::merge_pages`
//! (RFC 0557). This module is the two things around it that are the
//! merge's own: ask first, and land the user somewhere that exists. The
//! tail every page-level action shares — the projection order, the
//! invalidated history, the remembered paths, the wording — lives in
//! `actions/page_commit`, which `/rename` drives too.
//!
//! **Asks first, on the delete prompt.** A merge cannot be undone from
//! this client: `u` restores a page snapshot, and a snapshot of the
//! target taken before the merge would render the moved blocks away. So
//! it reuses the one-shot `y/n` the page delete already has
//! (`PageAction::MergeInto`) rather than growing a second modal, and it
//! **drops** the undo and redo entries for every page the merge touched
//! instead of leaving them to bite.
//!
//! **Opened from the command registry, not a chord**, for the reason the
//! trash gives (`actions/trash`): a chord needs an `Action` variant and
//! the three verdicts that come with it. The verdicts are recorded where
//! they belong, in `Capability::PageMerge`.
//!
//! **Projection order is the CLI's.** The source's `.md` is removed only
//! once the target itself projected, so a refused write never costs the
//! user the only file that still holds the blocks.
//!
//! **The rename shares the prompt.** `/rename` onto a name another page
//! owns arms this same confirmation with `PageAction::RenameCollision`
//! and `outl_actions::rename_or_merge` behind its `y`, which lands back
//! in [`App::finish_merge`] — see `actions/page_rename`.

use anyhow::Result;
use chrono::NaiveDate;

use outl_actions::{remove_page_projection, MergeReport};

use crate::actions::page_commit::merge_summary;
use crate::pending_page_action::{PageAction, PendingPageAction};
use crate::state::{App, ToastKind};

impl App {
    /// `/merge <page>`: resolve the target, then arm the `y/n` prompt.
    ///
    /// Nothing is written here. Every refusal is a sentence on the
    /// status line or a toast, never a silent no-op.
    pub(crate) fn arm_page_merge(&mut self, target: &str) {
        let target = target.trim();
        if target.is_empty() {
            self.status = "usage: merge <page name>".into();
            return;
        }
        let slug = self.current_slug();
        // Same exclusion as `delete_page_from_chord`: a journal day is
        // identified by its date, so `merge_pages` refuses it too.
        if slug.is_empty() || NaiveDate::parse_from_str(&slug, "%Y-%m-%d").is_ok() {
            self.toast(ToastKind::Warning, "can't merge a journal page");
            return;
        }
        let Some((target_slug, target_title)) = self.page_label(target) else {
            self.status = format!("no page '{target}' to merge into");
            return;
        };
        if target_slug == slug {
            self.status = "can't merge a page into itself".into();
            return;
        }
        let title = self
            .page_label(&slug)
            .map(|(_, title)| title)
            .unwrap_or_else(|| slug.clone());
        self.status = format!("merge '{title}' into '{target_title}'? can't be undone here. y/n");
        self.pending_page_action = Some(PendingPageAction {
            slug,
            title,
            action: PageAction::MergeInto {
                target: target_slug,
            },
        });
    }

    /// Resolve an armed merge: run it, project it, land the user on the
    /// target and say what happened.
    ///
    /// Returns `Err` only for a failure the caller cannot ignore;
    /// a refused merge is a toast.
    pub(crate) fn confirm_page_merge(&mut self, source: &str, target: &str) -> Result<()> {
        // The merge reads the op log, and the page on screen may hold an
        // edit that has not reached it yet.
        self.flush_pending_save();
        let was_current = self.current_slug() == source;

        let merged = outl_actions::merge_pages(&mut self.workspace, &self.hlc, source, target);
        let report = match merged {
            Ok(report) => report,
            Err(e) => {
                self.toast(ToastKind::Error, format!("merge failed: {e}"));
                self.status.clear();
                return Ok(());
            }
        };
        self.finish_merge(&report, was_current)
    }

    /// Everything after a merge's ops are in the log.
    ///
    /// Shared with `/rename`'s collision path, which reaches a merge
    /// through `rename_or_merge` and owes exactly this tail.
    pub(crate) fn finish_merge(&mut self, report: &MergeReport, was_current: bool) -> Result<()> {
        let projected = self.project_touched(&report.touched_pages, &report.target.slug);
        if projected.landing_ok {
            if let Err(e) = remove_page_projection(&self.workspace_root, &report.source) {
                tracing::warn!(
                    target: "outl::tui::merge",
                    "could not remove merged page projection for {}: {e}",
                    report.source.slug
                );
            }
        }
        // The source page is gone, so the sidebar's Recent row and any
        // persisted search hit pointing at its `.md` have nowhere to go.
        // Opening one re-creates the file and reconciles it back into a
        // page holding a single empty bullet.
        let gone = outl_actions::page_md_path(&self.workspace_root, &report.source);
        self.repoint_remembered_path(&gone, None);
        let mut stale_history: Vec<&str> =
            report.touched_pages.iter().map(String::as_str).collect();
        stale_history.push(&report.source.slug);
        self.forget_history_of_pages(&stale_history);
        self.spawn_index_rebuild();
        self.spawn_backlink_index_rebuild();
        if let Some(transport) = &self.sync_transport {
            // The commits announced the pages that gained content; the
            // source's move to the trash is what they cannot cover.
            transport.announce_local_ops(&report.source.slug, self.hlc.next());
        }
        if was_current {
            self.land_on_page_or_today(&report.target)?;
        } else {
            self.load_current();
        }

        let kind = if report.property_conflicts.is_empty() {
            ToastKind::Info
        } else {
            ToastKind::Warning
        };
        self.toast(kind, merge_summary(report));
        if let Some((slug, e)) = projected.failure {
            self.report_projection_failure("merged", &slug, e);
        }
        self.status.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
