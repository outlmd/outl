//! `ctx.net.fetch` — the plugin's only outbound network path.
//!
//! One rule: a request only ever reaches a host covered by an approved
//! `network:<domain>` grant. Everything here exists to keep that true for the
//! whole exchange, not just for the URL the plugin typed.
//!
//! ## Why the redirect chain is driven here instead of by reqwest
//!
//! The first version of this gate was a `reqwest::redirect::Policy::custom`
//! that re-asked the allowlist on every hop, which is the right *rule* and the
//! wrong *seam*. A redirect policy may answer follow, stop or error — it
//! cannot touch the headers of the request it is judging, and reqwest strips
//! only `Authorization`, `Cookie` and `Proxy-Authorization` across a cross-host
//! hop. So a plugin holding `secrets` and sending its keychain token the way a
//! real API wants it (`X-Api-Key: …`) handed that token to the next host
//! intact — and with a multi-tenant grant (`*.s3.amazonaws.com`, `*.github.io`)
//! the next host can be any bucket the attacker owns, still "approved".
//!
//! Driving the hops here buys the one thing the policy seam could not: the
//! plugin's headers stop at an origin change, without refusing a hop an API is
//! entitled to make. It also puts the scheme allowlist, the hop cap and the
//! overall deadline in one readable place, instead of leaving the scheme to a
//! check inside reqwest that a version bump can move with nothing here failing.
//!
//! ## What this gate does not cover
//!
//! Every rule here is about the **host string**, never about where that string
//! resolves to. A plugin author can ask for `network:api.their-own.example`,
//! have the user approve it, and point the A record at `127.0.0.1`,
//! `169.254.169.254` or anything on the LAN. The plugin has no filesystem and
//! no socket, so reaching loopback is a sandbox escape, not merely a request:
//! outl's own MCP server, a local model server, cloud instance metadata. The
//! redirect gate is no help, because the hop never leaves the approved host.
//!
//! Closing it means a `reqwest::dns::Resolve` that refuses to hand back a
//! non-global address, which is a bigger change than it sounds: `IpAddr::
//! is_global` is unstable on the pinned toolchain, resolving without blocking
//! reqwest's own runtime thread needs a spawn-blocking dependency this crate
//! does not have, and every test below talks to `127.0.0.1`, so the gate needs
//! a deliberate escape hatch before it can ship.

use std::io::Read;
use std::time::{Duration, Instant};

use reqwest::{Method, StatusCode, Url};
use serde_json::Value;

use crate::permission::{NetGrant, NetworkDomain};

mod gate;

use gate::{check_hop, check_host, check_scheme, clamp_timeout_ms, Hop};

/// Ceiling on how many bytes of a response body are handed back to the plugin.
///
/// The body is buffered whole and then embedded in a JSON string for the JS
/// side, so its size is this process's memory. An approved host answering with
/// a multi-gigabyte body is an OOM that needs no attacker creativity, and on
/// mobile it is a jetsam kill. 8 MiB is far past any JSON API response and far
/// short of trouble.
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

/// Ceiling on redirect hops.
///
/// Without it an approved host bouncing a request around its own domain loops
/// until the deadline — every hop legitimate, the chain never ending.
const MAX_REDIRECTS: usize = 10;

/// Perform a gated blocking HTTP request. Returns a JSON string the JS side
/// parses: `{ ok, status, body }` on success, `{ ok: false, status: 0, error }`
/// when denied or on a transport error.
///
/// The grant's `plugin_id` never reaches the plugin — it is there so a refusal
/// names *who* tried, which is the difference between a log line and an
/// actionable one.
///
/// The `network:<domain>` grant is enforced on **every hop**, the plugin's
/// headers stop at an origin change, the scheme is restricted to http/https,
/// the timeout is clamped, and the body is capped at [`MAX_RESPONSE_BYTES`].
/// Every refusal goes back in the error shape above **and** to the host log —
/// a plugin can `catch (e) {}` the first one.
pub(crate) fn fetch(grant: &NetGrant, url: &str, opts_json: &str) -> String {
    let Ok(start) = Url::parse(url) else {
        return refuse(grant, url, "invalid url");
    };
    if let Err(e) = check_scheme(&start) {
        return refuse(grant, url, &e);
    }
    if let Err(e) = check_host(&grant.domains, &start) {
        return refuse(grant, url, &e);
    }

    let opts: Value = serde_json::from_str(opts_json).unwrap_or(Value::Null);
    let req = PluginRequest::from_opts(&opts);
    let timeout_ms = clamp_timeout_ms(opts.get("timeoutMs").and_then(Value::as_u64));

    // No client-level timeout: with the chain driven here that would be a
    // *per-hop* budget, so ten hops could hold the thread for ten times what
    // the plugin asked for. The deadline below covers the whole exchange.
    let client = match reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(e) => return transport_error(&e.to_string()),
    };

    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    match send_chain(&client, &grant.domains, req, start, deadline) {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let ok = resp.status().is_success();
            match read_body_capped(resp) {
                Ok(body) => {
                    serde_json::json!({ "ok": ok, "status": status, "body": body }).to_string()
                }
                Err(e) => refuse(grant, url, &e),
            }
        }
        Err(SendError::Refused(e)) => refuse(grant, url, &e),
        Err(SendError::Transport(e)) => transport_error(&e),
    }
}

/// What the plugin asked to send, minus anything the gate has taken away.
struct PluginRequest {
    method: Method,
    /// The plugin's headers. Emptied on the first cross-origin hop; see
    /// [`Hop`].
    headers: Vec<(String, String)>,
    body: Option<String>,
}

impl PluginRequest {
    fn from_opts(opts: &Value) -> Self {
        let method = match opts
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_uppercase()
            .as_str()
        {
            "POST" => Method::POST,
            "PUT" => Method::PUT,
            "DELETE" => Method::DELETE,
            "PATCH" => Method::PATCH,
            _ => Method::GET,
        };
        let headers = opts
            .get("headers")
            .and_then(Value::as_object)
            .map(|h| {
                h.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        let body = opts.get("body").and_then(Value::as_str).map(str::to_owned);
        Self {
            method,
            headers,
            body,
        }
    }

    /// Apply the method / body rewrite a redirect status calls for, mirroring
    /// reqwest's redirect layer (RFC 7231 §6.4): `301`/`302` turn a POST into a
    /// GET, `303` turns anything but HEAD into a GET, `307`/`308` keep both.
    /// Getting this wrong replays a POST body at a destination that asked for
    /// a GET.
    fn rewrite_for(&mut self, status: StatusCode) {
        match status {
            StatusCode::MOVED_PERMANENTLY | StatusCode::FOUND if self.method == Method::POST => {
                self.method = Method::GET;
                self.body = None;
            }
            StatusCode::SEE_OTHER => {
                if self.method != Method::HEAD {
                    self.method = Method::GET;
                }
                self.body = None;
            }
            _ => {}
        }
    }
}

/// A refusal by this gate, or the network failing underneath it. Kept apart
/// because only the first is a security event worth a log line.
enum SendError {
    Refused(String),
    Transport(String),
}

/// Follow the redirect chain by hand, re-asking [`check_hop`] every time.
fn send_chain(
    client: &reqwest::blocking::Client,
    domains: &[NetworkDomain],
    mut req: PluginRequest,
    mut url: Url,
    deadline: Instant,
) -> Result<reqwest::blocking::Response, SendError> {
    let mut hops = 0usize;
    loop {
        let Some(remaining) = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
        else {
            return Err(SendError::Transport("request timed out".into()));
        };

        let mut builder = client
            .request(req.method.clone(), url.clone())
            .timeout(remaining);
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = &req.body {
            builder = builder.body(body.clone());
        }
        let resp = builder
            .send()
            .map_err(|e| SendError::Transport(describe_causes(&e)))?;

        let Some(next) = redirect_target(&resp, &url) else {
            return Ok(resp);
        };
        if hops >= MAX_REDIRECTS {
            return Err(SendError::Refused(format!(
                "too many redirects (limit {MAX_REDIRECTS})"
            )));
        }
        match check_hop(domains, &url, &next) {
            // The plugin's headers do not travel past an origin change. This
            // is the whole reason the chain is driven here: a redirect policy
            // could only have refused the hop, which breaks an API
            // legitimately redirecting to its own CDN.
            Ok(Hop::CrossOrigin) => req.headers.clear(),
            Ok(Hop::SameOrigin) => {}
            Err(e) => return Err(SendError::Refused(e)),
        }
        req.rewrite_for(resp.status());
        url = next;
        hops += 1;
    }
}

/// Where a response wants us to go next, or `None` if it is not a redirect we
/// follow. A 3xx with no usable `Location` is handed back as the answer.
fn redirect_target(resp: &reqwest::blocking::Response, current: &Url) -> Option<Url> {
    match resp.status() {
        StatusCode::MOVED_PERMANENTLY
        | StatusCode::FOUND
        | StatusCode::SEE_OTHER
        | StatusCode::TEMPORARY_REDIRECT
        | StatusCode::PERMANENT_REDIRECT => {}
        _ => return None,
    }
    let location = resp.headers().get(reqwest::header::LOCATION)?;
    current.join(location.to_str().ok()?).ok()
}

/// Record a refusal and render it for the plugin.
///
/// Both halves, always, from one place. The string handed back is the only
/// other witness, and a plugin can make it disappear with `catch (e) {}` — so
/// an attempted exfiltration through an open redirect, or a host answering
/// with a body sized to kill the process, would otherwise leave no trace on
/// the machine at all. Every refusal arm in this module goes through here, so
/// a new one cannot be added silently.
fn refuse(grant: &NetGrant, url: &str, reason: &str) -> String {
    tracing::warn!(
        plugin = grant.plugin_id,
        origin = %log_origin(url),
        "plugin fetch refused: {reason}"
    );
    error_json(reason)
}

/// The part of a plugin-supplied URL the host log may keep: its origin.
///
/// Path, query, fragment and userinfo are where credentials ride
/// (`?api_key=…`, `https://user:token@…`), and the log is readable by
/// anything local. The origin still says where the plugin tried to go,
/// which is what makes the line actionable. An unparseable URL is logged
/// as a placeholder, never verbatim, for the same reason.
fn log_origin(url: &str) -> String {
    Url::parse(url).map_or_else(
        |_| "<invalid url>".to_string(),
        |u| u.origin().ascii_serialization(),
    )
}

/// The network failed. Not a refusal — nobody decided anything — so it is not
/// logged as one.
fn transport_error(reason: &str) -> String {
    error_json(reason)
}

fn error_json(msg: &str) -> String {
    serde_json::json!({ "ok": false, "status": 0, "error": msg }).to_string()
}

/// Read the body, refusing rather than buffering past [`MAX_RESPONSE_BYTES`].
///
/// Two checks because they cover different responses: `content_length` catches
/// a declared oversize before a byte is read, and the capped reader is what
/// bounds a chunked or close-delimited body, whose size is not declared at all.
/// The second one is the one that actually prevents the OOM.
fn read_body_capped(resp: reqwest::blocking::Response) -> Result<String, String> {
    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BYTES {
            return Err(format!(
                "response body is {len} bytes, over the {MAX_RESPONSE_BYTES}-byte cap"
            ));
        }
    }
    // One byte past the cap, so "exactly at the cap" and "over it" are
    // distinguishable without reading the overage.
    let mut buf = Vec::new();
    resp.take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| e.to_string())?;
    if buf.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(format!(
            "response body exceeds the {MAX_RESPONSE_BYTES}-byte cap"
        ));
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Render an error with its cause chain.
///
/// `reqwest::Error`'s `Display` prints only its own kind ("error sending
/// request for url (…)") and leaves the detail in `source()`, so without the
/// chain a plugin gets an error naming the URL it asked for and nothing about
/// what actually went wrong.
fn describe_causes(e: &(dyn std::error::Error + 'static)) -> String {
    let mut out = e.to_string();
    let mut source = e.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

#[cfg(test)]
mod tests;
