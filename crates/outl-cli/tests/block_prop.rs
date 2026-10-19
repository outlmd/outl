//! `outl block prop` — set, get, list, and delete a property on a block.
//!
//! The write goes through `outl_actions::set_property` (an `Op::SetProp`).
//! Delete must remove the key, not store an empty value: an empty
//! `key::` line is the page-property ghost in issue #300, and a block
//! delete that leaves one is the same bug.

use serde_json::Value;
use std::process::{Command, Output};
use tempfile::TempDir;

fn outl() -> Command {
    Command::new(env!("CARGO_BIN_EXE_outl"))
}

fn init_workspace() -> TempDir {
    let dir = TempDir::new().unwrap();
    let device = dir.path().join("device");
    std::fs::create_dir_all(&device).unwrap();
    let status = outl()
        .env("OUTL_DEVICE_DIR", &device)
        .arg("init")
        .arg(dir.path())
        .status()
        .expect("init failed");
    assert!(status.success(), "outl init must succeed");
    dir
}

fn run(ws: &TempDir, args: &[&str]) -> Output {
    outl()
        .env("OUTL_DEVICE_DIR", ws.path().join("device"))
        .args(["--workspace"])
        .arg(ws.path())
        .args(args)
        .output()
        .expect("spawn outl")
}

fn envelope(out: &Output) -> Value {
    if out.stdout.is_empty() {
        panic!(
            "empty stdout\nstatus: {:?}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
        );
    }
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "non-JSON stdout ({e}): {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        )
    })
}

fn ok(out: Output) -> Value {
    let env = envelope(&out);
    assert!(
        out.status.success() && env["ok"] == true,
        "command failed:\nstatus: {:?}\nstdout: {}\nstderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    env
}

/// A page with one block, ready for a property.
fn page_with_block(ws: &TempDir) -> String {
    let _ = ok(run(
        ws,
        &["page", "create", "ideas", "--title", "Ideas", "--json"],
    ));
    let append = ok(run(
        ws,
        &[
            "block", "append", "--page", "ideas", "--text", "ship it", "--json",
        ],
    ));
    append["data"]["id"].as_str().unwrap().to_string()
}

fn page_md(ws: &TempDir) -> String {
    std::fs::read_to_string(ws.path().join("pages/ideas.md")).expect("pages/ideas.md")
}

#[test]
fn block_prop_set_get_and_list_round_trip() {
    let ws = init_workspace();
    let id = page_with_block(&ws);

    let set = ok(run(
        &ws,
        &["block", "prop", "set", &id, "priority=high", "--json"],
    ));
    assert_eq!(set["data"]["key"], "priority");
    assert_eq!(set["data"]["value"], "high");

    let md = page_md(&ws);
    assert!(
        md.contains("priority:: high"),
        "the projection must carry the property:\n{md}"
    );

    let get = ok(run(
        &ws,
        &["block", "prop", "get", &id, "priority", "--json"],
    ));
    assert_eq!(get["data"]["value"], "high");

    let _ = ok(run(
        &ws,
        &["block", "prop", "set", &id, "owner=ada", "--json"],
    ));
    let list = ok(run(&ws, &["block", "prop", "list", &id, "--json"]));
    let props = list["data"]["properties"].as_array().expect("properties");
    let keys: Vec<&str> = props.iter().map(|p| p["key"].as_str().unwrap()).collect();
    assert_eq!(
        keys,
        vec!["owner", "priority"],
        "list is alpha-sorted: {props:?}"
    );
}

#[test]
fn deleting_a_block_property_removes_the_line_from_the_markdown() {
    let ws = init_workspace();
    let id = page_with_block(&ws);
    let _ = ok(run(
        &ws,
        &["block", "prop", "set", &id, "priority=high", "--json"],
    ));
    let _ = ok(run(
        &ws,
        &["block", "prop", "set", &id, "owner=ada", "--json"],
    ));

    let deleted = ok(run(
        &ws,
        &["block", "prop", "delete", &id, "priority", "--json"],
    ));
    assert_eq!(deleted["data"]["key"], "priority");

    let md = page_md(&ws);
    assert!(
        !md.lines()
            .any(|line| line.trim_start().starts_with("priority::")),
        "delete must not leave a ghost `priority::` line:\n{md}"
    );
    assert!(
        md.contains("owner:: ada"),
        "the property that was not deleted stays:\n{md}"
    );

    let get = envelope(&run(
        &ws,
        &["block", "prop", "get", &id, "priority", "--json"],
    ));
    assert_eq!(get["ok"], false);
    assert_eq!(get["error"]["code"], "PROP_NOT_FOUND");

    let list = ok(run(&ws, &["block", "prop", "list", &id, "--json"]));
    let keys: Vec<&str> = list["data"]["properties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, vec!["owner"]);
}

#[test]
fn a_line_break_in_the_value_is_rejected_and_the_markdown_stays() {
    let ws = init_workspace();
    let id = page_with_block(&ws);
    let before = page_md(&ws);

    for value in ["high\nlow", "high\rlow", "high\r\nlow"] {
        let assignment = format!("priority={value}");
        let out = run(&ws, &["block", "prop", "set", &id, &assignment, "--json"]);
        let env = envelope(&out);
        assert!(
            !out.status.success() && env["ok"] == false,
            "a line break must not be stored:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        assert_eq!(env["error"]["code"], "INVALID_ARG");
        let message = env["error"]["message"].as_str().unwrap_or("");
        assert!(
            message.contains("newline") || message.contains("carriage return"),
            "the message should say why: {message}"
        );
        assert_eq!(
            page_md(&ws),
            before,
            "a rejected value must not touch the page"
        );
    }
}

#[test]
fn a_double_colon_inside_a_value_survives_a_reparse() {
    let ws = init_workspace();
    let id = page_with_block(&ws);
    let value = "https://a::b";

    let set = ok(run(
        &ws,
        &[
            "block",
            "prop",
            "set",
            &id,
            &format!("url={value}"),
            "--json",
        ],
    ));
    assert_eq!(set["data"]["value"], value);

    let md = page_md(&ws);
    let parsed = outl_md::parse(&md);
    let block = parsed.blocks.first().expect("the page has its block");
    let parsed_value = block
        .properties
        .iter()
        .find(|(key, _)| key == "url")
        .map(|(_, v)| v.as_str());
    assert_eq!(
        parsed_value,
        Some(value),
        "reparse of the rendered page must keep the value:\n{md}"
    );

    let get = ok(run(&ws, &["block", "prop", "get", &id, "url", "--json"]));
    assert_eq!(get["data"]["value"], value);
}

#[test]
fn an_empty_assignment_removes_the_property_instead_of_writing_a_bare_key() {
    let ws = init_workspace();
    let id = page_with_block(&ws);
    let _ = ok(run(
        &ws,
        &["block", "prop", "set", &id, "priority=high", "--json"],
    ));

    let _ = ok(run(
        &ws,
        &["block", "prop", "set", &id, "priority=", "--json"],
    ));

    let md = page_md(&ws);
    assert!(
        !md.lines()
            .any(|line| line.trim_start().starts_with("priority::")),
        "clearing with an empty value must not leave `priority::`:\n{md}"
    );
}
