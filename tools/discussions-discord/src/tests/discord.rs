use std::time::Duration;

use serde_json::json;

use super::{discussion, json_reply, Harness, Net, NEW_THREAD, THREAD, WEBHOOK};
use crate::discord::{create_thread, post, redact};
use crate::error::SyncError;
use crate::http::Response;
use crate::payload::plain_payload;

fn post_hi(net: &Net) -> (Result<serde_json::Value, SyncError>, Harness) {
    let harness = Harness::new();
    let env = harness.env("discussion", &json!({}), Some(WEBHOOK));
    let result = harness.with_ctx(&env, net, |ctx| {
        post(ctx, WEBHOOK, &plain_payload("hi"), None)
    });
    (result, harness)
}

#[test]
fn a_429_is_retried_once_after_retry_after() {
    let net = Net::with_discord(vec![
        json_reply(429, json!({"retry_after": 2.5})),
        json_reply(200, json!({"channel_id": THREAD})),
    ]);
    let (result, harness) = post_hi(&net);
    assert_eq!(result.expect("posted")["channel_id"], THREAD);
    assert_eq!(net.discord_calls().len(), 2);
    assert_eq!(*harness.sleeps.borrow(), vec![Duration::from_secs_f64(2.5)]);
}

#[test]
fn retry_after_falls_back_to_the_header_and_is_capped() {
    let net = Net::with_discord(vec![
        Ok(Response::new(429, "x").with_header("Retry-After", "9999")),
        json_reply(200, json!({})),
    ]);
    let (result, harness) = post_hi(&net);
    result.expect("posted");
    assert_eq!(*harness.sleeps.borrow(), vec![Duration::from_secs(60)]);
}

#[test]
fn a_429_with_no_hint_waits_a_second() {
    let net = Net::with_discord(vec![Ok(Response::new(429, "")), json_reply(200, json!({}))]);
    let (result, harness) = post_hi(&net);
    result.expect("posted");
    assert_eq!(*harness.sleeps.borrow(), vec![Duration::from_secs(1)]);
}

#[test]
fn a_second_429_fails() {
    let net = Net::with_discord(vec![
        json_reply(429, json!({"retry_after": 1})),
        json_reply(429, json!({"retry_after": 1})),
    ]);
    let (result, harness) = post_hi(&net);
    let message = result.expect_err("rate limited").to_string();
    assert!(message.contains("429"));
    assert_eq!(net.discord_calls().len(), 2);
    assert_eq!(harness.sleeps.borrow().len(), 1);
}

#[test]
fn other_errors_fail_without_the_url_and_without_sleeping() {
    let net = Net::with_discord(vec![json_reply(404, json!({"message": "Unknown Webhook"}))]);
    let (result, harness) = post_hi(&net);
    let message = result.expect_err("404").to_string();
    assert!(message.contains("404"));
    assert!(message.contains("Unknown Webhook"));
    assert!(!message.contains("secret-token"));
    assert!(harness.sleeps.borrow().is_empty());
}

#[test]
fn the_error_detail_is_cut_to_300_bytes() {
    let net = Net::with_discord(vec![Ok(Response::new(500, "x".repeat(5000)))]);
    let (result, _) = post_hi(&net);
    let message = result.expect_err("500").to_string();
    assert!(message.len() < 350, "{} bytes", message.len());
}

#[test]
fn an_echoed_webhook_in_the_reply_is_redacted() {
    let net = Net::with_discord(vec![Ok(Response::new(400, format!("bad {WEBHOOK}")))]);
    let (result, _) = post_hi(&net);
    let message = result.expect_err("400").to_string();
    assert!(!message.contains("secret-token"));
}

#[test]
fn a_transport_error_naming_the_url_is_redacted() {
    let net = Net::with_discord(vec![Err(SyncError::new(format!(
        "request failed: connect to {WEBHOOK}?wait=true timed out"
    )))]);
    let (result, _) = post_hi(&net);
    let message = result.expect_err("transport").to_string();
    assert!(message.contains("timed out"));
    assert!(!message.contains("secret-token"), "{message}");
}

#[test]
fn a_thread_post_asks_for_wait_and_a_comment_names_its_thread() {
    let net = Net::default();
    let harness = Harness::new();
    let env = harness.env("discussion", &json!({}), Some(WEBHOOK));
    harness.with_ctx(&env, &net, |ctx| {
        post(ctx, WEBHOOK, &plain_payload("a"), None).expect("first");
        post(ctx, WEBHOOK, &plain_payload("b"), Some(THREAD)).expect("second");
    });
    let calls = net.discord_calls();
    assert!(calls[0].1.ends_with("?wait=true"), "{}", calls[0].1);
    let expected = format!("?wait=true&thread_id={THREAD}");
    assert!(calls[1].1.ends_with(&expected), "{}", calls[1].1);
}

#[test]
fn a_webhook_that_already_has_a_query_gets_an_ampersand() {
    let net = Net::default();
    let harness = Harness::new();
    let env = harness.env("discussion", &json!({}), Some(WEBHOOK));
    // `Net` routes on the prefix, so this still reaches the Discord arm.
    let hook = format!("{WEBHOOK}?x=1");
    harness.with_ctx(&env, &net, |ctx| {
        post(ctx, &hook, &plain_payload("a"), None).expect("posted");
    });
    assert!(net.discord_calls()[0].1.ends_with("?x=1&wait=true"));
}

#[test]
fn the_thread_id_is_the_channel_id_of_the_created_message() {
    let net = Net::default();
    let harness = Harness::new();
    let env = harness.env("discussion", &json!({}), Some(WEBHOOK));
    let id = harness
        .with_ctx(&env, &net, |ctx| create_thread(ctx, WEBHOOK, &discussion()))
        .expect("thread");
    assert_eq!(id, NEW_THREAD);
}

#[test]
fn a_response_without_a_thread_id_is_an_error() {
    let net = Net::with_discord(vec![json_reply(200, json!({"id": "1"}))]);
    let harness = Harness::new();
    let env = harness.env("discussion", &json!({}), Some(WEBHOOK));
    let result = harness.with_ctx(&env, &net, |ctx| create_thread(ctx, WEBHOOK, &discussion()));
    assert!(result.expect_err("no id").to_string().contains("forum"));
}

#[test]
fn redact_replaces_every_occurrence() {
    let text = format!("{WEBHOOK} and {WEBHOOK}");
    assert_eq!(redact(&text, WEBHOOK), "*** and ***");
    assert_eq!(redact("nothing", ""), "nothing");
}
