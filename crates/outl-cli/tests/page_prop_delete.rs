//! `outl page prop delete` removes the key from the op log and from
//! the `.md`. A blank `prop set` (`foo=` or whitespace) is the same
//! removal, so `prop get` is `PROP_NOT_FOUND` and the file has no `foo::`.

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
fn setting_an_empty_value_clears_the_property() {
    let ws = init_workspace();
    let _ = ok(run(&ws, &["page", "create", "hb", "--json"]));
    let _ = ok(run(
        &ws,
        &["page", "prop", "set", "hb", "foo=bar", "--json"],
    ));
    assert!(page_md(&ws, "hb").contains("foo:: bar"));

    let set = ok(run(&ws, &["page", "prop", "set", "hb", "foo=", "--json"]));
    assert_eq!(set["ok"], true);
    let md = page_md(&ws, "hb");
    assert!(
        !md.contains("foo::"),
        "a blank set must not leave a line:\n{md}"
    );
    let get = run(&ws, &["page", "prop", "get", "hb", "foo", "--json"]);
    assert!(!get.status.success(), "a blank set removes the key");
    let err: Value = serde_json::from_slice(&get.stdout).unwrap();
    assert_eq!(err["error"]["code"], "PROP_NOT_FOUND");

    // The batch/MCP path does not trim before `set_kv`. Whitespace-only
    // is still a clear.
    let _ = ok(run(
        &ws,
        &["page", "prop", "set", "hb", "foo=kept", "--json"],
    ));
    let payload = r#"{"ops":[
        {"op":"page_prop_set","args":{"page":"hb","key":"foo","value":"   "}}
    ]}"#;
    let env = ok(run(&ws, &["batch", "--ops", payload, "--json"]));
    assert_eq!(env["data"]["applied"], 1);
    assert!(!page_md(&ws, "hb").contains("foo::"));
    let get = run(&ws, &["page", "prop", "get", "hb", "foo", "--json"]);
    assert!(!get.status.success());
    let err: Value = serde_json::from_slice(&get.stdout).unwrap();
    assert_eq!(err["error"]["code"], "PROP_NOT_FOUND");
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

/// **`outl page prop list` answers from the page, not from a guess.**
///
/// It used to probe a hardcoded list of nine "well-known" keys, on the
/// stated grounds that the tree exposed no per-node property iterator —
/// which stopped being true long before, and which `outl block prop
/// list` had never believed. So this command was the only property
/// listing in the binary answering from a list somebody maintained by
/// hand, and it got both directions wrong at once:
///
/// - every key the user actually typed was **missing**, because none of
///   them is in the nine;
/// - `page-slug` and `page-kind` were **present**, which is the one
///   thing `outl_actions::tree::is_page_model_key` exists to prevent —
///   they define the page, `prop set` refuses them, and listing them as
///   properties invites an edit that rewrites the page's identity on the
///   next projection.
///
/// Both halves are asserted, because fixing one and not the other is
/// what a hand-maintained list does next.
#[test]
fn page_prop_list_shows_the_users_properties_and_hides_the_page_model() {
    let ws = init_workspace();
    ok(run(&ws, &["page", "create", "proj", "--json"]));
    // Written out of alphabetical order on purpose: the listing is in
    // presentation order (`sort_in_presentation_order`), not in the
    // order the ops landed.
    for kv in ["zeta=last", "alpha=first", "owner=avelino"] {
        ok(run(&ws, &["page", "prop", "set", "proj", kv, "--json"]));
    }

    let list = ok(run(&ws, &["page", "prop", "list", "proj", "--json"]));
    let keys: Vec<String> = list["data"]["properties"]
        .as_array()
        .expect("properties array")
        .iter()
        .map(|p| p["key"].as_str().expect("key").to_string())
        .collect();

    assert_eq!(
        keys,
        ["alpha", "owner", "zeta"],
        "every property the user set, alphabetically, and nothing else: {list}"
    );
    for structural in ["page-slug", "page-kind"] {
        assert!(
            !keys.iter().any(|k| k == structural),
            "`{structural}` defines the page and must never be listed as one of \
             its properties: {list}"
        );
    }
}
