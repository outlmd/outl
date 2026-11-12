//! End-to-end: Roam JSON backup → workspace with real `((blk-XXXXXX))`
//! handles, embeds, collapsed ops, and a faithful report.

mod common;

use common::roam::{import_fixture, FIXTURE};
use common::{import_with_opts, open_test_ws, read};
use outl_import::adapters::RoamAdapter;
use outl_import::{dry_run, ImportOptions};
use std::fs;

/// A remote image link in a Roam block is scanned into a remote asset
/// ref and, with `--no-assets` (which the harness sets so no download
/// hits the network), the original `![](url)` is kept verbatim.
///
/// The real remote-download path needs network access and is exercised
/// only manually; the ext-derivation + remote-scan units cover its
/// pure pieces (`emit::assets` + `adapters::asset_scan` tests).
#[test]
fn remote_image_without_download_keeps_original_link() {
    let json = r#"[
        {
            "title": "Gallery",
            "children": [
                {"string": "shot ![cover](https://firebasestorage.example/o/img.png?alt=media)", "uid": "g1", "children": []}
            ]
        }
    ]"#;
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");

    let opts = ImportOptions {
        import_assets: false,
        ..Default::default()
    };
    let (ws, report) = import_with_opts(&RoamAdapter, &src, &opts);

    let gallery = read(&ws.root.join("pages/gallery.md"));
    assert!(
        gallery.contains("![cover](https://firebasestorage.example/o/img.png?alt=media)"),
        "original remote link kept with --no-assets:\n{gallery}"
    );
    assert!(
        !gallery.contains("outl-import-asset:"),
        "placeholder must not survive:\n{gallery}"
    );
    // Nothing was pulled (assets off) — neither copied nor missing.
    assert_eq!(report.assets_copied, 0);
    assert_eq!(report.assets_missing, 0);
}

#[test]
fn refs_and_embeds_resolve_to_real_handles() {
    let (ws, report) = import_fixture(FIXTURE);

    let referrer = read(&ws.root.join("pages/referrer.md"));
    assert!(
        !referrer.contains("outl-import:"),
        "placeholders must not survive:\n{referrer}"
    );
    assert!(
        referrer.contains("see ((blk-"),
        "block ref not resolved to a handle:\n{referrer}"
    );
    assert!(
        referrer.contains("context: !((blk-"),
        "embed not resolved to a handle:\n{referrer}"
    );
    assert!(
        referrer.contains("dangling ((nope-uid)) ref"),
        "an unknown uid keeps its source text:\n{referrer}"
    );

    // The handle written in referrer.md is exactly the source block's
    // sidecar handle.
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/source.md"),
    ))
    .expect("source sidecar");
    let handle = &sc.blocks[0].ref_handle;
    assert!(
        referrer.contains(&format!("(({handle}))")),
        "referrer should point at {handle}:\n{referrer}"
    );

    // Regression: user text ending in `!` glued to a ref must stay a
    // REFERENCE — `!((blk-…))` is outl's embed syntax, so the resolve
    // pass separates them with a space instead of misclassifying.
    assert!(
        referrer.contains("Done! ((blk-"),
        "`!`-adjacent ref must not become an embed:\n{referrer}"
    );

    // Regression: a Roam block whose text carries embedded `key:: value`
    // lines (Logseq residue pasted into Roam) has those lines lifted
    // into block PROPERTIES by outl's parser — the stored text differs
    // from the rendered continuation lines. The resolve pass must
    // still hash-match (texts come from the same parser now) and
    // rewrite the embed instead of leaving the placeholder behind.
    assert!(
        referrer.contains("negociar: !((blk-"),
        "placeholder must resolve even when prop-like lines were lifted:\n{referrer}"
    );

    assert_eq!(report.refs_resolved, 2);
    assert_eq!(report.embeds_resolved, 2);
    assert_eq!(report.refs_unresolved, 1);
}

#[test]
fn collapsed_state_lands_in_the_op_log() {
    let (ws, report) = import_fixture(FIXTURE);
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/source.md"),
    ))
    .expect("source sidecar");
    // Depth-first: [0] = parent, [1] = "supporting detail" (open: false).
    assert!(ws.workspace.tree().is_collapsed(sc.blocks[1].id));
    // Two folds: `child-uid`'s `open: false`, plus `r5`'s inline
    // `collapsed:: true` line — a structural attribute the adapter now
    // maps to the fold flag instead of leaving it as a literal property.
    assert_eq!(report.collapsed_applied, 2);
}

#[test]
fn journals_and_dialect_translations_land() {
    let (ws, report) = import_fixture(FIXTURE);
    let journal = read(&ws.root.join("journals/2026-05-25.md"));
    assert!(journal.contains("- TODO review #[[My Project]] *soon*"));
    assert!(!journal.starts_with("title::"));
    assert_eq!(report.journals, 1);
    assert_eq!(report.pages, 2);
    assert_eq!(report.tasks.get("TODO"), Some(&1));

    let source = read(&ws.root.join("pages/source.md"));
    assert!(source.contains("title:: Source"));
}

#[test]
fn dry_run_writes_nothing_and_predicts_resolution() {
    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, FIXTURE).expect("write fixture");

    let report = dry_run(&RoamAdapter, &src, &ImportOptions::default()).expect("dry run");
    assert_eq!(report.refs_resolved, 2);
    assert_eq!(report.embeds_resolved, 2);
    assert_eq!(report.refs_unresolved, 1);
    assert_eq!(report.pages, 2);
    assert_eq!(report.journals, 1);
    assert_eq!(report.blocks, 8);
}

#[test]
fn unmappable_page_placeholders_resolve_via_file_fallback() {
    // A page whose blocks the resolve pass cannot wire one by one still
    // gets its markers rewritten by the file-fallback sweep, and a ref
    // whose *target* is mappable resolves to that block's real handle
    // instead of degrading to a `[[Title]]` link. No marker may survive.
    let json = r#"[
        {"title": "Source", "children": [
            {"string": "the original", "uid": "src-uid", "children": []}
        ]},
        {"title": "Tricky", "children": [
            {"string": "see ((src-uid)) here", "uid": "t1", "children": []},
            {"string": "pasted text\n- embedded bullet line", "uid": "t2", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let tricky = read(&ws.root.join("pages/tricky.md"));
    assert!(
        !tricky.contains("outl-import"),
        "no placeholder marker may survive on disk:\n{tricky}"
    );
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/source.md"),
    ))
    .expect("source sidecar");
    assert!(
        tricky.contains(&format!("see (({})) here", sc.blocks[0].ref_handle)),
        "a mappable target resolves to its handle:\n{tricky}"
    );
    assert_eq!(report.refs_page_fallback, 0);
}

/// A `((uid))` in a block property value (`note:: ((uid))` under prose,
/// lifted by outl's parser) resolves to the target's real handle. It
/// used to degrade to a `[[Title]]` page link.
#[test]
fn a_ref_in_a_property_value_resolves_to_the_block() {
    let json = r#"[
        {"title": "Target", "children": [
            {"string": "the referenced block", "uid": "tgt-uid", "children": []}
        ]},
        {"title": "Notes", "children": [
            {"string": "a highlight\nnote:: ((tgt-uid))", "uid": "n1", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let notes = read(&ws.root.join("pages/notes.md"));
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/target.md"),
    ))
    .expect("target sidecar");
    assert!(
        notes.contains(&format!("note:: (({}))", sc.blocks[0].ref_handle)),
        "notes:\n{notes}"
    );
    assert_eq!(report.refs_page_fallback, 0);
    assert_eq!(report.refs_resolved, 1);
}

#[test]
fn markers_in_prop_values_and_post_prop_lines_never_survive() {
    // The Omnivore-integration shape: a multiline quote block whose
    // embedded lines the parser lifts into block PROPERTIES. The
    // `((uid))` then lives in a prop VALUE, never in any block text, so
    // it is invisible to the block-level resolve path and only the
    // file-fallback sweep can erase the marker.
    //
    // The bare `((uid))` line after a prop line used to be the second
    // half of that story — it wasn't in the AST at all, so a *known*
    // uid degraded to a plain `[[Page]]` link even though outl could
    // have pointed at the exact block. It is in the AST now (issue #210
    // fixed the arm of `parse_block_list` that dropped it), which means
    // the ordinary resolve path reaches it and the reference survives
    // the import as a reference. Asserted below against the target's
    // real `ref_handle`, because "contains a `((blk-…))`" would pass on
    // a handle pointing anywhere.
    let json = r#"[
        {"title": "Target", "children": [
            {"string": "the referenced post", "uid": "LX2n3H5HX", "children": []}
        ]},
        {"title": "omnivore-saved", "children": [
            {"string": "> quote one [link](https://x.com) \n\nnote:: ((missing-uid))\ndate-highlighted:: [[2024-04-25]]", "uid": "g1", "children": []},
            {"string": "> quote two [link](https://y.com) \n\nnote:: esse post esta linkado com\n((LX2n3H5HX))", "uid": "u1", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let page = read(&ws.root.join("pages/omnivore-saved.md"));
    assert!(
        !page.contains("outl-import"),
        "no marker may survive, prop values included:\n{page}"
    );
    assert!(
        page.contains("note:: ((missing-uid))"),
        "an unknown uid in a prop value keeps its source text:\n{page}"
    );
    let handle = read(&ws.root.join("pages/target.outl"))
        .split("\"ref_handle\": \"")
        .nth(1)
        .and_then(|rest| rest.split('"').next().map(str::to_string))
        .expect("target sidecar records a ref handle");
    assert!(
        page.contains(&format!("(({handle}))")),
        "a known uid must resolve to the target block's own handle \
         ({handle}), not degrade to a page link:\n{page}"
    );
    assert_eq!(report.refs_unresolved, 1);
}

#[test]
fn roam_page_attributes_land_as_page_properties() {
    // The dominant real-world shape: a contact/company page whose head
    // is Roam attribute blocks (`icon::`, `page-type::`, `work::`).
    // These must reach the `.md` as page properties in the header — the
    // form outl's index reads for the sidebar icon, `page-type` filter,
    // and `@` mention autocomplete — never as stray text bullets.
    let json = r#"[
        {"title": "@Tonico", "children": [
            {"string": "icon:: 👤\npage-type:: contact\nwork:: [[buser]]", "uid": "a1", "children": []},
            {"string": "related:: [[triathlon]]", "uid": "a2", "children": []},
            {"string": "met at the conference", "uid": "n1", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let page = read(&ws.root.join("pages/tonico.md"));
    for line in [
        "icon:: 👤",
        "page-type:: contact",
        "work:: [[buser]]",
        "related:: [[triathlon]]",
    ] {
        assert!(
            page.contains(line),
            "missing page property `{line}`:\n{page}"
        );
    }
    assert!(
        !page.contains("- icon::") && !page.contains("- page-type::"),
        "attribute lines must be page properties, not text bullets:\n{page}"
    );
    assert!(
        page.contains("- met at the conference"),
        "real note dropped:\n{page}"
    );
    assert!(
        report.props_pages >= 4,
        "page props not counted: {}",
        report.props_pages
    );
}

#[test]
fn namespaced_page_keeps_title_and_flattens_slug() {
    // A Roam page named `buser/tech/data` (1000+ of these in a real
    // graph): the title must survive verbatim with its slashes, while
    // the on-disk slug flattens to a single filesystem-safe component.
    // A ref to it keeps the namespaced form and resolves via the slug.
    let json = r#"[
        {"title": "buser/tech/data", "children": [{"string": "the note", "uid": "n1", "children": []}]},
        {"title": "elsewhere", "children": [{"string": "see [[buser/tech/data]] here", "uid": "r1", "children": []}]}
    ]"#;
    let (ws, _report) = import_fixture(json);
    let page = read(&ws.root.join("pages/buser-tech-data.md"));
    assert!(
        page.contains("title:: buser/tech/data"),
        "namespaced title must survive verbatim:\n{page}"
    );
    let referrer = read(&ws.root.join("pages/elsewhere.md"));
    assert!(
        referrer.contains("[[buser/tech/data]]"),
        "namespaced ref must stay verbatim (resolves via slug):\n{referrer}"
    );
}

#[test]
fn timestamps_dropped_by_default_kept_on_opt_in() {
    let json = r#"[
        {"title": "Stamped", "children": [
            {"string": "x", "uid": "s1", "children": [], "create-time": 1700000000000}
        ]}
    ]"#;
    let (_ws, report) = import_fixture(json);
    assert_eq!(report.timestamps_dropped, 1);

    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    let opts = ImportOptions {
        preserve_timestamps: true,
        ..Default::default()
    };
    let (ws, report) = import_with_opts(&RoamAdapter, &src, &opts);
    assert_eq!(report.timestamps_dropped, 0);
    let page = read(&ws.root.join("pages/stamped.md"));
    assert!(page.contains("created:: 2023-11-14T"), "page:\n{page}");

    let _ = open_test_ws(); // keep harness helpers exercised
}

/// Roam stamps pages too, not only blocks. Those stamps used to be
/// dropped at deserialization, so every imported page lost its
/// creation date even with `--preserve-timestamps`.
#[test]
fn page_timestamps_land_as_page_properties_on_opt_in() {
    let json = r#"[
        {"title": "Stamped Page", "create-time": 1700000000000, "edit-time": 1700000500000,
         "children": [{"string": "x", "uid": "p1", "children": []}]},
        {"title": "With Own Attr", "create-time": 1700000000000,
         "children": [{"string": "created:: by hand", "uid": "p2", "children": []},
                      {"string": "y", "uid": "p3", "children": []}]}
    ]"#;
    let (_ws, report) = import_fixture(json);
    assert_eq!(report.timestamps_dropped, 2, "one per stamped page");

    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    let opts = ImportOptions {
        preserve_timestamps: true,
        ..Default::default()
    };
    let (ws, report) = import_with_opts(&RoamAdapter, &src, &opts);
    assert_eq!(report.timestamps_dropped, 0);
    let page = read(&ws.root.join("pages/stamped-page.md"));
    let header: Vec<&str> = page.lines().take_while(|l| !l.is_empty()).collect();
    assert!(
        header
            .iter()
            .any(|l| l.starts_with("created:: 2023-11-14T22:13:20")),
        "page:\n{page}"
    );
    assert!(
        header
            .iter()
            .any(|l| l.starts_with("edited:: 2023-11-14T22:21:40")),
        "page:\n{page}"
    );
    // A source attribute of the same name is what the user wrote: it
    // stays, and the stamp does not duplicate the key.
    let own = read(&ws.root.join("pages/with-own-attr.md"));
    assert_eq!(own.matches("created::").count(), 1, "page:\n{own}");
    assert!(own.contains("created:: by hand"), "page:\n{own}");
}

/// Mid-block `{{[[DONE]]}}` keeps the word but loses the task state.
/// The count and the aggregate warning are the only trace the user
/// gets — they must survive a full import, not just the parse.
#[test]
fn midtext_task_markers_surface_in_the_final_report() {
    let json = r#"[
        {"title": "Log", "children": [
            {"string": "shipped {{[[DONE]]}} the migration", "uid": "l1", "children": []},
            {"string": "{{[[DONE]]}} a real task", "uid": "l2", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);

    assert_eq!(report.tasks_midtext_literal, 1);
    assert_eq!(report.tasks.get("DONE"), Some(&1), "head marker still wins");
    assert_eq!(
        report
            .warnings
            .iter()
            .filter(|w| w.detail.contains("mid-block TODO/DONE"))
            .count(),
        1
    );
    let log = read(&ws.root.join("pages/log.md"));
    assert!(
        log.contains("shipped {{[[DONE]]}} the migration"),
        "the marker is kept as written:\n{log}"
    );
}

/// A Roam block whose text carries a `- item` line, and a fence that one
/// block opens and a later block closes. Both used to change the block
/// count on the way through outl's parser (one block in, two back; three
/// in, one back), which broke the 1:1 mapping the resolve pass rests on:
/// every ref into the page degraded to a `[[link]]` and its folds were
/// lost.
#[test]
fn bullet_lines_and_cross_block_fences_keep_the_page_mappable() {
    let json = r#"[
        {"title": "Notes", "children": [
            {"string": "head\n- item one\n- item two", "uid": "n1", "children": [], "open": false},
            {"string": "config:\n\n```yaml", "uid": "n2", "children": [
                {"string": "a: 1", "uid": "n3", "children": []},
                {"string": "b: 2\n```", "uid": "n4", "children": []}
            ]},
            {"string": "after", "uid": "n5", "children": []}
        ]},
        {"title": "Refs", "children": [
            {"string": "see ((n1)) and ((n4)) and ((n5))", "uid": "r1", "children": []},
            {"string": "{{[[embed]]: ((n3))}}", "uid": "r2", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);

    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/notes.md"),
    ))
    .expect("notes sidecar");
    let texts: Vec<&str> = sc.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "head\n- item one\n- item two",
            "config:\n\n```yaml",
            "a: 1",
            "b: 2\n```",
            "after"
        ]
    );
    assert_eq!(report.landed_blocks, report.blocks);

    let refs = read(&ws.root.join("pages/refs.md"));
    assert_eq!(report.refs_page_fallback, 0, "refs:\n{refs}");
    assert_eq!(report.embeds_page_fallback, 0, "refs:\n{refs}");
    assert_eq!(report.refs_resolved, 3, "refs:\n{refs}");
    assert_eq!(report.embeds_resolved, 1, "refs:\n{refs}");
    assert!(
        refs.contains(&format!("(({}))", sc.blocks[0].ref_handle)),
        "refs:\n{refs}"
    );
    assert!(ws.workspace.tree().is_collapsed(sc.blocks[0].id));
}
