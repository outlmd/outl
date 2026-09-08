//! Endpoint binding — the single owner of how this crate reaches the network.
//!
//! Two things must hold for **every** endpoint outl binds, and neither is
//! optional per call site:
//!
//! - the IPv4-only STOPGAP below, because dial and accept must agree or the
//!   multipath stall comes back from whichever side forgot;
//! - LAN discovery, because a peer two metres away should not depend on an
//!   internet relay to be found (issue #149).
//!
//! [`bind_ipv4_only`] is therefore the only way to get an endpoint out of this
//! crate, and `every_endpoint_in_this_crate_binds_through_the_one_owner` fails
//! if a new one goes around it.
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

use anyhow::{Context, Result};
use iroh::endpoint::presets;
use iroh::endpoint::Builder;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, RelayMode, RelayUrl};
#[cfg(not(target_os = "ios"))]
use iroh_mdns_address_lookup::MdnsAddressLookup;
use tracing::{debug, warn};

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
/// Private to this module on purpose: callers want [`bind_ipv4_only`], which
/// wraps this and adds the LAN discovery a bare `.bind()` would skip. Both
/// halves have to be consistent across every endpoint — if only one side
/// dropped IPv6, the other could still advertise a dead IPv6 path and
/// re-trigger the stall. The **sync endpoint** is the only one that threads the
/// *configured* `relay_url`;
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

/// Bind an endpoint for `alpns` under `secret_key`, with LAN discovery attached.
///
/// **Every endpoint in this crate binds here** — sync engine, pairing, the CLI
/// status probe and the test-support harness. It is one function rather than
/// four call sites sharing a builder because the two things that must stay
/// consistent across every endpoint are not both expressible in a builder:
/// the IPv4-only STOPGAP is (see [`n0_builder_ipv4_only`]), and the mDNS
/// attach below is not.
///
/// # Why mDNS is attached after the bind instead of on the builder
///
/// The obvious wiring is `Builder::address_lookup(MdnsAddressLookup::builder())`,
/// and it is wrong here. iroh runs each registered builder inside `bind()` with
/// `into_address_lookup(&ep)?` (iroh-1.0.3 `endpoint.rs:303`), so an mDNS
/// service that cannot start takes the **whole bind** down with it — and
/// `MdnsAddressLookupBuilder::build` fails whenever the host allows neither
/// IPv4 nor IPv6 multicast. That is not a hypothetical: a locked-down corporate
/// network, a container with multicast disabled, and iOS without the
/// `com.apple.developer.networking.multicast` entitlement all land there.
///
/// Wiring it on the builder would therefore trade "same-LAN peers cannot find
/// each other" for "sync does not start at all", which is a strictly worse
/// failure and one the user cannot diagnose. Attaching post-bind through
/// [`iroh::address_lookup::AddressLookupServices::add`] keeps mDNS **additive**:
/// it either helps, or it logs one line and the relay path carries sync exactly
/// as it did before this existed.
pub(crate) async fn bind_ipv4_only(
    relay_url: Option<&str>,
    secret_key: &iroh::SecretKey,
    alpns: Vec<Vec<u8>>,
    advertise: Advertise,
) -> Result<Endpoint> {
    let endpoint = n0_builder_ipv4_only(relay_url)
        .secret_key(secret_key.clone())
        .alpns(alpns)
        .bind()
        .await
        .context("bind iroh endpoint")?;
    attach_mdns(&endpoint, advertise);
    Ok(endpoint)
}

/// Whether an endpoint publishes its own address over mDNS, or only reads
/// other devices'.
///
/// # Why this is not always `Yes`
///
/// Every endpoint in this crate binds under the **same device identity**, so
/// they all advertise the *same node id* — at different ephemeral ports. That
/// is fine for the long-lived ones, and actively harmful for a transient one:
/// `outl peer status` binds an endpoint, probes, and drops it seconds later.
/// If it advertised, every device on the LAN would learn a second address for
/// this node id that is dead the moment the command exits, and iroh 1.0.0
/// multipath stalls on a dead candidate rather than skipping it — the exact
/// ~30s hang the IPv4-only STOPGAP above exists to avoid.
///
/// So a diagnostic command would degrade sync for every peer that happened to
/// resolve during it. Reading is free and has no such effect, so the probe
/// still resolves; it just does not publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Advertise {
    /// Publish this endpoint's addresses on the LAN. Only the long-lived sync
    /// endpoint, which is the only one whose address stays true.
    Yes,
    /// Resolve other devices, publish nothing. For a transient endpoint whose
    /// address stops being true when the process exits: the `outl peer status`
    /// probe, and the one-shot pairing endpoint.
    No,
    /// Attach no LAN discovery at all: no advertise, and no browse.
    ///
    /// For the loopback test harness, and **not** a micro-optimisation.
    /// `Advertise::No` does not avoid the mDNS machinery: upstream gates only
    /// `with_addrs` on that flag and spawns the `Discoverer` regardless, so
    /// every test endpoint bound 5353 and browsed. At the endpoint counts
    /// `tests/chaos.rs` reaches, that contention was enough to time out
    /// loopback dials that have nothing to do with discovery.
    Off,
}

/// Attach mDNS local address lookup to an already-bound endpoint (issue #149).
///
/// # What this fixes
///
/// `presets::N0` gives the endpoint exactly one way to learn where a peer is:
/// the n0 relay/DNS-pkarr service, which is an **internet** service. So two
/// devices on the same WiFi have no local path to each other. When a peer takes
/// a new DHCP lease, the stored direct addr in `peers.json` goes dead, every
/// dial stalls on it, and the relay — the only fallback — is exactly what is
/// slow or unreachable on the networks where this gets reported. The devices
/// are two metres apart and cannot sync.
///
/// mDNS resolves a peer's **current** address off the LAN itself, so neither
/// the stale stored addr nor relay reachability is on the path any more.
///
/// `advertise` decides whether this endpoint also publishes itself; see
/// [`Advertise`] for why that is not unconditional.
///
/// # Best-effort on purpose
///
/// A failure here is logged and swallowed. Multicast is the kind of thing a
/// network administrator, a container runtime or a mobile OS sandbox switches
/// off, and none of those are reasons to refuse to sync — the relay path is
/// untouched and still carries everything.
///
/// # Not this implementation on iOS
///
/// iOS cannot join a multicast group without an Apple-granted entitlement, so
/// it discovers through the system's own mDNS daemon instead — same DNS-SD
/// records, different code. See [`crate::lan`], which also owns the two wire
/// constants both paths have to agree on.
#[cfg(target_os = "ios")]
fn attach_mdns(endpoint: &Endpoint, advertise: Advertise) {
    if advertise == Advertise::Off {
        return;
    }
    // iOS cannot join a multicast group without an Apple-granted entitlement,
    // so discovery is driven by the platform's own mDNS daemon through the
    // bridge in `outl-mobile`. Same DNS-SD records on the wire, same
    // best-effort contract: an unregistered bridge, or a user who declined the
    // local-network prompt, leaves the relay path untouched.
    let lookup = std::sync::Arc::new(crate::lan::PlatformAddressLookup::new(
        endpoint.id(),
        advertise == Advertise::Yes,
    ));
    match endpoint.address_lookup() {
        Ok(services) => {
            services.add(lookup);
            debug!("platform LAN peer discovery attached");
        }
        Err(e) => warn!("platform LAN peer discovery not attached, relay only: {e}"),
    }
}

#[cfg(not(target_os = "ios"))]
fn attach_mdns(endpoint: &Endpoint, advertise: Advertise) {
    if advertise == Advertise::Off {
        return;
    }
    // `ip_only` keeps the relay out of the mDNS record (a peer that reached us
    // on the LAN does not need it), and the endpoint is IPv4-only bound, so
    // what is left to publish is exactly the LAN addresses that work.
    let builder = MdnsAddressLookup::builder()
        .advertise(advertise == Advertise::Yes)
        .addr_filter(iroh::address_lookup::AddrFilter::ip_only());
    let mdns = match builder.build(endpoint.id()) {
        Ok(mdns) => mdns,
        Err(e) => {
            warn!("mDNS local peer discovery unavailable, relay only: {e}");
            return;
        }
    };
    match endpoint.address_lookup() {
        Ok(services) => {
            services.add(mdns);
            debug!("mDNS local peer discovery enabled");
        }
        // Only reachable if the endpoint closed between `bind()` and here.
        Err(e) => warn!("mDNS local peer discovery not attached, relay only: {e}"),
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
    /// Issue #149: every endpoint in this crate must get LAN discovery.
    ///
    /// The mDNS attach lives in [`bind_ipv4_only`], not in the builder, for the
    /// reason that function documents — which means a new endpoint that reaches
    /// for [`n0_builder_ipv4_only`] and calls `.bind()` itself compiles fine and
    /// silently has no LAN discovery. That is invisible: sync still works over
    /// the relay, so nothing fails, it just goes back to being unreachable on
    /// the exact networks issue #149 is about.
    ///
    /// So the guard is structural. `n0_builder_ipv4_only` is private to this
    /// module's own use; anything else that wants an endpoint goes through
    /// `bind_ipv4_only`.
    #[test]
    fn every_endpoint_in_this_crate_binds_through_the_one_owner() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut scanned = 0usize;
        for entry in std::fs::read_dir(&src).expect("read src/") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            // This file is the owner; it is the one place allowed to call it.
            if path.file_name().is_some_and(|n| n == "bind.rs") {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&path).expect("read source file");
            if text.contains("n0_builder_ipv4_only") {
                offenders.push(path.file_name().expect("file name").to_owned());
            }
        }
        assert!(
            scanned > 1,
            "the scan found no source files to check — it would pass vacuously",
        );
        assert!(
            offenders.is_empty(),
            "these files bind an endpoint without LAN discovery (issue #149); \
             call `bind::bind_ipv4_only` instead: {offenders:?}",
        );
    }

    /// Only the long-lived sync endpoint may advertise.
    ///
    /// Every endpoint in this crate binds under the **device's own node id**,
    /// so a short-lived one that advertises leaves an address for this device
    /// that is dead as soon as it closes, and iroh 1.0.0 multipath stalls on a
    /// dead candidate instead of skipping it. The pairing endpoint is worse
    /// than the probe: it accepts only `PAIRING_ALPN`, so a peer that resolved
    /// it and dials `SYNC_ALPN` gets CONNECTION_REFUSED.
    ///
    /// Asserted on the call sites because `Advertise` is consumed inside a
    /// builder that exposes no way to read the flag back. Coarse, and it fails
    /// for the right reason: flipping any of these to `Yes` is exactly the
    /// change this exists to stop.
    #[test]
    fn only_the_sync_endpoint_advertises() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut advertising = Vec::new();
        for entry in std::fs::read_dir(&src).expect("read src/") {
            let path = entry.expect("dir entry").path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            // `bind.rs` declares the enum; `lan.rs` names it in doc comments.
            let name = path.file_name().expect("file name").to_owned();
            if name == "bind.rs" || name == "lan.rs" {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source file");
            if text.contains("Advertise::Yes") {
                advertising.push(name);
            }
        }
        assert_eq!(
            advertising,
            vec![std::ffi::OsString::from("engine.rs")],
            "only the long-lived sync endpoint (engine.rs) may advertise on the LAN; \
             a transient endpoint publishes an address that dies with it",
        );
    }

    /// The mDNS service is actually attached, and the relay stack survives it.
    ///
    /// Pinned by count rather than by type because `AddressLookupServices`
    /// exposes no way to name what it holds. The delta is what matters: the
    /// `presets::N0` services must all still be there (mDNS is additive, not a
    /// replacement — it is useless across networks) and mDNS must be on top.
    ///
    /// Skipped where the host allows no multicast at all, which is the same
    /// condition [`attach_mdns`] degrades on. Asserting a hard `+1` there would
    /// fail the suite on locked-down CI for a case the production path is
    /// explicitly designed to survive.
    #[tokio::test]
    async fn a_bound_endpoint_carries_mdns_on_top_of_the_relay_stack() {
        let key = iroh::SecretKey::generate();

        let baseline = n0_builder_ipv4_only(None)
            .secret_key(key.clone())
            .bind()
            .await
            .expect("bind the baseline endpoint")
            .address_lookup()
            .expect("a freshly bound endpoint is not closed")
            .len();
        assert!(
            baseline > 0,
            "the relay/DNS stack must survive the IPv4-only STOPGAP",
        );

        let endpoint = bind_ipv4_only(None, &key, vec![b"outl/test/1".to_vec()], Advertise::Yes)
            .await
            .expect("mDNS must never fail the bind");
        let attached = endpoint
            .address_lookup()
            .expect("a freshly bound endpoint is not closed")
            .len();

        let host_has_multicast = MdnsAddressLookup::builder().build(endpoint.id()).is_ok();
        if host_has_multicast {
            assert_eq!(
                attached,
                baseline + 1,
                "mDNS must be attached on top of the relay stack, not instead of it",
            );
        } else {
            assert_eq!(
                attached, baseline,
                "with no multicast the endpoint must degrade to the relay stack, not lose it",
            );
        }
    }

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
