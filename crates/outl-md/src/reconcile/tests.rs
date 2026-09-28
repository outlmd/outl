//! End-to-end tests for one reconcile pass.
//!
//! Split out of `reconcile.rs` for size, the same way `parse/tests.rs`
//! was: the seam is implementation / tests, never a seam inside the
//! pass. Every test here drives the whole pipeline through
//! `reconcile_md` rather than any single module it calls, which is why
//! they did not follow the extracted helpers out.
//!
//! `super` is `crate::reconcile`, so every test keeps its exact path
//! and name — `cargo test <name>` still finds it.

use super::*;
use outl_core::id::ActorId;
use std::fs;
use tempfile::TempDir;

fn setup_workspace() -> (TempDir, Workspace, HlcGenerator) {
    let dir = TempDir::new().unwrap();
    let actor = ActorId::new();
    let ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    (dir, ws, hlc)
}

#[test]
fn first_reconcile_creates_sidecar_and_applies_ops() {
    let (dir, mut ws, hlc) = setup_workspace();
    let md_path = dir.path().join("foo.md");
    fs::write(&md_path, "title:: foo\n\n- alpha\n- beta\n").unwrap();

    let report = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    assert!(report.created_sidecar);
    assert!(report.ops_applied > 0);
    assert_eq!(report.orphans, 0);

    let sidecar_path = sidecar_path_for(&md_path);
    assert!(sidecar_path.exists());
    let sc = sidecar::read(&sidecar_path).unwrap();
    assert_eq!(sc.version, sidecar::SIDECAR_VERSION);
    assert_eq!(sc.blocks.len(), 2);
    // Every block must carry a non-empty `ref_handle` after a fresh
    // reconcile — the v2 invariant.
    assert!(
        sc.blocks.iter().all(|b| !b.ref_handle.is_empty()),
        "v2 sidecar must populate ref_handle on every block: {:?}",
        sc.blocks
    );
}

#[test]
fn idempotent_no_change_means_zero_ops() {
    let (dir, mut ws, hlc) = setup_workspace();
    let md_path = dir.path().join("foo.md");
    fs::write(&md_path, "- a\n- b\n").unwrap();

    let first = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    assert!(first.ops_applied > 0);

    let second = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    assert_eq!(second.ops_applied, 0);
}

/// Regression: `Fractional::between(None, None)` always returns
/// the midpoint, so two externally-authored pages reconciled in
/// the same session would land on identical positions and the
/// iteration order of `children_of(root)` would depend on
/// `HashMap` hashing.
#[test]
fn externally_authored_pages_get_distinct_positions_under_root() {
    let (dir, mut ws, hlc) = setup_workspace();
    let pages_dir = dir.path().join("pages");
    fs::create_dir_all(&pages_dir).unwrap();
    fs::write(
        pages_dir.join("avelino.md"),
        "title:: Avelino\ntype:: person\n\n- bio\n",
    )
    .unwrap();
    fs::write(
        pages_dir.join("samara.md"),
        "title:: Samara\ntype:: person\n\n- bio\n",
    )
    .unwrap();

    reconcile_md(&mut ws, &hlc, &pages_dir.join("avelino.md"), None).unwrap();
    reconcile_md(&mut ws, &hlc, &pages_dir.join("samara.md"), None).unwrap();

    // Collect (id, position) for every root child. Page nodes
    // must end up with distinct fractional positions.
    let positions: Vec<_> = ws
        .tree()
        .iter_nodes()
        .filter(|(_, parent, _)| *parent == outl_core::id::NodeId::root())
        .map(|(_, _, pos)| pos.clone())
        .collect();
    let mut dedup = positions.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(
        positions.len(),
        dedup.len(),
        "two externally-authored pages must not share a fractional position; got {positions:?}"
    );
}

/// Regression for the split-brain journal bug: a
/// `journals/YYYY-MM-DD.md` with **no sidecar** must materialise its
/// page root under the DETERMINISTIC id (`NodeId::from_slug(slug)`),
/// never a fresh time-based `NodeId::new()`. Otherwise the same day
/// reconciled on a device without the `.outl` yet (external editor,
/// peer that shipped only the `.md`) spawns a second, competing root
/// and the day's content splits in two.
#[test]
fn no_sidecar_journal_uses_deterministic_root_id() {
    let (dir, mut ws, hlc) = setup_workspace();
    let journals = dir.path().join("journals");
    fs::create_dir_all(&journals).unwrap();
    let md_path = journals.join("2026-07-10.md");
    fs::write(&md_path, "- morning\n- afternoon\n").unwrap();

    let report = reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    assert!(report.created_sidecar);

    // The page root must be the deterministic id, and it must be a
    // real child of root carrying the slug.
    let expected = NodeId::from_slug("2026-07-10");
    assert_eq!(
        ws.tree().parent(expected),
        Some(NodeId::root()),
        "journal root must be the deterministic id, parented under root"
    );
    assert_eq!(
        ws.tree().property(expected, "page-slug"),
        Some(&outl_core::property::PropValue::Text(
            "2026-07-10".to_string()
        )),
    );

    // And there is exactly ONE root child carrying that slug.
    let roots_with_slug = ws
        .tree()
        .iter_nodes()
        .filter(|(_, parent, _)| *parent == NodeId::root())
        .filter(|(id, _, _)| {
            ws.tree().property(*id, "page-slug")
                == Some(&outl_core::property::PropValue::Text(
                    "2026-07-10".to_string(),
                ))
        })
        .count();
    assert_eq!(roots_with_slug, 1, "exactly one journal root per slug");
}

/// Reconciling a sidecar-less `.md` twice — the second time the
/// deterministic root already exists in the tree — must NOT create a
/// second root. This is the convergence property the fix buys:
/// re-materialising the same slug is idempotent on the root node.
#[test]
fn reconcile_twice_without_sidecar_does_not_duplicate_root() {
    let (dir, mut ws, hlc) = setup_workspace();
    let journals = dir.path().join("journals");
    fs::create_dir_all(&journals).unwrap();
    let md_path = journals.join("2026-07-10.md");

    // First pass writes the sidecar; delete it to force the
    // no-sidecar arm again on the second pass (models a peer that
    // shipped only the `.md`, or a lost `.outl`).
    fs::write(&md_path, "- one\n").unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    let sidecar_path = sidecar_path_for(&md_path);
    fs::remove_file(&sidecar_path).unwrap();

    // Change the file so the pass actually runs (not short-circuited)
    // and reconcile again with no sidecar present.
    fs::write(&md_path, "- one\n- two\n").unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let roots_with_slug = ws
        .tree()
        .iter_nodes()
        .filter(|(_, parent, _)| *parent == NodeId::root())
        .filter(|(id, _, _)| {
            ws.tree().property(*id, "page-slug")
                == Some(&outl_core::property::PropValue::Text(
                    "2026-07-10".to_string(),
                ))
        })
        .count();
    assert_eq!(
        roots_with_slug, 1,
        "second sidecar-less reconcile must reuse the deterministic root, not spawn a duplicate"
    );
}

fn frontmatter_prop(ws: &Workspace, md_path: &Path) -> Option<outl_core::property::PropValue> {
    let page = sidecar::read(&sidecar_path_for(md_path)).unwrap().page_id;
    ws.tree()
        .property(page, crate::frontmatter::PAGE_FRONTMATTER_KEY)
        .cloned()
}

/// `page-frontmatter` is written only from the real `---` fence. A typed
/// `page-frontmatter::` line must not land after that op and replace the
/// fence body, or the next projection writes the fake value over the YAML.
#[test]
fn a_typed_page_frontmatter_line_never_replaces_the_fence() {
    let (dir, mut ws, hlc) = setup_workspace();
    let md_path = dir.path().join("note.md");
    let md = "---\ntitle: X\n---\npage-frontmatter:: fake\n\n- body\n";
    fs::write(&md_path, md).unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();

    let fence = parse(md).frontmatter.expect("the fence parses");
    assert_eq!(
        frontmatter_prop(&ws, &md_path),
        Some(outl_core::property::PropValue::Text(fence))
    );
}

#[test]
fn a_typed_page_frontmatter_line_cannot_invent_a_fence() {
    let (dir, mut ws, hlc) = setup_workspace();
    let md_path = dir.path().join("note.md");
    fs::write(&md_path, "page-frontmatter:: fake\n\n- body\n").unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, None).unwrap();
    assert_eq!(frontmatter_prop(&ws, &md_path), None);
}

/// A page reconciled by a pre-fence parser (issue #281): every YAML line
/// is a block, 24 of them, above two real ones.
fn legacy_fenced_page(dir: &TempDir, ws: &mut Workspace, hlc: &HlcGenerator) -> PathBuf {
    let md_path = dir.path().join("vault-note.md");
    let keys: String = (0..22).map(|i| format!("- k{i}: v{i}\n")).collect();
    fs::write(
        &md_path,
        format!("- ---\n{keys}- ---\n- body one\n- body two\n"),
    )
    .unwrap();
    reconcile_md(ws, hlc, &md_path, None).unwrap();
    assert_eq!(
        sidecar::read(&sidecar_path_for(&md_path))
            .unwrap()
            .blocks
            .len(),
        26
    );
    md_path
}

/// The migration pass orphans 24 of 26 blocks, past the bulk-delete
/// ceiling. They are the fence still on disk, so the guard must not
/// count them, or the page stays frozen with no `page-frontmatter` op.
#[test]
fn the_legacy_fence_blocks_do_not_trip_the_bulk_delete_guard() {
    let (dir, mut ws, hlc) = setup_workspace();
    let md_path = legacy_fenced_page(&dir, &mut ws, &hlc);
    let keys: String = (0..22).map(|i| format!("k{i}: v{i}\n")).collect();
    let md = format!("---\n{keys}---\n\n- body one\n- body two\n");
    fs::write(&md_path, &md).unwrap();

    let report =
        reconcile_md(&mut ws, &hlc, &md_path, None).expect("the migration is not a bulk delete");
    assert_eq!(report.orphans, 24);
    let fence = parse(&md).frontmatter.expect("the fence parses");
    assert_eq!(
        frontmatter_prop(&ws, &md_path),
        Some(outl_core::property::PropValue::Text(fence))
    );
}

/// The discount is grounded in the fence on disk, not in the sidecar: a
/// fence that lost its lines, or never closes, earns none of it.
#[test]
fn a_truncated_md_after_a_legacy_fence_is_still_refused() {
    for truncated in ["---\nk0: v0\n---\n", "---\nk0: v0\nk1: v1\n"] {
        let (dir, mut ws, hlc) = setup_workspace();
        let md_path = legacy_fenced_page(&dir, &mut ws, &hlc);
        fs::write(&md_path, truncated).unwrap();
        let err = reconcile_md(&mut ws, &hlc, &md_path, None)
            .expect_err("losing the page body and the fence is a bulk delete");
        assert!(matches!(err, ReconcileError::BulkDelete(_)), "{err}");
    }
}

#[test]
fn orphans_get_logged_when_log_path_set() {
    let (dir, mut ws, hlc) = setup_workspace();
    let md_path = dir.path().join("foo.md");
    let log_path = dir.path().join("orphans.log");
    fs::write(&md_path, "- a\n- b\n").unwrap();
    reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();

    fs::write(&md_path, "- a\n").unwrap();
    let report = reconcile_md(&mut ws, &hlc, &md_path, Some(&log_path)).unwrap();
    assert_eq!(report.orphans, 1);

    let log = fs::read_to_string(&log_path).unwrap();
    assert!(
        log.contains("id="),
        "orphans.log should contain entry:\n{log}"
    );
}
