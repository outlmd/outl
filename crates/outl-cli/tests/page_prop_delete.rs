//! `outl page prop delete` removes the key from the op log and from
//! the `.md`. `prop set key=` stores an empty string and must not
//! leave a `key::` line either.

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

fn run(ws: &TempDir, args: &[&str]) -> std::process::Output {
    outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(args)
        .output()
        .expect("spawn outl")
}

fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let status = outl().arg("init").arg(dir.path()).status().expect("init");
    assert!(status.success(), "outl init must succeed");
    dir
}

fn page_md(ws: &TempDir, slug: &str) -> String {
    std::fs::read_to_string(ws.path().join("pages").join(format!("{slug}.md"))).unwrap()
}

#[test]
fn delete_removes_the_property_from_the_log_and_the_markdown() {
    let ws = init_workspace();
    let _ = ok(run(
        &ws,
        &["page", "create", "hb", "--title", "HB", "--json"],
    ));
    let set = ok(run(
        &ws,
        &["page", "prop", "set", "hb", "foo=bar", "--json"],
    ));
    assert_eq!(set["data"]["value"], "bar");
    assert!(
        page_md(&ws, "hb").contains("foo:: bar"),
        "a real value is a line:\n{}",
        page_md(&ws, "hb")
    );

    let deleted = ok(run(&ws, &["page", "prop", "delete", "hb", "foo", "--json"]));
    assert_eq!(deleted["ok"], true);
    assert_eq!(deleted["data"]["page"], "hb");
    assert_eq!(deleted["data"]["key"], "foo");
    assert!(
        !page_md(&ws, "hb").contains("foo::"),
        "delete must not leave a ghost line:\n{}",
        page_md(&ws, "hb")
    );

    let get = run(&ws, &["page", "prop", "get", "hb", "foo", "--json"]);
    assert!(!get.status.success(), "a deleted key is gone");
    let err: Value = serde_json::from_slice(&get.stdout).unwrap();
    assert_eq!(err["error"]["code"], "PROP_NOT_FOUND");
}

#[test]
fn setting_an_empty_value_does_not_write_a_bare_key_line() {
    let ws = init_workspace();
    let _ = ok(run(&ws, &["page", "create", "hb", "--json"]));
    let set = ok(run(&ws, &["page", "prop", "set", "hb", "foo=", "--json"]));
    assert_eq!(set["data"]["value"], "");
    let md = page_md(&ws, "hb");
    assert!(
        !md.contains("foo::"),
        "`prop set foo=` stores an empty string and must not emit a line:\n{md}"
    );

    let get = ok(run(&ws, &["page", "prop", "get", "hb", "foo", "--json"]));
    assert_eq!(get["data"]["value"], "");
}

#[test]
fn delete_refuses_a_page_model_key() {
    let ws = init_workspace();
    let _ = ok(run(&ws, &["page", "create", "hb", "--json"]));
    let denied = run(
        &ws,
        &["page", "prop", "delete", "hb", "page-slug", "--json"],
    );
    assert!(!denied.status.success());
    let err: Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(err["error"]["code"], "INVALID_ARG");

    let still = ok(run(&ws, &["page", "get", "hb", "--json"]));
    assert_eq!(still["data"]["meta"]["slug"], "hb");
}

#[test]
fn batch_page_prop_delete_uses_the_same_handler() {
    let ws = init_workspace();
    let _ = ok(run(&ws, &["page", "create", "hb", "--json"]));
    let payload = r#"{"ops":[
        {"op":"page_prop_set","args":{"page":"hb","key":"foo","value":"bar"}},
        {"op":"page_prop_delete","args":{"page":"hb","key":"foo"}}
    ]}"#;
    let env = ok(run(&ws, &["batch", "--ops", payload, "--json"]));
    assert_eq!(env["data"]["applied"], 2);
    assert!(!page_md(&ws, "hb").contains("foo::"));
    let get = run(&ws, &["page", "prop", "get", "hb", "foo", "--json"]);
    assert!(!get.status.success());
}
