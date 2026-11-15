//! `/merge <page>`: fold the open page into another page.
//!
//! What a merge *is* (blocks, properties, aliases, the link rewrite, the
//! source going to the trash) belongs to `outl_actions::merge_pages`
//! (RFC 0557). This module is the three things around it the TUI has to
//! do itself: ask first, project what the merge touched, and land the
//! user somewhere that exists.
//!
//! **Asks first, on the delete prompt.** A merge cannot be undone from
//! this client: `u` restores a page snapshot, and a snapshot of the
//! target taken before the merge would render the moved blocks away. So
//! it reuses the one-shot `y/n` the page delete already has
//! (`PendingSidebarDelete`, with `merge_into` set) rather than growing
//! a second modal, and it **drops** the undo and redo entries for every
//! page the merge touched instead of leaving them to bite.
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
//! **No rename path.** The TUI has no page rename (nothing in this crate
//! calls `rename_page`), so there is no rename-onto-an-existing-name
//! refusal to turn into a prompt. When a rename lands, `PageExists` is the
//! signal to arm this same confirmation, and `rename_or_merge` the call
//! behind its `y`.

use anyhow::Result;
use chrono::NaiveDate;

use outl_actions::refusal::Surface;
use outl_actions::{
    commit_page, remove_page_projection, ActionError, MergeReport, ProjectionRefusal,
};

use crate::actions::lifecycle::capture::CaptureCommit;
use crate::state::{App, HistorySnapshot, PendingSidebarDelete, ToastKind};

/// What projecting a merge's touched pages came to.
struct Projected {
    /// The target's own `.md` was written, so the source's may go.
    target_ok: bool,
    /// The first page that would not project, with why.
    failure: Option<(String, ActionError)>,
}

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
        self.pending_sidebar_delete = Some(PendingSidebarDelete {
            slug,
            title,
            merge_into: Some(target_slug),
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

        let projected = self.project_merge(&report);
        if projected.target_ok {
            if let Err(e) = remove_page_projection(&self.workspace_root, &report.source) {
                tracing::warn!(
                    target: "outl::tui::merge",
                    "could not remove merged page projection for {}: {e}",
                    report.source.slug
                );
            }
        }
        self.forget_history_of(&report);
        self.spawn_index_rebuild();
        self.spawn_backlink_index_rebuild();
        if let Some(transport) = &self.sync_transport {
            // The commits announced the pages that gained content; the
            // source's move to the trash is what they cannot cover.
            transport.announce_local_ops(&report.source.slug, self.hlc.next());
        }
        self.land_after_merge(&report, was_current)?;

        let kind = if report.property_conflicts.is_empty() {
            ToastKind::Info
        } else {
            ToastKind::Warning
        };
        self.toast(kind, merge_summary(&report));
        if let Some((slug, e)) = projected.failure {
            self.report_merge_projection_failure(&slug, e);
        }
        self.status.clear();
        Ok(())
    }

    /// Slug and display title of the live page a typed name refers to.
    ///
    /// The literal slug first, then the slugified name, which is the
    /// order `open_page_by_name` and the CLI's resolver agree on.
    fn page_label(&self, name: &str) -> Option<(String, String)> {
        let by_slug = |slug: &str| outl_actions::find_by_slug(&self.workspace, slug);
        let id = by_slug(name).or_else(|| by_slug(&outl_md::slug::slugify(name)))?;
        let meta = outl_actions::page_meta(&self.workspace, id)?;
        let title = if meta.title.trim().is_empty() {
            meta.slug.clone()
        } else {
            meta.title.clone()
        };
        Some((meta.slug, title))
    }

    /// `commit_page` on every page the merge touched.
    ///
    /// The mutation already ran, so each commit is empty and exists for
    /// the steps around it: the guarded projection (root invariant 8),
    /// the backlink invalidation and the peer announce. The CLI does the
    /// same in `page_merge::finish`.
    fn project_merge(&mut self, report: &MergeReport) -> Projected {
        let mut hooks = CaptureCommit {
            root: &self.workspace_root,
            hlc: &self.hlc,
            transport: self.sync_transport.as_ref(),
            backlinks_stale: false,
            projection_failure: None,
        };
        let mut out = Projected {
            target_ok: false,
            failure: None,
        };
        for slug in &report.touched_pages {
            let Some(page) = outl_actions::find_by_slug(&self.workspace, slug) else {
                continue;
            };
            let failed = match commit_page(&mut self.workspace, &mut hooks, page, |_| Ok(())) {
                Err(e) => Some(e),
                Ok(()) => hooks.projection_failure.take(),
            };
            match failed {
                None => out.target_ok |= *slug == report.target.slug,
                Some(e) => {
                    out.failure.get_or_insert((slug.clone(), e));
                }
            }
        }
        out
    }

    /// Drop the undo / redo entries of every page the merge changed.
    ///
    /// A snapshot of the target taken before the merge would, restored,
    /// render the merged blocks away and reconcile that as a delete.
    /// The source's own entries point at a file that no longer exists.
    fn forget_history_of(&mut self, report: &MergeReport) {
        let stale = |snap: &HistorySnapshot| {
            snap.view_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| {
                    stem == report.source.slug.as_str()
                        || report.touched_pages.iter().any(|t| t.as_str() == stem)
                })
        };
        self.undo.retain(|snap| !stale(snap));
        self.redo.retain(|snap| !stale(snap));
    }

    /// Leave the user on the target when they were on the source, and
    /// re-read the view otherwise.
    ///
    /// Never opens the target by slug: `open_page_by_slug` reads a
    /// missing `.md` as "new page" and seeds one, so a target whose
    /// projection was withheld would be re-created empty. A page with no
    /// file on disk sends the user to today instead.
    fn land_after_merge(&mut self, report: &MergeReport, was_current: bool) -> Result<()> {
        if !was_current {
            self.load_current();
            return Ok(());
        }
        let path = outl_actions::page_md_path(&self.workspace_root, &report.target);
        if path.exists() {
            self.open_path(path)
        } else {
            self.go_today()
        }
    }

    /// Report a withheld projection, classified like the trash restore's.
    ///
    /// Unlike a restore, the user just asked for this, so even a
    /// retryable failure is said once: the source's `.md` stays on disk
    /// in that case and they would otherwise see two files for one page.
    fn report_merge_projection_failure(&mut self, slug: &str, e: ActionError) {
        match ProjectionRefusal::from_error(&e) {
            ProjectionRefusal::Retryable(message) => {
                tracing::warn!("merge: re-projection of {slug} failed: {message}");
                self.toast(
                    ToastKind::Warning,
                    format!("merged, but `{slug}` could not be written yet"),
                );
            }
            ProjectionRefusal::AheadOfLog { lines, sample, .. } => {
                let notice =
                    outl_actions::refusal::ahead_of_log_notice(Surface::Tui, lines, &sample);
                self.toast(
                    ToastKind::Warning,
                    format!("`{slug}`: {}", notice.one_line()),
                );
            }
        }
    }
}

/// One line for the toast: what moved, what was respelled, what clashed.
///
/// Only the nonzero parts, so the common merge reads `merged 'a' into
/// 'b': 3 blocks`.
fn merge_summary(report: &MergeReport) -> String {
    let head = format!(
        "merged '{}' into '{}'",
        report.source.title, report.target.title
    );
    if report.nothing_changed() {
        return format!("{head}: nothing left to move");
    }
    let mut parts = Vec::new();
    if report.moved_blocks > 0 {
        parts.push(count(report.moved_blocks, "block"));
    }
    if !report.rewritten_blocks.is_empty() {
        parts.push(format!(
            "links updated in {}",
            count(report.rewritten_blocks.len(), "block")
        ));
    }
    if !report.skipped_namespace_refs.is_empty() {
        parts.push(format!(
            "{} left alone",
            count(report.skipped_namespace_refs.len(), "sub-page link")
        ));
    }
    if !report.property_conflicts.is_empty() {
        parts.push(format!(
            "{} (target's kept)",
            count(report.property_conflicts.len(), "property conflict")
        ));
    }
    if parts.is_empty() {
        head
    } else {
        format!("{head}: {}", parts.join(", "))
    }
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests;
