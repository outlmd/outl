//! `outl query` — the page-level filter surface, and in particular
//! its negatives ([issue 323]).
//!
//! Split out of `cli_machine.rs` when that file crossed the size
//! ratchet. Same harness, one subject.
//!
//! What these pin, beyond "the flag works": **every negative is the
//! exact complement of its positive**. The DSL gets that structurally
//! (one `Filter::Not` wrapping the positive's own filter, root
//! `CLAUDE.md` invariant 14); the CLI's flags are hand-paired, so the
//! law has to be a test here instead of a type.
//!
//! The other theme is that a filter which can only match nothing is
//! rejected rather than run. On the positive side that reads as "no
//! results" and the user investigates. On the negative side it
//! excludes nothing and hands back exactly the pages they asked to
//! hide, with exit 0.
//!
//! [issue 323]: https://github.com/outlmd/outl/issues/323

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

/// Three pages: one live, one parked behind `#someday` + `status::`,
/// one carrying `#workflow` — the tag that a substring negative would
/// wrongly swallow under `--not-tag work`.
fn query_fixture() -> TempDir {
    let ws = init_workspace();
    for (slug, text) in [
        ("live", "TODO ship the parser #work"),
        ("parked", "TODO revisit this #work #someday"),
        ("adjacent", "TODO document the #workflow"),
    ] {
        ok(outl()
            .args(["--workspace"])
            .arg(ws.path())
            .args(["page", "create", slug, "--json"])
            .output()
            .unwrap());
        ok(outl()
            .args(["--workspace"])
            .arg(ws.path())
            .args(["block", "append", "--page", slug, "--text", text, "--json"])
            .output()
            .unwrap());
    }
    ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["page", "prop", "set", "parked", "status=later", "--json"])
        .output()
        .unwrap());
    ws
}

fn query_slugs(ws: &TempDir, args: &[&str]) -> Vec<String> {
    let env = ok(outl()
        .args(["--workspace"])
        .arg(ws.path())
        .arg("query")
        .args(args)
        .arg("--json")
        .output()
        .unwrap());
    let mut slugs: Vec<String> = env["data"]["results"]
        .as_array()
        .expect("results is an array")
        .iter()
        .map(|r| r["slug"].as_str().unwrap().to_string())
        .collect();
    slugs.sort();
    slugs
}

#[test]
fn query_not_tag_excludes_the_pages_carrying_it() {
    let ws = query_fixture();
    assert_eq!(
        query_slugs(&ws, &["--tag", "work", "--not-tag", "someday"]),
        vec!["live".to_string()]
    );
}

#[test]
fn query_not_tag_stops_at_the_tag_boundary() {
    // `#workflow` is a different tag from `#work`, so `--not-tag=work`
    // must leave it alone. A substring negative would delete it from
    // the answer with nothing to notice.
    let ws = query_fixture();
    let got = query_slugs(&ws, &["--not-tag", "work"]);
    assert!(got.contains(&"adjacent".to_string()), "got {got:?}");
    assert!(!got.contains(&"live".to_string()), "got {got:?}");
}

#[test]
fn query_a_tag_and_its_negation_return_nothing() {
    let ws = query_fixture();
    assert!(query_slugs(&ws, &["--tag", "work", "--not-tag", "work"]).is_empty());
}

#[test]
fn query_not_prop_takes_a_bare_key_or_a_pair() {
    let ws = query_fixture();
    // Bare key: the page carries `status::` at all.
    assert!(!query_slugs(&ws, &["--not-prop", "status"]).contains(&"parked".to_string()));
    assert!(query_slugs(&ws, &["--prop", "status"]).contains(&"parked".to_string()));
    // Key + value: only that pair is excluded.
    assert!(!query_slugs(&ws, &["--not-prop", "status=later"]).contains(&"parked".to_string()));
    assert!(query_slugs(&ws, &["--not-prop", "status=done"]).contains(&"parked".to_string()));
}

#[test]
fn query_rejects_a_property_filter_with_an_empty_value() {
    // `--not-prop status=` reading as "any status" would silently
    // drop every page carrying one.
    let ws = query_fixture();
    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["query", "--not-prop", "status=", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "an empty value must be rejected");
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["error"]["code"], "INVALID_ARG");
}

#[test]
fn query_rejects_an_empty_tag_filter() {
    // `--not-tag ""` reaches the tokenizer with an empty name, which it
    // never emits, so the exclusion never fires and the user gets back
    // every page they asked to hide.
    let ws = query_fixture();
    for args in [["--tag", ""], ["--not-tag", ""], ["--not-tag", "#"]] {
        let out = outl()
            .args(["--workspace"])
            .arg(ws.path())
            .arg("query")
            .args(args)
            .arg("--json")
            .output()
            .unwrap();
        assert!(!out.status.success(), "{args:?} must be rejected");
        let env: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(env["error"]["code"], "INVALID_ARG");
    }
}

#[test]
fn query_rejects_the_query_fence_separator_in_a_property_filter() {
    // `--not-prop "status: done"` is the ` ```query ` spelling. Taken
    // literally it is a key no page carries, so the exclusion silently
    // does nothing and the command returns what it was told to drop.
    let ws = query_fixture();
    let out = outl()
        .args(["--workspace"])
        .arg(ws.path())
        .args(["query", "--not-prop", "status: later", "--json"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let env: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(env["error"]["code"], "INVALID_ARG");
    assert!(
        env["error"]["message"]
            .as_str()
            .unwrap()
            .contains("KEY=VALUE"),
        "the error must name the separator the CLI wants, got {env}"
    );
}

#[test]
fn query_tag_filters_tolerate_a_leading_hash() {
    // `#work` is how the tag is spelled everywhere the user sees it.
    // Taking it literally makes `--tag` match nothing (visible) and
    // `--not-tag` exclude nothing (invisible).
    let ws = query_fixture();
    assert_eq!(
        query_slugs(&ws, &["--tag", "#work"]),
        query_slugs(&ws, &["--tag", "work"])
    );
    assert_eq!(
        query_slugs(&ws, &["--not-tag", "#someday"]),
        query_slugs(&ws, &["--not-tag", "someday"])
    );
    assert!(!query_slugs(&ws, &["--not-tag", "#someday"]).contains(&"parked".to_string()));
}

#[test]
fn query_every_filter_and_its_negation_return_nothing() {
    // `outl query` has one negative per filter, and each is `!` over
    // the positive's own predicate. If any pair ever returns a row,
    // the two sides have grown separate opinions about what the value
    // means — which is how a negative starts handing back the rows it
    // was told to hide.
    let ws = query_fixture();
    for pair in [
        vec!["--tag", "work", "--not-tag", "work"],
        vec!["--kind", "page", "--not-kind", "page"],
        vec!["--since", "3650d", "--not-since", "3650d"],
        vec!["--prop", "status=later", "--not-prop", "status=later"],
        vec!["--prop", "status", "--not-prop", "status"],
        vec!["--priority", "p1", "--not-priority", "p1"],
    ] {
        assert!(
            query_slugs(&ws, &pair).is_empty(),
            "{pair:?} must return nothing"
        );
    }
}

#[test]
fn query_not_kind_excludes_only_that_kind() {
    let ws = query_fixture();
    let pages = query_slugs(&ws, &["--not-kind", "journal"]);
    assert!(pages.contains(&"live".to_string()), "got {pages:?}");
    assert_eq!(pages, query_slugs(&ws, &["--kind", "page"]));
}
