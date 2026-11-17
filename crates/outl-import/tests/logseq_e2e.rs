//! End-to-end: Logseq graph dir → workspace with real handles, typed
//! props, collapsed ops, and journal routing.

mod common;

use common::{fixture_tree, import_with, read};
use outl_import::adapters::LogseqAdapter;

fn graph() -> tempfile::TempDir {
    fixture_tree(&[
        (
            "pages/source.md",
            "- the original block\n  id:: 6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90\n  \
             - folded child\n    collapsed:: true\n",
        ),
        (
            "pages/referrer.md",
            "- see ((6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90)) for context\n  note:: keep-me\n\
             - inline {{embed ((6601a2c1-4f31-4a45-1c2c-3a5e6b7d8f90))}}\n\
             - dangling ((deadbeef-0000-0000-0000-000000000000))\n",
        ),
        (
            "pages/tasks.md",
            "type:: project\n\n\
             - TODO buy milk\n\
             - DOING [#A] write spec\n  SCHEDULED: <2026-11-10 Tue>\n\
             - DONE ship\n  :LOGBOOK:\n  CLOCK: [2026-05-01]--[2026-05-01] => 01:00\n  :END:\n",
        ),
        (
            "journals/2026_05_25.md",
            "- morning thought #[[My Project]]\n",
        ),
        ("pages/meu___projeto.md", "- decoded name test\n"),
    ])
}

#[test]
fn refs_and_embeds_resolve_to_real_handles() {
    let g = graph();
    let (ws, report) = import_with(&LogseqAdapter, g.path());

    let referrer = read(&ws.root.join("pages/referrer.md"));
    assert!(
        !referrer.contains("outl-import:"),
        "no placeholders:\n{referrer}"
    );
    assert!(
        referrer.contains("see ((blk-"),
        "ref not resolved:\n{referrer}"
    );
    assert!(
        referrer.contains("inline !((blk-"),
        "embed not resolved:\n{referrer}"
    );
    assert!(
        referrer.contains("((deadbeef-0000-0000-0000-000000000000))"),
        "an unknown uid keeps its source text:\n{referrer}"
    );
    assert_eq!(report.refs_resolved, 1);
    assert_eq!(report.embeds_resolved, 1);
    assert_eq!(report.refs_unresolved, 1);

    // Regression: the resolve pass re-renders this page through
    // `apply_page_md_with_sidecar`; block properties must survive the
    // op → md projection (build_outline used to drop them).
    assert!(
        referrer.contains("note:: keep-me"),
        "block prop lost on re-render:\n{referrer}"
    );
}

#[test]
fn collapsed_lands_in_the_op_log_and_id_lines_vanish() {
    let g = graph();
    let (ws, _) = import_with(&LogseqAdapter, g.path());

    let source = read(&ws.root.join("pages/source.md"));
    assert!(!source.contains("id::"), "id:: must be stripped:\n{source}");
    assert!(
        !source.contains("collapsed::"),
        "collapsed:: is an op, not text:\n{source}"
    );

    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/source.md"),
    ))
    .expect("sidecar");
    // DFS: [0] parent, [1] folded child.
    assert!(ws.workspace.tree().is_collapsed(sc.blocks[1].id));
}

#[test]
fn task_states_props_and_org_dates_translate() {
    let g = graph();
    let (ws, report) = import_with(&LogseqAdapter, g.path());

    let tasks = read(&ws.root.join("pages/tasks.md"));
    assert!(tasks.contains("title:: tasks"));
    assert!(tasks.contains("type:: project"), "page prop:\n{tasks}");
    assert!(tasks.contains("- TODO buy milk"));
    assert!(
        tasks.contains("- DOING write spec [[2026-11-10]]"),
        "DOING → DOING + date link:\n{tasks}"
    );
    // The `state:: doing` property existed only because the prefix
    // could not say it. Now that it can, a property saying the same
    // thing is a second copy that queries and the toggle would drift
    // away from.
    assert!(
        !tasks.contains("state:: doing"),
        "DOING is the prefix now, not a property:\n{tasks}"
    );
    assert!(tasks.contains("priority:: A"), "priority prop:\n{tasks}");
    assert!(tasks.contains("- DONE ship"));
    assert!(!tasks.contains(":LOGBOOK:"), "logbook dropped:\n{tasks}");
    assert_eq!(report.tasks.get("DOING"), Some(&1));
    assert_eq!(report.org_dates_converted, 1);
}

#[test]
fn journals_route_to_iso_and_filenames_decode() {
    let g = graph();
    let (ws, report) = import_with(&LogseqAdapter, g.path());

    let journal = read(&ws.root.join("journals/2026-05-25.md"));
    assert!(journal.contains("- morning thought [[My Project]]"));
    assert!(!journal.starts_with("title::"));
    assert_eq!(report.journals, 1);

    // `___` is Logseq's namespace separator, so the title keeps the
    // slash the user typed (`[[meu/projeto]]`) while the slug stays
    // flat — reading it as a space renamed every namespaced page.
    let decoded = read(&ws.root.join("pages/meu-projeto.md"));
    assert!(
        decoded.contains("title:: meu/projeto"),
        "decoded:\n{decoded}"
    );
}

#[test]
fn local_asset_is_copied_content_addressed_and_linked() {
    let g = fixture_tree(&[
        (
            "pages/media.md",
            "- here is ![my pic](../assets/pic.png) inline\n",
        ),
        ("assets/pic.png", "PNG fake bytes"),
    ]);
    let (ws, report) = import_with(&LogseqAdapter, g.path());

    let media = read(&ws.root.join("pages/media.md"));
    // Rewritten to a content-addressed image embed — no leftover placeholder.
    assert!(
        !media.contains("outl-import-asset:"),
        "placeholder survived:\n{media}"
    );
    assert!(
        media.contains("![pic.png](assets/") && !media.contains("assets/pic.png"),
        "not a content-addressed image embed:\n{media}"
    );
    assert_eq!(report.assets_copied, 1);
    assert_eq!(report.assets_missing, 0);

    // The copied file exists under the workspace `assets/` dir.
    let assets = std::fs::read_dir(ws.root.join("assets")).expect("assets dir");
    let copied: Vec<_> = assets
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("png"))
        .collect();
    assert_eq!(copied.len(), 1, "exactly one png copied");
}

#[test]
fn missing_local_asset_keeps_original_link() {
    let g = fixture_tree(&[(
        "pages/broken.md",
        "- see ![gone](../assets/missing.png) here\n",
    )]);
    let (ws, report) = import_with(&LogseqAdapter, g.path());

    let broken = read(&ws.root.join("pages/broken.md"));
    assert!(
        broken.contains("![gone](../assets/missing.png)"),
        "original link not preserved:\n{broken}"
    );
    assert_eq!(report.assets_copied, 0);
    assert_eq!(report.assets_missing, 1);
}

#[test]
fn org_files_are_counted_as_skipped() {
    let g = fixture_tree(&[
        ("pages/keep.md", "- kept\n"),
        ("pages/old.org", "* org heading\n"),
    ]);
    let (_ws, report) = import_with(&LogseqAdapter, g.path());
    assert_eq!(report.pages, 1);
    assert!(report.skipped.iter().any(|s| s.path.ends_with("old.org")));
}

/// Issue #299: the Logseq adapter reported no denominator at all, so
/// the one report block that answers "did everything arrive" never ran.
#[test]
fn the_reconciliation_runs_and_balances() {
    let g = fixture_tree(&[
        ("logseq/config.edn", "{:file/name-format :triple-lowbar}"),
        ("pages/a.md", "- one\n  - two\n"),
        ("journals/2026_01_01.md", "- three\n"),
        ("whiteboards/w.edn", "{}"),
        ("pages/board.canvas", "{}"),
        ("assets/pic.png", "PNG"),
    ]);
    let (_ws, report) = import_with(&LogseqAdapter, g.path());

    assert_eq!(report.source_pages, 4, "2 md + 1 whiteboard + 1 canvas");
    assert_eq!(report.source_blocks, 3);
    let rec = report.reconciliation.expect("reconciliation runs now");
    assert_eq!(rec.pages_skipped, 2);
    assert_eq!(rec.pages_unaccounted, 0);
    assert_eq!(rec.blocks_unaccounted, 0);
    assert!(rec.balanced, "{rec:?}");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains("not read, and holds no pages")),
        "{:?}",
        report.warnings
    );
}

/// A block template becomes a template page, and the page it added does
/// not break the balance (issue #563).
#[test]
fn a_block_template_lands_as_a_template_page() {
    let g = fixture_tree(&[(
        "pages/Templates.md",
        "- Interview\n  template:: interview\n  template-including-parent:: false\n  \
         - **Candidate:**\n  - **Role:**\n",
    )]);
    let (ws, report) = import_with(&LogseqAdapter, g.path());

    let tpl = read(&ws.root.join("pages/templates-interview.md"));
    assert!(tpl.contains("template:: interview"), "{tpl}");
    assert!(tpl.contains("- **Candidate:**"), "{tpl}");
    assert!(
        !tpl.contains("template-including-parent"),
        "bookkeeping property must not travel into the template:\n{tpl}"
    );
    // The source page keeps what the user wrote.
    let src = read(&ws.root.join("pages/templates.md"));
    assert!(src.contains("template:: interview"), "{src}");

    assert_eq!(report.pages_added_as_templates, 1);
    let rec = report.reconciliation.expect("reconciliation");
    assert!(rec.balanced, "{rec:?}");
}

/// Every `.md` the import writes must already be the bytes the op log
/// projects.
///
/// It is the op log that is the source of truth, so the file is a
/// projection of it — and a projection written in a different order is
/// a *stale* projection. With the header in source order, a page with
/// two properties was stale the moment it was written: `outl doctor`
/// reported the whole import as "pages hold a stale `.md` projection
/// … run `outl doctor --repair`", which is the first thing someone
/// runs after migrating a graph they still half-trust.
#[test]
fn no_imported_page_is_born_stale() {
    let g = fixture_tree(&[
        (
            "pages/props.md",
            "type:: project\ntags:: dev\nstatus:: active\n\n- a block with no refs\n",
        ),
        (
            "pages/Templates.md",
            "- Interview\n  template:: interview\n  - **Candidate:**\n",
        ),
        ("journals/2026_01_01.md", "- a journal block\n"),
    ]);
    let (ws, _) = import_with(&LogseqAdapter, g.path());

    let pages = outl_actions::page::list_all(&ws.workspace);
    assert!(pages.len() >= 3, "pages: {}", pages.len());
    for meta in &pages {
        let path = outl_actions::journal::page_md_path(&ws.root, meta);
        if !path.exists() {
            continue;
        }
        let on_disk = read(&path);
        let id = outl_actions::page::find_by_slug(&ws.workspace, &meta.slug).expect("page node");
        let projected = outl_actions::journal::render_page_md(&ws.workspace, id);
        assert_eq!(
            on_disk,
            projected,
            "{} differs from what the op log projects — it is stale on arrival",
            path.display()
        );
    }
}
