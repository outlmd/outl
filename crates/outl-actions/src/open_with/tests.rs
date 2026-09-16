use std::path::PathBuf;

use outl_core::id::ActorId;
use tempfile::TempDir;

use super::*;
use crate::journal::render_page_md;
use crate::outline::project_outline;

fn ws() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).unwrap(),
        HlcGenerator::new(actor),
    )
}

fn write(dir: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, body).unwrap();
    path
}

/// Import once and return the page id, the way a client does it.
fn import(ws: &mut Workspace, hlc: &HlcGenerator, path: &std::path::Path) -> NodeId {
    let contents = read_source(path).unwrap();
    let target = resolve_target(ws, path).unwrap();
    import_into(ws, hlc, &target, &contents).unwrap()
}

#[test]
fn the_page_title_is_namespaced_and_the_slug_is_one_path_component() {
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "meeting notes.md", "- one\n- two\n");

    let target = resolve_target(&ws, &path).unwrap();
    assert_eq!(target.title(), "open-in/meeting notes");
    // The slash lives in `title::` only — a slug carrying one would be
    // rejected by `is_valid_slug` and break `pages/<slug>.md`.
    assert!(
        crate::page::is_valid_slug(target.slug()),
        "{}",
        target.slug()
    );
    assert!(!target.slug().contains('/'));

    let page = import(&mut ws, &hlc, &path);
    assert_eq!(page, target.page_id());
    assert_eq!(
        crate::page::page_meta(&ws, page).unwrap().title,
        "open-in/meeting notes"
    );
}

#[test]
fn the_page_id_is_known_before_the_page_exists() {
    // The client runs the whole import inside one `commit_page`, which
    // needs the node id up front. Ids derive from the slug, so a `New`
    // target can answer — if that ever stops being true, the commit
    // would snapshot and project the wrong page.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "plan.txt", "hello");

    let predicted = resolve_target(&ws, &path).unwrap().page_id();
    assert_eq!(import(&mut ws, &hlc, &path), predicted);
}

#[test]
fn a_bulleted_markdown_file_lands_as_an_outline() {
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "todo.md", "- parent\n  - child\n- sibling\n");

    let page = import(&mut ws, &hlc, &path);
    let outline = project_outline(&ws, page);
    let texts: Vec<&str> = outline.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, vec!["parent", "sibling"]);
    assert_eq!(outline[0].children.len(), 1);
    assert_eq!(outline[0].children[0].text, "child");
}

#[test]
fn a_plain_text_file_lands_as_one_block_per_line() {
    // Same rule a plain-text paste follows (`paste::split_paragraphs`):
    // in a bullet-free payload, a non-blank line is a block. Reaching
    // for a different split here would give the user two behaviours for
    // one kind of content.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "prose.txt", "first line\nsecond line\n\nthird line\n");

    let page = import(&mut ws, &hlc, &path);
    let outline = project_outline(&ws, page);
    let texts: Vec<&str> = outline.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, vec!["first line", "second line", "third line"]);
}

#[test]
fn reopening_the_same_file_navigates_instead_of_importing_again() {
    // Two imports into one page would duplicate every block, and
    // overwriting would delete whatever the user wrote after the first
    // import. The `source::` marker is what lets us refuse both.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "notes.md", "- one\n");

    let first = import(&mut ws, &hlc, &path);
    let before = render_page_md(&ws, first);

    let again = resolve_target(&ws, &path).unwrap();
    assert!(
        matches!(again, OpenWithTarget::Existing { page, .. } if page == first),
        "{again:?}",
    );
    // `import_into` on an Existing target is a no-op, not a second import.
    let contents = read_source(&path).unwrap();
    assert_eq!(
        import_into(&mut ws, &hlc, &again, &contents).unwrap(),
        first
    );
    assert_eq!(render_page_md(&ws, first), before);
}

#[test]
fn edits_made_after_an_import_survive_reopening_the_file() {
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "notes.md", "- imported\n");

    let page = import(&mut ws, &hlc, &path);
    crate::block::append_block(&mut ws, &hlc, Some(page), Some("mine")).unwrap();

    let contents = read_source(&path).unwrap();
    let again = resolve_target(&ws, &path).unwrap();
    import_into(&mut ws, &hlc, &again, &contents).unwrap();

    let outline = project_outline(&ws, page);
    let texts: Vec<&str> = outline.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, vec!["imported", "mine"]);
}

#[test]
fn two_different_files_with_the_same_name_get_separate_pages() {
    // Merging them would be silent data mixing — the failure mode the
    // `source::` marker exists to prevent.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a/notes.md", "- from a\n");
    let b = write(&dir, "b/notes.md", "- from b\n");

    let page_a = import(&mut ws, &hlc, &a);
    let target_b = resolve_target(&ws, &b).unwrap();
    assert_eq!(target_b.title(), "open-in/notes 2");
    let page_b = import(&mut ws, &hlc, &b);

    assert_ne!(page_a, page_b);
    let outline = project_outline(&ws, page_b);
    assert_eq!(outline[0].text, "from b");
    // And each still round-trips to itself on reopen.
    assert!(matches!(
        resolve_target(&ws, &a).unwrap(),
        OpenWithTarget::Existing { page, .. } if page == page_a
    ));
    assert!(matches!(
        resolve_target(&ws, &b).unwrap(),
        OpenWithTarget::Existing { page, .. } if page == page_b
    ));
}

#[test]
fn the_source_path_is_recorded_on_the_page() {
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "notes.md", "- one\n");

    let page = import(&mut ws, &hlc, &path);
    let recorded = read_text_prop(&ws, page, SOURCE_KEY).expect("source:: is set");
    assert_eq!(
        recorded,
        std::fs::canonicalize(&path).unwrap().to_string_lossy()
    );
}

#[test]
fn the_recorded_source_survives_the_file_moving_mid_import() {
    // `source::` used to be computed twice — once by `resolve_target`
    // to match against, once by `import_into` to write. A file that
    // moved between the two made `canonicalize` fail on the second
    // call, so the page recorded the *uncanonicalised* path while the
    // match had used the canonical one. The next open then failed to
    // recognise its own page and minted `open-in/notes 2`: the exact
    // duplicate this property exists to prevent. The target carries
    // the value now, so the two cannot disagree.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "notes.md", "- one\n");

    let contents = read_source(&path).unwrap();
    let target = resolve_target(&ws, &path).unwrap();
    let canonical = std::fs::canonicalize(&path).unwrap();
    std::fs::remove_file(&path).unwrap();

    let page = import_into(&mut ws, &hlc, &target, &contents).unwrap();
    assert_eq!(
        read_text_prop(&ws, page, SOURCE_KEY).as_deref(),
        canonical.to_str(),
    );

    // And the page still recognises itself once the file is back.
    std::fs::write(&path, "- one\n").unwrap();
    assert!(matches!(
        resolve_target(&ws, &path).unwrap(),
        OpenWithTarget::Existing { page: found, .. } if found == page
    ));
}

#[test]
fn an_empty_file_still_produces_a_page() {
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "empty.md", "   \n\n");

    let page = import(&mut ws, &hlc, &path);
    assert_eq!(
        crate::page::page_meta(&ws, page).unwrap().title,
        "open-in/empty"
    );
    assert!(project_outline(&ws, page).is_empty());
}

#[test]
fn a_non_latin_file_name_never_lands_on_the_namespace_page() {
    // `slugify` drops what it cannot fold to ASCII, so `open-in/会議メモ`
    // collapses to bare `open-in` — the parent page. Importing there
    // replaces the namespace index with one file's contents and hangs
    // every later import under it. For someone who names files in
    // their own script that is every file, not an edge case.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "会議メモ.md", "- 議題\n");

    let target = resolve_target(&ws, &path).unwrap();
    assert_eq!(
        target.title(),
        "open-in/会議メモ",
        "the title keeps the name"
    );
    assert_ne!(
        target.slug(),
        outl_md::slug::slugify(OPEN_WITH_NAMESPACE),
        "the slug must not be the namespace's own page",
    );

    let page = import(&mut ws, &hlc, &path);
    // The namespace page is untouched — it does not exist yet at all.
    assert!(find_by_slug(&ws, &outl_md::slug::slugify(OPEN_WITH_NAMESPACE)).is_none());
    assert_eq!(project_outline(&ws, page)[0].text, "議題");

    // A second non-Latin file gets its own page rather than colliding.
    let other = write(&dir, "Проект.md", "- план\n");
    let second = import(&mut ws, &hlc, &other);
    assert_ne!(second, page);
    assert_eq!(project_outline(&ws, second)[0].text, "план");
}

#[test]
fn an_unsupported_extension_is_refused_before_the_file_is_read() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "report.pdf", "%PDF-1.4");
    assert!(matches!(
        read_source(&path),
        Err(ActionError::UnsupportedExternalFile(_))
    ));
    assert!(!is_supported(&path));
    assert!(is_supported(std::path::Path::new("a.MD")));
    assert!(is_supported(std::path::Path::new("a.markdown")));
    assert!(is_supported(std::path::Path::new("a.txt")));
}

#[test]
fn a_binary_payload_with_a_text_extension_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("sneaky.txt");
    std::fs::write(&path, [0xff, 0xfe, 0x00, 0x9f]).unwrap();
    assert!(matches!(
        read_source(&path),
        Err(ActionError::ExternalFileNotText(_))
    ));
}

#[test]
fn a_file_with_no_usable_stem_still_gets_a_name() {
    let dir = TempDir::new().unwrap();
    let path = write(&dir, ".md", "- hi\n");
    // `.md` has stem `.md` on Rust's Path; what matters is that
    // resolution never yields an empty title the slug would reject.
    let (ws, _hlc) = ws();
    let target = resolve_target(&ws, &path).unwrap();
    assert!(target.title().starts_with("open-in/"));
    assert!(crate::page::is_valid_slug(target.slug()));
}
