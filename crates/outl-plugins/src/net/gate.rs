//! The rules one hop has to satisfy, as pure functions.
//!
//! Split out of `net.rs` so each rule has an address and a unit test that
//! does not need a socket. Two of them (the scheme allowlist, the downgrade
//! refusal) are otherwise only reachable through a server that can speak
//! TLS, which is how they went untested long enough to matter.

use reqwest::Url;

use crate::permission::NetworkDomain;

/// Request timeout used when the plugin names none.
pub(super) const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// Ceiling on a plugin-supplied `timeoutMs`, covering **every hop together**.
///
/// The fetch is blocking and runs on the one thread that owns the Boa
/// `Context` — the GUI clients' `outl-plugin-host` thread, or the CLI's main
/// thread. Every other plugin request (list commands, toolbar, keybindings,
/// transformers, run command) waits on that thread with a plain `recv()`, so
/// an unbounded timeout is not "this fetch is slow", it is *every plugin
/// feature in the app, permanently*, against a host that merely accepts the
/// connection and never answers.
///
/// 60s rather than the 10s default: a buffered call to a slow JSON API — an
/// LLM completion is the realistic worst case — legitimately runs past ten
/// seconds, and the body is read whole anyway (see `MAX_RESPONSE_BYTES`), so
/// there is no streaming case that needs minutes. The point of the ceiling is
/// that the thread comes back on its own, with no user action, in bounded
/// time.
pub(super) const MAX_TIMEOUT_MS: u64 = 60_000;

/// Resolve the plugin's requested timeout against the default and the ceiling.
pub(super) fn clamp_timeout_ms(requested: Option<u64>) -> u64 {
    requested.unwrap_or(DEFAULT_TIMEOUT_MS).min(MAX_TIMEOUT_MS)
}

/// Whether the plugin's headers may travel on to this hop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hop {
    /// Same origin as the previous URL — headers carry over.
    SameOrigin,
    /// Host, port or scheme changed — the plugin's headers stop here.
    CrossOrigin,
}

/// Refuse a URL a plugin fetch may never use, whatever the grants say.
///
/// The allowlist is a rule about **hosts**, so it has nothing to say about
/// schemes: `ftp://localhost/x` and `ws://localhost/x` both carry a host that
/// a `network:localhost` grant covers. Until now the only thing that refused
/// them was reqwest — and on a redirect that check runs *after* the policy
/// (`redirect.rs` calls `policy.check(..)`, then tests the scheme inside the
/// `Follow` arm), so the gate was already answering "follow" for a scheme it
/// had no opinion about. A backstop one dependency bump away from vanishing,
/// which is the same as not having one.
///
/// `file:` and `data:` do not reach here at all: the URL spec normalises
/// `file://localhost/x` to a hostless URL, so they fall out at the host check.
/// That is an accident of the parser, not a rule, which is why this exists.
pub(super) fn check_scheme(url: &Url) -> Result<(), String> {
    match url.scheme() {
        "http" | "https" => Ok(()),
        other => Err(format!(
            "scheme `{other}` blocked (only http and https are allowed for a plugin fetch)"
        )),
    }
}

/// Check the host of a URL against the plugin's approved domains.
pub(super) fn check_host(domains: &[NetworkDomain], url: &Url) -> Result<(), String> {
    let Some(host) = url.host_str() else {
        // No host to match a grant against. Nothing can approve it, so
        // nothing does.
        return Err("url has no host to match a network:<domain> permission".to_string());
    };
    if domains.iter().any(|d| d.matches_host(host)) {
        return Ok(());
    }
    Err(format!(
        "host `{host}` blocked (no matching network:<domain> permission)"
    ))
}

/// Judge a redirect from `from` to `to`, returning what the next request may
/// carry.
///
/// Order matters: scheme, then host, then downgrade. A refusal names the
/// first rule the hop broke, so the error a plugin sees is the reason and not
/// a generic "blocked".
pub(super) fn check_hop(domains: &[NetworkDomain], from: &Url, to: &Url) -> Result<Hop, String> {
    check_scheme(to)?;
    check_host(domains, to)?;
    if from.scheme() == "https" && to.scheme() == "http" {
        return Err(format!(
            "redirect from https to http on `{}` blocked (scheme downgrade)",
            to.host_str().unwrap_or("?")
        ));
    }
    Ok(origin_change(from, to))
}

/// Whether the two URLs share an origin, using **reqwest's own definition**
/// of the cross-host boundary — host, port or scheme differing.
///
/// Deliberately the same predicate `redirect::remove_sensitive_headers` uses,
/// because the fix this implements is exactly "treat every plugin-supplied
/// header the way reqwest already treats `Authorization`". reqwest strips
/// `Authorization`, `Cookie` and `Proxy-Authorization` there and forwards
/// everything else, so a plugin holding `secrets` and sending its keychain
/// token as `X-Api-Key` — the shape a real API actually wants — had no
/// protection at all. Re-deriving a different boundary here would be a second
/// opinion about the same question.
fn origin_change(from: &Url, to: &Url) -> Hop {
    let same = from.host_str() == to.host_str()
        && from.port_or_known_default() == to.port_or_known_default()
        && from.scheme() == to.scheme();
    if same {
        Hop::SameOrigin
    } else {
        Hop::CrossOrigin
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domains(raw: &[&str]) -> Vec<NetworkDomain> {
        raw.iter()
            .map(|d| NetworkDomain::parse(d).expect("valid test domain"))
            .collect()
    }

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("valid test url")
    }

    #[test]
    fn timeout_ms_is_clamped_to_the_ceiling() {
        // `fetch(url, { timeoutMs: 2**53 })` against a host that accepts the
        // connection and never answers is the whole attack: the plugin thread
        // is shared, and every caller waits on it with a bare `recv()`.
        assert_eq!(clamp_timeout_ms(Some(u64::MAX)), MAX_TIMEOUT_MS);
        assert_eq!(
            clamp_timeout_ms(Some(9_007_199_254_740_992)),
            MAX_TIMEOUT_MS
        );
        // Under the ceiling the plugin's own number is honoured, and a plugin
        // that asks for nothing gets the default rather than the ceiling.
        assert_eq!(clamp_timeout_ms(Some(250)), 250);
        assert_eq!(clamp_timeout_ms(None), DEFAULT_TIMEOUT_MS);
    }

    #[test]
    fn a_redirect_that_downgrades_https_to_http_is_refused() {
        // Same approved host, so every host-shaped check says yes. What
        // changes is that the request now goes out in clear — and because the
        // scheme changed, reqwest counts it cross-host and drops
        // `Authorization` while forwarding `X-Api-Key`, so the one header a
        // plugin with `secrets` actually uses is the one that survives.
        let err = check_hop(
            &domains(&["api.example.com"]),
            &url("https://api.example.com/a"),
            &url("http://api.example.com/b"),
        )
        .expect_err("a downgrade must be refused");
        assert!(
            err.contains("downgrade") && err.contains("api.example.com"),
            "the refusal must name the downgrade and the host: {err}"
        );
    }

    #[test]
    fn an_https_upgrade_on_the_same_host_is_still_followed() {
        // The load-bearing allow for the rule above: `http → https` is the
        // legitimate direction and must not be caught by it.
        let hop = check_hop(
            &domains(&["api.example.com"]),
            &url("http://api.example.com/a"),
            &url("https://api.example.com/b"),
        )
        .expect("an upgrade is legitimate");
        // The origin still changed, so the headers still stop — the same
        // verdict reqwest reaches for `Authorization`.
        assert_eq!(hop, Hop::CrossOrigin);
    }

    #[test]
    fn a_hop_inside_one_origin_keeps_the_plugins_headers() {
        let hop = check_hop(
            &domains(&["api.example.com"]),
            &url("https://api.example.com/a"),
            &url("https://api.example.com/b"),
        )
        .expect("same origin");
        assert_eq!(hop, Hop::SameOrigin);
    }

    #[test]
    fn a_port_change_on_one_host_is_a_cross_origin_hop() {
        // The allowlist is host-only, so both ports are "approved" — but they
        // are different services, and reqwest counts the port too.
        let hop = check_hop(
            &domains(&["localhost"]),
            &url("http://localhost:8080/a"),
            &url("http://localhost:9999/b"),
        )
        .expect("both ports are on an approved host");
        assert_eq!(hop, Hop::CrossOrigin);
    }

    #[test]
    fn a_non_http_scheme_is_refused_before_the_host_is_even_consulted() {
        let err = check_hop(
            &domains(&["localhost"]),
            &url("http://localhost/a"),
            &url("ftp://localhost/x"),
        )
        .expect_err("ftp is not a plugin fetch scheme");
        assert!(err.contains("only http"), "got: {err}");
    }
}
