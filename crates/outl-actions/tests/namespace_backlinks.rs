//! Nested tags (`#os/linux`) roll up to their ancestor namespaces —
//! issue #275.
//!
//! Two claims, and they are separate: a **mention** of `#os/linux` is
//! also a mention of `os` (this file), and the page `os` can *list*
//! the pages nested under it (`outl_actions::namespace`'s own unit
//! tests). The rollup lives in the backlink index's `Namespace`
//! channel, so it must hold on **both** builders — the workspace walk
//! and the from-disk walk. A rule that holds on one and not the other
//! is the exact drift `backlinks_index`'s module doc exists to
//! prevent, and the user sees it as "the tag works in the TUI but not
//! in the desktop".

use std::path::Path;

use outl_actions::{
    append_block, apply_page_md_with_sidecar, build_backlink_index, build_backlink_index_from_disk,
    find_by_slug, list_pages, namespace_descendants, open_or_create_by_name, open_or_create_page,
    page_meta, PageKind, PageMeta,
};
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use tempfile::TempDir;

/// A workspace with one page per `(slug, block text)` pair, projected
/// to `.md` + sidecar and reopened from disk so both index builders
/// see the same content.
fn workspace_with(root: &Path, notes: &[(&str, &str)]) -> (Workspace, Vec<PageMeta>) {
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    {
        let storage = JsonlStorage::open(ops_dir.clone(), actor).unwrap();
        let mut w =
            Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf()))
                .unwrap();
        for (slug, text) in notes {
            let page = open_or_create_page(&mut w, &hlc, slug, slug, PageKind::Page).unwrap();
            append_block(&mut w, &hlc, Some(page), Some(text)).unwrap();
        }
        for meta in list_pages(&w) {
            let id = find_by_slug(&w, &meta.slug).unwrap();
            apply_page_md_with_sidecar(&w, root, id).unwrap();
        }
    }
    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let w =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap();
    let metas = list_pages(&w);
    (w, metas)
}

/// The page carrying `slug`, as both builders need its `PageMeta`.
fn meta_for(w: &Workspace, slug: &str) -> PageMeta {
    let id = find_by_slug(w, slug).expect("page exists");
    page_meta(w, id).expect("page meta")
}

/// Texts of the blocks that back-link to `slug`, under both builders,
/// asserted to agree. Sorted so the assertion doesn't depend on walk
/// order.
fn backlink_texts(w: &Workspace, metas: &[PageMeta], root: &Path, slug: &str) -> Vec<String> {
    let meta = meta_for(w, slug);
    let mut from_tree: Vec<String> = build_backlink_index(w, root)
        .for_page(w, &meta)
        .into_iter()
        .map(|b| b.block_text)
        .collect();
    let mut from_disk: Vec<String> = build_backlink_index_from_disk(metas, root)
        .for_page(w, &meta)
        .into_iter()
        .map(|b| b.block_text)
        .collect();
    from_tree.sort();
    from_disk.sort();
    assert_eq!(
        from_tree, from_disk,
        "the workspace and from-disk builders disagree about `{slug}`"
    );
    from_tree
}

#[test]
fn a_nested_tag_backlinks_its_ancestor_namespace() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (w, metas) = workspace_with(
        root,
        &[
            ("os", "the namespace root"),
            ("debian-notes", "running #os/linux/debian here"),
        ],
    );

    assert_eq!(
        backlink_texts(&w, &metas, root, "os"),
        vec!["running #os/linux/debian here".to_string()],
        "`#os/linux/debian` must reach the `os` page"
    );
}

#[test]
fn every_level_of_the_namespace_collects_the_mention() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (w, metas) = workspace_with(
        root,
        &[
            ("os", "root"),
            ("os-linux", "middle"),
            ("debian-notes", "running #os/linux/debian here"),
        ],
    );

    // `os-linux` is the slug of the page whose title is `os/linux`;
    // here it was created slug-first, so title == slug and the lookup
    // rides the slug channel. Both must see the deep mention.
    assert_eq!(backlink_texts(&w, &metas, root, "os").len(), 1);
    assert_eq!(backlink_texts(&w, &metas, root, "os-linux").len(), 1);

    // **The asymmetry, recorded rather than left to be discovered.**
    // The two halves of this feature read different things: the
    // backlinks channel keys off the *mention* (`#os/linux/debian`,
    // which carries the `/`), while `descendants` compares the *page
    // title*. This page was created slug-first, so its title is the
    // flat `os-linux` and it lists nothing nested — even though the
    // mention above reached it.
    //
    // That is not a bug in either half: deriving the hierarchy from a
    // slug would need `-` to mean "nest", which makes `meu-projeto` a
    // child of `meu`. It is a dependency worth knowing: the listing
    // half only works for pages whose `title::` survived, and a page
    // ingested from disk without one falls back to its slug
    // (`page::page_meta`). On a real 2,575-page workspace that was 14
    // pages with `title::`, so the listing showed nothing while the
    // backlinks channel collected thousands.
    let pages = list_pages(&w);
    assert!(
        namespace_descendants(&pages, "os-linux").is_empty(),
        "a page whose title is the flat slug lists nothing nested"
    );
}

#[test]
fn a_namespaced_page_ref_rolls_up_too() {
    // `[[os/linux]]` is the same claim through the other channel. A
    // `Ref` is matched verbatim, so without the namespace channel the
    // `os` page would never see it.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (w, metas) = workspace_with(
        root,
        &[("os", "root"), ("notes", "see [[os/linux]] for details")],
    );

    assert_eq!(
        backlink_texts(&w, &metas, root, "os"),
        vec!["see [[os/linux]] for details".to_string()]
    );
}

#[test]
fn a_sibling_sharing_a_string_prefix_does_not_roll_up() {
    // The rollup is per-segment, not per-substring: `#oscar/wilde` is
    // not in the `os` namespace. A naive `starts_with("os")` would
    // make every `os*` page noise on the `os` page.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (w, metas) = workspace_with(
        root,
        &[("os", "root"), ("lit", "reading #oscar/wilde tonight")],
    );

    assert!(
        backlink_texts(&w, &metas, root, "os").is_empty(),
        "`#oscar/wilde` must not reach the `os` page"
    );
}

#[test]
fn a_namespaced_page_is_not_its_own_backlink() {
    // `ancestors` is a *proper* prefix list. If it included the name
    // itself, the block `#os/linux` would index under `Namespace(os-linux)`
    // and the `os/linux` page would list its own mention twice — once
    // as a tag, once as its own namespace.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (w, metas) = workspace_with(root, &[("os-linux", "root"), ("notes", "see #os/linux")]);

    assert_eq!(
        backlink_texts(&w, &metas, root, "os-linux").len(),
        1,
        "one mentioning block, listed once"
    );
}

#[test]
fn a_flat_tag_is_unchanged_by_the_rollup() {
    // The pre-existing exact-match behaviour must not move: `#project`
    // has no ancestors, so it emits no namespace key at all.
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let (w, metas) = workspace_with(
        root,
        &[
            ("project", "root"),
            ("notes", "tagged #project"),
            ("other", "tagged #projector"),
        ],
    );

    assert_eq!(
        backlink_texts(&w, &metas, root, "project"),
        vec!["tagged #project".to_string()],
        "`#projector` is a different tag and stays out"
    );
}

/// The listing half, against a **real workspace** rather than a
/// hand-built `PageMeta`.
///
/// This is not a duplicate of `namespace`'s unit tests: those pass a
/// `PageMeta` whose `title` the test wrote itself. Here the title
/// makes the round trip the product makes — `open_or_create_by_name`
/// parks it in the `title::` property, `page_meta` resolves it back
/// (property first, node text second, slug last). A regression in
/// that resolution would leave every namespaced page titled by its
/// flattened slug (`os-linux`), and the hierarchy would silently
/// collapse to nothing while the unit tests stayed green.
#[test]
fn descendants_read_the_titles_a_real_workspace_stores() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    let ops_dir = root.join("ops");
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let storage = JsonlStorage::open(ops_dir, actor).unwrap();
    let mut w =
        Workspace::open_with_storage(actor, Box::new(storage), Some(root.to_path_buf())).unwrap();

    // The path a tag click takes: a typed name, slugified for disk,
    // kept verbatim as the title.
    for name in [
        "os",
        "os/linux",
        "os/linux/debian",
        "os/freebsd",
        "oscar/wilde",
    ] {
        open_or_create_by_name(&mut w, &hlc, name, PageKind::Page).unwrap();
    }

    let pages = list_pages(&w);
    // The slug really is flat — this is the reason the hierarchy is
    // read off the title, and the assertion that would fail first if
    // someone "fixed" slugs to carry `/`.
    assert!(
        pages.iter().any(|p| p.slug == "os-linux-debian"),
        "a namespaced page must slugify to one flat path component: {:?}",
        pages.iter().map(|p| &p.slug).collect::<Vec<_>>()
    );

    let rows: Vec<(String, usize, String)> = namespace_descendants(&pages, "os")
        .into_iter()
        .map(|c| (c.page.title, c.depth, c.label))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("os/freebsd".to_string(), 1, "freebsd".to_string()),
            ("os/linux".to_string(), 1, "linux".to_string()),
            ("os/linux/debian".to_string(), 2, "debian".to_string()),
        ],
        "`oscar/wilde` must stay out and every level must carry its depth"
    );
}
