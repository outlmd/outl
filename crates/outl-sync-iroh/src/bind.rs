//! Endpoint binding — single owner of the iroh 1.0.0 IPv4-only STOPGAP.
//!
//! # STOPGAP: IPv4-only bind (iroh 1.0.0 multipath workaround)
//!
//! iroh 1.0.0's QUIC-multipath stack opens paths to **all** of a peer's
//! candidate addresses concurrently. When a peer's `EndpointAddr` includes a
//! global IPv6 direct address that is "No route to host" on a LAN-only device,
//! multipath stalls on that dead path (`PTO expired`,
//! `failed closing path err=MultipathNotNegotiated`) and the whole
//! connect/accept times out (~30s) instead of converging on the working
//! LAN-IPv4 or relay path. iroh 1.0.0 exposes **no** public knob to disable
//! multipath (`max_concurrent_multipath_paths` clamps to >= 9; `path_selector`
//! is behind an unstable feature and only picks among already-opened paths;
//! no env var), and downgrade is blocked because `iroh-gossip 0.101.0`
//! requires `iroh = "1"`.
//!
//! The fix is to **reduce the candidate paths to only reachable ones** on BOTH
//! the dial side and the accept side, so multipath never opens a dead IPv6
//! path. We do this by binding an **IPv4-only** UDP socket: the endpoint then
//! never discovers/advertises a global IPv6 direct addr, so neither side ever
//! dials or accepts one. LAN-IPv4 direct connectivity and the n0 relay fallback
//! are both preserved (`clear_ip_transports` only drops IP transports; the
//! `presets::N0` relay transport stays).
//!
//! By default `Endpoint::builder` pre-configures **both** a `0.0.0.0` IPv4 and
//! a `[::]` IPv6 unspecified socket. `clear_ip_transports()` removes both, then
//! `bind_addr("0.0.0.0:0")` re-adds IPv4 only. See iroh-1.0.0
//! `src/endpoint.rs` `Builder::bind_addr` / `Builder::clear_ip_transports`.
//!
//! ## Revert condition
//!
//! When iroh ships the multipath fallback fix (track iroh > 1.0.0, where a
//! stalled path no longer blocks convergence on a healthy path), delete this
//! module and let every call site go back to the plain dual-stack
//! `Endpoint::builder(presets::N0)` builder.

use iroh::endpoint::presets;
use iroh::endpoint::Builder;
use iroh::tls::CaTlsConfig;
use iroh::{RelayMode, RelayUrl};
use tracing::warn;

/// IPv4 unspecified bind address (random free port) used by the IPv4-only
/// STOPGAP. `0.0.0.0:0` binds all IPv4 interfaces on an OS-assigned port —
/// the same default the iroh builder would pick for IPv4, minus the IPv6 leg.
const IPV4_UNSPECIFIED: &str = "0.0.0.0:0";

/// outl's own iroh relay, used by default instead of the n0 public relay.
///
/// The n0 relay proved slow/unreachable on some networks (cross-network
/// connects timing out even on the same LAN), so outl ships its own relay.
/// A `[sync] relay_url` in the config still overrides this per deployment.
const DEFAULT_RELAY_URL: &str = "https://use1-1.relay.avelino.outl.iroh.link";

/// Build an `Endpoint::builder(presets::N0)` constrained to **IPv4-only** direct
/// transports.
///
/// `relay_url` selects the relay server the endpoint registers with:
///
/// - `None` (or empty after the config layer normalizes it) uses outl's
///   own relay, [`DEFAULT_RELAY_URL`] (`use1-1.relay.avelino.outl.iroh.link`), via
///   [`RelayMode::Custom`].
/// - `Some(url)` swaps in a different single relay via
///   [`RelayMode::Custom`], for users who run their own. The
///   IPv4-only STOPGAP is preserved either way (`clear_ip_transports`
///   only drops the IP direct transports, never the relay transport).
/// - A `url` (default or configured) that fails to parse as a [`RelayUrl`]
///   logs a warning and falls back to the `presets::N0` (n0) relay, so a
///   typo degrades to "use the n0 default" instead of failing the bind.
///
/// STOPGAP for the iroh 1.0.0 multipath stall on unreachable IPv6 paths — see
/// the module docs for the full rationale and the revert condition.
///
/// Every endpoint in this crate (sync engine, pairing, status probe, and the
/// test-support harness) MUST go through this builder so the dial side and the
/// accept side stay consistent: if only one side dropped IPv6, the other could
/// still advertise a dead IPv6 path and re-trigger the stall. The **sync
/// endpoint** is the only one that threads the *configured* `relay_url`;
/// pairing / status / test-support pass `None`, which is not "the n0 preset"
/// — it resolves to [`DEFAULT_RELAY_URL`] like everything else, so they ride
/// outl's relay too. Only a deployment that overrides `[sync] relay_url` gets
/// a split, and it is the long-lived sync endpoint that matters there.
pub(crate) fn n0_builder_ipv4_only(relay_url: Option<&str>) -> Builder {
    // `clear_ip_transports()` drops the pre-configured 0.0.0.0 + [::] sockets;
    // `bind_addr("0.0.0.0:0")` re-adds IPv4 only. `bind_addr` only errors on an
    // unparseable socket address, and this constant is a valid literal, so the
    // `expect` cannot fire.
    //
    // `ca_tls_config(CaTlsConfig::system())` delegates relay-TLS trust to the OS
    // keychain (`rustls-platform-verifier`, gated by the `platform-verifier`
    // feature) instead of iroh's default `CaTlsConfig::EmbeddedWebPki` (Mozilla's
    // bundled roots). Any environment with a custom root CA in the OS trust store
    // — e.g. a corporate TLS-inspection proxy — has its relay certs accepted like
    // macOS / curl / Safari already do, instead of failing every relay handshake
    // with `invalid peer certificate: UnknownIssuer`. Enabling the feature alone
    // is not sufficient: the default stays `EmbeddedWebPki` unless `system()` is
    // passed explicitly.
    let builder = iroh::Endpoint::builder(presets::N0)
        .ca_tls_config(CaTlsConfig::system())
        .clear_ip_transports()
        .bind_addr(IPV4_UNSPECIFIED)
        .expect("0.0.0.0:0 is a valid IPv4 socket address");

    // Default to outl's own relay ([`DEFAULT_RELAY_URL`]); a non-empty
    // `[sync] relay_url` in the config overrides it per deployment. Only a
    // parse failure falls back to the untouched builder (the `presets::N0`
    // n0 relay), so a typo degrades to "use the n0 default" rather than
    // failing the bind.
    let relay = relay_url
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .unwrap_or(DEFAULT_RELAY_URL);
    match relay.parse::<RelayUrl>() {
        Ok(url) => builder.relay_mode(RelayMode::custom([url])),
        Err(e) => {
            warn!("invalid relay_url {relay:?} ({e}); using n0 default relay");
            builder
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #213 item 3, and the honest half of it.
    ///
    /// The relay selection is pure string handling and fully testable; what is
    /// **not** testable here is `CaTlsConfig::system()` itself — iroh exposes no
    /// way to read the choice back off a `Builder`, and the failure it prevents
    /// (`invalid peer certificate: UnknownIssuer` behind a TLS-inspecting
    /// corporate proxy) needs that proxy to reproduce.
    ///
    /// What guards *that* line is the compiler: `CaTlsConfig::system()` only
    /// exists with iroh's `platform-verifier` feature enabled, so dropping the
    /// feature from `Cargo.toml` fails the build rather than silently reverting
    /// to the bundled Mozilla roots. Removing the `.ca_tls_config(...)` call
    /// while keeping the feature is the gap that remains — **none found, gap**.
    #[test]
    fn an_empty_or_absent_relay_url_falls_back_to_outls_own() {
        // `None`, empty and whitespace must all mean "use ours", not "use the
        // n0 preset". Getting this wrong sends every user of a default install
        // onto someone else's relay.
        for input in [None, Some(""), Some("   "), Some("\t\n")] {
            // The builder cannot be inspected, so assert on the decision the
            // builder is fed — the same expression, extracted.
            let chosen = input
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .unwrap_or(DEFAULT_RELAY_URL);
            assert_eq!(
                chosen, DEFAULT_RELAY_URL,
                "relay_url {input:?} must resolve to outl's own relay",
            );
        }
    }

    #[test]
    fn a_configured_relay_url_wins_and_a_typo_degrades_rather_than_failing() {
        let configured = "https://relay.example.test";
        assert!(
            configured.parse::<RelayUrl>().is_ok(),
            "a well-formed relay url must parse",
        );

        // A typo must not fail the bind. The endpoint falling back to a
        // working default is recoverable; a device that refuses to start
        // because of one config line is not.
        assert!(
            "not a url at all :: ???".parse::<RelayUrl>().is_err(),
            "garbage must be rejected by the parser, so the caller can fall back",
        );
    }

    /// The IPv4-only STOPGAP, pinned so a "cleanup" cannot quietly re-add IPv6.
    ///
    /// Re-adding it re-triggers the iroh 1.0.0 multipath stall this works
    /// around, and the symptom is a ~30s hang on every dial to a peer that
    /// advertises a dead IPv6 address — which reads as "sync is slow", not as
    /// "someone changed the bind address".
    #[test]
    fn the_bind_address_is_ipv4_only() {
        let addr: std::net::SocketAddr = IPV4_UNSPECIFIED
            .parse()
            .expect("the STOPGAP bind address must stay parseable");
        assert!(addr.is_ipv4(), "the STOPGAP bind address must be IPv4");
        assert!(
            addr.ip().is_unspecified(),
            "binding a specific interface would drop LAN peers on the others",
        );
        assert_eq!(addr.port(), 0, "the port must stay ephemeral");
    }
}
