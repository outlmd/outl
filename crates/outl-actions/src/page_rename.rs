//! Renaming a page: the single owner of "change what a page is called and
//! make every mention agree" (RFC 0107, page identity).
//!
//! # The model
//!
//! A page has two names and they are different facts:
//!
//! - the **slug** (`page-slug`) is its filesystem identity, the stem of
//!   `pages/<slug>.md`, validated by [`crate::page::is_valid_slug`];
//! - the **title** (`title::`) is what a person reads. [`page_meta`] resolves
//!   it most-specific-first: the `title::` property, then the root node's
//!   text, then the slug.
//!
//! A rename keeps the page's node id and its blocks. It is two ordinary
//! `Op::SetProp`s (last-write-wins by HLC, so two devices renaming at once
//! converge on one name instead of forking the page) plus the edits that
//! respell every mention, which are [`crate::link_rewrite::rewrite_mentions`]'s
//! job and nothing in this file. All of it goes into the op log inside one
//! batch (root invariants 1 and 7); nothing here touches a `.md` or a
//! sidecar.
//!
//! # What the caller does next
//!
//! A rename dirties more than one page and moves a file, which is more than
//! [`crate::commit::commit_page`] (scoped to one page) can carry, so the
//! caller drives the tail from [`RenameReport`]: run its commit pipeline
//! over each slug in [`RenameReport::touched_pages`], and only then remove
//! [`RenameReport::stale_projection`] with
//! [`crate::journal::remove_page_projection`]. Removing the old `.md` last
//! means a crash in between leaves two copies of the page, never none.
//!
//! # Title rules
//!
//! - `new_title: None` leaves the title alone. A page whose title was
//!   **derived from its slug** (no `title::` and no root text, so
//!   `page_meta` fell back to the slug) follows the new slug on its own,
//!   because the fallback is read, not stored; mentions are respelled to
//!   that new title.
//! - `Some(t)` sets `title::` to `t.trim()` when that differs from the
//!   current title (or when the title was derived and the slug changes,
//!   so asking for the current spelling pins it instead of letting it
//!   drift).
//! - A blank title, or one with a line break (it would break the
//!   `title:: ...` line it is projected as), is refused with
//!   [`ActionError::InvalidSlug`] carrying the offending text. No title
//!   variant exists and a new error variant is not worth it for one
//!   input check; the `Display` text is the only imprecise part.
//!
//! # Order of operations
//!
//! The mentions are rewritten **before** the slug is changed, with the
//! names the page is about to have. `rewrite_mentions` asks the workspace
//! whether a `[[name]]` slug-resolves to some *other* page (and leaves it
//! alone when so); once the old slug is gone that question can no longer
//! be answered about the renamed page, so the rewrite has to run while
//! `find_by_slug(old)` still finds it. The cost is that blocks of the
//! renamed page are reported under the old slug by the rewriter, which
//! [`rename_page`] maps back to the new one.
//!
//! # Known limit: reopening the old slug
//!
//! A page id is derived from the slug it was *created* under
//! (`page_id_from_slug`), and a rename does not change it. After `a -> b`
//! the page keeps the id that `open_or_create("a", ..)` would derive.
//! If anything later opens slug `a` (a stale `[[a]]` followed by
//! navigation, a peer on an older build, a script), `find_by_slug("a")`
//! finds nothing, `Op::Create` for the deterministic id is a no-op because
//! that node is live, and `open_or_create` then writes `page-slug = a`
//! (and `page-kind`, and a title) onto **the renamed page**. The rename is
//! silently undone: the page flips back to `a`, `b` stops resolving, and a
//! mention rewritten to `b` now points at nothing.
//! `known_limit_reopening_the_old_slug_renames_the_page_back` pins that
//! behaviour so a fix has to be deliberate. It is not fixed here because
//! the fix is not one line: `open_or_create` would have to notice that the
//! deterministic id is live under a different slug and mint another id,
//! and every `page_id_from_slug` caller (`outl-md` reconcile, `desync`,
//! `open_with`) leans on id == f(slug). Merge has no such problem: the
//! merged-away root is trashed under its own slug.
//!
//! A rename onto a slug that only differs by letter case from another
//! page is allowed (the check is exact, like [`find_by_slug`]), and so is
//! a date-shaped slug for a regular page; neither is policed here.

use std::collections::BTreeSet;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use serde::Serialize;

use crate::error::ActionError;
use crate::link_rewrite::{rewrite_mentions, PageNames, SkippedRef};
use crate::merge_pages::{merge_pages, MergeReport};
use crate::page::{
    find_by_slug, is_valid_slug, page_meta, set_property, PageKind, PageMeta, SLUG_KEY, TITLE_KEY,
};

#[cfg(test)]
mod merge_tests;
#[cfg(test)]
mod tests;

/// What [`rename_page`] did.
///
/// A caller runs its commit pipeline over [`Self::touched_pages`] and then
/// removes [`Self::stale_projection`]; thin CLI, MCP and GUI wrappers
/// around [`rename_page`] share that tail rather than re-deriving it.
#[derive(Debug, Clone, Serialize)]
pub struct RenameReport {
    /// The page after the rename.
    pub meta: PageMeta,
    /// The slug the page had.
    pub old_slug: String,
    /// The title the page had.
    pub old_title: String,
    /// Blocks whose text the link rewriter edited (stringified `NodeId`).
    pub rewritten_blocks: Vec<String>,
    /// Namespace-child mentions (`[[old/child]]`) that were left as they
    /// were: they name a sub-page that is not being renamed.
    pub skipped_namespace_refs: Vec<SkippedRef>,
    /// Slugs a caller must re-project: the renamed page (under its new
    /// slug) plus every page owning a rewritten block. Sorted, deduplicated.
    /// Empty when the rename was a no-op.
    pub touched_pages: Vec<String>,
    /// `Some(old meta)` when the slug changed: after projecting the new
    /// page the caller removes `.md` and sidecar with
    /// [`crate::journal::remove_page_projection`]. `None` for a title-only
    /// rename, where the file stays where it is.
    pub stale_projection: Option<PageMeta>,
}

/// What to do when the rename target is already a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnExisting {
    /// Return [`ActionError::PageExists`] and write nothing.
    Refuse,
    /// Fold the renamed page into the existing one with
    /// [`merge_pages`].
    MergeInto,
}

/// The result of [`rename_or_merge`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RenameOutcome {
    /// The page now lives under the new name.
    Renamed(RenameReport),
    /// The new name was taken and the page was merged into its owner.
    Merged(MergeReport),
}

/// Rename the page `old_slug` to `new_slug` and, when `new_title` is
/// `Some`, retitle it, then respell every mention of it.
///
/// See the module docs for the title rules, the order of operations and
/// the known limit about reopening the old slug.
///
/// Refuses, before any op is written:
///
/// - [`ActionError::PageNotFound`]: no live page has `old_slug`.
/// - [`ActionError::CannotRenameJournal`]: a journal's slug is its date.
/// - [`ActionError::InvalidSlug`]: `new_slug` is not a legal slug, or
///   `new_title` is blank or multi-line.
/// - [`ActionError::PageExists`]: another live page already owns
///   `new_slug`. Its `existing` field is that page's slug, so a caller can
///   offer [`rename_or_merge`] instead.
///
/// `new_slug == old_slug` is allowed (a title-only rename). When neither
/// the slug nor the title would change, nothing is written and the report
/// is empty (`touched_pages` and `rewritten_blocks` are empty,
/// `stale_projection` is `None`).
pub fn rename_page(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    old_slug: &str,
    new_slug: &str,
    new_title: Option<&str>,
) -> Result<RenameReport, ActionError> {
    let id = find_by_slug(workspace, old_slug)
        .ok_or_else(|| ActionError::PageNotFound(old_slug.to_string()))?;
    let old_meta =
        page_meta(workspace, id).ok_or_else(|| ActionError::NotInTree(id.to_string()))?;
    if old_meta.kind == PageKind::Journal {
        return Err(ActionError::CannotRenameJournal(old_slug.to_string()));
    }
    if !is_valid_slug(new_slug) {
        return Err(ActionError::InvalidSlug(new_slug.to_string()));
    }
    refuse_if_taken(workspace, id, new_slug)?;
    let title = clean_title(new_title)?;

    let slug_changes = new_slug != old_meta.slug;
    let follows_slug = slug_changes && title_follows_slug(workspace, id);
    let set_title = title.filter(|t| *t != old_meta.title || follows_slug);
    if !slug_changes && set_title.is_none() {
        return Ok(RenameReport {
            old_slug: old_meta.slug.clone(),
            old_title: old_meta.title.clone(),
            meta: old_meta,
            rewritten_blocks: Vec::new(),
            skipped_namespace_refs: Vec::new(),
            touched_pages: Vec::new(),
            stale_projection: None,
        });
    }

    let from = PageNames::of(&old_meta);
    let to = PageNames {
        slug: new_slug.to_string(),
        title: set_title.clone().unwrap_or_else(|| {
            if follows_slug {
                new_slug.to_string()
            } else {
                old_meta.title.clone()
            }
        }),
        person: from.person,
    };

    let mut batch = workspace.begin_batch();
    // Before the slug moves: see "Order of operations" in the module docs.
    let rewrite = rewrite_mentions(&mut batch, hlc, &from, &to)?;
    if slug_changes {
        set_property(
            &mut batch,
            hlc,
            id,
            SLUG_KEY,
            Some(PropValue::Text(new_slug.to_string())),
        )?;
    }
    if let Some(title) = set_title {
        set_property(&mut batch, hlc, id, TITLE_KEY, Some(PropValue::Text(title)))?;
    }
    batch.commit()?;

    let meta = page_meta(workspace, id).ok_or_else(|| ActionError::NotInTree(id.to_string()))?;
    debug_assert_eq!(PageNames::of(&meta), to, "predicted names drifted");

    // The rewriter read each block's page before the slug moved, so the
    // renamed page's own blocks come back under the old slug.
    let mut touched: BTreeSet<String> = rewrite
        .touched_pages
        .into_iter()
        .map(|slug| {
            if slug == old_meta.slug {
                meta.slug.clone()
            } else {
                slug
            }
        })
        .collect();
    touched.insert(meta.slug.clone());

    Ok(RenameReport {
        old_slug: old_meta.slug.clone(),
        old_title: old_meta.title.clone(),
        rewritten_blocks: rewrite.rewritten_blocks,
        skipped_namespace_refs: rewrite.skipped_namespace_refs,
        touched_pages: touched.into_iter().collect(),
        stale_projection: slug_changes.then_some(old_meta),
        meta,
    })
}

/// [`rename_page`], and on a name collision either refuse or merge.
///
/// With [`OnExisting::Refuse`] this is [`rename_page`]: a collision is
/// [`ActionError::PageExists`], unchanged. With [`OnExisting::MergeInto`] a
/// collision calls `merge_pages(old_slug, existing)`: the page being
/// renamed is the **source**, the page that already owns the name is the
/// **target** and keeps its own name.
///
/// So `new_title` is ignored on the merge path: the existing page keeps
/// its title, and a title the caller wanted for the renamed page has
/// nowhere to go. Any other error (unknown page, journal, invalid slug)
/// is returned as-is on both paths.
pub fn rename_or_merge(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    old_slug: &str,
    new_slug: &str,
    new_title: Option<&str>,
    on_existing: OnExisting,
) -> Result<RenameOutcome, ActionError> {
    match rename_page(workspace, hlc, old_slug, new_slug, new_title) {
        Ok(report) => Ok(RenameOutcome::Renamed(report)),
        Err(ActionError::PageExists { existing }) if on_existing == OnExisting::MergeInto => {
            merge_pages(workspace, hlc, old_slug, &existing).map(RenameOutcome::Merged)
        }
        Err(other) => Err(other),
    }
}

/// [`ActionError::PageExists`] when a live page other than `id` owns `slug`.
fn refuse_if_taken(workspace: &Workspace, id: NodeId, slug: &str) -> Result<(), ActionError> {
    match find_by_slug(workspace, slug) {
        Some(other) if other != id => {
            let existing = page_meta(workspace, other)
                .map(|meta| meta.slug)
                .unwrap_or_else(|| slug.to_string());
            Err(ActionError::PageExists { existing })
        }
        _ => Ok(()),
    }
}

/// The trimmed title to set, `None` when the caller gave none.
fn clean_title(title: Option<&str>) -> Result<Option<String>, ActionError> {
    let Some(raw) = title else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains(['\n', '\r']) {
        return Err(ActionError::InvalidSlug(raw.to_string()));
    }
    Ok(Some(trimmed.to_string()))
}

/// Whether `page_meta` resolves this page's title by falling back to the
/// slug: no non-blank `title::` text and no non-blank root text. Mirrors the
/// ladder in `page_meta`, which is the owner of it.
fn title_follows_slug(workspace: &Workspace, id: NodeId) -> bool {
    let has_title_prop = matches!(
        workspace.tree().property(id, TITLE_KEY),
        Some(PropValue::Text(s)) if !s.trim().is_empty()
    );
    let has_root_text = workspace
        .block_text(id)
        .is_some_and(|text| !text.trim().is_empty());
    !has_title_prop && !has_root_text
}
