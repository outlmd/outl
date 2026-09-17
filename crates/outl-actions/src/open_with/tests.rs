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
    import_into(ws, hlc, &target, &contents).unwrap().page
}

/// Today's journal as a list of block texts.
fn root() -> &'static std::path::Path {
    std::path::Path::new("/tmp/outl-open-with-test")
}

fn journal_lines(ws: &Workspace) -> Vec<String> {
    let slug = crate::dates::journal_slug(crate::page::today());
    match find_by_slug(ws, &slug) {
        None => Vec::new(),
        Some(journal) => project_outline(ws, journal)
            .iter()
            .map(|b| b.text.clone())
            .collect(),
    }
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
        import_into(&mut ws, &hlc, &again, &contents).unwrap().page,
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

    let page = import_into(&mut ws, &hlc, &target, &contents).unwrap().page;
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
fn an_imported_page_is_linked_from_todays_journal() {
    // outl is journal-first. A page reachable only by search is a page
    // the user forgets they have, so the import leaves a trail on the
    // day it happened.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "notes.md", "- one\n");

    assert!(journal_lines(&ws).is_empty(), "nothing before the import");
    let page = import(&mut ws, &hlc, &path);
    assert_eq!(journal_lines(&ws), vec!["[[open-in/notes]]".to_string()]);

    // It is a ref, not plain text, so the page can answer where it came
    // from without anyone remembering which day's journal to scroll.
    let meta = crate::page::page_meta(&ws, page).unwrap();
    let links = crate::backlinks::backlinks_for_page(&ws, root(), &meta);
    assert_eq!(links.len(), 1, "the journal entry is a backlink");
    assert_eq!(
        links[0].source_page.as_ref().map(|p| p.slug.as_str()),
        Some(crate::dates::journal_slug(crate::page::today()).as_str()),
    );
}

#[test]
fn the_journal_link_opens_the_page_it_names() {
    // The link is only worth writing if it lands somewhere. For a stem
    // that survives slugify this is trivially true; for one that does
    // not, `slug_for` gave the page a slug the title no longer derives,
    // and `resolve_or_create_by_name` tries `slugify(name)` before an
    // exact title match, so `[[open-in/会議メモ]]` resolved to the
    // namespace page `open-in` once that page existed.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let ascii = write(&dir, "notes.md", "- one\n");
    let other = write(&dir, "会議メモ.md", "- 議題\n");

    let ascii_page = import(&mut ws, &hlc, &ascii);
    let other_page = import(&mut ws, &hlc, &other);

    // The namespace index page exists the moment anyone opens it, which
    // is the whole point of the namespace.
    crate::resolve::open_or_create_by_name(
        &mut ws,
        &hlc,
        OPEN_WITH_NAMESPACE,
        crate::page::PageKind::Page,
    )
    .unwrap();

    for (line, expected) in journal_lines(&ws).iter().zip([ascii_page, other_page]) {
        let target = line.trim_start_matches("[[").trim_end_matches("]]");
        let landed = crate::resolve::open_or_create_by_ref(&mut ws, &hlc, target).unwrap();
        assert_eq!(landed, expected, "`{line}` opened the wrong page");
    }
}

#[test]
fn reopening_a_file_does_not_add_a_second_journal_entry() {
    // Re-opening resolves to `Existing` and imports nothing, so the
    // journal must not collect one line per time the user opened the
    // same file.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "notes.md", "- one\n");

    import(&mut ws, &hlc, &path);
    let after_first = journal_lines(&ws);

    let contents = read_source(&path).unwrap();
    for _ in 0..3 {
        let again = resolve_target(&ws, &path).unwrap();
        let outcome = import_into(&mut ws, &hlc, &again, &contents).unwrap();
        assert_eq!(outcome.journal, None, "an existing target writes nothing");
    }
    assert_eq!(journal_lines(&ws), after_first);
}

#[test]
fn a_stale_target_for_the_same_file_lands_on_the_page_that_won() {
    // A client resolves under a read lock and imports under a mutation
    // lock. Two deliveries of one file can both resolve `New` for the
    // same slug before either commits; the second must behave like an
    // `Existing` target rather than paste on top of the first's page
    // and link the journal twice.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "notes.md", "- one\n");
    let contents = read_source(&path).unwrap();

    let first = resolve_target(&ws, &path).unwrap();
    let second = resolve_target(&ws, &path).unwrap();
    assert_eq!(first, second);
    assert!(matches!(first, OpenWithTarget::New { .. }));

    let page = import_into(&mut ws, &hlc, &first, &contents).unwrap().page;
    let before = render_page_md(&ws, page);
    let after_first = journal_lines(&ws);

    let outcome = import_into(&mut ws, &hlc, &second, &contents).unwrap();
    assert_eq!(outcome.page, page);
    assert_eq!(
        outcome.journal, None,
        "a stale same-file target writes nothing"
    );
    assert_eq!(render_page_md(&ws, page), before);
    assert_eq!(journal_lines(&ws), after_first);
}

#[test]
fn a_stale_target_for_a_different_file_is_refused_not_merged() {
    // Same race, two *different* files named alike. The slug the second
    // resolved against now belongs to the first's page, and importing
    // there would merge the two files. `import_into` cannot pick the
    // next free slug itself (the caller already committed to this
    // target's page id), so it refuses and the caller resolves again.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a/notes.md", "- from a\n");
    let b = write(&dir, "b/notes.md", "- from b\n");

    let target_a = resolve_target(&ws, &a).unwrap();
    let target_b = resolve_target(&ws, &b).unwrap();
    assert_eq!(target_a.slug(), target_b.slug());

    let page_a = import_into(&mut ws, &hlc, &target_a, &read_source(&a).unwrap())
        .unwrap()
        .page;
    let before = render_page_md(&ws, page_a);
    let after_first = journal_lines(&ws);

    let refused = import_into(&mut ws, &hlc, &target_b, &read_source(&b).unwrap());
    assert!(
        matches!(refused, Err(ActionError::ExternalFileTargetTaken(ref slug)) if slug == target_b.slug()),
        "{refused:?}"
    );
    assert_eq!(render_page_md(&ws, page_a), before);
    assert_eq!(journal_lines(&ws), after_first);

    // Resolving again lands where a sequential second open would.
    let retried = resolve_target(&ws, &b).unwrap();
    assert_eq!(retried.title(), "open-in/notes 2");
    assert!(matches!(retried, OpenWithTarget::New { .. }));
}

#[test]
fn two_imports_on_one_day_both_show_up() {
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "notes.md", "- one\n");
    let b = write(&dir, "plan.txt", "two\n");

    import(&mut ws, &hlc, &a);
    import(&mut ws, &hlc, &b);
    assert_eq!(
        journal_lines(&ws),
        vec![
            "[[open-in/notes]]".to_string(),
            "[[open-in/plan]]".to_string()
        ],
    );
}

#[test]
fn the_source_path_never_reaches_the_markdown() {
    // The value is the user's directory structure. A `.md` is often in
    // git, and `outl export hugo` copies every property outside its
    // deny-list into published front matter, so a visible `source::`
    // put private paths on a public site.
    let (mut ws, hlc) = ws();
    let dir = TempDir::new().unwrap();
    let path = write(&dir, "notes.md", "- one\n");

    let page = import(&mut ws, &hlc, &path);
    let md = render_page_md(&ws, page);
    let canonical = std::fs::canonicalize(&path).unwrap();
    assert!(
        !md.contains(canonical.to_str().unwrap()),
        "the absolute path is in the .md:\n{md}",
    );
    assert!(!md.contains(SOURCE_KEY), "the key is in the .md:\n{md}");

    // Hidden, not lost: the op log still has it, which is what the
    // re-open check reads.
    assert_eq!(
        read_text_prop(&ws, page, SOURCE_KEY).as_deref(),
        canonical.to_str(),
    );
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
