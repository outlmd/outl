//! End-to-end: Obsidian vault → workspace. Ports every scenario the
//! legacy string-pipeline importer covered, now through the adapter.

mod common;

use common::{fixture_tree, import_into, import_with, open_test_ws, read};
use outl_import::adapters::ObsidianAdapter;
use outl_import::ImportOptions;

#[test]
fn basic_page_with_bullets_round_trips() {
    let v = fixture_tree(&[("Note.md", "- first\n- second\n  - child\n")]);
    let (ws, report) = import_with(&ObsidianAdapter, v.path());
    let out = read(&ws.root.join("pages/note.md"));
    assert!(out.contains("title:: Note"));
    assert!(out.contains("- first"));
    assert!(out.contains("  - child"));
    assert_eq!(report.pages, 1);
}

#[test]
fn iso_filename_routes_to_journals_regardless_of_folder() {
    let v = fixture_tree(&[
        ("2026-05-25.md", "- at root\n"),
        ("Daily/2026-05-26.md", "- in a folder\n"),
    ]);
    let (ws, report) = import_with(&ObsidianAdapter, v.path());
    assert!(ws.root.join("journals/2026-05-25.md").exists());
    let in_folder = read(&ws.root.join("journals/2026-05-26.md"));
    assert!(in_folder.contains("- in a folder"));
    assert!(!in_folder.contains("path::"), "journals get no path::");
    assert_eq!(report.journals, 2);
}

#[test]
fn non_date_file_in_daily_folder_stays_a_page() {
    let v = fixture_tree(&[("Daily/Sprint kickoff.md", "- notes\n")]);
    let (ws, _) = import_with(&ObsidianAdapter, v.path());
    let out = read(&ws.root.join("pages/sprint-kickoff.md"));
    assert!(out.contains("title:: Sprint kickoff"));
    assert!(out.contains("path:: Daily"));
}

#[test]
fn skips_obsidian_and_trash_dirs() {
    let v = fixture_tree(&[
        ("Real.md", "- content\n"),
        (".obsidian/app.json", "{}"),
        (".obsidian/plugins/x.md", "- app metadata\n"),
        (".trash/Deleted.md", "- gone\n"),
    ]);
    let (ws, report) = import_with(&ObsidianAdapter, v.path());
    assert_eq!(report.pages, 1);
    assert!(ws.root.join("pages/real.md").exists());
    assert!(!ws.root.join("pages/deleted.md").exists());
}

#[test]
fn nested_folder_emits_path_property_root_does_not() {
    let v = fixture_tree(&[
        ("Projects/Ideas/Note.md", "- nested\n"),
        ("Root.md", "- at root\n"),
    ]);
    let (ws, _) = import_with(&ObsidianAdapter, v.path());
    let nested = read(&ws.root.join("pages/note.md"));
    assert!(nested.contains("path:: Projects/Ideas"));
    let root = read(&ws.root.join("pages/root.md"));
    assert!(!root.contains("path::"));
}

#[test]
fn wikilink_variants_collapse() {
    let v = fixture_tree(&[(
        "Links.md",
        "- see [[Note|the alias]] and [[Other#section]] and [[folder/Deep]]\n",
    )]);
    let (ws, _) = import_with(&ObsidianAdapter, v.path());
    let out = read(&ws.root.join("pages/links.md"));
    assert!(out.contains("[[Note]]"), "alias stripped:\n{out}");
    assert!(out.contains("[[Other]]"), "heading stripped:\n{out}");
    assert!(out.contains("[[Deep]]"), "folder prefix stripped:\n{out}");
    assert!(!out.contains("the alias"));
}

#[test]
fn note_embeds_keep_shape_image_embeds_become_md_links() {
    let v = fixture_tree(&[
        (
            "Media.md",
            "- embed ![[Other Note]]\n- image ![[assets/pic.png]]\n",
        ),
        ("assets/pic.png", "PNG fake bytes"),
    ]);
    let (ws, report) = import_with(&ObsidianAdapter, v.path());
    let out = read(&ws.root.join("pages/media.md"));
    assert!(out.contains("![[Other Note]]"), "note embed kept:\n{out}");
    // The image embed's file was pulled into the workspace and its link
    // rewritten to the content-addressed path as an `![]` image embed.
    assert!(
        !out.contains("outl-import-asset:") && !out.contains("assets/pic.png"),
        "image link not content-addressed:\n{out}"
    );
    assert!(
        out.contains("![pic.png](assets/"),
        "no image embed emitted:\n{out}"
    );
    assert_eq!(report.assets_copied, 1);
    assert_eq!(report.assets_missing, 0);
    let copied: Vec<_> = std::fs::read_dir(ws.root.join("assets"))
        .expect("assets dir")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("png"))
        .collect();
    assert_eq!(copied.len(), 1, "exactly one png copied");
}

#[test]
fn colliding_titles_get_path_derived_suffixes() {
    let v = fixture_tree(&[
        ("Ideas.md", "- root ideas\n"),
        ("Docs/Ideas.md", "- docs ideas\n"),
    ]);
    let (ws, report) = import_with(&ObsidianAdapter, v.path());
    // Lex-smallest relative path wins the bare slug: `Docs/Ideas.md`
    // sorts before `Ideas.md`.
    let bare = read(&ws.root.join("pages/ideas.md"));
    assert!(
        bare.contains("- docs ideas"),
        "lex-smallest path wins:\n{bare}"
    );
    let suffixed = read(&ws.root.join("pages/ideas-ideas.md"));
    assert!(
        suffixed.contains("- root ideas"),
        "collider suffixed:\n{suffixed}"
    );
    assert_eq!(report.slug_collisions, 1);
}

#[test]
fn frontmatter_props_title_and_date_normalize() {
    let v = fixture_tree(&[(
        "Meta.md",
        "---\ntitle: Real Title\ntags: [alpha, beta]\ndate: May 25th, 2026\naliases: [x]\n---\n\n- body\n",
    )]);
    let (ws, report) = import_with(&ObsidianAdapter, v.path());
    let out = read(&ws.root.join("pages/real-title.md"));
    assert!(out.contains("title:: Real Title"));
    assert!(out.contains("#alpha"), "tags normalized:\n{out}");
    assert!(out.contains("date:: 2026-05-25"), "date normalized:\n{out}");
    assert!(!out.contains("aliases"), "dropped key:\n{out}");
    assert!(report.artifacts_stripped >= 1, "dropped keys counted");
}

#[test]
fn malformed_frontmatter_is_restored_verbatim() {
    let v = fixture_tree(&[("Broken.md", "---\ntitle: [unclosed\n---\n\n- body\n")]);
    let (ws, report) = import_with(&ObsidianAdapter, v.path());
    let out = read(&ws.root.join("pages/broken.md"));
    assert!(out.contains("- body"));
    assert!(
        out.contains("[unclosed"),
        "malformed YAML must not vanish:\n{out}"
    );
    assert!(report
        .warnings
        .iter()
        .any(|w| w.detail.contains("frontmatter")));
}

#[test]
fn leading_h1_becomes_title_but_frontmatter_wins() {
    let v = fixture_tree(&[
        ("h1-only.md", "# From The H1\n\n- content\n"),
        (
            "both.md",
            "---\ntitle: From Frontmatter\n---\n\n# Stays In Body\n\n- content\n",
        ),
    ]);
    let (ws, _) = import_with(&ObsidianAdapter, v.path());

    let h1 = read(&ws.root.join("pages/from-the-h1.md"));
    assert!(h1.contains("title:: From The H1"));
    assert!(!h1.contains("# From The H1"), "used H1 is stripped:\n{h1}");

    let both = read(&ws.root.join("pages/from-frontmatter.md"));
    assert!(both.contains("title:: From Frontmatter"));
    assert!(both.contains("# Stays In Body"), "unused H1 stays:\n{both}");
}

#[test]
fn reimport_into_same_destination_is_idempotent() {
    let v = fixture_tree(&[("Note.md", "- alpha\n- beta\n")]);
    let (mut ws, _) = import_with(&ObsidianAdapter, v.path());
    let first = read(&ws.root.join("pages/note.md"));
    let _ = import_into(
        &ObsidianAdapter,
        v.path(),
        &mut ws,
        &ImportOptions::default(),
    );
    let second = read(&ws.root.join("pages/note.md"));
    assert_eq!(first, second, "reimport must not mutate the page");
}

#[test]
fn auto_detect_shapes() {
    use outl_import::SourceAdapter;
    let v = fixture_tree(&[(".obsidian/app.json", "{}"), ("Note.md", "- x\n")]);
    assert!(outl_import::adapters::ObsidianAdapter::detect(v.path()));
    assert!(!outl_import::adapters::LogseqAdapter::detect(v.path()));

    let l = fixture_tree(&[("pages/a.md", "- x\n"), ("journals/2026_01_01.md", "- y\n")]);
    assert!(outl_import::adapters::LogseqAdapter::detect(l.path()));
    assert!(!outl_import::adapters::ObsidianAdapter::detect(l.path()));

    let _ = open_test_ws(); // keep harness helpers exercised
}

/// Issue #299: a vault document the importer cannot read has to be
/// named, and the page denominator has to exist so the reconciliation
/// runs at all. Both used to be lost to a bare `continue`.
#[test]
fn unsupported_vault_documents_are_named_and_the_books_balance() {
    let v = fixture_tree(&[
        (".obsidian/app.json", "{}"),
        ("Note.md", "# Note\n\nbody\n"),
        ("board.canvas", "{\"nodes\":[]}"),
        ("sub/deep.base", "filters: []\n"),
        ("attachments/pic.png", "PNG"),
    ]);
    let (_ws, report) = import_with(&ObsidianAdapter, v.path());

    assert_eq!(report.source_pages, 3, "one note + two documents");
    let skipped: Vec<&str> = report.skipped.iter().map(|s| s.path.as_str()).collect();
    assert!(
        skipped.iter().any(|p| p.ends_with("board.canvas")),
        "{skipped:?}"
    );
    assert!(
        skipped.iter().any(|p| p.ends_with("deep.base")),
        "{skipped:?}"
    );

    // An attachment is not a page, so it stays out of the denominator
    // and is reported once, in aggregate.
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains("not read, and not notes") && w.detail.contains("1 .png")),
        "{:?}",
        report.warnings
    );

    let rec = report.reconciliation.expect("reconciliation runs now");
    assert_eq!(rec.pages_skipped, 2);
    assert_eq!(rec.pages_unaccounted, 0);
    assert!(
        !rec.source_blocks_measured,
        "a vault is free markdown — the page is its unit"
    );
    assert!(rec.balanced, "{rec:?}");
}

/// The same contract as the Logseq suite's `no_imported_page_is_born_stale`:
/// what the import writes has to be what the op log projects.
#[test]
fn no_imported_page_is_born_stale() {
    let v = fixture_tree(&[
        (
            "Note.md",
            "---\ntitle: My Note\ntags: [dev, x]\nstatus: active\n---\n\n- a bullet\n- another\n",
        ),
        ("Plain.md", "# Plain\n\nsome prose\n"),
    ]);
    let (ws, _) = import_with(&ObsidianAdapter, v.path());
    for meta in outl_actions::page::list_all(&ws.workspace) {
        let path = outl_actions::journal::page_md_path(&ws.root, &meta);
        if !path.exists() {
            continue;
        }
        let id = outl_actions::page::find_by_slug(&ws.workspace, &meta.slug).expect("page node");
        assert_eq!(
            read(&path),
            outl_actions::journal::render_page_md(&ws.workspace, id),
            "{} is stale on arrival",
            path.display()
        );
    }
}
