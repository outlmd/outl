//! Refusals and detection: export shapes the adapter does not read,
//! syntax errors, `detect()` and the `db.sqlite` graph directory.

use super::tests::fixture;
use super::*;

// --- rejection ---------------------------------------------------------------------

fn parse_err(edn: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("x.edn");
    std::fs::write(&file, edn).expect("write");
    let mut report = ImportReport::new("logseq-db");
    LogseqDbAdapter
        .parse(&file, &mut report)
        .expect_err("must be rejected")
        .to_string()
}

#[test]
fn a_page_or_block_export_is_refused_with_a_clear_message() {
    let e = parse_err("{:logseq.db.sqlite.export/export-type :page, :pages-and-blocks []}");
    assert!(e.contains("unsupported export shape"), "{e}");
    assert!(e.contains(":page"), "{e}");
}

#[test]
fn the_human_graph_export_is_refused() {
    let e = parse_err(
        "{:logseq.db.sqlite.export/export-type :graph-human, :pages-and-blocks [], :classes {}}",
    );
    assert!(e.contains(":graph-human"), "{e}");
}

#[test]
fn another_graph_format_is_refused() {
    let e = parse_err("{:logseq.db.sqlite.export/graph-format :other, :datoms []}");
    assert!(e.contains("graph-format :other"), "{e}");
}

#[test]
fn not_a_map_and_bad_datoms_are_refused() {
    assert!(parse_err("[1 2 3]").contains("not an EDN map"));
    let e = parse_err("{:logseq.db.sqlite.export/graph-format :datoms, :datoms [[1 :a]]}");
    assert!(e.contains("datom #0"), "{e}");
    let e = parse_err("{:logseq.db.sqlite.export/graph-format :datoms}");
    assert!(e.contains("`:datoms`"), "{e}");
}

#[test]
fn a_syntax_error_reports_line_and_column() {
    let e = parse_err("{:a 1\n :b [1 2");
    assert!(e.contains("line 2"), "{e}");
}

// --- detection and the db.sqlite directory --------------------------------------------

#[test]
fn detect_accepts_only_a_datoms_edn_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let good = dir.path().join("good.edn");
    std::fs::write(&good, fixture()).expect("write");
    assert!(LogseqDbAdapter::detect(&good));

    let page = dir.path().join("page.edn");
    std::fs::write(&page, "{:logseq.db.sqlite.export/export-type :page}").expect("write");
    assert!(!LogseqDbAdapter::detect(&page));

    let wrong_ext = dir.path().join("good.json");
    std::fs::write(&wrong_ext, fixture()).expect("write");
    assert!(!LogseqDbAdapter::detect(&wrong_ext));
    assert!(!LogseqDbAdapter::detect(dir.path()));
}

#[test]
fn a_db_graph_directory_is_explained_not_silently_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("db.sqlite"), b"SQLite format 3\0").expect("write");
    assert!(is_db_graph_dir(dir.path()));
    let mut report = ImportReport::new("logseq-db");
    let e = LogseqDbAdapter
        .parse(dir.path(), &mut report)
        .expect_err("a directory is refused")
        .to_string();
    assert!(e.contains("Export EDN file"), "{e}");
    assert!(e.contains("outl import logseq-db"), "{e}");

    // A file graph that happens to carry a db.sqlite is still a file graph.
    std::fs::create_dir(dir.path().join("pages")).expect("mkdir");
    std::fs::create_dir(dir.path().join("journals")).expect("mkdir");
    assert!(!is_db_graph_dir(dir.path()));
}
