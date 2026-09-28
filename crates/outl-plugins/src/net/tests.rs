//! Loopback tests for the `ctx.net.fetch` gate.
//!
//! Split out of `net.rs` so the gate itself stays readable; the suite is
//! deliberately against **real** servers on loopback rather than a mocked
//! client, because the bug this file exists for (#280) lived entirely in
//! what reqwest does after the socket opens.

use super::*;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// The plugin every test fetches as. A refusal must name it — knowing
/// *which* plugin tried is what makes the log line actionable.
const PLUGIN: &str = "acme.sync";

/// A grant for `PLUGIN` covering exactly these hosts.
fn domains(raw: &[&str]) -> NetGrant {
    NetGrant {
        plugin_id: PLUGIN.to_string(),
        domains: raw
            .iter()
            .map(|d| NetworkDomain::parse(d).expect("valid test domain"))
            .collect(),
    }
}

/// A loopback HTTP server that serves canned bytes and reports every
/// request head it saw.
///
/// Bound on `host` so one test can hold two servers whose **host strings**
/// differ (`127.0.0.1` vs `localhost`) while both land on this machine —
/// the host string is what [`NetworkDomain::matches_host`] matches on, so
/// that is what makes one of them "off the allowlist".
fn spawn(host: &'static str, reply: Vec<u8>) -> (u16, Receiver<String>) {
    let listener = TcpListener::bind((host, 0)).expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let head = read_head(&mut s);
            if tx.send(head).is_err() {
                break;
            }
            // A peer that walked away mid-body is the expected end of the
            // oversized-body case, not a test failure.
            let _ = s.write_all(&reply);
            let _ = s.flush();
        }
    });
    (port, rx)
}

fn read_head(s: &mut TcpStream) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match s.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => break,
        }
    }
    String::from_utf8_lossy(&head).into_owned()
}

fn json(raw: &str) -> Value {
    serde_json::from_str(raw).expect("fetch returns json")
}

fn redirect_to(port: u16) -> Vec<u8> {
    redirect_raw(&format!("http://localhost:{port}/next"))
}

/// A 302 pointing anywhere — including at a scheme no allowlist can cover.
fn redirect_raw(location: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

fn body_reply(body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

#[test]
fn a_redirect_off_the_allowlist_never_reaches_the_unapproved_host() {
    let (collector_port, collected) = spawn("localhost", body_reply("collected"));
    let (start_port, started) = spawn("127.0.0.1", redirect_to(collector_port));

    let out = json(&fetch(
        &domains(&["127.0.0.1"]),
        &format!("http://127.0.0.1:{start_port}/start"),
        r#"{"headers":{"X-Api-Key":"plugin-secret"}}"#,
    ));

    // Sanity: the approved host really was handed the secret. Without this
    // the leak assertion below could pass for the wrong reason.
    let first = started
        .recv_timeout(Duration::from_secs(5))
        .expect("the approved host served the first request");
    assert!(
        first.to_lowercase().contains("x-api-key: plugin-secret"),
        "the plugin's header never went out at all: {first}"
    );

    // reqwest strips `Authorization` across a cross-host redirect but
    // forwards every custom header, so `X-Api-Key` is the shape this leak
    // actually takes against a real API.
    if let Ok(head) = collected.recv_timeout(Duration::from_millis(750)) {
        panic!("a host outside the allowlist received the plugin's request:\n{head}");
    }

    assert_eq!(out["ok"], Value::Bool(false), "got: {out}");
    assert_eq!(out["status"], serde_json::json!(0), "got: {out}");
    let error = out["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("localhost") && error.contains("blocked"),
        "the error must name the blocked host: {error}"
    );
}

#[test]
fn a_redirect_to_another_approved_host_is_still_followed() {
    // Pins that the gate is an allowlist re-check and not a blanket
    // `Policy::none()`: an http→https upgrade, or an API redirecting
    // within its own approved domain, has to keep working.
    let (landing_port, landed) = spawn("localhost", body_reply("landed"));
    let (start_port, _started) = spawn("127.0.0.1", redirect_to(landing_port));

    let out = json(&fetch(
        &domains(&["127.0.0.1", "localhost"]),
        &format!("http://127.0.0.1:{start_port}/start"),
        "{}",
    ));

    assert_eq!(out["ok"], Value::Bool(true), "got: {out}");
    assert_eq!(out["body"], serde_json::json!("landed"), "got: {out}");
    assert!(
        landed.recv_timeout(Duration::from_secs(5)).is_ok(),
        "the approved redirect target should have been reached"
    );
}

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

// ---------------------------------------------------------------------------
// Log capture
//
// A refusal that only reaches the caller is a refusal a plugin can swallow
// with `catch (e) {}`. These tests need to see the *host's* record of it, so
// they install a subscriber for the duration of one call and read back what
// was emitted. Hand-rolled rather than pulling in `tracing-subscriber`: the
// whole need is "collect the message and the fields of each event".
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl Captured {
    fn lines(&self) -> Vec<String> {
        self.0.lock().expect("capture lock").clone()
    }

    /// The captured events as one blob, for a single readable assertion.
    fn joined(&self) -> String {
        self.lines().join("\n")
    }
}

struct Visitor(String);

impl tracing::field::Visit for Visitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{}={value:?} ", field.name()));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.push_str(&format!("{}={value} ", field.name()));
    }
}

impl Captured {
    /// Record one event. Called by `Router` on the emitting thread.
    fn event(&self, event: &tracing::Event<'_>) {
        let mut v = Visitor(String::new());
        event.record(&mut v);
        let line = format!("{} {}", event.metadata().level(), v.0);
        self.0.lock().expect("capture lock").push(line);
    }
}

// Where a capturing test's events land while it runs. Thread-local, so
// parallel tests cannot read each other's events.
thread_local! {
    static SINK: std::cell::RefCell<Option<Captured>> = const { std::cell::RefCell::new(None) };
}

/// Routes every event to the calling thread's `SINK`, if it has one.
///
/// This is installed **globally, once**, and that is the whole point.
/// `with_default` alone does not work here: `tracing`'s level filter is
/// process-wide, and a thread leaving its `with_default` drops that filter back
/// to off. A sibling test emitting its `warn!` inside that window has the event
/// discarded before any subscriber sees it, so the capture comes back empty and
/// the assertion reads as "the refusal was never logged" — a flake that only
/// shows up under parallelism and passes clean under `--test-threads=1`.
///
/// A global subscriber that always reports `TRACE` keeps the filter open for
/// the whole process, so no thread can close it under another.
struct Router;

impl tracing::Subscriber for Router {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        SINK.with(|s| {
            if let Some(cap) = s.borrow().as_ref() {
                cap.event(event);
            }
        });
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Run `f` with this thread's events collected.
fn capturing<T>(f: impl FnOnce() -> T) -> (T, Captured) {
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        // A second install would mean someone added another global subscriber;
        // the tests below would then be reading an empty sink and passing for
        // the wrong reason, so fail loudly instead.
        tracing::subscriber::set_global_default(Router)
            .expect("no other global tracing subscriber may be installed in this test binary");
    });

    let cap = Captured::default();
    SINK.with(|s| *s.borrow_mut() = Some(cap.clone()));
    let out = f();
    SINK.with(|s| *s.borrow_mut() = None);
    (out, cap)
}

#[test]
fn a_blocked_redirect_is_logged_not_only_returned() {
    // The plugin is handed the refusal as a string it may discard. If that
    // is the only witness, an attempted exfiltration through an open
    // redirect leaves no trace anywhere on the machine.
    let (collector_port, _collected) = spawn("localhost", body_reply("collected"));
    let (start_port, _started) = spawn("127.0.0.1", redirect_to(collector_port));

    let (_out, log) = capturing(|| {
        fetch(
            &domains(&["127.0.0.1"]),
            &format!("http://127.0.0.1:{start_port}/start"),
            r#"{"headers":{"X-Api-Key":"plugin-secret"}}"#,
        )
    });

    let seen = log.joined();
    assert!(
        seen.contains("localhost") && seen.to_lowercase().contains("block"),
        "the host must record the refused hop, not just hand it to the plugin: {seen:?}"
    );
    // Naming the plugin is what makes the line actionable: the user's next
    // move is to disable or uninstall something, and "a plugin tried to
    // exfiltrate a token" does not tell them which.
    assert!(
        seen.contains(PLUGIN),
        "the refusal must name the plugin that attempted it: {seen:?}"
    );
}

#[test]
fn a_refusal_logs_the_origin_never_the_credentials_in_the_url() {
    // The log is readable by anything local; the plugin's URL may carry a
    // token in its query, fragment or userinfo. The origin is enough to act on.
    for url in [
        "https://user:SECRET-USERINFO@blocked.test/search?api_key=SECRET-QUERY#SECRET-FRAG",
        "not a url?api_key=SECRET-INVALID",
    ] {
        let (_out, log) = capturing(|| fetch(&domains(&["example.com"]), url, "{}"));
        let seen = log.joined();
        assert!(
            seen.contains(PLUGIN),
            "the refusal must still be logged: {seen:?}"
        );
        assert!(
            !seen.contains("SECRET"),
            "a credential reached the log: {seen:?}"
        );
    }
    let (_out, log) = capturing(|| {
        fetch(
            &domains(&["example.com"]),
            "https://blocked.test/p?api_key=x",
            "{}",
        )
    });
    assert!(
        log.joined().contains("origin=https://blocked.test"),
        "the refusal must say where the plugin tried to go: {:?}",
        log.lines()
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

#[test]
fn a_non_http_scheme_is_refused_before_a_request_is_built() {
    // The allowlist is a rule about hosts, and `ftp://localhost/x` HAS a host
    // — so the grant covers it and nothing here says no. (`file://localhost`
    // does not: the URL spec normalises that host away, which is why the
    // hostless arm catches `file:` and this one does not.) Today the only
    // thing between an approved host on a non-HTTP scheme and a connection is
    // reqwest's own check, several layers down and pinned by no test here: a
    // reqwest bump or a client swap removes that backstop in silence.
    let out = json(&fetch(&domains(&["localhost"]), "ftp://localhost/x", "{}"));

    assert_eq!(out["ok"], Value::Bool(false), "got: {out}");
    let error = out["error"].as_str().unwrap_or_default();
    // Pinned against OUR wording, not on the word "scheme": reqwest's own
    // refusal ("builder error for url (ftp://…): URL scheme is not allowed")
    // says that too, so a looser assertion passes without this gate existing
    // — which is exactly how the backstop disappears unnoticed.
    assert!(
        error.contains("only http") && error.contains("ftp"),
        "the refusal must be this crate's, naming the scheme: {error}"
    );
}

#[test]
fn a_redirect_to_a_non_http_scheme_is_refused_even_when_the_host_is_approved() {
    // Same hole, reached through a hop: the redirect gate runs *before*
    // reqwest's scheme check (`redirect.rs` calls the policy, then checks the
    // scheme inside the `Follow` arm), so a grant covering `localhost` makes
    // the gate answer "follow" for `ftp://localhost/x`.
    let (start_port, _started) = spawn("127.0.0.1", redirect_raw("ftp://localhost/x"));

    let out = json(&fetch(
        &domains(&["127.0.0.1", "localhost"]),
        &format!("http://127.0.0.1:{start_port}/start"),
        "{}",
    ));

    assert_eq!(out["ok"], Value::Bool(false), "got: {out}");
    let error = out["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("only http") && error.contains("ftp"),
        "the refusal must be this crate's, naming the scheme: {error}"
    );
}

#[test]
fn a_custom_header_never_crosses_to_another_approved_host() {
    // reqwest strips `Authorization` / `Cookie` across a cross-host hop and
    // forwards everything else. With a multi-tenant grant (`*.s3.amazonaws.com`,
    // `*.github.io`) every attacker-controlled bucket is already "approved",
    // so the allowlist re-check alone does not keep the token home.
    let (landing_port, landed) = spawn("localhost", body_reply("landed"));
    let (start_port, started) = spawn("127.0.0.1", redirect_to(landing_port));

    let out = json(&fetch(
        &domains(&["127.0.0.1", "localhost"]),
        &format!("http://127.0.0.1:{start_port}/start"),
        r#"{"headers":{"X-Api-Key":"plugin-secret"}}"#,
    ));

    let first = started
        .recv_timeout(Duration::from_secs(5))
        .expect("the approved host served the first request");
    assert!(
        first.to_lowercase().contains("x-api-key: plugin-secret"),
        "the plugin's header never went out at all: {first}"
    );

    let second = landed
        .recv_timeout(Duration::from_secs(5))
        .expect("the approved redirect target should have been reached");
    assert!(
        !second.to_lowercase().contains("x-api-key"),
        "the plugin's secret crossed to a different host:\n{second}"
    );

    // Cleared, not refused: the hop is legitimate and keeps working.
    assert_eq!(out["ok"], Value::Bool(true), "got: {out}");
    assert_eq!(out["body"], serde_json::json!("landed"), "got: {out}");
}

/// A server that accepts, reads the request, and then just holds the socket
/// open forever. `spawn` cannot do this — it closes the stream at the end of
/// each iteration, which the client sees as a connection reset, not a hang.
fn spawn_silent(host: &'static str) -> u16 {
    let listener = TcpListener::bind((host, 0)).expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            read_head(&mut s);
            held.push(s); // never answered, never closed
        }
    });
    port
}

#[test]
fn the_requested_timeout_actually_bounds_the_call() {
    // The hop loop is what carries the deadline now, so this pins that the
    // plugin's `timeoutMs` reaches it at all. Drop the per-request timeout
    // and this hangs forever against a host that accepts and never answers —
    // which, on the shared plugin thread, is every plugin feature in the app.
    // `timeout_ms_is_clamped_to_the_ceiling` is the other half: it pins what
    // an *absurd* number becomes, which cannot be tested by waiting.
    let port = spawn_silent("127.0.0.1");

    let started = std::time::Instant::now();
    let out = json(&fetch(
        &domains(&["127.0.0.1"]),
        &format!("http://127.0.0.1:{port}/hang"),
        r#"{"timeoutMs":300}"#,
    ));
    let elapsed = started.elapsed();

    assert_eq!(out["ok"], Value::Bool(false), "got: {out}");
    assert!(
        elapsed < Duration::from_secs(5),
        "the call ran {elapsed:?} against a 300ms budget"
    );
}
