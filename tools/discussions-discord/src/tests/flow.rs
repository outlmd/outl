use serde_json::{json, Value};

use super::{
    comment_json, content, discussion_json, json_reply, Harness, Net, NEW_THREAD, THREAD, WEBHOOK,
};

fn discussion_event() -> Value {
    json!({"discussion": discussion_json()})
}

fn comment_event() -> Value {
    json!({"discussion": discussion_json(), "comment": comment_json()})
}

fn mapped_net() -> Net {
    Net::one_mapping(THREAD)
}

#[test]
fn a_missing_secret_skips_with_a_warning_and_exit_zero() {
    for webhook in [Some(""), Some("  \n"), None] {
        let harness = Harness::new();
        let net = Net::default();
        let env = harness.env("discussion", &discussion_event(), webhook);
        let code = harness.with_ctx(&env, &net, crate::execute);
        assert_eq!(code, 0);
        assert!(harness.output().contains("::warning::"));
        assert!(net.calls.borrow().is_empty());
        assert!(!harness.output().contains("::add-mask::"));
    }
}

#[test]
fn the_webhook_is_masked_before_anything_else() {
    let harness = Harness::new();
    let net = Net::default();
    assert_eq!(harness.execute("discussion", &discussion_event(), &net), 0);
    assert_eq!(harness.log.borrow()[0], format!("::add-mask::{WEBHOOK}"));
}

#[test]
fn created_posts_a_forum_thread_and_saves_its_id() {
    let harness = Harness::new();
    let net = Net::default();
    assert_eq!(harness.execute("discussion", &discussion_event(), &net), 0);
    let calls = net.discord_calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.contains("wait=true"));
    assert!(!calls[0].1.contains("thread_id"));
    let body = &net.discord_bodies()[0];
    assert_eq!(body["thread_name"], "#42 How do I sync two devices?");
    let content = body["content"].as_str().expect("content");
    assert!(content.contains("alice") && content.contains("Q&A"));
    let expected = json!({"discussion": 42, "thread_id": NEW_THREAD});
    assert_eq!(harness.saved(), expected);
    let outs = harness.outputs();
    assert_eq!(outs["save"], "true");
    assert_eq!(outs["artifact_name"], "discord-thread-42");
    assert!(outs["thread_file"].ends_with("discord-thread/thread.json"));
}

#[test]
fn created_for_a_mapped_discussion_posts_nothing() {
    let harness = Harness::new();
    let net = mapped_net();
    assert_eq!(harness.execute("discussion", &discussion_event(), &net), 0);
    assert!(net.discord_calls().is_empty());
    assert!(harness.output().contains("::notice::"));
    assert!(!harness.outputs().contains_key("save"));
}

#[test]
fn a_comment_without_a_mapping_creates_the_post_then_comments_in_it() {
    let harness = Harness::new();
    let net = Net::default();
    let event = comment_event();
    assert_eq!(harness.execute("discussion_comment", &event, &net), 0);
    let calls = net.discord_calls();
    assert_eq!(calls.len(), 2);
    assert!(!calls[0].1.contains("thread_id"));
    assert!(calls[1].1.contains(&format!("thread_id={NEW_THREAD}")));
    let second = &net.discord_bodies()[1];
    assert!(second.get("thread_name").is_none());
    let link = comment_json()["html_url"].to_string();
    assert!(content(second).contains(link.trim_matches('"')));
    assert_eq!(harness.saved()["thread_id"], NEW_THREAD);
}

#[test]
fn a_comment_with_a_mapping_posts_into_that_thread_only() {
    let harness = Harness::new();
    let net = mapped_net();
    let event = comment_event();
    assert_eq!(harness.execute("discussion_comment", &event, &net), 0);
    let calls = net.discord_calls();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1.contains(&format!("thread_id={THREAD}")));
    // Saved again so the retention clock restarts.
    let expected = json!({"discussion": 42, "thread_id": THREAD});
    assert_eq!(harness.saved(), expected);
    assert_eq!(harness.outputs()["save"], "true");
}

#[test]
fn a_failed_comment_still_saves_the_new_thread() {
    let harness = Harness::new();
    let net = Net::with_discord(vec![
        json_reply(200, json!({"channel_id": NEW_THREAD})),
        json_reply(500, json!({"message": "boom"})),
    ]);
    let event = comment_event();
    assert_eq!(harness.execute("discussion_comment", &event, &net), 1);
    assert_eq!(harness.saved()["thread_id"], NEW_THREAD);
    assert_eq!(harness.outputs()["save"], "true");
    let output = harness.output();
    assert!(output.contains("::error::Discord returned HTTP 500"));
}

#[test]
fn every_payload_of_a_run_blocks_mentions() {
    let harness = Harness::new();
    let net = Net::default();
    harness.execute("discussion_comment", &comment_event(), &net);
    let bodies = net.discord_bodies();
    assert_eq!(bodies.len(), 2);
    for body in bodies {
        assert_eq!(body["allowed_mentions"], json!({"parse": []}));
        assert_eq!(body["username"], "GitHub Discussions");
    }
}

#[test]
fn dispatch_fetches_the_discussion_and_posts_it() {
    let harness = Harness::new();
    let net = Net {
        graphql: Some(json!({"data": {"repository": {"discussion": {
            "number": 42, "title": "Old one", "body": "text",
            "url": "https://github.com/outlmd/outl/discussions/42",
            "author": {"login": "carol"}, "category": {"name": "Ideas"}}}}})),
        ..Net::default()
    };
    let event = json!({"inputs": {"discussion": "42"}});
    assert_eq!(harness.execute("workflow_dispatch", &event, &net), 0);
    let body = &net.discord_bodies()[0];
    assert_eq!(body["thread_name"], "#42 Old one");
    assert!(content(body).contains("carol"));
    assert_eq!(harness.saved()["thread_id"], NEW_THREAD);
}

#[test]
fn dispatch_for_a_missing_discussion_fails() {
    let harness = Harness::new();
    let net = Net {
        graphql: Some(json!({"data": {"repository": {"discussion": null}}})),
        ..Net::default()
    };
    let event = json!({"inputs": {"discussion": "7"}});
    assert_eq!(harness.execute("workflow_dispatch", &event, &net), 1);
    assert!(harness.output().contains("discussion #7 not found"));
}

#[test]
fn the_webhook_never_reaches_the_error_output() {
    let harness = Harness::new();
    let echo = json!({"message": format!("bad {WEBHOOK}")});
    let net = Net::with_discord(vec![json_reply(400, echo)]);
    assert_eq!(harness.execute("discussion", &discussion_event(), &net), 1);
    let shown: Vec<String> = harness
        .log
        .borrow()
        .iter()
        .filter(|line| !line.starts_with("::add-mask::"))
        .cloned()
        .collect();
    let shown = shown.join("\n");
    assert!(shown.contains("HTTP 400"));
    assert!(!shown.contains("secret-token"), "{shown}");
}

#[test]
fn an_unsupported_event_fails() {
    let harness = Harness::new();
    assert_eq!(harness.execute("push", &json!({}), &Net::default()), 1);
    assert!(harness.output().contains("unsupported event"));
}
