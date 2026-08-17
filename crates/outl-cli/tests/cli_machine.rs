//! Integration tests for the machine-shaped CLI surface (page / block /
//! daily / search / etc.). Each test runs the real `outl` binary in a
//! tempdir so the assertions exercise the same code paths a user
//! would.
//!
//! Pairs with `e2e_full.rs` (TUI-style smoke) and `mcp_smoke.rs` (MCP
//! over stdio). Together they keep the CLI + MCP surface honest end to
//! end.

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn ok(out: std::process::Output) -> Value {
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

fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let status = outl()
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success(), "outl init must succeed");
    dir
}

#[test]
fn page_create_and_get() {
    let ws = init_workspace();
    let env = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--title", "Ideas", "--json"])
        .output()
        .unwrap());
    assert_eq!(env["ok"], true);
    assert_eq!(env["data"]["meta"]["slug"], "ideas");

    let got = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "get", "ideas", "--json"])
        .output()
        .unwrap());
    assert_eq!(got["data"]["meta"]["title"], "Ideas");
    assert!(got["data"]["outline"].is_array());
}

#[test]
fn block_append_then_toggle_todo() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--json"])
        .output()
        .unwrap());
    let append = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args([
            "block", "append", "--page", "ideas", "--text", "ship it", "--json",
        ])
        .output()
        .unwrap());
    let id = append["data"]["id"].as_str().unwrap().to_string();

    let toggle = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["block", "toggle-todo", &id, "--json"])
        .output()
        .unwrap());
    assert_eq!(toggle["data"]["todo"], "TODO");

    let toggle = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["block", "toggle-todo", &id, "--json"])
        .output()
        .unwrap());
    assert_eq!(toggle["data"]["todo"], "DOING");

    let toggle = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["block", "toggle-todo", &id, "--json"])
        .output()
        .unwrap());
    assert_eq!(toggle["data"]["todo"], "DONE");
}

#[test]
fn search_finds_appended_block() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--json"])
        .output()
        .unwrap());
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args([
            "block",
            "append",
            "--page",
            "ideas",
            "--text",
            "shipping #shipping today",
            "--json",
        ])
        .output()
        .unwrap());
    let search = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["search", "shipping", "--in", "blocks", "--json"])
        .output()
        .unwrap());
    let blocks = search["data"]["blocks"].as_array().unwrap();
    assert!(
        !blocks.is_empty(),
        "search should find at least one block, got {search}"
    );
}

#[test]
fn page_delete_requires_confirm() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "throwaway", "--json"])
        .output()
        .unwrap());

    let no_confirm = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "delete", "throwaway", "--json"])
        .output()
        .unwrap();
    assert!(
        !no_confirm.status.success(),
        "delete without --confirm must error"
    );
    let env: Value = serde_json::from_slice(&no_confirm.stdout).unwrap();
    assert_eq!(env["error"]["code"], "CONFIRM_REQUIRED");

    let yes = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "delete", "throwaway", "--confirm", "--json"])
        .output()
        .unwrap());
    assert_eq!(yes["data"]["slug"], "throwaway");
}

#[test]
fn malicious_slug_is_rejected() {
    let ws = init_workspace();
    // Path traversal attempt — slug must not be accepted because it
    // would end up joined into a filesystem path on export.
    for bad in ["../escape", "with/slash", "with\\backslash", "..", "."] {
        let out = outl()
            .args(["--workspace"])
            .arg(ws.path())
            .args(["page", "create", bad, "--json"])
            .output()
            .unwrap();
        assert!(
            !out.status.success(),
            "slug `{bad}` should be rejected, got success"
        );
        let env: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(
            env["error"]["code"], "INVALID_ARG",
            "slug `{bad}` should map to INVALID_ARG"
        );
    }
}

#[test]
fn doctor_json_runs_cleanly() {
    let ws = init_workspace();
    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["ok"], true, "doctor --json must succeed on a fresh ws");
    assert!(env["data"]["findings"].is_array());
    // Fresh workspace has no errors (warnings are allowed: missing
    // sidecar for the seed journal is expected on first init).
    let errors = env["data"]["error_count"].as_u64().unwrap_or(0);
    assert_eq!(errors, 0, "fresh workspace should have zero errors");
}

#[test]
fn delete_is_idempotent_when_md_is_missing() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "throwaway", "--json"])
        .output()
        .unwrap());
    // Drop the on-disk .md before deleting. The op log must still
    // succeed and the missing file must not crash anything.
    let md = ws.path().join("pages").join("throwaway.md");
    std::fs::remove_file(&md).ok();
    let v = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "delete", "throwaway", "--confirm", "--json"])
        .output()
        .unwrap());
    assert_eq!(v["data"]["slug"], "throwaway");
}

#[test]
fn batch_applies_every_op_in_one_session() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--json"])
        .output()
        .unwrap());

    let payload = r#"{"ops":[
        {"op":"block_append","args":{"page":"ideas","text":"first"}},
        {"op":"block_append","args":{"page":"ideas","text":"second"}}
    ]}"#;
    let env = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["batch", "--ops", payload, "--json"])
        .output()
        .unwrap());
    assert_eq!(env["ok"], true);
    assert_eq!(env["data"]["applied"], 2);
    assert!(env["data"]["failed_at"].is_null());
    assert_eq!(env["data"]["results"].as_array().unwrap().len(), 2);

    // Both blocks must be searchable from a fresh process (the batch
    // flushed the ops to the op log).
    for needle in ["first", "second"] {
        let search = ok(outl()
            .args(["--workspace"])
            .arg(ws.path())
            .args(["search", needle, "--in", "blocks", "--json"])
            .output()
            .unwrap());
        assert!(
            !search["data"]["blocks"].as_array().unwrap().is_empty(),
            "block `{needle}` must be persisted, got {search}"
        );
    }
}

#[test]
fn batch_stops_on_first_error_and_persists_prefix() {
    let ws = init_workspace();
    let _ = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "create", "ideas", "--json"])
        .output()
        .unwrap());

    // First op is valid; the second targets an invalid block id and
    // fails, so the run stops at index 1 with index 0 applied.
    let payload = r#"{"ops":[
        {"op":"block_append","args":{"page":"ideas","text":"kept"}},
        {"op":"block_update","args":{"id":"not-a-valid-ulid","text":"never"}}
    ]}"#;
    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["batch", "--ops", payload, "--json"])
        .output()
        .unwrap();
    // A partial batch exits 1 (user error) but still emits an `ok:true`
    // envelope carrying the `failed_at` / `applied` outcome.
    assert!(!out.status.success(), "partial batch must exit non-zero");
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["ok"], true, "partial batch still returns an envelope");
    assert_eq!(env["data"]["applied"], 1);
    assert_eq!(env["data"]["failed_at"], 1);
    assert_eq!(env["data"]["failed_op"], "block_update");

    // The applied prefix (op 0) must survive the failure: the guard
    // flushes it before the error response is built.
    let search = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["search", "kept", "--in", "blocks", "--json"])
        .output()
        .unwrap());
    assert!(
        !search["data"]["blocks"].as_array().unwrap().is_empty(),
        "prefix op must be persisted after a partial batch, got {search}"
    );
}

#[test]
fn asset_add_daily_imports_file_and_links_it() {
    let ws = init_workspace();

    // Fixture file living outside the workspace.
    let src = TempDir::new().unwrap();
    let file = src.path().join("report.pdf");
    std::fs::write(&file, b"%PDF-1.7 fake bytes").unwrap();

    let env = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .arg("asset")
        .arg("add")
        .arg(&file)
        .arg("--json")
        .output()
        .unwrap());
    assert_eq!(env["ok"], true);
    let rel = env["data"]["rel_path"].as_str().unwrap();
    assert!(
        rel.starts_with("assets/"),
        "rel_path under assets/, got {rel}"
    );
    assert!(rel.ends_with(".pdf"), "keeps the extension, got {rel}");
    assert_eq!(env["data"]["target"]["kind"], "daily");
    let md = env["data"]["markdown"].as_str().unwrap();
    assert!(md.contains(rel), "markdown must link the asset, got {md}");

    // The file was copied into the workspace's assets/ dir.
    assert!(
        ws.path().join(rel).exists(),
        "asset must be copied into the workspace"
    );

    // The link block was appended to today's journal (op log → .md).
    let today = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["daily", "today", "--json"])
        .output()
        .unwrap());
    assert!(
        today["data"]["md"].as_str().unwrap().contains(rel),
        "today's journal md must contain the asset link, got {today}"
    );
}

#[test]
fn asset_add_page_and_daily_are_mutually_exclusive() {
    let ws = init_workspace();
    let src = TempDir::new().unwrap();
    let file = src.path().join("pic.png");
    std::fs::write(&file, b"\x89PNG fake").unwrap();

    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .arg("asset")
        .arg("add")
        .arg(&file)
        .args(["--page", "ideas", "--daily", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "--page + --daily must error");
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["error"]["code"], "INVALID_ARG");
}

#[test]
fn workspace_info_returns_summary() {
    let ws = init_workspace();
    let info = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["workspace", "info", "--json"])
        .output()
        .unwrap());
    assert!(info["data"]["root"].is_string());
    assert!(info["data"]["actor"].is_string());
    assert!(info["data"]["ops"].is_number());
}
