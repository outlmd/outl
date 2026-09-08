//! LAN peer discovery driven by the host platform, for platforms that cannot
//! do socket-level mDNS themselves (issue #149).
//!
//! # Why this seam exists
//!
//! Every platform but one discovers same-LAN peers with
//! `iroh-mdns-address-lookup`, which speaks mDNS over a plain BSD socket:
//! `bind` on `0.0.0.0:5353`, then `IP_ADD_MEMBERSHIP` on `224.0.0.251`. Since
//! iOS 14 that group join needs `com.apple.developer.networking.multicast`, a
//! restricted entitlement Apple grants by request — and commonly declines,
//! suggesting Bonjour instead. So iOS has to go through `mDNSResponder`, the
//! system's own mDNS daemon, which is reachable only from Objective-C / Swift.
//!
//! That means **FFI**, and this crate is `#![forbid(unsafe_code)]` — not an
//! obstacle to route around but the correct answer: a sync transport should not
//! hold a platform bridge. So the bridge lives in the client that already has
//! one (`outl-mobile`'s `ios_bonjour.rs`, next to the background-sync exports),
//! and what crosses into this crate is plain Rust: strings in, strings out, no
//! pointers.
//!
//! # The two directions
//!
//! - **Out** — [`register_advertiser`] takes something that knows how to
//!   advertise. The lookup's `publish` hands it the endpoint id,
//!   port and relay whenever iroh's address set changes.
//! - **In** — [`peer_discovered`] is called by the platform for each peer it
//!   resolves, and wakes whichever `resolve` calls
//!   were waiting for that endpoint.
//!
//! Registration order does not matter. The lookup is attached when the endpoint
//! binds; if no advertiser has registered yet, `publish` is a no-op and the
//! next one lands. Nothing here fails a bind.
//!
//! # This is not a second protocol
//!
//! `swarm-discovery` publishes plain DNS-SD (RFC 6763), so the platform side
//! reimplements no wire format — it advertises and browses the same records:
//!
//! - service type `_irohv1._udp.local.` ([`SERVICE_TYPE`])
//! - instance name = the endpoint id in lowercase base32 ([`instance_label`],
//!   **not** how iroh's `Display` prints it)
//! - TXT key `relay` ([`RELAY_TXT_KEY`]) holding the home relay URL
//! - SRV + A/AAAA for the direct addresses
//!
//! A laptop advertising through `swarm-discovery` and an iPhone browsing
//! through `mDNSResponder` therefore resolve each other unchanged. Drift in
//! those two constants breaks discovery **silently and in both directions** —
//! nothing errors, the peers simply never meet, which is indistinguishable from
//! an empty LAN. `the_lan_wire_constants_match_the_platform_bridge` pins them.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use iroh::address_lookup::{
    AddressLookup, EndpointData, EndpointInfo, Error as LookupError, Item as LookupItem,
};
use iroh::{EndpointId, TransportAddr};
use n0_future::boxed::BoxStream;
use tokio::sync::mpsc;
use tracing::debug;

/// DNS-SD service type the platform bridge must advertise and browse.
///
/// **Must equal `N0_SERVICE_NAME` (`"irohv1"`) in `iroh-mdns-address-lookup`**,
/// or a phone and a laptop publish into different namespaces and never meet.
pub const SERVICE_TYPE: &str = "_irohv1._udp.";

/// TXT key carrying the home relay URL, matching
/// `iroh-mdns-address-lookup`'s `RELAY_URL_ATTRIBUTE`.
pub const RELAY_TXT_KEY: &str = "relay";

/// Render an [`EndpointId`] the way the DNS-SD instance label must spell it.
///
/// **Not `EndpointId::to_string()`.** That is `Display`, which `iroh-base`
/// implements as `HEXLOWER` — 64 characters. RFC 6763 §4.1.1 caps one DNS label
/// at 63 bytes, so a hex label is one byte too long and the publish fails
/// outright. `iroh-mdns-address-lookup` sidesteps that by encoding the id as
/// lowercase `BASE32_NOPAD` (52 characters) before handing it to
/// `swarm-discovery`, and this has to match it byte for byte.
///
/// The failure this prevents is one-directional, which is what makes it worth a
/// named function: `PublicKey::from_str` accepts **both** encodings (hex when
/// the input is 64 chars, base32 otherwise), so a phone publishing hex still
/// *reads* a laptop's base32 label fine. The phone finds the laptop, the laptop
/// never finds the phone, and neither side logs anything.
pub fn instance_label(endpoint_id: &EndpointId) -> String {
    data_encoding::BASE32_NOPAD
        .encode(endpoint_id.as_bytes())
        .to_ascii_lowercase()
}

/// Identifies this service in an [`LookupItem`]'s provenance, the way
/// `iroh-mdns-address-lookup` uses `"mdns"`.
const PROVENANCE: &str = "platform-lan";

/// How many resolved peers may queue for one waiting `resolve` before the
/// channel pushes back. A resolve is answered by the first usable item, so
/// depth beyond a couple only delays noticing that the stream was dropped.
const RESOLVE_CHANNEL_DEPTH: usize = 8;

/// How long one `resolve` waits before its sender is dropped, ending the
/// stream. Matches `iroh-mdns-address-lookup`'s `LOOKUP_DURATION`.
const LOOKUP_DURATION: std::time::Duration = std::time::Duration::from_secs(10);

/// Something that can advertise this endpoint on the local network.
///
/// Implemented by the client that owns the platform bridge. Calls are
/// fire-and-forget: iroh cannot wait on publishing, so an implementation that
/// needs to hop threads should do that itself and return.
pub trait LanAdvertiser: Send + Sync + 'static {
    /// Advertise `endpoint_id` at `port`, carrying `relay` in the TXT record.
    ///
    /// Called again whenever iroh's address set changes, so an implementation
    /// must treat this as "replace what you were advertising", not "add".
    fn publish(&self, endpoint_id: &str, port: u16, relay: Option<&str>);
}

static ADVERTISER: OnceLock<RwLock<Option<Arc<dyn LanAdvertiser>>>> = OnceLock::new();

fn advertiser() -> &'static RwLock<Option<Arc<dyn LanAdvertiser>>> {
    ADVERTISER.get_or_init(|| RwLock::new(None))
}

/// Install the platform's advertiser. Replaces any previous one.
pub fn register_advertiser(adv: Arc<dyn LanAdvertiser>) {
    *advertiser().write().unwrap_or_else(|p| p.into_inner()) = Some(adv);
    debug!("lan: platform advertiser registered");
}

/// Everyone currently waiting on a `resolve`, keyed by the endpoint they want.
///
/// Process-wide because the platform bridge is a singleton (one mDNS
/// registration per app), so a per-instance map would leave every lookup but
/// one deaf.
type Waiters = Mutex<HashMap<EndpointId, Vec<mpsc::Sender<Result<LookupItem, LookupError>>>>>;

static WAITERS: OnceLock<Waiters> = OnceLock::new();

fn waiters() -> &'static Waiters {
    WAITERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Peers the platform has already resolved, replayed to a later `resolve`.
///
/// **Without this the iOS path discards almost every answer it gets.** The
/// browser starts at process boot (`ios_bonjour::install`), but iroh's first
/// `resolve` for a peer only happens on the first dial, which is after the
/// workspace opens and the endpoint binds. `NetServiceBrowser` does not re-emit
/// `didFind` for a service it has already reported, so the LAN answers arrive
/// while nothing is listening, get dropped, and never come again until the peer
/// re-announces. `iroh-mdns-address-lookup` keeps the same cache for the same
/// reason.
type Discovered = Mutex<HashMap<EndpointId, LookupItem>>;

/// Upper bound on [`DISCOVERED`]. Every parseable instance name on the LAN
/// lands in the cache, and the LAN is untrusted input, so without a cap a noisy
/// or hostile network (or a long-lived process moving across many of them) is
/// an unbounded memory sink. A workspace has a handful of peers; this is
/// generous for every real LAN and still small enough that the whole cache
/// costs nothing to hold.
///
/// Coarse on purpose: when it fills, the map is cleared rather than evicted by
/// age. A peer this drops re-announces on its own, and the replay this cache
/// exists for (an answer that arrived before the first `resolve`) is lost only
/// for peers seen before the flood.
const DISCOVERED_CAP: usize = 256;

static DISCOVERED: OnceLock<Discovered> = OnceLock::new();

fn discovered() -> &'static Discovered {
    DISCOVERED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Report a peer the platform resolved on the LAN.
///
/// `endpoint_id` is the service instance name; `addrs` is a comma-separated
/// list of `ip:port` (a v6 literal bracketed, as `SocketAddr`'s own parser
/// wants); `relay` is the TXT `relay` value, or empty.
///
/// Text rather than parsed types on purpose: `SocketAddr::from_str` is then the
/// single owner of address parsing on both sides of the bridge, instead of a
/// hand-rolled parser per language. Anything unparseable is dropped with a
/// `debug!` — a malformed record from some other iroh app on the LAN is not
/// this device's problem, and must not take the batch down with it.
pub fn peer_discovered(endpoint_id: &str, addrs: &str, relay: &str) {
    let Ok(endpoint_id) = endpoint_id.parse::<EndpointId>() else {
        debug!("lan: ignoring a service whose name is not an endpoint id");
        return;
    };

    let socket_addrs: Vec<SocketAddr> = addrs
        .split(',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<SocketAddr>().ok())
        // IPv4 only, for the same reason `bind` binds IPv4-only and
        // `PeerEntry::iroh_endpoint_addr` strips v6 from the stored addrs:
        // iroh 1.0.0 multipath stalls on an unreachable candidate instead of
        // skipping it. Discovery must not re-introduce the exact class of
        // address the rest of the crate goes out of its way to drop.
        .filter(SocketAddr::is_ipv4)
        .collect();
    if socket_addrs.is_empty() {
        return;
    }

    let mut data = EndpointData::new(socket_addrs.into_iter().map(TransportAddr::Ip).collect());
    if !relay.is_empty() {
        match relay.parse() {
            Ok(url) => data.add_relay_url(url),
            Err(e) => debug!("lan: ignoring an unparseable relay url: {e}"),
        }
    }

    let item = LookupItem::new(
        EndpointInfo::from_parts(endpoint_id, data),
        PROVENANCE,
        None,
    );

    // Record before waking anyone: a `resolve` that arrives after this point
    // still gets the answer, which is the common ordering on iOS.
    {
        let mut cache = discovered().lock().unwrap_or_else(|p| p.into_inner());
        if cache.len() >= DISCOVERED_CAP && !cache.contains_key(&endpoint_id) {
            debug!("lan: discovered cache full, clearing");
            cache.clear();
        }
        cache.insert(endpoint_id, item.clone());
    }

    let mut map = waiters().lock().unwrap_or_else(|p| p.into_inner());
    let Some(senders) = map.get_mut(&endpoint_id) else {
        return;
    };
    // Drop senders whose receiver is gone: iroh drops the stream once it has
    // what it needs, and nothing else prunes this map.
    senders.retain(|tx| match tx.try_send(Ok(item.clone())) {
        Ok(()) => true,
        Err(mpsc::error::TrySendError::Full(_)) => true,
        Err(mpsc::error::TrySendError::Closed(_)) => false,
    });
    if senders.is_empty() {
        map.remove(&endpoint_id);
    }
}

/// `AddressLookup` that publishes through the registered [`LanAdvertiser`] and
/// resolves from [`peer_discovered`].
#[derive(Debug)]
pub(crate) struct PlatformAddressLookup {
    endpoint_id: EndpointId,
    advertise: bool,
}

impl PlatformAddressLookup {
    /// `advertise` false still resolves: a transient endpoint needs to find
    /// peers, it just must not publish an address that dies with it (see
    /// `bind::Advertise`).
    pub(crate) fn new(endpoint_id: EndpointId, advertise: bool) -> Self {
        Self {
            endpoint_id,
            advertise,
        }
    }
}

impl AddressLookup for PlatformAddressLookup {
    fn publish(&self, data: &EndpointData) {
        if !self.advertise {
            return;
        }
        // One SRV record carries one port. iroh binds a single UDP socket here
        // (the IPv4-only STOPGAP in `bind`), so every direct address shares it
        // and the first is representative; a peer learns our other interfaces
        // from the A records the platform publishes for this host.
        let Some(port) = data.ip_addrs().next().map(SocketAddr::port) else {
            return;
        };
        let guard = advertiser().read().unwrap_or_else(|p| p.into_inner());
        let Some(adv) = guard.as_ref() else {
            // The bridge has not registered yet. Not an error: iroh republishes
            // on every address change, so the next one lands.
            return;
        };
        let relay = data.relay_urls().next().map(ToString::to_string);
        adv.publish(&instance_label(&self.endpoint_id), port, relay.as_deref());
    }

    fn resolve(
        &self,
        endpoint_id: EndpointId,
    ) -> Option<BoxStream<Result<LookupItem, LookupError>>> {
        let (tx, rx) = mpsc::channel(RESOLVE_CHANNEL_DEPTH);

        // Answer from the cache first, so a peer already seen on the LAN
        // resolves without waiting for it to announce itself again.
        if let Some(item) = discovered()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&endpoint_id)
            .cloned()
        {
            let _ = tx.try_send(Ok(item));
        }

        {
            let mut map = waiters().lock().unwrap_or_else(|p| p.into_inner());
            let senders = map.entry(endpoint_id).or_default();
            // Prune here as well as on discovery. `peer_discovered` only ever
            // touches the id being discovered, so a peer that is NOT on this
            // LAN would otherwise accumulate a dead sender per attempt and
            // nothing would ever collect them.
            senders.retain(|tx| !tx.is_closed());
            senders.push(tx);
        }

        // Bound the wait. Without this the stream never ends, so iroh's
        // `MergeBounded` over every lookup service never completes and it can
        // never conclude `NoResults` for an absent peer. Mirrors
        // `iroh-mdns-address-lookup`'s own `LOOKUP_DURATION`.
        tokio::spawn(async move {
            tokio::time::sleep(LOOKUP_DURATION).await;
            let mut map = waiters().lock().unwrap_or_else(|p| p.into_inner());
            if let Some(senders) = map.get_mut(&endpoint_id) {
                senders.retain(|tx| !tx.is_closed());
                if senders.is_empty() {
                    map.remove(&endpoint_id);
                }
            }
        });

        Some(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The wire constants are the entire agreement with every other client, and
    /// getting one wrong fails in the one way nothing catches: no error, no
    /// log, the peers just never see each other.
    ///
    /// So this reads the other two copies **off disk**. The first version of
    /// this test asserted `SERVICE_TYPE == "_irohv1._udp."` against a literal
    /// in this same file, which is a tautology: it stayed green no matter what
    /// the Swift said, while three docs claimed it pinned the contract.
    #[test]
    fn the_lan_wire_constants_match_the_platform_bridge() {
        let mobile = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/")
            .join("outl-mobile/src-tauri/gen/apple");

        let swift = std::fs::read_to_string(mobile.join("Sources/outl-mobile/OutlBonjour.swift"))
            .expect("read OutlBonjour.swift");
        assert!(
            swift.contains(&format!("\"{SERVICE_TYPE}\"")),
            "the Swift bridge must advertise {SERVICE_TYPE}, or a phone and a laptop \
             publish into different namespaces and silently never meet",
        );
        assert!(
            swift.contains(&format!("\"{RELAY_TXT_KEY}\"")),
            "the Swift bridge must read the {RELAY_TXT_KEY} TXT key",
        );

        // `NSBonjourServices` takes the bare type; `NetService` wants the
        // dotted form. Same fact, two spellings, so compare against the
        // trimmed constant rather than hardcoding a third literal.
        let plist = std::fs::read_to_string(mobile.join("outl-mobile_iOS/Info.plist"))
            .expect("read Info.plist");
        let bare = SERVICE_TYPE.trim_end_matches('.');
        assert!(
            plist.contains(&format!("<string>{bare}</string>")),
            "Info.plist must declare {bare} in NSBonjourServices, or iOS treats it \
             as an arbitrary service type and requires the multicast entitlement",
        );
    }

    /// The DNS-SD instance label, which is where this first shipped broken.
    ///
    /// `EndpointId`'s `Display` is 64 hex chars, one byte over RFC 6763's
    /// 63-byte cap on a label, so publishing it fails outright. It also would
    /// not have matched `iroh-mdns-address-lookup`, which encodes base32.
    #[test]
    fn the_instance_label_fits_a_dns_label_and_round_trips() {
        for _ in 0..16 {
            let id = iroh::SecretKey::generate().public();
            let label = instance_label(&id);
            assert!(
                label.len() <= 63,
                "a DNS-SD instance label is capped at 63 bytes, got {} for {label}",
                label.len(),
            );
            assert_ne!(
                label,
                id.to_string(),
                "the label must NOT be Display (hex, 64 chars) — that is the bug this pins",
            );
            assert_eq!(
                label.parse::<EndpointId>().expect("label parses back"),
                id,
                "every other client parses this label back into an endpoint id",
            );
        }
    }

    /// The label must match what `iroh-mdns-address-lookup` publishes, since
    /// that is what every non-iOS client puts on the wire.
    #[test]
    fn the_instance_label_matches_what_the_other_clients_publish() {
        let id = iroh::SecretKey::generate().public();
        let upstream = data_encoding::BASE32_NOPAD
            .encode(id.as_bytes())
            .to_ascii_lowercase();
        assert_eq!(instance_label(&id), upstream);
    }

    #[derive(Debug)]
    struct Counting(AtomicUsize);

    impl LanAdvertiser for Counting {
        fn publish(&self, _endpoint_id: &str, _port: u16, _relay: Option<&str>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// A non-advertising lookup must stay silent even with a bridge installed.
    ///
    /// This is `bind::Advertise::No` reaching the platform path: the status
    /// probe binds under the device's own node id and exits seconds later, so
    /// publishing would leave a dead address for this device on every LAN peer.
    #[test]
    fn a_non_advertising_lookup_never_publishes() {
        let counter = Arc::new(Counting(AtomicUsize::new(0)));
        register_advertiser(counter.clone());

        let id = iroh::SecretKey::generate().public();
        let lookup = PlatformAddressLookup::new(id, false);
        let data = EndpointData::new(vec![TransportAddr::Ip(
            "192.168.1.9:41234".parse().unwrap(),
        )]);
        lookup.publish(&data);

        assert_eq!(
            counter.0.load(Ordering::SeqCst),
            0,
            "Advertise::No must not reach the platform bridge",
        );
    }

    /// Garbage from the LAN is dropped, not propagated and not fatal.
    ///
    /// These records come off the network from anything else speaking
    /// `_irohv1._udp` — another iroh application, a stale cache entry, a
    /// half-written TXT record. None of it is this device's problem, and none
    /// of it may panic a callback the platform invokes from its main run loop.
    #[test]
    fn malformed_discoveries_are_dropped_without_panicking() {
        peer_discovered("not-an-endpoint-id", "192.168.1.9:41234", "");
        peer_discovered("", "", "");

        let id = iroh::SecretKey::generate().public().to_string();
        // Valid id, nothing dialable.
        peer_discovered(&id, "", "");
        peer_discovered(&id, "not-an-address,also-not-one", "");
        // Valid id and address, unusable relay — the addresses must still count.
        peer_discovered(&id, "192.168.1.9:41234", "not a url");
    }

    /// The discovered cache is fed by the LAN, which is untrusted input, so it
    /// must stay bounded no matter how many distinct ids show up.
    ///
    /// Asserted as `<= DISCOVERED_CAP` rather than an exact count because the
    /// cache is process-wide and other tests in this module feed it
    /// concurrently; the bound is the invariant, the exact size is not.
    #[test]
    fn the_discovered_cache_never_outgrows_its_cap() {
        for _ in 0..(DISCOVERED_CAP + 8) {
            let id = iroh::SecretKey::generate().public();
            peer_discovered(&id.to_string(), "192.168.1.9:41234", "");
        }
        let len = discovered().lock().unwrap_or_else(|p| p.into_inner()).len();
        assert!(
            len <= DISCOVERED_CAP,
            "the discovered cache must be bounded, got {len} entries",
        );
    }

    /// A resolve with nobody listening for it must not leak a waiter, and a
    /// discovery for an endpoint nobody asked about must not create one.
    #[test]
    fn a_discovery_for_an_unwatched_endpoint_creates_no_waiter() {
        let id = iroh::SecretKey::generate().public();
        peer_discovered(&id.to_string(), "192.168.1.9:41234", "");
        let map = waiters().lock().unwrap_or_else(|p| p.into_inner());
        assert!(
            !map.contains_key(&id),
            "an unwatched discovery must not allocate a waiter entry",
        );
    }
}
