//! Adapter tests over a hand-written synthetic graph: one page, a
//! journal, nested ordered blocks, refs, a tag, tasks, a code block, a
//! user property, built-ins that must be skipped, a folded block and
//! timestamps. Nothing here comes from a real graph.

use super::*;
use crate::ir::{BlockContent, ImportBlock, ImportPage, Inline, PageBody, PageName, TaskState};
use chrono::NaiveDate;
use std::fmt::Write as _;

const NOON_NOV_12: i64 = 1_794_484_800_000;
const NOON_NOV_10: i64 = 1_794_312_000_000;

/// Deterministic uuid for an entity id.
pub(super) fn uuid(id: i64) -> String {
    format!("{id:08x}-0000-4000-8000-000000000000")
}

/// `[[<uuid>]]` as Logseq stores a page ref in a title.
pub(super) fn r(id: i64) -> String {
    format!("[[{}]]", uuid(id))
}

/// Tiny datoms builder: pretty-prints like Logseq's `pprint` would.
#[derive(Default)]
pub(super) struct Fx {
    rows: Vec<String>,
}

impl Fx {
    pub(super) fn raw(&mut self, e: i64, attr: &str, v: &str) {
        self.rows.push(format!("[{e} :{attr} {v}]"));
    }
    pub(super) fn s(&mut self, e: i64, attr: &str, v: &str) {
        let esc = v
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n");
        self.raw(e, attr, &format!("\"{esc}\""));
    }
    /// Entity with uuid, optional ident and title.
    pub(super) fn ent(&mut self, id: i64, ident: Option<&str>, title: Option<&str>) {
        if let Some(i) = ident {
            self.raw(id, "db/ident", &format!(":{i}"));
        }
        self.raw(id, "block/uuid", &format!("#uuid \"{}\"", uuid(id)));
        if let Some(t) = title {
            self.s(id, "block/title", t);
        }
    }
    pub(super) fn tag(&mut self, e: i64, class: i64) {
        self.raw(e, "block/tags", &class.to_string());
    }
    pub(super) fn block(&mut self, id: i64, page: i64, parent: i64, order: &str, title: &str) {
        self.ent(id, None, Some(title));
        self.raw(id, "block/page", &page.to_string());
        self.raw(id, "block/parent", &parent.to_string());
        self.s(id, "block/order", order);
    }
    pub(super) fn finish(mut self) -> String {
        self.rows.sort_by_key(|r| {
            r.trim_start_matches('[')
                .split(' ')
                .next()
                .and_then(|n| n.parse::<i64>().ok())
                .unwrap_or(0)
        });
        let mut out = String::from(
            "{:logseq.db.sqlite.export/schema-version {:major 65, :minor 34},\n \
             :logseq.db.sqlite.export/graph-format :datoms,\n :datoms\n [",
        );
        for (i, row) in self.rows.iter().enumerate() {
            if i > 0 {
                out.push_str("\n  ");
            }
            out.push_str(row);
        }
        write!(out, "],\n :logseq.db.sqlite.export/export-type :graph}}\n").expect("string write");
        out
    }
}

// Entity ids used by the fixture.
pub(super) const PAGE_CLASS: i64 = 1;
pub(super) const JOURNAL_CLASS: i64 = 2;
pub(super) const TAG_CLASS: i64 = 3;
pub(super) const PROPERTY_CLASS: i64 = 4;
pub(super) const CODE_CLASS: i64 = 5;
pub(super) const QUOTE_CLASS: i64 = 6;
pub(super) const TASK_CLASS: i64 = 7;
pub(super) const WHITEBOARD_CLASS: i64 = 8;
pub(super) const STATUS: i64 = 10;
pub(super) const STATUS_TODO: i64 = 11;
pub(super) const STATUS_DONE: i64 = 12;
pub(super) const STATUS_BACKLOG: i64 = 13;
pub(super) const PRIORITY: i64 = 15;
pub(super) const PRIORITY_HIGH: i64 = 16;
pub(super) const DEADLINE: i64 = 17;
pub(super) const READING: i64 = 20;
pub(super) const READING_VALUE: i64 = 21;
pub(super) const RATING: i64 = 22;
pub(super) const RATING_VALUE: i64 = 23;
pub(super) const LIBRARY: i64 = 30;
pub(super) const PROJECT: i64 = 40;
pub(super) const BOOK_TAG: i64 = 55;
pub(super) const JOURNAL: i64 = 60;
pub(super) const BOARD: i64 = 80;
pub(super) const GONE: i64 = 99;

pub(super) fn built_in(fx: &mut Fx, id: i64) {
    fx.raw(id, "logseq.property/built-in?", "true");
}

fn fixture() -> String {
    let mut fx = Fx::default();
    // Built-in classes.
    for (id, name, title) in [
        (PAGE_CLASS, "Page", "Page"),
        (JOURNAL_CLASS, "Journal", "Journal"),
        (TAG_CLASS, "Tag", "Tag"),
        (PROPERTY_CLASS, "Property", "Property"),
        (CODE_CLASS, "Code-block", "Code"),
        (QUOTE_CLASS, "Quote-block", "Quote"),
        (TASK_CLASS, "Task", "Task"),
        (WHITEBOARD_CLASS, "Whiteboard", "Whiteboard"),
    ] {
        fx.ent(id, Some(&format!("logseq.class/{name}")), Some(title));
        fx.tag(id, TAG_CLASS);
        built_in(&mut fx, id);
    }
    // Built-in properties + closed values.
    for (id, ident, title) in [
        (STATUS, "logseq.property/status", "Status"),
        (PRIORITY, "logseq.property/priority", "Priority"),
        (DEADLINE, "logseq.property/deadline", "Deadline"),
    ] {
        fx.ent(id, Some(ident), Some(title));
        fx.tag(id, PROPERTY_CLASS);
        built_in(&mut fx, id);
    }
    for (id, ident, title, prop) in [
        (STATUS_TODO, "logseq.property/status.todo", "Todo", STATUS),
        (STATUS_DONE, "logseq.property/status.done", "Done", STATUS),
        (STATUS_BACKLOG, "logseq.property/status.backlog", "Backlog", STATUS),
        (PRIORITY_HIGH, "logseq.property/priority.high", "High", PRIORITY),
    ] {
        fx.ent(id, Some(ident), Some(title));
        fx.raw(id, "block/closed-value-property", &prop.to_string());
        built_in(&mut fx, id);
    }
    // User properties: a text one and a number one.
    fx.ent(READING, Some("user.property/reading-status-aB1"), Some("Reading status"));
    fx.tag(READING, PROPERTY_CLASS);
    fx.raw(READING, "logseq.property/type", ":default");
    fx.ent(READING_VALUE, None, Some("In progress"));
    fx.raw(READING_VALUE, "logseq.property/created-from-property", &READING.to_string());
    fx.raw(READING_VALUE, "block/page", &PROJECT.to_string());
    fx.ent(RATING, Some("user.property/rating-cD2"), Some("Rating"));
    fx.tag(RATING, PROPERTY_CLASS);
    fx.raw(RATING, "logseq.property/type", ":number");
    fx.ent(RATING_VALUE, None, None);
    fx.raw(RATING_VALUE, "logseq.property/value", "4");
    fx.raw(RATING_VALUE, "logseq.property/created-from-property", &RATING.to_string());
    fx.raw(RATING_VALUE, "block/page", &PROJECT.to_string());
    // A built-in page that must be skipped, a whiteboard, a user tag.
    fx.ent(LIBRARY, None, Some("Library"));
    fx.tag(LIBRARY, PAGE_CLASS);
    built_in(&mut fx, LIBRARY);
    fx.ent(BOARD, None, Some("Sketch"));
    fx.tag(BOARD, WHITEBOARD_CLASS);
    fx.ent(BOOK_TAG, Some("user.class/book-xY9"), Some("book"));
    fx.tag(BOOK_TAG, TAG_CLASS);

    project_page(&mut fx);
    journal_page(&mut fx);
    fx.finish()
}

fn project_page(fx: &mut Fx) {
    fx.ent(PROJECT, None, Some("My Project"));
    fx.tag(PROJECT, PAGE_CLASS);
    fx.tag(PROJECT, BOOK_TAG);
    fx.raw(PROJECT, "block/created-at", "1700000000000");
    fx.raw(PROJECT, "block/updated-at", "1700000100000");
    // Datom order is deliberately not outline order: "Zz" < "a0" < "a1"
    // byte-wise, which is the order Logseq's fractional index uses.
    fx.block(41, PROJECT, PROJECT, "a0", "second root");
    fx.raw(41, "block/collapsed?", "true");
    fx.raw(41, "block/created-at", "1700000000000");
    fx.raw(41, "block/updated-at", "1700000200000");
    let child = format!("child of {} tagged #[[{}]]", r(PROJECT), uuid(BOOK_TAG));
    fx.block(42, PROJECT, 41, "a0", &child);
    fx.block(43, PROJECT, PROJECT, "Zz", "first root");
    fx.block(44, PROJECT, PROJECT, "a1", &format!("see {} and [[{}]]", r(42), uuid(GONE)));
    fx.block(45, PROJECT, PROJECT, "a2", "buy milk");
    fx.tag(45, TASK_CLASS);
    fx.raw(45, "logseq.property/status", &STATUS_TODO.to_string());
    fx.raw(45, "logseq.property/deadline", &NOON_NOV_12.to_string());
    fx.block(46, PROJECT, PROJECT, "a3", "ship it");
    fx.tag(46, TASK_CLASS);
    fx.raw(46, "logseq.property/status", &STATUS_DONE.to_string());
    fx.raw(46, "logseq.property/priority", &PRIORITY_HIGH.to_string());
    fx.block(47, PROJECT, PROJECT, "a4", "fn main() {}");
    fx.tag(47, CODE_CLASS);
    fx.s(47, "logseq.property.code/lang", "rust");
    fx.block(48, PROJECT, PROJECT, "a5", "to be or not\nto be");
    fx.tag(48, QUOTE_CLASS);
    fx.block(49, PROJECT, PROJECT, "a6", "someday");
    fx.tag(49, TASK_CLASS);
    fx.raw(49, "logseq.property/status", &STATUS_BACKLOG.to_string());
    fx.raw(49, "logseq.property/scheduled", &NOON_NOV_10.to_string());
    fx.block(50, PROJECT, PROJECT, "a7", "a book");
    fx.raw(50, "user.property/reading-status-aB1", &READING_VALUE.to_string());
    fx.raw(50, "user.property/rating-cD2", &RATING_VALUE.to_string());
    fx.tag(50, BOOK_TAG);
    fx.raw(50, "logseq.property/icon", "{:type :emoji}");
    // In the recycle bin: must not be imported.
    fx.block(51, PROJECT, PROJECT, "a8", "thrown away");
    fx.raw(51, "logseq.property/deleted-at", "1700000300000");
}

fn journal_page(fx: &mut Fx) {
    fx.ent(JOURNAL, None, Some("Nov 11th, 2026"));
    fx.tag(JOURNAL, JOURNAL_CLASS);
    fx.raw(JOURNAL, "block/journal-day", "20261111");
    fx.block(61, JOURNAL, JOURNAL, "a0", &format!("worked on #[[{}]]", uuid(PROJECT)));
}

// --- helpers over the IR ---------------------------------------------------

pub(super) fn parse_text(edn: &str) -> (ImportGraph, ImportReport) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("export.edn");
    std::fs::write(&file, edn).expect("write fixture");
    let mut report = ImportReport::new("logseq-db");
    let graph = LogseqDbAdapter
        .parse(&file, &mut report)
        .expect("fixture parses");
    (graph, report)
}

fn parse_fixture() -> (ImportGraph, ImportReport) {
    parse_text(&fixture())
}

pub(super) fn named<'a>(g: &'a ImportGraph, title: &str) -> &'a ImportPage {
    g.pages
        .iter()
        .find(|p| p.name == PageName::Named(title.to_string()))
        .unwrap_or_else(|| panic!("no page {title:?}"))
}

pub(super) fn outline(p: &ImportPage) -> &[ImportBlock] {
    match &p.body {
        PageBody::Outline(b) => b,
        PageBody::Raw(_) => panic!("expected an outline"),
    }
}

pub(super) fn text(b: &ImportBlock) -> String {
    match &b.content {
        BlockContent::Inline(toks) => toks
            .iter()
            .map(|t| match t {
                Inline::Text(s) => s.clone(),
                Inline::CodeSpan(s) => format!("`{s}`"),
                Inline::BlockRef { uid, .. } => format!("(({uid}))"),
                Inline::Embed(_) => "<embed>".to_string(),
                Inline::Component { raw, .. } => raw.clone(),
            })
            .collect(),
        BlockContent::Verbatim(s) => s.clone(),
        BlockContent::Code { body, .. } => body.clone(),
    }
}

pub(super) fn prop<'a>(b: &'a ImportBlock, key: &str) -> Option<&'a str> {
    b.props
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

// --- pages -------------------------------------------------------------------

#[test]
fn pages_journals_and_user_tags_are_imported_and_built_ins_skipped() {
    let (g, report) = parse_fixture();
    let titles: Vec<String> = g
        .pages
        .iter()
        .map(|p| match &p.name {
            PageName::Journal(d) => d.to_string(),
            PageName::Named(t) => t.clone(),
        })
        .collect();
    assert!(titles.contains(&"2026-11-11".to_string()), "{titles:?}");
    assert!(titles.contains(&"My Project".to_string()), "{titles:?}");
    assert!(titles.contains(&"book".to_string()), "user tag class is a page");
    assert_eq!(titles.len(), 3, "no property, closed value or built-in leaks: {titles:?}");
    assert!(g.pages.iter().any(|p| p.name
        == PageName::Journal(NaiveDate::from_ymd_opt(2026, 11, 11).expect("date"))));

    let skipped: Vec<&str> = report.skipped.iter().map(|s| s.path.as_str()).collect();
    assert!(skipped.iter().any(|p| p.contains("Library")), "{skipped:?}");
    assert!(skipped.iter().any(|p| p.contains("Sketch")), "whiteboard: {skipped:?}");
    // Structural definitions land in the aggregate warning instead.
    let agg = report
        .warnings
        .iter()
        .find(|w| w.detail.starts_with("not imported"))
        .expect("aggregate warning");
    assert!(agg.detail.contains("property definitions"), "{}", agg.detail);
    assert!(agg.detail.contains("closed-value"), "{}", agg.detail);
    assert!(agg.detail.contains("built-in tags"), "{}", agg.detail);
    assert!(agg.detail.contains("recycle bin"), "{}", agg.detail);
}

#[test]
fn the_source_counts_close_over_pages_and_blocks() {
    let (_, report) = parse_fixture();
    // 3 imported pages + Library + Sketch skipped.
    assert_eq!(report.source_pages, 5);
    // 11 live blocks on My Project (41..=50 minus none + 42/43…) and 1 in the journal:
    // 41,42,43,44,45,46,47,48,49,50 = 10, plus 61 = 11. The recycled one is excluded.
    assert_eq!(report.source_blocks, 11);
    assert_eq!(report.skipped.len(), 2);
}

#[test]
fn page_tags_and_timestamps_ride_along() {
    let (g, _) = parse_fixture();
    let p = named(&g, "My Project");
    assert!(
        p.props.contains(&("tags".to_string(), "#book".to_string())),
        "{:?}",
        p.props
    );
    assert_eq!(p.created.map(|d| d.timestamp_millis()), Some(1_700_000_000_000));
    assert_eq!(p.edited.map(|d| d.timestamp_millis()), Some(1_700_000_100_000));
}

// --- blocks ------------------------------------------------------------------

#[test]
fn siblings_follow_the_block_order_string_bytewise() {
    let (g, _) = parse_fixture();
    let roots: Vec<String> = outline(named(&g, "My Project")).iter().map(text).collect();
    assert_eq!(roots[0], "first root", "\"Zz\" sorts before \"a0\": {roots:?}");
    assert_eq!(roots[1], "second root");
    assert!(roots[2].starts_with("see "), "{roots:?}");
}

#[test]
fn children_nest_under_their_parent_and_collapse_is_kept() {
    let (g, _) = parse_fixture();
    let second = &outline(named(&g, "My Project"))[1];
    assert!(second.collapsed);
    assert_eq!(second.children.len(), 1);
    assert_eq!(second.uid.as_deref(), Some(uuid(41).as_str()));
    assert!(!outline(named(&g, "My Project"))[0].collapsed);
}

#[test]
fn recycled_blocks_are_not_imported() {
    let (g, _) = parse_fixture();
    let all: Vec<String> = outline(named(&g, "My Project")).iter().map(text).collect();
    assert!(!all.iter().any(|t| t.contains("thrown away")), "{all:?}");
}

#[test]
fn timestamps_land_on_blocks() {
    let (g, _) = parse_fixture();
    let b = &outline(named(&g, "My Project"))[1];
    assert_eq!(b.created.map(|d| d.timestamp_millis()), Some(1_700_000_000_000));
    assert_eq!(b.edited.map(|d| d.timestamp_millis()), Some(1_700_000_200_000));
}

// --- text --------------------------------------------------------------------

#[test]
fn page_refs_and_tags_are_spelled_with_titles() {
    let (g, _) = parse_fixture();
    let child = &outline(named(&g, "My Project"))[1].children[0];
    assert_eq!(text(child), "child of [[My Project]] tagged #book");
    let journal = g
        .pages
        .iter()
        .find(|p| matches!(p.name, PageName::Journal(_)))
        .expect("journal");
    // A multi-word tag has no outl spelling: it becomes a page ref.
    assert_eq!(text(&outline(journal)[0]), "worked on [[My Project]]");
}

#[test]
fn a_block_ref_becomes_the_ir_block_ref() {
    let (g, _) = parse_fixture();
    let b = &outline(named(&g, "My Project"))[2];
    match &b.content {
        BlockContent::Inline(toks) => assert!(
            toks.contains(&Inline::BlockRef {
                uid: uuid(42),
                alias: None
            }),
            "{toks:?}"
        ),
        other => panic!("expected inline, got {other:?}"),
    }
}

#[test]
fn an_unknown_uuid_stays_verbatim_and_is_counted_with_a_warning() {
    let (g, report) = parse_fixture();
    let b = &outline(named(&g, "My Project"))[2];
    assert!(text(b).contains(&r(GONE)), "{}", text(b));
    assert_eq!(report.refs_unresolved, 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains(&uuid(GONE))),
        "{:?}",
        report.warnings
    );
}

// --- tasks -------------------------------------------------------------------

#[test]
fn statuses_map_to_task_states_and_nuance_is_kept() {
    let (g, report) = parse_fixture();
    let blocks = outline(named(&g, "My Project"));
    let todo = &blocks[3];
    assert_eq!(todo.task, Some(TaskState::Todo));
    let done = &blocks[4];
    assert_eq!(done.task, Some(TaskState::Done));
    assert_eq!(prop(done, "priority"), Some("high"));
    let backlog = &blocks[7];
    assert_eq!(backlog.task, Some(TaskState::Todo));
    assert_eq!(prop(backlog, "state"), Some("backlog"));
    assert_eq!(report.tasks.get("TODO"), Some(&1));
    assert_eq!(report.tasks.get("DONE"), Some(&1));
    assert_eq!(report.tasks.get("BACKLOG"), Some(&1));
}

#[test]
fn deadline_and_scheduled_become_date_links_in_the_text() {
    let (g, report) = parse_fixture();
    let blocks = outline(named(&g, "My Project"));
    assert_eq!(text(&blocks[3]), "buy milk [[2026-11-12]]");
    assert_eq!(text(&blocks[7]), "someday [[2026-11-10]]");
    assert_eq!(report.org_dates_converted, 2);
}

// --- special nodes -------------------------------------------------------------

#[test]
fn a_code_block_becomes_a_code_block_with_its_language() {
    let (g, _) = parse_fixture();
    match &outline(named(&g, "My Project"))[5].content {
        BlockContent::Code { lang, body } => {
            assert_eq!(lang.as_deref(), Some("rust"));
            assert_eq!(body, "fn main() {}");
        }
        other => panic!("expected code, got {other:?}"),
    }
}

#[test]
fn a_quote_keeps_its_text_and_is_counted() {
    let (g, report) = parse_fixture();
    assert_eq!(text(&outline(named(&g, "My Project"))[6]), "> to be or not\n> to be");
    assert_eq!(report.components_verbatim.get("quote"), Some(&1));
}

// --- user properties -------------------------------------------------------------

#[test]
fn user_properties_use_the_definition_title_as_key() {
    let (g, _) = parse_fixture();
    let b = &outline(named(&g, "My Project"))[8];
    assert_eq!(prop(b, "reading-status"), Some("In progress"));
    assert_eq!(prop(b, "rating"), Some("4"));
    // The node is tagged with the user class and the title does not
    // spell it, so the tag is appended.
    assert_eq!(text(b), "a book #book");
}

#[test]
fn built_in_attributes_without_an_equivalent_are_counted_not_silent() {
    let (_, report) = parse_fixture();
    assert!(report.artifacts_stripped >= 1);
    assert!(
        report
            .warnings
            .iter()
            .any(|w| w.detail.contains("logseq.property/icon")),
        "{:?}",
        report.warnings
    );
}

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
