//! Rendering a page to `.md`, path routing, and the bulk apply paths.

use super::*;

#[test]
fn render_page_md_outputs_title_prop_then_children() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("first")).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("second")).unwrap();

    // The title lives in the `title::` property (not the root's text),
    // so it renders as a page property above the children.
    let md = render_page_md(&ws, page);
    assert_eq!(md, "title:: Ideas\n\n- first\n- second\n");
}

/// Copy (`Cmd+C` in view mode) snapshots a block via
/// `render_block_md`. It must capture the **whole** subtree — every
/// descendant at every depth — so a paste reproduces the block in
/// full. This pins the "are we grabbing all the sub-blocks?" review
/// concern: the renderer walks `build_outline` recursively, so a
/// four-level-deep subtree round-trips with its indentation intact.
#[test]
fn render_block_md_captures_the_full_deep_subtree() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    let src = append_block(&mut ws, &hlc, Some(page), Some("src")).unwrap();
    let c1 = append_block(&mut ws, &hlc, Some(src), Some("c1")).unwrap();
    let c1a = append_block(&mut ws, &hlc, Some(c1), Some("c1a")).unwrap();
    append_block(&mut ws, &hlc, Some(c1a), Some("c1a_i")).unwrap();
    append_block(&mut ws, &hlc, Some(c1), Some("c1b")).unwrap();
    append_block(&mut ws, &hlc, Some(src), Some("c2")).unwrap();

    let md = render_block_md(&ws, src);
    assert_eq!(
        md,
        "- src\n  - c1\n    - c1a\n      - c1a_i\n    - c1b\n  - c2\n"
    );
}

#[test]
fn render_page_md_emits_page_level_properties() {
    // Regression for the silent divergence between the op log
    // and the rendered `.md`. Page-level properties (`type::`,
    // `icon::`, etc.) used to be dropped on render because
    // `render_page_md` always passed `properties: Vec::new()`.
    // Result: a person page created via `@` autocomplete in the
    // TUI carried `Op::SetProp { type: person }` in the log but
    // its `.md` had only the blocks — the `WorkspaceIndex`
    // (which parses `.md`) didn't list it under `pages_by_type`,
    // so the next `@` mention never surfaced it.
    use crate::page::set_property;
    use outl_core::property::PropValue;

    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "avelino", "Avelino", PageKind::Page).unwrap();
    set_property(
        &mut ws,
        &hlc,
        page,
        crate::person::TYPE_KEY,
        Some(PropValue::Text(crate::person::PERSON_TYPE.to_string())),
    )
    .unwrap();
    set_property(
        &mut ws,
        &hlc,
        page,
        "icon",
        Some(PropValue::Text("🦀".to_string())),
    )
    .unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("bio")).unwrap();

    let md = render_page_md(&ws, page);
    assert!(
        md.contains("type:: person"),
        "rendered .md must carry the type:: person property; got:\n{md}"
    );
    assert!(
        md.contains("icon:: 🦀"),
        "rendered .md must carry the icon property; got:\n{md}"
    );
    // `page-slug` / `page-kind` stay internal — they're owned by
    // the page-model layer, not by the rendered `.md`.
    assert!(
        !md.contains("page-slug"),
        "internal book-keeping property leaked into rendered .md:\n{md}"
    );
    assert!(
        !md.contains("page-kind"),
        "internal book-keeping property leaked into rendered .md:\n{md}"
    );
    // Body still renders the block.
    assert!(md.contains("- bio"));
}

#[test]
fn page_md_path_routes_journals_and_pages_separately() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    let regular = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    let journal =
        open_journal(&mut ws, &hlc, NaiveDate::from_ymd_opt(2026, 5, 27).unwrap()).unwrap();

    let r_meta = page_meta(&ws, regular).unwrap();
    let j_meta = page_meta(&ws, journal).unwrap();

    assert!(page_md_path(tmp.path(), &r_meta).ends_with("pages/ideas.md"));
    assert!(page_md_path(tmp.path(), &j_meta).ends_with("journals/2026-05-27.md"));
}

#[test]
fn apply_all_pages_writes_each_to_disk() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("first idea")).unwrap();

    let report = apply_all_pages_md(&ws, tmp.path());
    assert!(report.failures.is_empty());
    assert_eq!(report.written.len(), 1);
    let body = std::fs::read_to_string(&report.written[0]).unwrap();
    // In-app pages store their title in the `title::` property (not the
    // root's Yrs text — see `open_or_create`), so it renders at the top.
    assert_eq!(body, "title:: Ideas\n\n- first idea\n");
}

#[test]
fn apply_all_pages_refuses_to_overwrite_content_ahead_of_the_log() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    let page = open_or_create(&mut ws, &hlc, "ideas", "Ideas", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("logged")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

    let path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
    let ahead = format!(
        "{}- never entered the op log\n",
        std::fs::read_to_string(&path).unwrap()
    );
    std::fs::write(&path, &ahead).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("plugin mutation")).unwrap();

    let report = apply_all_pages_md(&ws, tmp.path());
    assert_eq!(report.failures.len(), 1);
    assert!(matches!(
        &report.failures[0].error,
        crate::ActionError::PageMarkdownAheadOfLog { .. }
    ));
    assert_eq!(std::fs::read_to_string(path).unwrap(), ahead);
}

#[test]
fn apply_all_pages_continues_after_a_refused_page() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    let frozen = open_or_create(&mut ws, &hlc, "a-frozen", "Frozen", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(frozen), Some("logged")).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), frozen).unwrap();
    let frozen_path = page_md_path(tmp.path(), &page_meta(&ws, frozen).unwrap());
    let ahead = format!(
        "{}- never entered the op log\n",
        std::fs::read_to_string(&frozen_path).unwrap()
    );
    std::fs::write(&frozen_path, &ahead).unwrap();

    let healthy = open_or_create(&mut ws, &hlc, "z-healthy", "Healthy", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(healthy), Some("project me")).unwrap();
    let healthy_path = page_md_path(tmp.path(), &page_meta(&ws, healthy).unwrap());

    let report = apply_all_pages_md(&ws, tmp.path());

    assert_eq!(report.failures.len(), 1);
    assert!(
        healthy_path.exists(),
        "a later healthy page must still project"
    );
    assert_eq!(std::fs::read_to_string(frozen_path).unwrap(), ahead);
}

/// Regression for https://github.com/outlmd/outl/issues/120 —
/// a page synced from a peer exists in the CRDT tree but has no
/// `.md` on this device's disk. `open_page_by_slug` calls
/// `apply_page_md_with_sidecar_if_absent`; without the projection
/// `read_page_outline` returns an empty outline and the page opens
/// blank. This test models that scenario: page in workspace, no
/// file on disk → helper writes the projection → outline is populated.
#[test]
fn apply_if_absent_projects_when_md_is_missing() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    // Simulate a synced page: exists in the CRDT tree, no .md on disk.
    let page = open_or_create(&mut ws, &hlc, "synced", "Synced", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("peer block")).unwrap();

    // Pre-condition: no .md on disk yet.
    let meta = page_meta(&ws, page).unwrap();
    let path = page_md_path(tmp.path(), &meta);
    assert!(!path.exists(), "test setup error: .md should not exist yet");

    // Call the guarded helper — should project because the file is absent.
    let result = apply_page_md_with_sidecar_if_absent(&ws, tmp.path(), page).unwrap();
    assert!(
        result.is_some(),
        "expected Some(path) when .md was absent, got None"
    );
    assert!(path.exists(), ".md must be on disk after projection");

    // The projected content must match the CRDT tree (not be empty).
    let body = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        body, "title:: Synced\n\n- peer block\n",
        "projected .md must contain the peer's block, not be blank"
    );

    // `read_page_outline` (the path `open_page_by_slug` takes after
    // the projection) must now return populated content.
    let outline = crate::outline::read_page_outline(tmp.path(), &meta).unwrap();
    assert_eq!(outline.nodes.len(), 1, "outline must have the peer's block");
    assert_eq!(outline.nodes[0].text, "peer block");
}

/// Guard against sync churn: calling `apply_page_md_with_sidecar_if_absent`
/// on a page whose `.md` is already on disk must be a **no-op** — it must
/// not rewrite the `.outl` sidecar. `build_sidecar` stamps
/// `last_synced_at: now()`, so an unconditional call would rewrite
/// the sidecar bytes on every page open, generating noise for every
/// file-transport peer (iCloud / Syncthing) even when nothing changed.
#[test]
fn apply_if_absent_is_noop_when_md_already_exists() {
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let tmp = TempDir::new().unwrap();

    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some("a block")).unwrap();

    // First projection: write the .md and .outl to disk.
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();

    let meta = page_meta(&ws, page).unwrap();
    let md_path = page_md_path(tmp.path(), &meta);
    let sidecar_path = outl_md::sidecar::sidecar_path_for(&md_path);

    // Capture the sidecar bytes before the guarded call.
    let sidecar_before = std::fs::read(&sidecar_path).unwrap();

    // Give the clock a chance to tick so a second `now()` stamp
    // would differ if the sidecar were rewritten.
    std::thread::sleep(std::time::Duration::from_millis(5));

    // Guarded call — file exists, must be a no-op.
    let result = apply_page_md_with_sidecar_if_absent(&ws, tmp.path(), page).unwrap();
    assert!(
        result.is_none(),
        "expected None when .md already exists, got Some"
    );

    // Sidecar bytes must be unchanged (no `last_synced_at: now()` rewrite).
    let sidecar_after = std::fs::read(&sidecar_path).unwrap();
    assert_eq!(
        sidecar_before, sidecar_after,
        ".outl sidecar must not be rewritten when .md already existed"
    );
}
