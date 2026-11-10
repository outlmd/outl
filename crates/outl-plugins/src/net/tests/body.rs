//! What `ctx.net.fetch` does with a response **body**, once the gate in the
//! parent module has decided the connection is allowed.
//!
//! Two promises live here and they pull against each other: the body is
//! capped so an approved host cannot OOM the app (#280), and it can be read
//! as base64 so a plugin can see bytes at all. The cap has to hold for both,
//! which is why they are tested next to each other rather than apart.

use super::{body_reply, capturing, domains, json, spawn};
use crate::net::{fetch, MAX_RESPONSE_BYTES};
use serde_json::Value;

#[test]
fn a_response_declaring_a_body_over_the_cap_is_refused() {
    let over = MAX_RESPONSE_BYTES + 1;
    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {over}\r\nConnection: close\r\n\r\n")
        .into_bytes();
    let (port, _seen) = spawn("127.0.0.1", reply);

    let out = json(&fetch(
        &domains(&["127.0.0.1"]),
        &format!("http://127.0.0.1:{port}/big"),
        "{}",
    ));
    assert_eq!(out["ok"], Value::Bool(false), "got: {out}");
    assert!(
        out["error"].as_str().unwrap_or_default().contains("cap"),
        "got: {out}"
    );
}

#[test]
fn a_close_delimited_body_over_the_cap_is_refused_mid_stream() {
    // No `Content-Length`, so the up-front check cannot see the size —
    // this is the path that has to stop reading on its own.
    let mut reply = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    reply.resize(reply.len() + MAX_RESPONSE_BYTES as usize + 1, b'a');
    let (port, _seen) = spawn("127.0.0.1", reply);

    let out = json(&fetch(
        &domains(&["127.0.0.1"]),
        &format!("http://127.0.0.1:{port}/stream"),
        "{}",
    ));
    assert_eq!(out["ok"], Value::Bool(false), "got: {out}");
    assert!(
        out["error"].as_str().unwrap_or_default().contains("cap"),
        "got: {out}"
    );
}

#[test]
fn a_body_over_the_cap_is_logged_not_only_returned() {
    let over = MAX_RESPONSE_BYTES + 1;
    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {over}\r\nConnection: close\r\n\r\n")
        .into_bytes();
    let (port, _seen) = spawn("127.0.0.1", reply);

    let (_out, log) = capturing(|| {
        fetch(
            &domains(&["127.0.0.1"]),
            &format!("http://127.0.0.1:{port}/big"),
            "{}",
        )
    });

    assert!(
        log.joined().contains("cap"),
        "the declared-oversize refusal must reach the host log: {:?}",
        log.lines()
    );
}

#[test]
fn a_streamed_body_over_the_cap_is_logged_not_only_returned() {
    let mut reply = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    reply.resize(reply.len() + MAX_RESPONSE_BYTES as usize + 1, b'a');
    let (port, _seen) = spawn("127.0.0.1", reply);

    let (_out, log) = capturing(|| {
        fetch(
            &domains(&["127.0.0.1"]),
            &format!("http://127.0.0.1:{port}/stream"),
            "{}",
        )
    });

    assert!(
        log.joined().contains("cap"),
        "the mid-stream refusal must reach the host log: {:?}",
        log.lines()
    );
}

/// Bytes that are **not** valid UTF-8 — the shape of any real binary body
/// (a cipher text, an image, a compressed stream).
fn binary_body() -> Vec<u8> {
    let payload: Vec<u8> = (0u8..=255).collect();
    let mut reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .into_bytes();
    reply.extend_from_slice(&payload);
    reply
}

#[test]
fn a_binary_body_survives_as_base64_and_is_destroyed_as_text() {
    // Both halves in one test on purpose: the second is *why* the first
    // exists. A body crosses into JS inside a JSON string, so bytes that are
    // not valid UTF-8 cannot survive as text — `from_utf8_lossy` replaces
    // each one with U+FFFD, irreversibly. Without `encoding: "base64"` a
    // plugin cannot read a binary response at all.
    let (port, _seen) = spawn("127.0.0.1", binary_body());
    let url = format!("http://127.0.0.1:{port}/payload");
    let grant = domains(&["127.0.0.1"]);

    let encoded = json(&fetch(&grant, &url, r#"{"encoding":"base64"}"#));
    assert_eq!(encoded["ok"], Value::Bool(true), "got: {encoded}");
    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        encoded["body"].as_str().expect("a base64 string"),
    )
    .expect("the body round-trips through base64");
    assert_eq!(decoded, (0u8..=255).collect::<Vec<u8>>());

    let (port2, _seen2) = spawn("127.0.0.1", binary_body());
    let as_text = json(&fetch(
        &grant,
        &format!("http://127.0.0.1:{port2}/payload"),
        "{}",
    ));
    assert_ne!(
        as_text["body"].as_str().expect("a string").as_bytes(),
        decoded.as_slice(),
        "a lossy-UTF-8 read must not be mistaken for the bytes"
    );
}

#[test]
fn base64_does_not_raise_the_response_cap() {
    // base64 grows a body by a third. The cap is measured on the raw bytes
    // before encoding, so asking for base64 cannot be a way to pull a bigger
    // body through the gate than a text read may.
    let over = MAX_RESPONSE_BYTES + 1;
    let reply = format!("HTTP/1.1 200 OK\r\nContent-Length: {over}\r\nConnection: close\r\n\r\n")
        .into_bytes();
    let (port, _seen) = spawn("127.0.0.1", reply);

    let out = json(&fetch(
        &domains(&["127.0.0.1"]),
        &format!("http://127.0.0.1:{port}/big"),
        r#"{"encoding":"base64"}"#,
    ));

    assert_eq!(out["ok"], Value::Bool(false), "got: {out}");
    assert!(
        out["error"].as_str().unwrap_or_default().contains("cap"),
        "got: {out}"
    );
}

#[test]
fn an_unknown_encoding_reads_as_text() {
    // `encoding` is opt-in: anything that is not exactly "base64" keeps the
    // existing behaviour rather than failing a request over a typo.
    let (port, _seen) = spawn("127.0.0.1", body_reply("plain"));
    let out = json(&fetch(
        &domains(&["127.0.0.1"]),
        &format!("http://127.0.0.1:{port}/x"),
        r#"{"encoding":"utf-7"}"#,
    ));
    assert_eq!(out["body"], serde_json::json!("plain"), "got: {out}");
}
