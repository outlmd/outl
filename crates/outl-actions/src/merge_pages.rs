//! Folding one page into another: the single owner of "what does it mean
//! to merge page A into page B".
//!
//! Not to be confused with [`crate::page_merge`], which repairs the
//! split-brain where two roots carry the *same* slug. This merges two
//! pages that legitimately exist under different names, at the user's
//! request (the second half of `rename` onto an existing name).
//!
//! # The decision: trash the source, leave an alias behind
//!
//! The source is deleted the ordinary way (`Move` to the trash root, so
//! history is kept and it is reversible) and its names become **aliases**
//! of the target, so a `[[Alpha]]` typed tomorrow still lands on the
//! merged page. The rejected alternative is a stub redirect page: it
//! would keep a second root alive, show up in every page list and need a
//! second rule saying it is not a real page.
//!
//! # Op order, all in one batch
//!
//! 1. **Blocks.** Every direct child of the source root moves under the
//!    target root, in the source's order, after the target's existing
//!    children. A move preserves block ids and subtrees, so nothing is
//!    copied and a concurrent edit to a moved block still lands on it.
//! 2. **Page properties**, merged onto the target (see below).
//! 3. **Redirect aliases.** The source's slug and title join the
//!    target's `alias::` list.
//! 4. **Backlinks.** [`rewrite_mentions`] respells every mention of the
//!    source so it names the target. Mentions written with one of the
//!    source's *aliases* are not rewritten: the alias moved to the
//!    target in step 2, so it already resolves there. Namespace children
//!    (`[[alpha/child]]`) are not the source and are returned in
//!    [`MergeReport::skipped_namespace_refs`].
//! 5. **Delete the source**, skipped when it is already in the trash.
//!
//! Everything is an ordinary op through `Workspace::apply` and nothing
//! here touches the filesystem (root invariant 1).
//!
//! # Property decisions
//!
//! | key | rule |
//! | --- | --- |
//! | `page-slug`, `page-kind`, `page-source`, `page-frontmatter` (`tree::is_page_model_key`) | never copied: they are the source root's identity |
//! | `title` | never copied: step 3 turns it into an alias |
//! | `template`, `params` | never copied: a template's invocation name is its identity. A differing `template` is reported as a conflict, `params` travels with it and is dropped silently |
//! | `alias` | union with the redirect names, case-insensitive, target's entries first; a name holding a comma is written `[[a, b]]`; always written as text, the only shape the alias readers understand |
//! | `tags` | union, case-insensitive and ignoring a leading `#`, target's entries first, written in the target's shape (text separated like the target's, or a list) |
//! | anything else | absent on the target: copied. Equal: nothing. Different: **the target wins** and a [`PropertyConflict`] records both values |
//!
//! A union writes a `SetProp` only when the result differs from what the
//! target already holds, which is what makes a second run emit nothing.
//!
//! # Idempotent, and it catches late blocks
//!
//! A source that is already trashed (and still carries its `page-slug`)
//! is a valid source: this is the "run it again" case. It moves only the
//! children still under it (a peer may have appended some before the
//! merge reached it), writes nothing that is already there, rewrites
//! nothing new and does not delete again; the report says
//! `already_merged`. Property conflicts are reported again on every run,
//! because the dropped value is still on the trashed source.
//!
//! # What a caller does next
//!
//! Run `commit_page` on each of [`MergeReport::touched_pages`] and call
//! [`crate::remove_page_projection`] with [`MergeReport::source`].
//! `merge_pages` itself never projects, so a caller that forgets leaves
//! the `.md` files behind the log, not the log behind them.

use std::collections::BTreeSet;
use std::str::FromStr;

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use serde::Serialize;

use crate::block::{delete, move_under};
use crate::error::ActionError;
use crate::link_rewrite::{rewrite_mentions, PageNames, SkippedRef};
use crate::page::{
    find_by_slug, page_id_from_slug, page_meta, set_property, PageKind, PageMeta, SLUG_KEY,
};
use crate::tree::{children_of, is_trashed};

mod props;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_links;
#[cfg(test)]
mod tests_props;

/// A page property both pages held with different values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PropertyConflict {
    /// The property key.
    pub key: String,
    /// The value the target keeps (empty when the target had none).
    pub kept: String,
    /// The source's value, which was not copied.
    pub dropped: String,
}

/// What [`merge_pages`] did.
///
/// A caller runs `commit_page` on each of [`Self::touched_pages`] and
/// removes the source's projection with
/// `outl_actions::remove_page_projection(root, &report.source)`.
#[derive(Debug, Clone, Serialize)]
pub struct MergeReport {
    /// The source's meta as it was before the merge.
    pub source: PageMeta,
    /// The target's meta after the merge.
    pub target: PageMeta,
    /// Direct children moved under the target (each carries its subtree).
    pub moved_blocks: usize,
    /// Those children, stringified `NodeId`s, in the order they landed.
    pub moved_block_ids: Vec<String>,
    /// Blocks whose text was edited to follow the new name.
    pub rewritten_blocks: Vec<String>,
    /// Properties both pages held with different values (target kept).
    pub property_conflicts: Vec<PropertyConflict>,
    /// Property keys this call wrote on the target (copies and unions).
    pub properties_written: Vec<String>,
    /// Every alias newly present on the target after this merge: the
    /// source's own aliases plus its slug and title.
    pub aliases_added: Vec<String>,
    /// Namespace-child mentions the rewrite left alone.
    pub skipped_namespace_refs: Vec<SkippedRef>,
    /// True when this call moved the source to the trash.
    pub source_deleted: bool,
    /// True when the source was already in the trash on entry.
    pub already_merged: bool,
    /// Slugs a caller must re-project: the target plus the pages owning
    /// rewritten blocks, sorted and deduplicated. Never the source.
    pub touched_pages: Vec<String>,
}

impl MergeReport {
    /// Whether the call changed nothing in the op log: the shape of a
    /// repeated merge.
    pub fn nothing_changed(&self) -> bool {
        self.moved_blocks == 0
            && self.rewritten_blocks.is_empty()
            && self.properties_written.is_empty()
            && self.aliases_added.is_empty()
            && !self.source_deleted
    }
}

/// Merge page `source` into page `target`.
///
/// Both resolve as a page slug, else as the stringified id of a page
/// root. A trashed source is accepted when it still carries its
/// `page-slug` (see the module docs); a trashed target is
/// [`ActionError::PageNotFound`].
///
/// A journal **source** is refused with [`ActionError::CannotMergeJournal`],
/// because a journal day is identified by its date. A journal **target**
/// is allowed: folding a stray page into a day's journal is a legitimate
/// thing to do and loses no identity.
pub fn merge_pages(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    source: &str,
    target: &str,
) -> Result<MergeReport, ActionError> {
    let source_id = resolve_source(workspace, source)?;
    let target_id = resolve_live(workspace, target)
        .ok_or_else(|| ActionError::PageNotFound(target.to_string()))?;
    if source_id == target_id {
        return Err(ActionError::MergeIntoSelf(source.to_string()));
    }
    let source_meta = page_meta(workspace, source_id)
        .ok_or_else(|| ActionError::PageNotFound(source.to_string()))?;
    let target_before = page_meta(workspace, target_id)
        .ok_or_else(|| ActionError::PageNotFound(target.to_string()))?;
    if source_meta.kind == PageKind::Journal {
        return Err(ActionError::CannotMergeJournal(source_meta.slug));
    }
    let already_merged = is_trashed(workspace, source_id);

    let mut batch = workspace.begin_batch();

    let children: Vec<NodeId> = children_of(&batch, source_id)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    for child in &children {
        move_under(&mut batch, hlc, *child, target_id)?;
    }

    let plan = props::plan(
        &batch,
        source_id,
        target_id,
        &[&target_before.slug, &target_before.title],
        &[&source_meta.slug, &source_meta.title],
    );
    let mut properties_written = Vec::new();
    for (key, value) in plan.writes {
        set_property(&mut batch, hlc, target_id, &key, Some(value))?;
        properties_written.push(key);
    }

    // Slug and title never change on a merge, but the aliases did, and
    // the report promises the target as it stands afterwards.
    let target_meta = page_meta(&batch, target_id)
        .ok_or_else(|| ActionError::PageNotFound(target.to_string()))?;
    let rewrite = rewrite_mentions(
        &mut batch,
        hlc,
        &PageNames::of(&source_meta),
        &PageNames::of(&target_meta),
    )?;

    let source_deleted = !already_merged;
    if source_deleted {
        delete(&mut batch, hlc, source_id)?;
    }
    batch.commit()?;

    let mut touched: BTreeSet<String> = rewrite.touched_pages.into_iter().collect();
    touched.insert(target_meta.slug.clone());
    touched.remove(&source_meta.slug);

    Ok(MergeReport {
        source: source_meta,
        target: target_meta,
        moved_blocks: children.len(),
        moved_block_ids: children.iter().map(ToString::to_string).collect(),
        rewritten_blocks: rewrite.rewritten_blocks,
        property_conflicts: plan.conflicts,
        properties_written,
        aliases_added: plan.aliases_added,
        skipped_namespace_refs: rewrite.skipped_namespace_refs,
        source_deleted,
        already_merged,
        touched_pages: touched.into_iter().collect(),
    })
}

/// A live page: by slug, else by the id of a live page root.
fn resolve_live(workspace: &Workspace, name: &str) -> Option<NodeId> {
    find_by_slug(workspace, name).or_else(|| {
        node_id(name).filter(|id| is_page_root(workspace, *id) && !is_trashed(workspace, *id))
    })
}

/// The source: a live page, else a **trashed** page root still carrying
/// its slug, by deterministic id or by the id the caller passed. That is
/// how a second run after the merge finished finds its source again.
fn resolve_source(workspace: &Workspace, name: &str) -> Result<NodeId, ActionError> {
    if let Some(id) = resolve_live(workspace, name) {
        return Ok(id);
    }
    [Some(page_id_from_slug(name)), node_id(name)]
        .into_iter()
        .flatten()
        .find(|id| is_page_root(workspace, *id) && is_trashed(workspace, *id))
        .ok_or_else(|| ActionError::PageNotFound(name.to_string()))
}

fn is_page_root(workspace: &Workspace, id: NodeId) -> bool {
    workspace.tree().property(id, SLUG_KEY).is_some()
}

fn node_id(text: &str) -> Option<NodeId> {
    ulid::Ulid::from_str(text).ok().map(NodeId)
}
