//! Adapter-level tests: what `parse` accepts, refuses, and names.
//!
//! The outline parser's own golden tests live in `outline/tests.rs`,
//! filename decoding in `names.rs`, and the file survey in `files.rs`.

use super::*;
use crate::ir::PageBody;

fn graph(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (path, body) in files {
        let p = dir.path().join(path);
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(&p, body).expect("write");
    }
    dir
}

fn parse(dir: &Path) -> (ImportGraph, ImportReport) {
    let mut report = ImportReport::new("logseq");
    let graph = LogseqAdapter.parse(dir, &mut report).expect("parse");
    (graph, report)
}

fn page_named<'a>(g: &'a ImportGraph, name: &str) -> &'a ImportPage {
    g.pages
        .iter()
        .find(|p| p.name == PageName::Named(name.to_string()))
        .unwrap_or_else(|| {
            let have: Vec<String> = g.pages.iter().map(|p| format!("{:?}", p.name)).collect();
            panic!("no page named {name}; have {have:?}")
        })
}

/// A DB-version graph directory holds `db.sqlite` and none of the file
/// layout; scanning it as a file graph reported 0 pages with no error.
#[test]
fn a_logseq_db_graph_directory_is_refused_instead_of_importing_zero_pages() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("db.sqlite"), b"SQLite format 3\0").expect("write");
    let mut report = ImportReport::new("logseq");
    let err = LogseqAdapter
        .parse(dir.path(), &mut report)
        .expect_err("a DB graph is refused")
        .to_string();
    assert!(err.contains("Export EDN file"), "{err}");
    assert!(err.contains("outl import logseq-db"), "{err}");
}

/// Logseq's markdown export hands you a zip, and pointing outl at it
/// is the obvious next thing to try. Say what to do with it.
#[test]
fn a_zip_says_what_to_do_with_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let zip = dir.path().join("graph.zip");
    std::fs::write(&zip, b"PK\x03\x04").expect("write");
    let mut report = ImportReport::new("logseq");
    let err = LogseqAdapter
        .parse(&zip, &mut report)
        .expect_err("a zip is refused")
        .to_string();
    assert!(err.contains("unzip"), "{err}");
    assert!(err.contains("logseq-db"), "{err}");
}

/// A graph whose owner never used journals is still a graph — and so
/// is an unzipped markdown export holding only one of the two dirs.
#[test]
fn one_content_directory_is_enough_to_detect_a_graph() {
    let g = graph(&[("pages/a.md", "- x\n")]);
    assert!(LogseqAdapter::detect(g.path()));
    let j = graph(&[("journals/2026_01_01.md", "- x\n")]);
    assert!(LogseqAdapter::detect(j.path()));
    let empty = tempfile::tempdir().expect("tempdir");
    assert!(!LogseqAdapter::detect(empty.path()));
}

/// The namespace bug, end to end: the page's title has to be what
/// Logseq shows, or every `[[parent/child]]` in the graph points at a
/// page that does not exist under that name.
#[test]
fn a_namespaced_filename_becomes_a_namespaced_title() {
    let g = graph(&[("pages/parent___child.md", "- x\n")]);
    let (graph, _) = parse(g.path());
    page_named(&graph, "parent/child");
}

#[test]
fn a_legacy_graph_decodes_dots_as_namespaces() {
    let g = graph(&[
        ("logseq/config.edn", "{:file/name-format :legacy}"),
        ("pages/parent.child.md", "- x\n"),
    ]);
    let (graph, _) = parse(g.path());
    page_named(&graph, "parent/child");
}

/// Both halves of the reconciliation denominator, which the Logseq
/// adapter never reported (issue #299).
#[test]
fn the_source_counts_are_reported() {
    let g = graph(&[
        ("pages/a.md", "- one\n  - two\n"),
        ("journals/2026_01_01.md", "- three\n"),
    ]);
    let (_, report) = parse(g.path());
    assert_eq!(report.source_pages, 2);
    assert_eq!(report.source_blocks, 3);
}

/// `journals/` filenames follow a configurable format.
#[test]
fn journal_filenames_parse_in_the_formats_logseq_writes() {
    let g = graph(&[
        ("journals/2026_01_02.md", "- a\n"),
        ("journals/2026-01-03.md", "- b\n"),
        ("journals/2026.01.04.md", "- c\n"),
    ]);
    let (graph, report) = parse(g.path());
    let mut dates: Vec<String> = graph
        .pages
        .iter()
        .filter_map(|p| match &p.name {
            PageName::Journal(d) => Some(d.to_string()),
            PageName::Named(_) => None,
        })
        .collect();
    dates.sort();
    assert_eq!(dates, vec!["2026-01-02", "2026-01-03", "2026-01-04"]);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

/// A block template becomes a template page, which is what makes it
/// show up in `/template` after the migration (issue #563).
#[test]
fn a_block_template_reaches_the_graph_as_a_page() {
    let g = graph(&[(
        "pages/Templates.md",
        "- Interview\n  template:: interview\n  - **Candidate:**\n  - **Role:**\n",
    )]);
    let (graph, report) = parse(g.path());
    let tpl = page_named(&graph, "templates/interview");
    assert_eq!(
        tpl.props,
        vec![("template".to_string(), "interview".to_string())]
    );
    let PageBody::Outline(body) = &tpl.body else {
        panic!("outline expected");
    };
    assert_eq!(body.len(), 2);
    assert_eq!(report.pages_added_as_templates, 1);
}

/// An empty source directory is a user mistake worth naming, not a
/// successful import of nothing.
#[test]
fn a_directory_with_no_pages_says_so() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("logseq")).expect("mkdir");
    let (graph, report) = parse(dir.path());
    assert!(graph.pages.is_empty());
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains("no markdown pages found")),
        "{:?}",
        report.warnings
    );
}
