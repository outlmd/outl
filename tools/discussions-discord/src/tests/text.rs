use serde_json::{json, Value};

use super::{comment, comment_json, content, discussion, discussion_json};
use crate::payload::{comment_payload, discussion_payload, plain_payload, Comment, Discussion};
use crate::text::{build_content, thread_name, truncate, units, ELLIPSIS};

#[test]
fn thread_name_is_cut_to_100() {
    let name = thread_name(7, &"x".repeat(300));
    assert_eq!(units(&name), 100);
    assert!(name.starts_with("#7 xxx"));
    assert!(name.ends_with(ELLIPSIS));
}

#[test]
fn short_thread_name_is_untouched_and_one_line() {
    assert_eq!(thread_name(7, "a\nb"), "#7 a b");
    assert_eq!(thread_name(7, "  a \t\r\n  b  "), "#7 a b");
}

#[test]
fn an_emoji_counts_as_two_units() {
    assert_eq!(units("😀"), 2);
    let name = thread_name(1, &"😀".repeat(200));
    assert!(units(&name) <= 100, "{} units", units(&name));
    assert!(name.ends_with(ELLIPSIS));
    // 3 units for "#1 ", 48 emoji = 96, 1 for the ellipsis: 100, not 101.
    assert_eq!(units(&name), 100);
    // The cut never splits a surrogate pair.
    assert!(name.chars().all(|c| c != '\u{fffd}'));
}

#[test]
fn truncate_handles_tiny_limits() {
    assert_eq!(truncate("hello", 0), "");
    assert_eq!(truncate("hello", 1), "…");
    assert_eq!(truncate("hello", 5), "hello");
    assert_eq!(truncate("hello world", 8), "hello w…");
}

#[test]
fn content_never_exceeds_2000_and_keeps_the_link() {
    let link = "Reply on GitHub: <https://github.com/outlmd/outl/discussions/42>";
    for body in ["y".repeat(50_000), "😀".repeat(5000), String::new(), "line\n".repeat(900)] {
        let mut value = discussion_json();
        value["body"] = Value::String(body);
        let discussion: Discussion = serde_json::from_value(value).expect("discussion");
        let payload = discussion_payload(&discussion);
        assert!(units(content(&payload)) <= 2000);
        assert!(content(&payload).ends_with(link));
    }
}

#[test]
fn comment_content_keeps_its_link_when_the_excerpt_is_cut() {
    let mut value = comment_json();
    value["body"] = json!("z".repeat(9000));
    let comment: Comment = serde_json::from_value(value).expect("comment");
    let payload = comment_payload(&comment);
    assert!(units(content(&payload)) <= 2000);
    let link = format!("Comment on GitHub: <{}>", comment.html_url);
    assert!(content(&payload).ends_with(&link));
    assert!(content(&payload).contains(ELLIPSIS));
}

#[test]
fn a_huge_head_still_leaves_the_tail_alone() {
    let out = build_content("h", &"e".repeat(100), "tail", 10);
    assert_eq!(out, "h\n\ntail");
}

#[test]
fn a_reply_says_so() {
    let mut value = comment_json();
    value["parent_id"] = json!(3);
    let reply: Comment = serde_json::from_value(value).expect("comment");
    assert!(content(&comment_payload(&reply)).contains("replied"));
    assert!(content(&comment_payload(&comment())).contains("commented"));
}

#[test]
fn mentions_in_the_body_stay_in_the_text_and_cannot_ping() {
    let payload = discussion_payload(&discussion());
    assert!(content(&payload).contains("@everyone"));
    assert_eq!(payload["allowed_mentions"], json!({"parse": []}));
}

#[test]
fn every_payload_blocks_mentions_and_names_the_sender() {
    for payload in [
        discussion_payload(&discussion()),
        comment_payload(&comment()),
        plain_payload("hi"),
    ] {
        assert_eq!(payload["allowed_mentions"], json!({"parse": []}));
        assert_eq!(payload["username"], "GitHub Discussions");
    }
}

#[test]
fn only_the_forum_post_carries_a_thread_name() {
    let post = discussion_payload(&discussion());
    let reply = comment_payload(&comment());
    assert!(post.get("thread_name").is_some());
    assert!(reply.get("thread_name").is_none());
}

#[test]
fn a_missing_author_and_category_fall_back() {
    let mut value = discussion_json();
    value["user"] = Value::Null;
    value["category"] = Value::Null;
    value["body"] = Value::Null;
    let discussion: Discussion = serde_json::from_value(value).expect("discussion");
    let payload = discussion_payload(&discussion);
    let text = content(&payload);
    assert!(text.contains("**ghost** opened a discussion in **General**"));
}
