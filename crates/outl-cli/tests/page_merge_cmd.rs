//! `outl page merge` folds one page into another through the real binary.
//!
//! What the merge itself does (blocks, properties, aliases, link rewrite,
//! trashing the source) is pinned in `outl-actions`. These tests pin what
//! only the CLI owns: the `--confirm` gate and its `--yes` alias, the
//! projection of every page the merge touched, the removal of the source's
//! `.md` + sidecar, the stable error codes, the JSON reply keys and the
//! human listing, plus the same merge reached through `outl batch`.
//!
//! `init_workspace` is copied rather than taken from `mcp_support`: that
//! module also holds the MCP client, which this file never uses and which
//! would then be dead code here.

use serde_json::{json, Value};
use std::process::{Command, Output};
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let status = outl()
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success());
    dir
}

fn run(ws: &TempDir, args: &[&str]) -> Output {
    outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(args)
        .output()
        .expect("spawn outl")
}

fn ok(out: Output) -> Value {
    if !out.status.success() {
        panic!(
            "command failed:\nstatus: {:?}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
    serde_json::from_slice(&out.stdout).expect("non-JSON stdout")
}

/// A command that must fail with a `--json` error envelope.
fn fails(out: Output) -> Value {
    assert!(
        !out.status.success(),
        "command must fail, stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    serde_json::from_slice(&out.stdout).expect("non-JSON stdout")
}

fn merge(ws: &TempDir, source: &str, target: &str, flag: &str) -> Output {
    run(ws, &["page", "merge", source, target, flag, "--json"])
}

fn page_md(ws: &TempDir, slug: &str) -> String {
    std::fs::read_to_string(ws.path().join("pages").join(format!("{slug}.md")))
        .unwrap_or_else(|e| panic!("pages/{slug}.md must exist: {e}"))
}

fn create(ws: &TempDir, slug: &str) {
    ok(run(ws, &["page", "create", slug, "--json"]));
}

fn append(ws: &TempDir, page: &str, text: &str) {
    let mut args = vec!["block", "append", "--page", page];
    args.extend(["--text", text, "--json"]);
    ok(run(ws, &args));
}

fn set_prop(ws: &TempDir, page: &str, assignment: &str) {
    let args = ["page", "prop", "set", page, assignment, "--json"];
    ok(run(ws, &args));
}

/// The text of every top-level block of `slug`, in outline order.
fn outline_texts(ws: &TempDir, slug: &str) -> Vec<String> {
    let env = ok(run(ws, &["page", "get", slug, "--json"]));
    env["data"]["outline"]
        .as_array()
        .expect("outline is an array")
        .iter()
        .map(|node| node["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn page_slugs(ws: &TempDir) -> Vec<String> {
    let env = ok(run(ws, &["page", "list", "--json"]));
    env["data"]["pages"]
        .as_array()
        .expect("pages is an array")
        .iter()
        .map(|page| page["slug"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// `alpha` (two blocks), `beta` (one block) and `gamma`, which mentions
/// `alpha` as a ref and as a tag.
fn workspace_with_three_pages() -> TempDir {
    let ws = init_workspace();
    for slug in ["alpha", "beta", "gamma"] {
        create(&ws, slug);
    }
    append(&ws, "alpha", "alpha one");
    append(&ws, "alpha", "alpha two");
    append(&ws, "beta", "beta first");
    append(&ws, "gamma", "see [[alpha]] and #alpha today");
    ws
}

/// `alpha` and `beta`, each holding a different value for `owner`.
fn workspace_with_a_property_conflict() -> TempDir {
    let ws = init_workspace();
    create(&ws, "alpha");
    create(&ws, "beta");
    append(&ws, "alpha", "alpha one");
    set_prop(&ws, "alpha", "owner=alice");
    set_prop(&ws, "beta", "owner=bob");
    ws
}

#[test]
fn merge_without_confirm_is_refused_and_changes_nothing() {
    let ws = workspace_with_three_pages();

    let env = fails(run(&ws, &["page", "merge", "alpha", "beta", "--json"]));
    assert_eq!(env["error"]["code"], "CONFIRM_REQUIRED");

    assert!(ws.path().join("pages/alpha.md").exists());
    assert!(page_slugs(&ws).contains(&"alpha".to_string()));
    assert_eq!(outline_texts(&ws, "beta"), ["beta first"]);
    assert_eq!(outline_texts(&ws, "alpha"), ["alpha one", "alpha two"]);
    assert!(page_md(&ws, "gamma").contains("[[alpha]]"));
}

#[test]
fn merge_moves_blocks_rewrites_refs_and_removes_the_source() {
    let ws = workspace_with_three_pages();

    let env = ok(merge(&ws, "alpha", "beta", "--confirm"));
    let data = &env["data"];
    assert_eq!(data["outcome"], "merged");
    assert_eq!(data["source"]["slug"], "alpha");
    assert_eq!(data["target"]["slug"], "beta");
    assert_eq!(data["moved_blocks"], 2);
    assert_eq!(data["source_deleted"], true);
    assert_eq!(data["nothing_changed"], false);
    assert_eq!(
        data["rewritten_refs"].as_array().map(Vec::len),
        Some(1),
        "only gamma's block names alpha: {env}"
    );
    let aliases: Vec<&str> = data["aliases_added"]
        .as_array()
        .expect("aliases_added is an array")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        aliases.contains(&"alpha"),
        "the source slug must become an alias of the target: {env}"
    );

    // The target keeps its own blocks first, then the source's in order.
    assert_eq!(
        outline_texts(&ws, "beta"),
        ["beta first", "alpha one", "alpha two"]
    );
    let beta = page_md(&ws, "beta");
    assert!(
        beta.contains("alpha one") && beta.contains("alpha two"),
        "{beta}"
    );

    let gamma = page_md(&ws, "gamma");
    assert!(
        gamma.contains("see [[beta]] and #beta today"),
        "refs and tags follow the merge: {gamma}"
    );

    assert!(!page_slugs(&ws).contains(&"alpha".to_string()));
    assert!(
        !ws.path().join("pages/alpha.md").exists(),
        "the source projection must be gone"
    );
    assert!(
        !ws.path().join("pages/alpha.outl").exists(),
        "the source sidecar must be gone"
    );
}

#[test]
fn merging_again_succeeds_and_reports_nothing_changed() {
    let ws = workspace_with_three_pages();
    ok(merge(&ws, "alpha", "beta", "--confirm"));

    let env = ok(merge(&ws, "alpha", "beta", "--confirm"));
    let data = &env["data"];
    assert_eq!(data["outcome"], "merged");
    assert_eq!(data["nothing_changed"], true, "{env}");
    assert_eq!(data["moved_blocks"], 0);
    assert_eq!(data["source_deleted"], false);
    assert_eq!(data["already_merged"], true);
    assert_eq!(data["rewritten_refs"].as_array().map(Vec::len), Some(0));

    // The first run's result is untouched.
    assert_eq!(
        outline_texts(&ws, "beta"),
        ["beta first", "alpha one", "alpha two"]
    );
    assert!(!ws.path().join("pages/alpha.md").exists());
}

#[test]
fn a_conflicting_property_keeps_the_targets_value_and_is_reported() {
    let ws = workspace_with_a_property_conflict();

    let env = ok(merge(&ws, "alpha", "beta", "--confirm"));
    assert_eq!(
        env["data"]["property_conflicts"],
        json!([{ "key": "owner", "kept": "bob", "dropped": "alice" }]),
        "{env}"
    );

    let get_owner = ["page", "prop", "get", "beta", "owner", "--json"];
    let owner = ok(run(&ws, &get_owner));
    assert_eq!(owner["data"]["value"], "bob");
}

#[test]
fn the_human_listing_names_each_property_conflict() {
    let ws = workspace_with_a_property_conflict();

    let out = run(&ws, &["page", "merge", "alpha", "beta", "--confirm"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("merged: alpha into beta"), "{stdout}");
    assert!(stdout.contains("moved 1 block(s)"), "{stdout}");
    assert!(stdout.contains("property conflicts"), "{stdout}");
    assert!(
        stdout.contains("owner: kept `bob`, dropped `alice`"),
        "{stdout}"
    );
}

#[test]
fn merging_a_page_into_itself_is_invalid_arg() {
    let ws = workspace_with_three_pages();

    let env = fails(merge(&ws, "beta", "beta", "--confirm"));
    assert_eq!(env["error"]["code"], "INVALID_ARG");
    assert_eq!(outline_texts(&ws, "beta"), ["beta first"]);
}

#[test]
fn an_unknown_page_on_either_side_is_page_not_found() {
    let ws = workspace_with_three_pages();

    let env = fails(merge(&ws, "nowhere", "beta", "--confirm"));
    assert_eq!(env["error"]["code"], "PAGE_NOT_FOUND");

    let env = fails(merge(&ws, "alpha", "nowhere", "--confirm"));
    assert_eq!(env["error"]["code"], "PAGE_NOT_FOUND");
    assert!(
        ws.path().join("pages/alpha.md").exists(),
        "a refused merge must not remove the source"
    );
}

#[test]
fn yes_is_an_alias_of_confirm() {
    let ws = workspace_with_three_pages();

    let env = ok(merge(&ws, "alpha", "beta", "--yes"));
    assert_eq!(env["data"]["outcome"], "merged");
    assert_eq!(env["data"]["moved_blocks"], 2);
    assert!(!ws.path().join("pages/alpha.md").exists());
}

#[test]
fn batch_page_merge_needs_confirm_true() {
    let ws = workspace_with_three_pages();

    let refused = json!({"ops": [
        {"op": "page_merge", "args": {"source": "alpha", "target": "beta"}}
    ]})
    .to_string();
    let out = run(&ws, &["batch", "--ops", &refused, "--json"]);
    // A stopped batch exits 1 but still emits an `ok: true` envelope.
    assert!(!out.status.success(), "a refused batch exits non-zero");
    let env: Value = serde_json::from_slice(&out.stdout).expect("non-JSON stdout");
    assert_eq!(env["data"]["applied"], 0, "{env}");
    assert_eq!(env["data"]["failed_at"], 0);
    assert_eq!(env["data"]["failed_op"], "page_merge");
    assert_eq!(env["data"]["error"]["code"], "CONFIRM_REQUIRED");
    assert!(ws.path().join("pages/alpha.md").exists());
    assert_eq!(outline_texts(&ws, "beta"), ["beta first"]);

    let confirmed = json!({"ops": [
        {"op": "page_merge", "args": {"source": "alpha", "target": "beta", "confirm": true}}
    ]})
    .to_string();
    let env = ok(run(&ws, &["batch", "--ops", &confirmed, "--json"]));
    assert_eq!(env["data"]["applied"], 1, "{env}");
    assert!(env["data"]["failed_at"].is_null());
    let reply = &env["data"]["results"][0];
    assert_eq!(reply["op"], "page_merge");
    assert_eq!(reply["data"]["outcome"], "merged");
    assert_eq!(reply["data"]["moved_blocks"], 2);

    assert_eq!(
        outline_texts(&ws, "beta"),
        ["beta first", "alpha one", "alpha two"]
    );
    assert!(!ws.path().join("pages/alpha.md").exists());
    let gamma = page_md(&ws, "gamma");
    assert!(gamma.contains("see [[beta]] and #beta today"), "{gamma}");
}
