//! End-to-end: a Roam task marker right after a list or quote prefix at
//! the head of the block (`- {{[[TODO]]}} x`) is the block's task, the
//! way Roam draws it, and the prefix stays in the text.

mod common;

use common::read;
use common::roam::import_fixture;

#[test]
fn a_marker_after_a_list_or_quote_prefix_is_the_block_task() {
    let json = r#"[
        {"title": "Checklist", "children": [
            {"string": "- {{[[TODO]]}} proofread the draft", "uid": "t1", "children": []},
            {"string": "1. {{[[TODO]]}} first step", "uid": "t2", "children": []},
            {"string": "12) {{TODO}} later step", "uid": "t3", "children": []},
            {"string": "> {{[[DONE]]}} quoted and done", "uid": "t4", "children": []},
            {"string": "* {{DONE}} starred", "uid": "t5", "children": []},
            {"string": "still open: {{[[TODO]]}} call back", "uid": "t6", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let path = ws.root.join("pages/checklist.md");
    let sc = outl_md::sidecar::read(&outl_md::sidecar::sidecar_path_for(&path)).expect("sidecar");
    let projected = outl_actions::journal::render_page_md(&ws.workspace, sc.page_id);

    for text in [read(&path), projected] {
        assert!(
            text.contains("- TODO - proofread the draft\n"),
            "page:\n{text}"
        );
        assert!(text.contains("- TODO 1. first step\n"), "page:\n{text}");
        assert!(text.contains("- TODO 12) later step\n"), "page:\n{text}");
        // outl's canonical order for a task that is also a quote.
        assert!(text.contains("- DONE > quoted and done\n"), "page:\n{text}");
        assert!(text.contains("- DONE * starred\n"), "page:\n{text}");
        // A marker inside the sentence is still kept as written.
        assert!(
            text.contains("- still open: {{[[TODO]]}} call back\n"),
            "page:\n{text}"
        );
    }
    assert_eq!(report.tasks.get("TODO"), Some(&3));
    assert_eq!(report.tasks.get("DONE"), Some(&2));
    assert_eq!(report.tasks_midtext_literal, 1);
}

#[test]
fn a_prefix_without_a_marker_is_left_alone() {
    let json = r#"[
        {"title": "Plain", "children": [
            {"string": "- just a dash", "uid": "p1", "children": []},
            {"string": "2024. a year, not a list", "uid": "p2", "children": []},
            {"string": "-{{[[TODO]]}} no space after the dash", "uid": "p3", "children": []}
        ]}
    ]"#;
    let (ws, report) = import_fixture(json);
    let text = read(&ws.root.join("pages/plain.md"));
    assert!(text.contains("- - just a dash\n"), "page:\n{text}");
    assert!(
        text.contains("- 2024. a year, not a list\n"),
        "page:\n{text}"
    );
    assert!(
        text.contains("- -{{[[TODO]]}} no space after the dash\n"),
        "page:\n{text}"
    );
    assert!(report.tasks.is_empty(), "tasks: {:?}", report.tasks);
    assert_eq!(report.tasks_midtext_literal, 1);
}
