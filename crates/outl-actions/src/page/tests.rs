//! The page-model battery: slug lookup, metadata, open / delete,
//! split-brain merge, and the journal helpers.
//!
//! Split out of `page.rs` the way `trash/tests.rs` is split out of
//! `trash.rs`: the tests were a third of that file, and the revive path
//! ([`super::revive`]) needs room beside them. Keeping every page test in
//! one module is deliberate — a revive is only meaningful next to the
//! delete it undoes.

use super::*;
use crate::block::append_block;
use crate::refusal::Surface;
use outl_core::id::ActorId;

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
    )
}

#[test]
fn create_and_find_by_slug() {
    let (mut w, hlc) = ws();
    let id = open_or_create(&mut w, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    assert_eq!(find_by_slug(&w, "ideas"), Some(id));
    let pages = list_all(&w);
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].slug, "ideas");
    assert_eq!(pages[0].title, "Ideas");
    assert_eq!(pages[0].kind, PageKind::Page);
}

#[test]
fn toggle_pin_flips_the_property_and_reports_the_new_state() {
    let (mut w, hlc) = ws();
    let id = open_or_create(&mut w, &hlc, "inbox", "Inbox", PageKind::Page).unwrap();
    assert!(!page_meta(&w, id).unwrap().pinned);

    let now_pinned = toggle_pin(&mut w, &hlc, id).unwrap();
    assert!(now_pinned);
    assert!(page_meta(&w, id).unwrap().pinned);

    let now_unpinned = toggle_pin(&mut w, &hlc, id).unwrap();
    assert!(!now_unpinned);
    assert!(!page_meta(&w, id).unwrap().pinned);
}

#[test]
fn toggle_pin_refuses_a_journal_page() {
    let (mut w, hlc) = ws();
    let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
    let journal = open_journal(&mut w, &hlc, date).unwrap();

    let err = toggle_pin(&mut w, &hlc, journal).unwrap_err();
    assert!(matches!(err, ActionError::CannotPinJournal(_)));
    // Refusing must not leave a half-written property behind.
    assert!(!page_meta(&w, journal).unwrap().pinned);
}

/// Define a `journal` template page with one body block.
fn make_journal_template(w: &mut Workspace, hlc: &HlcGenerator) {
    let tpl = open_or_create(w, hlc, "templates-journal", "journal", PageKind::Page).unwrap();
    set_property(
        w,
        hlc,
        tpl,
        crate::template::TEMPLATE_KEY,
        Some(PropValue::Text(
            crate::template::JOURNAL_TEMPLATE_NAME.into(),
        )),
    )
    .unwrap();
    append_block(w, hlc, Some(tpl), Some("**Standup:**")).unwrap();
}

#[test]
fn open_journal_stamps_template_on_fresh_daily() {
    let (mut w, hlc) = ws();
    make_journal_template(&mut w, &hlc);
    let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
    let j = open_journal(&mut w, &hlc, date).unwrap();
    let bodies: Vec<String> = children_of(&w, j)
        .into_iter()
        .filter_map(|(id, _)| w.block_text(id))
        .collect();
    assert!(
        bodies.iter().any(|t| t == "**Standup:**"),
        "fresh daily should carry the journal template body"
    );
}

#[test]
fn open_journal_does_not_restamp_existing_daily() {
    let (mut w, hlc) = ws();
    make_journal_template(&mut w, &hlc);
    let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
    let first = open_journal(&mut w, &hlc, date).unwrap();
    let count_after_first = children_of(&w, first).len();
    // Re-opening the same daily must return it as-is, never re-stamp.
    let second = open_journal(&mut w, &hlc, date).unwrap();
    assert_eq!(first, second);
    assert_eq!(children_of(&w, second).len(), count_after_first);
}

#[test]
fn open_journal_without_template_is_empty() {
    let (mut w, hlc) = ws();
    let date = NaiveDate::from_ymd_opt(2026, 7, 9).unwrap();
    let j = open_journal(&mut w, &hlc, date).unwrap();
    assert!(
        children_of(&w, j).is_empty(),
        "no journal template → empty daily"
    );
}

/// The #473 symptom, through the call every client makes every morning.
/// `open_journal` has its own `find_by_slug` short-circuit ahead of
/// `open_or_create`, so reviving has to survive both.
#[test]
fn open_journal_revives_a_day_the_user_deleted() {
    let (mut w, hlc) = ws();
    let date = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
    let first = open_journal(&mut w, &hlc, date).unwrap();
    delete(&mut w, &hlc, &crate::dates::journal_slug(date)).unwrap();

    let again = open_journal(&mut w, &hlc, date).unwrap();

    assert_eq!(again, first);
    assert!(!crate::tree::is_trashed(&w, again));
    let block = append_block(&mut w, &hlc, Some(again), Some("the morning note")).unwrap();
    assert_eq!(w.block_text(block).as_deref(), Some("the morning note"));
}

/// A revived day is empty, so it is a *fresh* day and gets the journal
/// template — once. `open_journal` decides that from the `find_by_slug`
/// miss at entry, which the revive happens after, and the next open hits
/// the live page and skips the stamp (`open_journal_does_not_restamp_existing_daily`).
#[test]
fn a_revived_journal_day_is_stamped_like_a_fresh_one() {
    let (mut w, hlc) = ws();
    make_journal_template(&mut w, &hlc);
    let date = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
    let slug = crate::dates::journal_slug(date);
    open_journal(&mut w, &hlc, date).unwrap();
    delete(&mut w, &hlc, &slug).unwrap();

    let revived = open_journal(&mut w, &hlc, date).unwrap();

    let bodies: Vec<String> = children_of(&w, revived)
        .into_iter()
        .filter_map(|(id, _)| w.block_text(id))
        .collect();
    assert_eq!(
        bodies,
        vec!["**Standup:**".to_string()],
        "exactly one stamp, and none of the deleted blocks"
    );
}

// Resolution of user-typed names / ref targets (`open_or_create_by_name`,
// `open_or_create_by_ref`, the shared ladder) is tested in
// `crate::resolve::tests`; the person-typed arm in `crate::person::tests`.

#[test]
fn page_meta_prefers_title_property_over_node_text() {
    // Regression (issue #88): disk-sourced / ingested pages park the
    // human-readable name in the `title::` property
    // (`diff_to_ops_with_page_props` emits it as `Op::SetProp` key
    // "title") while the page root node's text stays empty. The
    // mobile / desktop autocomplete then rendered the slug because
    // `page_meta` read `block_text` only. `title::` must win.
    let (mut w, hlc) = ws();
    // Create with an empty node text, mimicking an ingested page.
    let id = open_or_create(&mut w, &hlc, "avelino-outl", "", PageKind::Page).unwrap();
    set_prop(
        &mut w,
        &hlc,
        id,
        "title",
        PropValue::Text("avelino/outl".to_string()),
    )
    .unwrap();
    let meta = page_meta(&w, id).unwrap();
    assert_eq!(meta.title, "avelino/outl");
    assert_eq!(meta.slug, "avelino-outl");
}

#[test]
fn page_meta_falls_back_to_slug_when_no_title_or_text() {
    // No `title::` property and empty node text → slug is the last
    // resort so a title is never blank.
    let (mut w, hlc) = ws();
    let id = open_or_create(&mut w, &hlc, "orphan", "", PageKind::Page).unwrap();
    let meta = page_meta(&w, id).unwrap();
    assert_eq!(meta.title, "orphan");
}

#[test]
fn open_or_create_is_idempotent() {
    let (mut w, hlc) = ws();
    let a = open_or_create(&mut w, &hlc, "foo", "Foo", PageKind::Page).unwrap();
    let b = open_or_create(&mut w, &hlc, "foo", "Foo Renamed", PageKind::Page).unwrap();
    assert_eq!(a, b);
    // Title is NOT updated by a second open_or_create; the caller
    // is expected to use edit_text if they want to rename.
    let pages = list_all(&w);
    assert_eq!(pages[0].title, "Foo");
}

#[test]
fn journal_round_trip() {
    let (mut w, hlc) = ws();
    let date = NaiveDate::from_ymd_opt(2026, 5, 27).unwrap();
    let id = open_journal(&mut w, &hlc, date).unwrap();
    let meta = page_meta(&w, id).unwrap();
    assert_eq!(meta.slug, "2026-05-27");
    assert_eq!(meta.kind, PageKind::Journal);
    assert!(meta.title.contains("2026"));
}

#[test]
fn migration_moves_legacy_blocks() {
    let (mut w, hlc) = ws();
    let a = append_block(&mut w, &hlc, None, Some("legacy 1")).unwrap();
    let b = append_block(&mut w, &hlc, None, Some("legacy 2")).unwrap();
    let moved = migrate_legacy_into_today(&mut w, &hlc).unwrap();
    assert_eq!(moved, 2);
    let today_id = open_today(&mut w, &hlc).unwrap();
    assert_eq!(w.tree().parent(a), Some(today_id));
    assert_eq!(w.tree().parent(b), Some(today_id));
}

#[test]
fn delete_removes_page_from_listings() {
    // `page::delete` moves the page root to `NodeId::trash()`, so
    // `find_by_slug` / `list_all` stop surfacing it. The op stays
    // in the log; only the materialised tree drops the node.
    let (mut w, hlc) = ws();
    let id = open_or_create(&mut w, &hlc, "doomed", "Doomed", PageKind::Page).unwrap();
    // Sanity: exists before delete.
    assert_eq!(find_by_slug(&w, "doomed"), Some(id));
    assert_eq!(list_all(&w).len(), 1);

    let meta = delete(&mut w, &hlc, "doomed").unwrap();
    assert_eq!(meta.slug, "doomed");

    // After delete: gone from slug lookup and from the listing.
    assert_eq!(find_by_slug(&w, "doomed"), None);
    assert!(list_all(&w).is_empty());
}

#[test]
fn delete_unknown_slug_is_page_not_found() {
    // Deleting a slug that doesn't resolve surfaces
    // `ActionError::PageNotFound` — the caller decides how to
    // present it (toast / status line / silent no-op).
    let (mut w, hlc) = ws();
    let err = delete(&mut w, &hlc, "never-existed").unwrap_err();
    assert!(matches!(err, ActionError::PageNotFound(s) if s == "never-existed"));
}

/// Issue #473: every morning the desktop and iOS clients could not write
/// in today's journal, and no error was shown.
///
/// `delete` is `Move(root, TRASH_ROOT)` and the root keeps its id
/// (invariant 6). Journal ids are deterministic, so the next open resolved
/// the *same* node — but `find_by_slug` only walks `children_of(root())`,
/// so it reported the page absent and `open_or_create` fell through to
/// `Op::Create`. `Create` is idempotent ("if the node already exists, keep
/// its current parent/position"), so the root stayed in the trash while the
/// page opened normally, and every keystroke under it was dropped.
///
/// The assertion that matters is the last one: an edit has to land.
#[test]
fn reopening_a_deleted_page_accepts_writes_again() {
    let (mut w, hlc) = ws();
    let id = open_or_create(&mut w, &hlc, "2026-11-06", "2026-11-06", PageKind::Journal).unwrap();
    delete(&mut w, &hlc, "2026-11-06").unwrap();
    assert!(crate::tree::is_trashed(&w, id));

    let again =
        open_or_create(&mut w, &hlc, "2026-11-06", "2026-11-06", PageKind::Journal).unwrap();

    assert_eq!(again, id, "the page keeps its deterministic id");
    assert!(!crate::tree::is_trashed(&w, again));
    assert_eq!(w.tree().parent(again), Some(NodeId::root()));
    assert_eq!(find_by_slug(&w, "2026-11-06"), Some(again));

    let block = append_block(&mut w, &hlc, Some(again), Some("first line")).unwrap();
    crate::block::edit_text(&mut w, &hlc, block, "first line, edited").unwrap();
    assert_eq!(
        w.block_text(block).as_deref(),
        Some("first line, edited"),
        "a keystroke on the reopened page must land"
    );
}

/// The other half of the revive, and the one that decides what "revive"
/// may mean: the user **deleted** this page, and its blocks went to the
/// trash with it in one `Op::Move`. Bringing the root back so today's
/// writing works must not hand yesterday's blocks back unasked.
///
/// Hiding them is not destroying them. Invariant 6 keeps every block in
/// the op log, `trash::list` names them, and `trash::restore` puts one
/// back — under the revived page, with its own subtree. That recoverability
/// is what makes trashing the children the right answer rather than the
/// cheap one, so it is asserted here and not left implied.
#[test]
fn reviving_a_page_leaves_its_deleted_blocks_in_the_trash() {
    let (mut w, hlc) = ws();
    let id = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let a = append_block(&mut w, &hlc, Some(id), Some("yesterday a")).unwrap();
    let nested = append_block(&mut w, &hlc, Some(a), Some("yesterday a.1")).unwrap();
    let b = append_block(&mut w, &hlc, Some(id), Some("yesterday b")).unwrap();
    delete(&mut w, &hlc, "notes").unwrap();

    let again = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();

    assert_eq!(again, id);
    assert!(
        !crate::tree::is_trashed(&w, again),
        "the page is live again"
    );
    assert!(
        children_of(&w, again).is_empty(),
        "yesterday's blocks must not come back behind the user's back"
    );
    for block in [a, nested, b] {
        assert!(
            crate::tree::is_trashed(&w, block),
            "block {block} left the trash on a revive"
        );
    }

    // Hidden, not destroyed: the trash lists them and a restore lands
    // under the page that just came back.
    let listed: Vec<NodeId> = crate::trash::list(&w, Surface::Cli)
        .into_iter()
        .map(|entry| entry.node)
        .collect();
    assert!(
        listed.contains(&a) && listed.contains(&b),
        "the revive's own trashing must be restorable: {listed:?}"
    );
    crate::trash::restore(&mut w, &hlc, a).unwrap();
    assert_eq!(w.tree().parent(a), Some(again));
    assert_eq!(
        w.tree().parent(nested),
        Some(a),
        "a restored block brings its subtree"
    );
}

/// The issue's second bullet, at this layer: editing a block the revive
/// left in the trash is an **error**, never a silent no-op. A client that
/// still holds the old block id has to be told, or the keystroke vanishes
/// exactly the way #473 reported.
#[test]
fn an_edit_to_a_block_the_revive_left_behind_is_refused() {
    let (mut w, hlc) = ws();
    let id = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let block = append_block(&mut w, &hlc, Some(id), Some("yesterday")).unwrap();
    delete(&mut w, &hlc, "notes").unwrap();
    open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();

    let result = crate::block::edit_text(&mut w, &hlc, block, "typed into a dead id");

    assert!(
        matches!(result, Err(ActionError::NotInTree(_))),
        "a trashed block must refuse the edit, got {result:?}"
    );
    assert_eq!(w.block_text(block).as_deref(), Some("yesterday"));
}

/// Reopening a page that is simply *there* must stay a pure read. The
/// revive check runs before `Op::Create`, so a workspace where nothing was
/// deleted must come out of a second open with the same op count.
#[test]
fn reopening_a_live_page_writes_no_op() {
    let (mut w, hlc) = ws();
    let id = open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    let before = w.log().len();

    assert_eq!(
        open_or_create(&mut w, &hlc, "notes", "Notes", PageKind::Page).unwrap(),
        id
    );
    assert_eq!(w.log().len(), before, "a live page costs no ops to reopen");
}

/// Mint a SECOND root for `slug` with a fresh (non-deterministic)
/// id — the split-brain shape the merge repairs. Returns its id.
fn spawn_duplicate_root(
    w: &mut Workspace,
    hlc: &HlcGenerator,
    slug: &str,
    kind: PageKind,
) -> NodeId {
    let dup_id = NodeId(ulid::Ulid::generate());
    let pos = position_for_new_last_child(w, NodeId::root());
    create_with_explicit_id(w, hlc, dup_id, NodeId::root(), pos, Some(slug)).unwrap();
    set_prop(w, hlc, dup_id, SLUG_KEY, PropValue::Text(slug.to_string())).unwrap();
    set_prop(
        w,
        hlc,
        dup_id,
        KIND_KEY,
        PropValue::Text(kind.as_str().to_string()),
    )
    .unwrap();
    dup_id
}

#[test]
fn merge_duplicate_slug_roots_consolidates_two_journal_roots() {
    let (mut w, hlc) = ws();
    let slug = "2026-07-10";
    // Canonical root via the normal path (deterministic id) + one child.
    let canonical = open_or_create(&mut w, &hlc, slug, "2026-07-10", PageKind::Journal).unwrap();
    assert_eq!(canonical, page_id_from_slug(slug));
    let c0 = append_block(&mut w, &hlc, Some(canonical), Some("canonical child")).unwrap();

    // Second root, same slug, fresh id, its own two children.
    let dup = spawn_duplicate_root(&mut w, &hlc, slug, PageKind::Journal);
    let d0 = append_block(&mut w, &hlc, Some(dup), Some("dup child 1")).unwrap();
    let d1 = append_block(&mut w, &hlc, Some(dup), Some("dup child 2")).unwrap();

    let merged = merge_duplicate_slug_roots(&mut w, &hlc).unwrap();
    assert_eq!(merged, 1, "exactly one duplicate root merged");

    // Only the canonical root remains for the slug.
    let remaining: Vec<NodeId> = children_of(&w, NodeId::root())
        .into_iter()
        .filter(|(id, _)| {
            matches!(
                w.tree().property(*id, SLUG_KEY),
                Some(PropValue::Text(s)) if s == slug
            )
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(remaining, vec![canonical]);

    // Canonical holds ALL children from both roots, dup's order preserved
    // and appended after the canonical's own child.
    let kids: Vec<NodeId> = children_of(&w, canonical)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(kids, vec![c0, d0, d1], "no child lost, order preserved");

    // The emptied duplicate is trashed.
    assert_eq!(w.tree().parent(dup), Some(NodeId::trash()));
    // find_by_slug now resolves to the single survivor.
    assert_eq!(find_by_slug(&w, slug), Some(canonical));
}

#[test]
fn merge_duplicate_slug_roots_is_idempotent_and_noop_when_clean() {
    let (mut w, hlc) = ws();
    // Clean workspace: nothing to merge.
    assert_eq!(merge_duplicate_slug_roots(&mut w, &hlc).unwrap(), 0);

    let slug = "ideas";
    open_or_create(&mut w, &hlc, slug, "Ideas", PageKind::Page).unwrap();
    let dup = spawn_duplicate_root(&mut w, &hlc, slug, PageKind::Page);
    append_block(&mut w, &hlc, Some(dup), Some("stray")).unwrap();

    // First run merges the duplicate.
    assert_eq!(merge_duplicate_slug_roots(&mut w, &hlc).unwrap(), 1);
    // Second run is a no-op: the workspace is clean again.
    assert_eq!(merge_duplicate_slug_roots(&mut w, &hlc).unwrap(), 0);
}

/// Mint the canonical root for `slug` directly (deterministic id),
/// bypassing `open_or_create`'s `find_by_slug` short-circuit — which
/// would otherwise return an already-present duplicate.
fn spawn_canonical_root(
    w: &mut Workspace,
    hlc: &HlcGenerator,
    slug: &str,
    kind: PageKind,
) -> NodeId {
    let id = page_id_from_slug(slug);
    let pos = position_for_new_last_child(w, NodeId::root());
    create_with_explicit_id(w, hlc, id, NodeId::root(), pos, Some(slug)).unwrap();
    set_prop(w, hlc, id, SLUG_KEY, PropValue::Text(slug.to_string())).unwrap();
    set_prop(
        w,
        hlc,
        id,
        KIND_KEY,
        PropValue::Text(kind.as_str().to_string()),
    )
    .unwrap();
    id
}

#[test]
fn find_by_slug_is_deterministic_with_duplicate_roots() {
    let (mut w, hlc) = ws();
    let slug = "2026-07-10";
    // Duplicate root FIRST (would win a naive "first match"), then the
    // canonical deterministic-id root minted directly (bypassing the
    // `open_or_create` short-circuit that would return the dup).
    let dup = spawn_duplicate_root(&mut w, &hlc, slug, PageKind::Journal);
    let canonical = spawn_canonical_root(&mut w, &hlc, slug, PageKind::Journal);
    assert_eq!(canonical, page_id_from_slug(slug));
    assert_ne!(dup, canonical);

    // Deterministic: always the canonical id, stable across repeated calls.
    assert_eq!(find_by_slug(&w, slug), Some(canonical));
    assert_eq!(find_by_slug(&w, slug), Some(canonical));
}

// Person-typed page tests (`type:: person`, `@` mention resolution,
// edge cases) live in `crate::person::tests`. Keep this module
// focused on the page-model primitives (slug, kind, title, journal).
