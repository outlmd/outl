//! `/rename <new name>`: change what the open page is called, and offer
//! the merge when the name is taken.
//!
//! What a rename *is* (the two `SetProp`s, the link rewrite, the
//! sub-page mentions deliberately left alone) belongs to
//! `outl_actions::rename_page` (RFC 0557). This module is the four
//! things around it the TUI has to do itself: refuse what the terminal
//! can refuse cheaply, drive the shared commit tail, re-point
//! everything in this client that remembers a page by file name, and
//! say what the rewrite did.
//!
//! **Only the collision asks first.** A rename onto a free name is
//! reversible — rename it back — so it runs on the keystroke. A
//! collision is not: `ActionError::PageExists` is turned into the
//! one-shot `y/n` the page delete and `/merge` already share
//! (`PendingPageAction::RenameCollision`), with
//! `outl_actions::rename_or_merge` behind the `y`. That is what
//! `actions/merge`'s module doc has prescribed since the merge landed.
//!
//! **Opened from the command registry, not a chord**, for the reason
//! `/merge` and `/trash` give: a chord needs an `Action` variant, and
//! `outl_shortcuts::support` is an exhaustive `match` over three
//! clients. The verdicts are recorded where they belong, in
//! `Capability::PageRename`.
//!
//! **The `:` line is prefilled with the command name only.** Seeding
//! the current title would read as an edit field, and the `:` editor has
//! no cursor and no word-delete (`input/overlay.rs`) — so a prefilled
//! name is worse than an empty one: the only way to change its middle
//! is to backspace the whole thing.
//!
//! # A slug change breaks four things in this client
//!
//! All four are handled here, and each is a silent corruption rather
//! than an error:
//!
//! 1. **The view.** `persist` renders `self.page` to
//!    `self.current_path()`; leaving `View::Page` on the old path
//!    recreates `pages/<old>.md` on the next keystroke and reconciles it
//!    back into a page. `current_slug()` is the file stem, so every
//!    downstream reader is stale until the view moves. Hence
//!    `flush_pending_save()` before the rename and
//!    `land_on_page_or_today` after it.
//! 2. **`recent_paths` and `last_search`**, both of which hold a
//!    `PathBuf` — `App::repoint_remembered_path` owns that.
//! 3. **Undo / redo**, keyed by `view_path` —
//!    `App::forget_history_of_pages` over `touched_pages` *plus* the
//!    old slug, since the entry that matters most is the one naming the
//!    file that just went away.
//! 4. **The projection order**, which is `App::project_touched`'s and
//!    root `CLAUDE.md` invariant 8's.

use anyhow::Result;
use chrono::NaiveDate;

use outl_actions::{ActionError, OnExisting, RenameOutcome, RenameReport};

use crate::actions::page_commit::rename_summary;
use crate::pending_page_action::{PageAction, PendingPageAction};
use crate::state::{App, ToastKind};

impl App {
    /// `/rename <new name>`: rename the open page, or arm the merge
    /// prompt when another page already owns that name.
    ///
    /// Every refusal here is a sentence on the status line or a toast
    /// and leaves the op log untouched — the same three the backend
    /// refuses before `begin_batch`, asked where the message can name
    /// what the user typed instead of a slug.
    pub(crate) fn rename_current_page(&mut self, new_name: &str) -> Result<()> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            self.status = "usage: rename <new page name>".into();
            return Ok(());
        }
        let slug = self.current_slug();
        // Same exclusion as `/merge` and `delete_page_from_chord`: a
        // journal day is identified by its date, so `rename_page`
        // refuses it too (`CannotRenameJournal`).
        if slug.is_empty() || NaiveDate::parse_from_str(&slug, "%Y-%m-%d").is_ok() {
            self.toast(ToastKind::Warning, "can't rename a journal page");
            return Ok(());
        }
        let Some((_, title)) = self.page_label(&slug) else {
            self.status = format!("no page '{slug}' to rename");
            return Ok(());
        };
        // The same split the Tauri command layer makes, through the same
        // owner: the slug is the page's filename, the title is what the
        // user typed. A second spelling of this is a page that lands on
        // a different file depending on which client renamed it — which
        // is also why there is no extra "that can't be a page name"
        // refusal here. `slugify` always answers with a legal slug
        // (`untitled` when the name folds to nothing), so a guard would
        // be this client refusing what the GUIs accept.
        let new_slug = outl_md::slug::slugify(new_name);
        // The rename reads the op log, and the page on screen may hold
        // an edit that has not reached it yet.
        self.flush_pending_save();
        match outl_actions::rename_page(
            &mut self.workspace,
            &self.hlc,
            &slug,
            &new_slug,
            Some(new_name),
        ) {
            Ok(report) => self.finish_rename(&report),
            Err(ActionError::PageExists { existing }) => {
                self.arm_rename_collision(slug, title, new_slug, new_name, &existing);
                Ok(())
            }
            Err(e) => {
                self.toast(ToastKind::Error, format!("rename failed: {e}"));
                Ok(())
            }
        }
    }

    /// The name is taken: ask before folding the page into its owner.
    ///
    /// Nothing is written — `rename_page` refuses `PageExists` before it
    /// opens a batch, so the collision costs no ops.
    fn arm_rename_collision(
        &mut self,
        slug: String,
        title: String,
        new_slug: String,
        new_title: &str,
        existing: &str,
    ) {
        let existing_title = self
            .page_label(existing)
            .map(|(_, t)| t)
            .unwrap_or_else(|| existing.to_string());
        self.status = format!(
            "'{existing_title}' already has that name — merge '{title}' into it? \
             can't be undone here. y/n"
        );
        self.pending_page_action = Some(PendingPageAction {
            slug,
            title,
            action: PageAction::RenameCollision {
                new_slug,
                new_title: new_title.to_string(),
            },
        });
    }

    /// Resolve an armed rename collision.
    ///
    /// `rename_or_merge` and not `merge_pages`: the page that owned the
    /// name can be gone by the time the `y` lands, and then the rename
    /// the user asked for is what should happen.
    pub(crate) fn confirm_page_rename(
        &mut self,
        slug: &str,
        new_slug: &str,
        new_title: &str,
    ) -> Result<()> {
        self.flush_pending_save();
        let outcome = outl_actions::rename_or_merge(
            &mut self.workspace,
            &self.hlc,
            slug,
            new_slug,
            Some(new_title),
            OnExisting::MergeInto,
        );
        let landed = match outcome {
            Ok(RenameOutcome::Renamed(report)) => self.finish_rename(&report),
            Ok(RenameOutcome::Merged(report)) => self.finish_merge(&report, true),
            Err(e) => {
                self.toast(ToastKind::Error, format!("rename failed: {e}"));
                Ok(())
            }
        };
        self.status.clear();
        landed
    }

    /// Change the page's `title::` through the rename.
    ///
    /// `title` is half of the page's identity: `page_meta` resolves the
    /// page's display name from it and every `[[old name]]` in the
    /// workspace resolves through it. Writing it as an ordinary property
    /// changed the name and respelled nothing — the silent half-rename
    /// that `page-slug`'s own refusal ("rename the page instead")
    /// already points away from, pointing at a feature this client did
    /// not have. `rename_page` with `new_slug == old_slug` is that
    /// rename, and the slug deliberately does **not** move: a title is
    /// not a filename.
    ///
    /// `Err` is the sentence the overlay shows on its own status line.
    pub(crate) fn retitle_current_page(&mut self, title: &str) -> Result<(), String> {
        let title = title.trim();
        if title.is_empty() {
            // `d d` on this row, or an emptied value field, reaches
            // here. `rename_page` refuses a blank title and its
            // `InvalidSlug` names a slug, which this is not — so the
            // refusal is worded where the question was asked.
            return Err("a page needs a name — `/rename <new name>` changes it".into());
        }
        let slug = self.current_slug();
        if slug.is_empty() || NaiveDate::parse_from_str(&slug, "%Y-%m-%d").is_ok() {
            return Err("a journal's name is its date".into());
        }
        if outl_actions::find_by_slug(&self.workspace, &slug).is_none() {
            return Err(format!("no page '{slug}' to retitle"));
        }
        self.flush_pending_save();
        let report =
            outl_actions::rename_page(&mut self.workspace, &self.hlc, &slug, &slug, Some(title))
                .map_err(|e| format!("retitle failed: {e}"))?;
        self.finish_rename(&report)
            .map_err(|e| format!("retitle failed: {e}"))
    }

    /// Everything after the ops are in the log.
    ///
    /// The order is load-bearing and the module doc owns why: project,
    /// drop the stale `.md` only if the page's new file landed, repoint
    /// what this client remembers by name, forget the invalidated
    /// history, then move the view.
    fn finish_rename(&mut self, report: &RenameReport) -> Result<()> {
        let projected = self.project_touched(&report.touched_pages, &report.meta.slug);
        if let Some(stale) = &report.stale_projection {
            if projected.landing_ok {
                if let Err(e) = outl_actions::remove_page_projection(&self.workspace_root, stale) {
                    tracing::warn!(
                        target: "outl::tui::rename",
                        "could not remove renamed page projection for {}: {e}",
                        stale.slug
                    );
                }
            }
            let from = outl_actions::page_md_path(&self.workspace_root, stale);
            let to = outl_actions::page_md_path(&self.workspace_root, &report.meta);
            // Only onto a file that is really there — the same question
            // `land_on_page_or_today` asks, so the row and the view
            // cannot disagree about where the page is. An I/O failure
            // can leave the projection written nowhere, and a remembered
            // path aimed at a missing `.md` is read as an empty page
            // whose next save writes that emptiness over the renamed
            // one. Forgetting it is the half that loses nothing.
            self.repoint_remembered_path(&from, to.exists().then_some(to.as_path()));
        }
        let mut stale_history: Vec<&str> =
            report.touched_pages.iter().map(String::as_str).collect();
        stale_history.push(&report.old_slug);
        self.forget_history_of_pages(&stale_history);
        self.spawn_index_rebuild();
        self.spawn_backlink_index_rebuild();
        // A title-only rename leaves the file where it is, so there is
        // nowhere to land — but the AST on screen still carries the old
        // `title::`, and the next save would render it back over the
        // page the commit just projected.
        if report.stale_projection.is_some() {
            self.land_on_page_or_today(&report.meta)?;
        } else {
            self.load_current();
        }
        self.toast(ToastKind::Info, rename_summary(report));
        if let Some((slug, e)) = projected.failure {
            self.report_projection_failure("renamed", &slug, e);
        }
        self.status.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
