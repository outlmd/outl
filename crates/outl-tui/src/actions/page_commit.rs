//! The tail a page-level action owes once the op log holds it.
//!
//! `outl_actions::merge_pages` and `outl_actions::rename_page` both
//! dirty more than one page and both leave a `.md` behind that has
//! stopped being the page's file, so neither projects (RFC 0557). The
//! caller drives that, and `/merge` and `/rename` would otherwise drive
//! it twice — which is the drift to avoid, because the half that is
//! easy to get wrong is an ordering rule rather than a call.
//!
//! **The ordering is root `CLAUDE.md` invariant 8's.** Every touched
//! page is projected first, and the stale `.md` is removed **last and
//! only when the landing page's own projection went through** — so a
//! withheld write never costs the user the only file still holding the
//! page. A crash in between leaves two copies, never none.
//!
//! What lives here is what the two actions cannot disagree about:
//! the projection loop, which undo entries the action invalidated, how a
//! withheld projection reaches the user, where the user lands, the file
//! paths this client remembers by name, and the wording of both
//! summaries. `merge_summary` and `rename_summary` sit side by side on
//! purpose: the TUI's phrasing of these two reports is one fact with two
//! spellings, and the workspace already carries three copies of it (here,
//! `outl-cli`, `@outl/shared/pages/rename-merge`).

use std::path::Path;

use anyhow::Result;

use outl_actions::page_summary::{rewrite_parts, BLOCK, PROPERTY_CONFLICT};
use outl_actions::refusal::Surface;
use outl_actions::{
    commit_page, ActionError, MergeReport, PageMeta, ProjectionRefusal, RenameReport,
};

use crate::actions::lifecycle::capture::CaptureCommit;
use crate::state::{App, HistorySnapshot, ToastKind};

/// What projecting the pages an action touched came to.
pub(crate) struct Projected {
    /// The landing page's own `.md` was written, so a stale one may go.
    pub(crate) landing_ok: bool,
    /// The first page that would not project, with why.
    pub(crate) failure: Option<(String, ActionError)>,
}

impl App {
    /// `commit_page` on every page an action touched.
    ///
    /// The mutation already ran, so each commit is empty and exists for
    /// the steps around it: the guarded projection (root invariant 8),
    /// the backlink invalidation and the peer announce. The CLI does the
    /// same in `page_merge::finish`.
    ///
    /// `landing` is the slug the user ends up on — the merge's target,
    /// the renamed page under its new name. Its verdict is the one that
    /// licenses removing the stale projection, so it is tracked
    /// separately from the first failure rather than inferred from it.
    pub(crate) fn project_touched(&mut self, touched: &[String], landing: &str) -> Projected {
        let mut hooks = CaptureCommit {
            root: &self.workspace_root,
            hlc: &self.hlc,
            transport: self.sync_transport.as_ref(),
            backlinks_stale: false,
            projection_failure: None,
        };
        let mut out = Projected {
            landing_ok: false,
            failure: None,
        };
        for slug in touched {
            let Some(page) = outl_actions::find_by_slug(&self.workspace, slug) else {
                continue;
            };
            let failed = match commit_page(&mut self.workspace, &mut hooks, page, |_| Ok(())) {
                Err(e) => Some(e),
                Ok(()) => hooks.projection_failure.take(),
            };
            match failed {
                None => out.landing_ok |= slug == landing,
                Some(e) => {
                    out.failure.get_or_insert((slug.clone(), e));
                }
            }
        }
        out
    }

    /// Drop the undo / redo entries belonging to any of `stems`.
    ///
    /// [`HistorySnapshot`] is keyed by `view_path`, and
    /// `restore_snapshot` refuses one from another view — so a stale
    /// entry is inert *until the old path comes back*, at which point
    /// restoring it writes a pre-action render to a file that
    /// reconciles the action away. A snapshot of a merge target taken
    /// before the merge renders the moved blocks out; a snapshot under
    /// a renamed page's old slug recreates `pages/<old>.md`.
    pub(crate) fn forget_history_of_pages(&mut self, stems: &[&str]) {
        let stale = |snap: &HistorySnapshot| {
            snap.view_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stems.contains(&stem))
        };
        self.undo.retain(|snap| !stale(snap));
        self.redo.retain(|snap| !stale(snap));
    }

    /// Repoint every `.md` path this client remembers by name, or
    /// forget it when `to` is `None`.
    ///
    /// The sidebar's Recent rows open by path and the persisted search
    /// hits carry one, so after a rename both hold a file that is no
    /// longer the page's. Opening it is not a dead end: `load_current`
    /// reads a missing `.md` as an empty page and the next save writes
    /// it back, which reconciles `page-slug = <old>` onto the renamed
    /// page and undoes the rename (RFC 0557 known limit 5). The merged
    /// source's file has the same shape with no page to repoint to, so
    /// it is forgotten instead.
    pub(crate) fn repoint_remembered_path(&mut self, from: &Path, to: Option<&Path>) {
        match to {
            Some(to) => {
                for path in &mut self.recent_paths {
                    if path == from {
                        *path = to.to_path_buf();
                    }
                }
            }
            None => self.recent_paths.retain(|path| path != from),
        }
        // A repoint can collide with an entry the LRU already holds
        // (the user visited both names); keep the first.
        let mut seen = Vec::with_capacity(self.recent_paths.len());
        self.recent_paths.retain(|path| {
            let fresh = !seen.contains(path);
            if fresh {
                seen.push(path.clone());
            }
            fresh
        });
        let Some(last) = self.last_search.as_mut() else {
            return;
        };
        match to {
            Some(to) => {
                for hit in &mut last.hits {
                    if hit.md_path == from {
                        hit.md_path = to.to_path_buf();
                    }
                }
            }
            None => last.hits.retain(|hit| hit.md_path != from),
        }
        if last.hits.is_empty() {
            self.last_search = None;
        } else if last.cursor >= last.hits.len() {
            last.cursor = last.hits.len() - 1;
        }
    }

    /// Open `meta`'s `.md` when it is really on disk, else land on today.
    ///
    /// **Never opens the page by slug**: `open_page_by_slug` reads a
    /// missing `.md` as "new page" and seeds one, so a page whose
    /// projection was withheld would be re-created empty on the very
    /// path that was refused for holding content the log never saw.
    pub(crate) fn land_on_page_or_today(&mut self, meta: &PageMeta) -> Result<()> {
        let path = outl_actions::page_md_path(&self.workspace_root, meta);
        if path.exists() {
            self.open_path(path)
        } else {
            self.go_today()
        }
    }

    /// Slug and display title of the live page a typed name refers to.
    ///
    /// The literal slug first, then the slugified name, which is the
    /// order `open_page_by_name` and the CLI's resolver agree on.
    pub(crate) fn page_label(&self, name: &str) -> Option<(String, String)> {
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

    /// Report a withheld projection, classified like the trash restore's.
    ///
    /// Unlike a restore, the user just asked for this, so even a
    /// retryable failure is said once: the stale `.md` stays on disk in
    /// that case and they would otherwise see two files for one page.
    /// `verb` is the action in the past tense (`merged`, `renamed`) —
    /// every other word belongs to `outl_actions::refusal`.
    pub(crate) fn report_projection_failure(&mut self, verb: &str, slug: &str, e: ActionError) {
        match ProjectionRefusal::from_error(&e) {
            ProjectionRefusal::Retryable(message) => {
                tracing::warn!("{verb}: re-projection of {slug} failed: {message}");
                self.toast(
                    ToastKind::Warning,
                    format!("{verb}, but `{slug}` could not be written yet"),
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
pub(crate) fn merge_summary(report: &MergeReport) -> String {
    let head = format!(
        "merged '{}' into '{}'",
        report.source.title, report.target.title
    );
    if report.nothing_changed() {
        return format!("{head}: nothing left to move");
    }
    let mut parts = Vec::new();
    if report.moved_blocks > 0 {
        parts.push(BLOCK.count(report.moved_blocks));
    }
    parts.extend(rewrite_parts(
        report.rewritten_blocks.len(),
        report.skipped_namespace_refs.len(),
    ));
    if !report.property_conflicts.is_empty() {
        parts.push(format!(
            "{} (target's kept)",
            PROPERTY_CONFLICT.count(report.property_conflicts.len())
        ));
    }
    join(head, parts)
}

/// The same line for a rename: the names, then what the link rewrite did.
///
/// The sub-page count is not optional chrome. `rename_page` leaves
/// `[[old/child]]` alone on purpose — it names a page that is not being
/// renamed — and a summary that drops the number leaves the user with
/// mentions they cannot see were not updated.
pub(crate) fn rename_summary(report: &RenameReport) -> String {
    let head = format!("renamed '{}' to '{}'", report.old_title, report.meta.title);
    if report.touched_pages.is_empty() {
        return format!("already called '{}'", report.meta.title);
    }
    let parts = rewrite_parts(
        report.rewritten_blocks.len(),
        report.skipped_namespace_refs.len(),
    );
    join(head, parts)
}

fn join(head: String, parts: Vec<String>) -> String {
    if parts.is_empty() {
        head
    } else {
        format!("{head}: {}", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::join;

    /// The nouns and their plurals belong to
    /// `outl_actions::page_summary`, which is where they are tested —
    /// this file owns only the shape of the one line a toast gets.
    #[test]
    fn the_summary_says_only_what_happened() {
        assert_eq!(join("renamed".into(), Vec::new()), "renamed");
        assert_eq!(
            join("renamed".into(), vec!["a".into(), "b".into()]),
            "renamed: a, b"
        );
    }
}
