//! Reviving a page root that was deleted and reopened by its slug.
//!
//! Lives beside [`crate::page`] rather than inside it for the same reason
//! [`crate::page_merge`] does: this is a self-contained repair pass
//! (detect trashed deterministic root -> re-parent it -> trash its stale
//! children individually -> re-stamp props), and `page.rs` already owns
//! the full page-model surface.
//!
//! Page ids are deterministic ([`crate::page::page_id_from_slug`]), so a
//! slug that was deleted and later reopened resolves to the *same* node
//! rather than minting a fresh one. `Op::Create` is a no-op for a node
//! already in the tree, so routing a reopen through the plain create
//! path left the root parented under `TRASH_ROOT` forever: every block
//! under it kept answering `NotInTree` to `ensure_live`, and a page
//! reopened on a schedule (most visibly the journal, reopened every day)
//! re-stamped its template's blocks under a subtree nobody could edit on
//! every single open (issue #473).
//!
//! The reopened page starts fresh, the same as a brand-new page with
//! this slug would: the content that existed before the delete is moved
//! to the trash individually, one `Op::Move` per direct child, rather
//! than coming back attached to the revived root. Each of those children
//! is still an ordinary, restorable trash entry (`trash::list` reports
//! it, `trash::restore` can bring it back on its own) -- it is just not
//! attached to the page by default the way it was before the delete.
//!
//! Both halves -- the move back out of the trash, and the per-child move
//! into it -- are ordinary `Op::Move`s with a fresh HLC, so two devices
//! reviving the same slug concurrently still converge: the CRDT orders
//! the two `Move`s on the root by HLC like any other pair of concurrent
//! moves on one node.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::block::{delete as trash_block, move_under};
use crate::error::ActionError;
use crate::page::{set_property, PageKind, KIND_KEY, SLUG_KEY, TITLE_KEY};
use crate::tree::children_of;

/// Revive `node` (the deterministic root for `slug`, currently sitting
/// in the trash) so [`crate::page::open_or_create`] can hand it back as
/// a live page instead of leaving it stuck.
pub(crate) fn revive_trashed_page(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    node: NodeId,
    slug: &str,
    title: &str,
    kind: PageKind,
) -> Result<NodeId, ActionError> {
    let stale_children: Vec<NodeId> = children_of(workspace, node)
        .into_iter()
        .map(|(id, _)| id)
        .collect();

    let mut batch = workspace.begin_batch();
    move_under(&mut batch, hlc, node, NodeId::root())?;
    for child in stale_children {
        trash_block(&mut batch, hlc, child)?;
    }
    set_property(
        &mut batch,
        hlc,
        node,
        SLUG_KEY,
        Some(PropValue::Text(slug.to_string())),
    )?;
    set_property(
        &mut batch,
        hlc,
        node,
        KIND_KEY,
        Some(PropValue::Text(kind.as_str().to_string())),
    )?;
    let title_prop = if title != slug {
        Some(PropValue::Text(title.to_string()))
    } else {
        None
    };
    set_property(&mut batch, hlc, node, TITLE_KEY, title_prop)?;
    batch.commit()?;
    Ok(node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{append_block, edit_text};
    use crate::page::{find_by_slug, open_journal, open_or_create, page_meta};
    use crate::tree::is_trashed;
    use chrono::NaiveDate;
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn reopening_a_deleted_page_revives_it_instead_of_leaving_it_trashed() {
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        append_block(&mut w, &hlc, Some(id), Some("old content")).unwrap();
        crate::page::delete(&mut w, &hlc, "notes").unwrap();
        assert!(is_trashed(&w, id), "sanity: page root is trashed");

        let reopened = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        assert_eq!(reopened, id, "same slug -> same deterministic id");
        assert!(
            !is_trashed(&w, reopened),
            "reopened page root must be live, not under the trash"
        );
        assert_eq!(find_by_slug(&w, "notes"), Some(reopened));

        // A block on the revived page can be edited: this is the bug's
        // user-visible symptom (ensure_live rejected every edit on a
        // trashed page with ActionError::NotInTree).
        let block = append_block(&mut w, &hlc, Some(reopened), Some("fresh content")).unwrap();
        edit_text(&mut w, &hlc, block, "edited content").unwrap();
        assert_eq!(w.block_text(block), Some("edited content".to_string()));
    }

    #[test]
    fn reopening_a_deleted_page_trashes_its_old_content_individually() {
        let (mut w, hlc) = ws();
        let id = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        let old_block = append_block(&mut w, &hlc, Some(id), Some("old content")).unwrap();
        crate::page::delete(&mut w, &hlc, "notes").unwrap();

        let reopened = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        assert!(
            children_of(&w, reopened).is_empty(),
            "reopened page starts fresh, like a brand-new page would"
        );
        // The old block is still in the trash, as its own entry, directly
        // under TRASH_ROOT -- not lost, just detached from the revived
        // page.
        assert_eq!(w.tree().parent(old_block), Some(NodeId::trash()));
    }

    #[test]
    fn reopening_a_deleted_page_several_times_does_not_accumulate_blocks() {
        let (mut w, hlc) = ws();
        open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
        crate::page::delete(&mut w, &hlc, "notes").unwrap();

        for _ in 0..5 {
            let reopened = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
            assert!(children_of(&w, reopened).is_empty());
        }
    }

    #[test]
    fn reopening_a_deleted_journal_revives_it_as_a_fresh_daily() {
        let (mut w, hlc) = ws();
        let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
        let first = open_journal(&mut w, &hlc, date).unwrap();
        let block = append_block(&mut w, &hlc, Some(first), Some("yesterday's note")).unwrap();
        crate::page::delete(&mut w, &hlc, "2026-07-09").unwrap();

        let reopened = open_journal(&mut w, &hlc, date).unwrap();
        assert_eq!(reopened, first);
        assert!(!is_trashed(&w, reopened));
        let meta = page_meta(&w, reopened).unwrap();
        assert_eq!(meta.slug, "2026-07-09");
        assert_eq!(meta.kind, PageKind::Journal);
        assert!(children_of(&w, reopened).is_empty());

        // The old block still exists in the trash, just detached.
        assert_eq!(w.tree().parent(block), Some(NodeId::trash()));

        let new_block = append_block(&mut w, &hlc, Some(reopened), Some("today's note")).unwrap();
        edit_text(&mut w, &hlc, new_block, "edited today's note").unwrap();
        assert_eq!(
            w.block_text(new_block),
            Some("edited today's note".to_string())
        );
    }

    #[test]
    fn reopening_a_deleted_journal_several_times_stamps_the_template_once() {
        let (mut w, hlc) = ws();
        let tpl =
            open_or_create(&mut w, &hlc, "templates-journal", "journal", PageKind::Page).unwrap();
        set_property(
            &mut w,
            &hlc,
            tpl,
            crate::template::TEMPLATE_KEY,
            Some(PropValue::Text(
                crate::template::JOURNAL_TEMPLATE_NAME.into(),
            )),
        )
        .unwrap();
        append_block(&mut w, &hlc, Some(tpl), Some("**Standup:**")).unwrap();

        let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
        let first = open_journal(&mut w, &hlc, date).unwrap();
        assert_eq!(children_of(&w, first).len(), 1);
        crate::page::delete(&mut w, &hlc, "2026-07-09").unwrap();

        // Reopening the deleted daily several times must stamp the
        // template exactly once -- the first reopen revives it fresh,
        // every subsequent one finds it live via `find_by_slug` and
        // returns early.
        for _ in 0..3 {
            let reopened = open_journal(&mut w, &hlc, date).unwrap();
            assert_eq!(reopened, first);
            assert_eq!(children_of(&w, reopened).len(), 1);
        }
    }
}
