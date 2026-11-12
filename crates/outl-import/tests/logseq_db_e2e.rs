//! End-to-end: Logseq DB-version EDN export → workspace with real
//! handles, task prefixes, a fenced code block, a user property, the
//! journal routed by date, and a reconciliation that balances.
//!
//! The graph is a hand-written synthetic export (Logseq 2.0.2's
//! `export-type :graph`, `graph-format :datoms`).

mod common;

use common::{import_with, read, TestWs};
use outl_import::adapters::LogseqDbAdapter;
use outl_import::ImportReport;
use std::fs;

/// Deterministic uuid for an entity id.
fn u(id: i64) -> String {
    format!("{id:08x}-0000-4000-8000-000000000000")
}

/// Datom rows, one `[entity attribute value]` per line like Logseq's
/// pretty-printer writes them.
#[derive(Default)]
struct Rows(Vec<String>);

impl Rows {
    fn add(&mut self, e: i64, attr: &str, value: &str) {
        self.0.push(format!("[{e} :{attr} {value}]"));
    }

    fn text(&mut self, e: i64, attr: &str, value: &str) {
        self.add(e, attr, &format!("\"{value}\""));
    }

    /// Entity with a uuid and a title.
    fn node(&mut self, id: i64, title: &str) {
        self.add(id, "block/uuid", &format!("#uuid \"{}\"", u(id)));
        self.text(id, "block/title", title);
    }

    fn class(&mut self, id: i64, name: &str) {
        self.add(id, "db/ident", &format!(":logseq.class/{name}"));
        self.node(id, name);
        self.add(id, "block/tags", "3");
        self.add(id, "logseq.property/built-in?", "true");
    }

    fn block(&mut self, id: i64, parent: i64, order: &str, title: &str) {
        self.node(id, title);
        self.add(id, "block/page", "30");
        self.add(id, "block/parent", &parent.to_string());
        self.text(id, "block/order", order);
    }

    fn finish(self) -> String {
        format!(
            "{{:logseq.db.sqlite.export/schema-version {{:major 65, :minor 34}},\n \
             :logseq.db.sqlite.export/graph-format :datoms,\n :datoms\n [{}],\n \
             :logseq.db.sqlite.export/export-type :graph}}\n",
            self.0.join("\n  ")
        )
    }
}

fn export() -> String {
    let mut r = Rows::default();
    for (id, name) in [(1, "Page"), (2, "Journal"), (3, "Tag")] {
        r.class(id, name);
    }
    r.class(4, "Property");
    r.class(5, "Code-block");
    r.class(6, "Task");
    // Built-in status property and two closed values.
    r.add(10, "db/ident", ":logseq.property/status");
    r.node(10, "Status");
    r.add(10, "block/tags", "4");
    r.add(10, "logseq.property/built-in?", "true");
    for (id, name) in [(11, "todo"), (12, "done")] {
        r.add(id, "db/ident", &format!(":logseq.property/status.{name}"));
        r.node(id, name);
        r.add(id, "block/closed-value-property", "10");
    }
    // A user text property and its value node.
    r.add(20, "db/ident", ":user.property/reading-status-aB1");
    r.node(20, "Reading status");
    r.add(20, "block/tags", "4");
    r.add(20, "logseq.property/type", ":default");
    r.node(21, "In progress");
    r.add(21, "logseq.property/created-from-property", "20");
    r.add(21, "block/page", "30");
    // Page "Notes": "a0" sorts before "b0", so block 33 leads.
    r.node(30, "Notes");
    r.add(30, "block/tags", "1");
    r.block(31, 30, "b0", "plan the trip");
    r.add(31, "block/collapsed?", "true");
    r.block(32, 31, "a0", "pick dates");
    r.block(33, 30, "a0", &format!("see [[{}]] first", u(31)));
    r.block(34, 30, "b1", "pack bags");
    r.add(34, "block/tags", "6");
    r.add(34, "logseq.property/status", "11");
    r.block(35, 30, "b2", "buy ticket");
    r.add(35, "block/tags", "6");
    r.add(35, "logseq.property/status", "12");
    r.block(36, 30, "b3", "fn main() {}");
    r.add(36, "block/tags", "5");
    r.text(36, "logseq.property.code/lang", "rust");
    r.block(37, 30, "b4", "a good book");
    r.add(37, "user.property/reading-status-aB1", "21");
    // Journal whose only block tags the page above.
    r.node(40, "Nov 11th, 2026");
    r.add(40, "block/tags", "2");
    r.add(40, "block/journal-day", "20261111");
    r.node(41, &format!("wrote #[[{}]]", u(30)));
    r.add(41, "block/page", "40");
    r.add(41, "block/parent", "40");
    r.text(41, "block/order", "a0");
    // A built-in page that must not be emitted.
    r.node(50, "Library");
    r.add(50, "block/tags", "1");
    r.add(50, "logseq.property/built-in?", "true");
    r.finish()
}

fn import_export() -> (TestWs, ImportReport) {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("graph.edn");
    fs::write(&src, export()).expect("write export");
    import_with(&LogseqDbAdapter, &src)
}

#[test]
fn the_page_lands_with_order_tasks_code_and_properties() {
    let (ws, _) = import_export();
    let notes = read(&ws.root.join("pages/notes.md"));
    assert!(notes.contains("title:: Notes"), "{notes}");
    // "a0" sorts before "b0", so the referrer comes first.
    let see = notes.find("- see ").expect("referrer block");
    let plan = notes.find("- plan the trip").expect("parent block");
    assert!(see < plan, "block order not honoured:\n{notes}");
    assert!(notes.contains("  - pick dates"), "child nesting:\n{notes}");
    assert!(notes.contains("- TODO pack bags"), "{notes}");
    assert!(notes.contains("- DONE buy ticket"), "{notes}");
    assert!(notes.contains("```rust\nfn main() {}\n```"), "{notes}");
    assert!(notes.contains("reading-status:: In progress"), "{notes}");
    assert!(!notes.contains("outl-import:"), "no placeholders:\n{notes}");
}

#[test]
fn a_block_ref_resolves_to_a_real_handle() {
    let (ws, report) = import_export();
    let notes = read(&ws.root.join("pages/notes.md"));
    assert!(notes.contains("see ((blk-"), "ref not resolved:\n{notes}");
    assert_eq!(report.refs_resolved, 1);
    assert_eq!(report.refs_unresolved, 0);
}

#[test]
fn the_fold_lands_in_the_op_log() {
    let (ws, _) = import_export();
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(
        &ws.root.join("pages/notes.md"),
    ))
    .expect("sidecar");
    // DFS: [0] see, [1] plan the trip (folded), [2] pick dates, ...
    assert!(ws.workspace.tree().is_collapsed(sc.blocks[1].id));
    assert!(!ws.workspace.tree().is_collapsed(sc.blocks[0].id));
}

#[test]
fn the_journal_routes_by_date_and_its_tag_keeps_its_name() {
    let (ws, report) = import_export();
    let journal = read(&ws.root.join("journals/2026-11-11.md"));
    assert!(journal.contains("- wrote #Notes"), "{journal}");
    assert!(!journal.starts_with("title::"), "{journal}");
    assert_eq!(report.journals, 1);
    assert_eq!(report.pages, 1);
}

#[test]
fn built_ins_are_skipped_and_the_books_balance() {
    let (ws, report) = import_export();
    assert!(!ws.root.join("pages/library.md").exists());
    assert!(report.skipped.iter().any(|s| s.path.contains("Library")));
    let rec = report.reconciliation.as_ref().expect("source counts set");
    assert!(rec.balanced, "{rec:?}");
    assert_eq!(rec.source_pages, 3);
    assert_eq!(rec.source_blocks, 8);
    assert_eq!(report.tasks.get("TODO"), Some(&1));
    assert_eq!(report.tasks.get("DONE"), Some(&1));
}
