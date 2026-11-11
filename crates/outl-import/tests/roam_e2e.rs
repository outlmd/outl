//! End-to-end: Roam JSON backup → workspace with real `((blk-XXXXXX))`
//! handles, embeds, collapsed ops, and a faithful report.

mod common;

use common::{import_with, import_with_opts, open_test_ws, read, TestWs};
use outl_import::adapters::RoamAdapter;
use outl_import::{dry_run, ImportOptions, ImportReport};
use std::fs;

const FIXTURE: &str = r#"[
    {
        "title": "Source",
        "children": [
            {"string": "the original decision", "uid": "src-uid", "children": [
                {"string": "supporting detail", "uid": "child-uid", "children": [], "open": false}
            ]}
        ]
    },
    {
        "title": "Referrer",
        "children": [
            {"string": "see ((src-uid)) please", "uid": "r1", "children": []},
            {"string": "context: {{[[embed]]: ((src-uid))}}", "uid": "r2", "children": []},
            {"string": "dangling ((nope-uid)) ref", "uid": "r3", "children": []},
            {"string": "Done!((src-uid))", "uid": "r4", "children": []},
            {"string": "negociar: {{embed: ((src-uid))}}\ncollapsed:: true\nid:: 6908fc01-aaaa", "uid": "r5", "children": []}
        ]
    },
    {
        "title": "May 25th, 2026",
        "children": [
            {"string": "{{[[TODO]]}} review #[[My Project]] __soon__", "uid": "j1", "children": []}
        ]
    }
]"#;

fn import_fixture(json: &str) -> (TestWs, ImportReport) {
    let src_dir = tempfile::tempdir().expect("src tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, json).expect("write fixture");
    import_with(&RoamAdapter, &src)
}

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
        referrer.contains("((unresolved:nope-uid))"),
        "unknown uid must stay greppable:\n{referrer}"
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
    assert!(journal.contains("- TODO review [[My Project]] *soon*"));
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
fn unmappable_page_placeholders_degrade_via_file_fallback() {
    // The second block's string embeds a `- ` line, which outl's parser
    // SPLITS into a child block — the page's block count diverges from
    // the renderer's walk, so the whole page is unmappable for handle
    // wiring. The ref on it must still degrade to a `[[Title]]` link
    // (file fallback), never survive as a literal placeholder.
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
    assert!(
        tricky.contains("see [[Source]] here"),
        "unmappable ref must degrade to a page link:\n{tricky}"
    );
    assert!(report.refs_page_fallback >= 1);
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
        page.contains("note:: ((unresolved:missing-uid))"),
        "unknown uid in a prop value stays greppable:\n{page}"
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

/// The report's own counters only prove what the pipeline *knows* it
/// emitted. The reconciliation adds the denominator, taken straight off
/// the source JSON, so a block lost anywhere in between shows up.
#[test]
fn reconciliation_balances_on_a_clean_import() {
    let (_ws, report) = import_fixture(FIXTURE);
    let r = report.reconciliation.expect("roam reports source counts");

    // 3 source pages → 3 files, nothing merged or skipped.
    assert_eq!(r.source_pages, 3);
    assert_eq!(r.emitted_pages, 3);
    assert_eq!(r.pages_skipped, 0);
    assert_eq!(r.pages_merged, 0);
    // 8 source blocks: 2 on Source, 5 on Referrer, 1 on the journal.
    assert_eq!(r.source_blocks, 8);
    assert_eq!(r.emitted_blocks, 8);
    assert_eq!(r.blocks_unaccounted, 0);
    assert!(r.balanced, "clean import must reconcile: {r:?}");
}

/// Every legitimate reducer (page skipped, block promoted to a page
/// property) is subtracted by name, so the books still close — the
/// unaccounted counters stay reserved for real silent loss.
#[test]
fn reconciliation_accounts_for_skipped_pages_and_lifted_props() {
    let json = r#"[
        {"title": "  ", "children": [
            {"string": "dropped", "uid": "d1", "children": [
                {"string": "dropped child", "uid": "d2", "children": []}
            ]}
        ]},
        {"title": "@Tonico", "children": [
            {"string": "icon:: 👤", "uid": "a1", "children": []},
            {"string": "the note", "uid": "n1", "children": []}
        ]},
        {"title": "May 25th, 2026", "children": [{"string": "x", "uid": "j1", "children": []}]},
        {"title": "2026-05-25", "children": [{"string": "y", "uid": "j2", "children": []}]}
    ]"#;
    let (_ws, report) = import_fixture(json);
    let r = report.reconciliation.expect("reconciliation present");

    assert_eq!(r.source_pages, 4);
    assert_eq!(r.emitted_pages, 2, "one skipped, two journals merged");
    assert_eq!(r.pages_skipped, 1);
    assert_eq!(r.pages_merged, 1);
    assert_eq!(r.pages_unaccounted, 0);

    assert_eq!(r.source_blocks, 6);
    assert_eq!(r.emitted_blocks, 3, "2 dropped with the page, 1 lifted");
    assert_eq!(r.blocks_skipped, 2);
    assert_eq!(r.blocks_lifted_to_props, 1);
    assert_eq!(r.blocks_unaccounted, 0);
    assert!(r.balanced, "explained losses must still balance: {r:?}");

    // The skipped page is named in the report, not silently gone.
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].blocks_dropped, 2);
}

/// Every counter except the landing ones is bumped in `render`, before
/// a byte hits disk — they prove the parser and the renderer agree, not
/// that the content is in a workspace. The landing numbers are summed
/// off the sidecars `reconcile_md` stamps from the materialized tree, so
/// they are the only ones that answer "did it actually arrive".
#[test]
fn the_report_confirms_the_blocks_reached_the_op_log() {
    let (_ws, report) = import_fixture(FIXTURE);
    assert!(report.landing_measured, "a real import measures landing");
    assert_eq!(report.landed_pages, 3);
    assert_eq!(
        report.landed_blocks, report.blocks,
        "every emitted block must be confirmed in the op log"
    );

    let r = report.reconciliation.expect("reconciliation present");
    assert!(r.landing_measured);
    assert_eq!(r.landed_blocks, 8);
    assert_eq!(r.blocks_not_landed, 0);
    assert!(r.balanced, "{r:?}");
}

/// A dry-run answers "what would I lose" — the reconciliation has to be
/// part of that answer, not only of the real run. But it writes nothing,
/// so it must say the landing was **not** measured rather than imply the
/// content is somewhere.
#[test]
fn dry_run_reports_the_reconciliation_too() {
    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, FIXTURE).expect("write fixture");

    let report = dry_run(&RoamAdapter, &src, &ImportOptions::default()).expect("dry run");
    let r = report.reconciliation.expect("reconciliation present");
    assert_eq!(r.source_blocks, 8);
    assert!(r.balanced, "{r:?}");
    assert!(
        !r.landing_measured,
        "a dry-run writes nothing — it cannot claim the blocks landed"
    );
    assert_eq!(r.landed_blocks, 0);
    assert_eq!(r.blocks_not_landed, 0);
}

/// The landing check has to notice a page that never reached the tree,
/// not repeat the renderer's optimism. Blocking one page's sidecar (a
/// directory sits where the file must go) makes `reconcile_md` fail for
/// exactly that page — the same shape as the write/reconcile failures
/// the real pipeline can hit at block 40k of 66k, where the in-memory
/// counters happily report success.
#[test]
fn a_page_that_never_reconciled_breaks_the_balance() {
    let mut ws = open_test_ws();
    let src_dir = tempfile::tempdir().expect("tempdir");
    let src = src_dir.path().join("backup.json");
    fs::write(&src, FIXTURE).expect("write fixture");
    // `Source` (2 blocks) can never get a sidecar written or read.
    fs::create_dir_all(ws.root.join("pages/source.outl")).expect("block the sidecar");

    let report = common::import_into(&RoamAdapter, &src, &mut ws, &ImportOptions::default());
    let r = report.reconciliation.expect("reconciliation present");

    assert_eq!(r.emitted_blocks, 8, "the renderer still emitted all 8");
    assert_eq!(r.blocks_unaccounted, 0, "parse → render still adds up");
    assert_eq!(
        r.blocks_not_landed, 2,
        "the two Source blocks are not confirmed anywhere: {r:?}"
    );
    assert!(
        !r.balanced,
        "content that never reached the op log must not report as balanced: {r:?}"
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains("NOT confirmed in the op log")),
        "the failure is named per page: {:?}",
        report.warnings
    );
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
        log.contains("shipped DONE the migration"),
        "literal text is kept:\n{log}"
    );
}
