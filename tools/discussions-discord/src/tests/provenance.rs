//! Only artifacts a genuine run of this workflow produced are believed.

use serde_json::{json, Value};

use super::artifacts::lookup;
use super::{artifact, mapping_zip, run_info, Fixed, Harness, Net, DAY1, DAY2, THREAD};
use crate::github::{lookup_thread, WORKFLOW_PATH};

const OLD: &str = "111";

fn net_with(entries: Vec<(Value, &str)>) -> Net {
    let mapped = entries
        .into_iter()
        .map(|(art, thread)| (art, mapping_zip(42, thread)))
        .collect();
    Net::mapped(mapped)
}

fn found(net: &Net, event: &Value) -> Option<String> {
    lookup(net, event).0.expect("lookup")
}

fn forged(mut art: Value, field: &str, value: Value) -> Value {
    art["workflow_run"][field] = value;
    art
}

fn fork_artifact(id: u64, created_at: &str) -> Value {
    forged(artifact(id, created_at), "head_repository_id", json!(777))
}

#[test]
fn a_genuine_artifact_is_trusted() {
    let net = net_with(vec![(artifact(1, DAY1), THREAD)]);
    assert_eq!(found(&net, &json!({})).as_deref(), Some(THREAD));
}

#[test]
fn a_fork_artifact_is_rejected() {
    let net = net_with(vec![(fork_artifact(1, DAY1), "999")]);
    let (result, harness) = lookup(&net, &json!({}));
    assert_eq!(result.expect("lookup"), None);
    assert!(harness.output().contains("::notice::ignoring artifact 1"));
    // Rejected before any download, so a forged blob is never even fetched.
    let calls = net.calls.borrow();
    assert!(calls.iter().all(|call| !call.1.contains("artifact-zip")));
}

#[test]
fn an_artifact_of_another_repository_is_rejected() {
    let other = forged(artifact(1, DAY1), "repository_id", json!(1));
    assert_eq!(found(&net_with(vec![(other, "999")]), &json!({})), None);
}

#[test]
fn an_artifact_from_a_non_default_branch_is_rejected() {
    let branch = forged(artifact(1, DAY1), "head_branch", json!("evil"));
    assert_eq!(found(&net_with(vec![(branch, "999")]), &json!({})), None);
}

#[test]
fn the_default_branch_comes_from_the_event() {
    let trunk = forged(artifact(1, DAY1), "head_branch", json!("trunk"));
    let event = json!({"repository": {"default_branch": "trunk"}});
    let net = net_with(vec![(trunk.clone(), THREAD)]);
    assert_eq!(found(&net, &event).as_deref(), Some(THREAD));
    // And `main` is no longer enough once the event says otherwise.
    let net = net_with(vec![(artifact(2, DAY1), THREAD)]);
    assert_eq!(found(&net, &event), None);
    // Without the field it falls back to `main`.
    let net = net_with(vec![(trunk, THREAD)]);
    assert_eq!(found(&net, &json!({})), None);
}

#[test]
fn an_artifact_from_another_workflow_is_rejected() {
    let mut net = net_with(vec![(artifact(1, DAY1), "999")]);
    let other = run_info(".github/workflows/ci.yml", "discussion");
    net.runs.insert(100, other);
    assert_eq!(found(&net, &json!({})), None);
}

#[test]
fn an_artifact_from_a_run_with_an_untrusted_event_is_rejected() {
    for event in ["pull_request", "pull_request_target", "push", "schedule"] {
        let mut net = net_with(vec![(artifact(1, DAY1), "999")]);
        net.runs.insert(100, run_info(WORKFLOW_PATH, event));
        assert_eq!(found(&net, &json!({})), None, "{event}");
    }
}

#[test]
fn every_event_this_workflow_triggers_on_is_trusted() {
    for event in ["discussion", "discussion_comment", "workflow_dispatch"] {
        let mut net = net_with(vec![(artifact(1, DAY1), THREAD)]);
        net.runs.insert(100, run_info(WORKFLOW_PATH, event));
        assert_eq!(found(&net, &json!({})).as_deref(), Some(THREAD), "{event}");
    }
}

#[test]
fn a_run_path_with_a_ref_suffix_still_matches() {
    let mut net = net_with(vec![(artifact(1, DAY1), THREAD)]);
    let path = format!("{WORKFLOW_PATH}@refs/heads/main");
    net.runs.insert(100, run_info(&path, "discussion"));
    assert_eq!(found(&net, &json!({})).as_deref(), Some(THREAD));
}

#[test]
fn an_expired_artifact_is_rejected() {
    let mut expired = artifact(1, DAY1);
    expired["expired"] = json!(true);
    assert_eq!(found(&net_with(vec![(expired, "999")]), &json!({})), None);
}

#[test]
fn an_artifact_without_a_run_is_rejected() {
    let mut orphan = artifact(1, DAY1);
    orphan["workflow_run"] = Value::Null;
    assert_eq!(found(&net_with(vec![(orphan, "999")]), &json!({})), None);
}

#[test]
fn the_newest_valid_artifact_wins_over_a_newer_invalid_one() {
    let mut other_workflow = artifact(4, "2026-04-01T00:00:00Z");
    other_workflow["workflow_run"]["id"] = json!(9999);
    let mut net = net_with(vec![
        (artifact(1, DAY1), OLD),
        (artifact(2, DAY2), THREAD),
        (fork_artifact(3, "2026-03-01T00:00:00Z"), "999"),
        (other_workflow, "888"),
    ]);
    let ci = run_info(".github/workflows/other.yml", "discussion");
    net.runs.insert(9999, ci);
    let (result, harness) = lookup(&net, &json!({}));
    assert_eq!(result.expect("lookup").as_deref(), Some(THREAD));
    let output = harness.output();
    assert!(output.contains("ignoring artifact 4"));
    assert!(output.contains("ignoring artifact 3"));
}

#[test]
fn a_run_that_no_longer_exists_is_skipped_but_other_failures_are_loud() {
    let harness = Harness::new();
    let env = harness.env("discussion", &json!({}), Some("x"));
    for (status, ok) in [(404, true), (500, false)] {
        let http = Fixed {
            artifacts: vec![artifact(1, DAY1)],
            status,
        };
        let result = harness.with_ctx(&env, &http, |ctx| lookup_thread(ctx, &json!({}), 42));
        assert_eq!(result.is_ok(), ok, "{status}");
    }
}

#[test]
fn a_missing_repository_id_fails_closed() {
    let harness = Harness::new();
    let mut env = harness.env("discussion", &json!({}), Some("x"));
    env.remove("GITHUB_REPOSITORY_ID");
    let net = Net::default();
    let result = harness.with_ctx(&env, &net, |ctx| lookup_thread(ctx, &json!({}), 42));
    let err = result.expect_err("no id");
    assert!(err.to_string().contains("GITHUB_REPOSITORY_ID"));
    assert!(net.calls.borrow().is_empty());
}
