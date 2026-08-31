//! Trusted peer store — read/written at `<workspace>/.outl/peers.json`.
//!
//! The paired-peer list belongs to the **graph** (workspace), not the OS:
//! pairing device B into workspace X must not silently expose B to workspace Y.
//! The device *identity* (`identity.key`) stays global — one node id per
//! device — but the trust list is per-workspace.

use anyhow::{Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use crate::peers_lock::PeersWriteLock;

/// One local IPv4 interface (address + netmask), used to decide whether a peer's
/// stored direct addr shares a subnet with this machine — i.e. is reachable over
/// the LAN at all. See [`is_reachable_lan_ipv4`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct LocalV4 {
    ip: Ipv4Addr,
    mask: Ipv4Addr,
}

/// Enumerate this machine's IPv4 interfaces (address + netmask).
///
/// Returns an empty `Vec` on any enumeration error — callers treat "no known
/// interfaces" as **fail-open** (keep every IPv4 direct addr, the pre-filter
/// behaviour) rather than dropping reachability on a transient syscall failure.
///
/// This walks the OS interface list (`getifaddrs`), so a caller resolving a
/// **batch** of peers (the catch-up resolver, `force_sync_all`) should call it
/// **once** and pass the result to each
/// [`PeerEntry::iroh_endpoint_addr_with_ifaces`], instead of letting the
/// per-peer [`PeerEntry::iroh_endpoint_addr`] re-enumerate on every entry.
pub(crate) fn local_v4_ifaces() -> Vec<LocalV4> {
    match if_addrs::get_if_addrs() {
        Ok(ifaces) => ifaces
            .into_iter()
            .filter_map(|iface| match iface.addr {
                if_addrs::IfAddr::V4(v4) => Some(LocalV4 {
                    ip: v4.ip,
                    mask: v4.netmask,
                }),
                if_addrs::IfAddr::V6(_) => None,
            })
            .collect(),
        Err(e) => {
            warn!("could not enumerate local interfaces ({e}); keeping all IPv4 direct addrs");
            Vec::new()
        }
    }
}

/// Does `peer` share a subnet with `local` (i.e. `peer & mask == local & mask`)?
fn same_subnet_v4(peer: Ipv4Addr, local: &LocalV4) -> bool {
    let mask = u32::from(local.mask);
    (u32::from(peer) & mask) == (u32::from(local.ip) & mask)
}

/// Is `peer` on the same LAN subnet as any local interface?
///
/// This is the load-bearing filter for a peer's **stored** direct addrs: a
/// direct addr on a subnet no local interface belongs to can only ever be a
/// stale capture (a VPN/tunnel IP grabbed at pairing time, `100.x` CGNAT, a
/// public WAN addr, …). Dialing it can never establish a direct path — the
/// relay already covers cross-network reachability — but iroh 1.0.0's multipath
/// opens a path to it anyway and stalls the whole connect on the dead path
/// (`MultipathNotNegotiated`, ~30s). Dropping it loses nothing and removes the
/// stall.
///
/// **Loopback (`127.0.0.0/8`) is always kept**, independent of the interface
/// list, so loopback dials (tests, same-host peers) never drop even if the OS
/// enumeration omits `lo0` or reports it with an odd netmask.
/// **`ifaces` empty ⇒ keep everything** (fail-open — see [`local_v4_ifaces`]).
fn is_reachable_lan_ipv4(peer: Ipv4Addr, ifaces: &[LocalV4]) -> bool {
    peer.is_loopback()
        || ifaces.is_empty()
        || ifaces.iter().any(|iface| same_subnet_v4(peer, iface))
}

/// Build the per-workspace peers path: `<workspace_root>/.outl/peers.json`.
///
/// The `.outl/` directory already holds the workspace's `workspace-id` and
/// `config.toml`, so the peer list sits next to the other graph-scoped state.
pub fn workspace_peers_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".outl").join("peers.json")
}

/// One-time migration: when a workspace has no `peers.json` yet but the legacy
/// **global** peers file exists — [`crate::default_device_dir`]`/peers.json`,
/// which is where `$OUTL_DEVICE_DIR` moves it, NOT a hand-rolled `~/.outl` —
/// copy the global list into the workspace so an already-paired user keeps
/// their peers after the move from device-global to per-workspace storage.
///
/// Best-effort: any failure is logged and swallowed (the workspace just starts
/// with an empty peer list, recoverable by re-pairing). The global file is
/// **never** deleted — other not-yet-migrated workspaces may still read it, and
/// a fresh per-workspace copy is the safe outcome on a partial migration.
///
/// Idempotent: once the workspace file exists this is a no-op, so the copy
/// happens exactly once per workspace and later edits stay workspace-local.
pub fn migrate_global_peers_if_absent(workspace_root: &Path) {
    let dest = workspace_peers_path(workspace_root);
    if dest.exists() {
        return;
    }
    let Ok(device_dir) = crate::device::default_device_dir() else {
        return;
    };
    let global = device_dir.join("peers.json");
    if !global.exists() {
        return;
    }
    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!(
                "peers migration: create {} failed, starting empty: {e}",
                parent.display()
            );
            return;
        }
    }
    match std::fs::copy(&global, &dest) {
        Ok(_) => debug!(
            "peers migration: copied {} -> {}",
            global.display(),
            dest.display()
        ),
        Err(e) => warn!(
            "peers migration: copy {} -> {} failed, starting empty: {e}",
            global.display(),
            dest.display()
        ),
    }
}

/// A single trusted peer entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerEntry {
    /// Peer's iroh node id (hex-encoded public key).
    pub node_id: String,
    /// Human-readable label (e.g. "iPhone 15").
    pub alias: Option<String>,
    /// Relay URL for initial contact (may be outdated; iroh uses it as a hint).
    ///
    /// Kept for display + back-compat with peers.json files written before the
    /// full `endpoint_addr` field existed. The relay URL captured here is a
    /// subset of what `endpoint_addr` carries; prefer the latter for dialing.
    pub relay_url: Option<String>,
    /// Peer's **full** [`iroh::EndpointAddr`] (node id + relay URL + direct
    /// socket addrs), base64-encoded JSON.
    ///
    /// This is the field that makes device↔device connect reliable: it carries
    /// the peer's direct addresses (so two devices on the same LAN connect
    /// immediately) and its home relay, captured at pairing time *after* the
    /// endpoint came online. Older peers.json entries (pre-0.6) lack it and
    /// deserialize as `None` (see `#[serde(default)]`); the dial then falls back
    /// to node id + `relay_url`, then to a bare id.
    #[serde(default)]
    pub endpoint_addr: Option<String>,
    /// ISO 8601 timestamp when pairing occurred.
    pub added_at: String,
}

/// Base64-encode an [`iroh::EndpointAddr`] (as JSON) for storage in a
/// [`PeerEntry`] or a pairing ticket.
pub fn encode_endpoint_addr(addr: &iroh::EndpointAddr) -> Result<String> {
    let json = serde_json::to_vec(addr).context("serialize endpoint addr")?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json))
}

/// Decode a base64 JSON [`iroh::EndpointAddr`] produced by
/// [`encode_endpoint_addr`].
pub fn decode_endpoint_addr(encoded: &str) -> Result<iroh::EndpointAddr> {
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .context("base64-decode endpoint addr")?;
    serde_json::from_slice(&json).context("deserialize endpoint addr")
}

/// Self-heal a moved peer's stale direct address from a live inbound
/// connection.
///
/// When a peer changes LAN IP (new DHCP lease on the same subnet), its
/// `peers.json` entry still holds the OLD direct addr. Every outbound dial
/// then stalls ~30s on that dead address (iroh 1.0.0 multipath) before the
/// relay can carry the connection, and nothing ever prunes it — pairing is
/// the only writer. So a moved peer can go unreachable on a direct dial
/// indefinitely.
///
/// This rewrites the peer's entry with the address it actually connected
/// **from** (dropping the stale direct addr, keeping the relay as the
/// cross-network fallback), so the next outbound dial goes straight to the
/// working address. Called from the sync responder on every inbound direct
/// connection — the NAT-friendly mobile→desktop direction that reliably
/// lands even when the reverse dial can't.
///
/// Best-effort and idempotent: an unknown peer (not paired) is ignored; an
/// entry already carrying exactly this address is left untouched (no write).
/// Returns `Ok(true)` when the entry was refreshed.
///
/// **Only an on-LAN IPv4 inbound is adopted.** A peer's iroh multipath opens
/// paths on *every* interface it has, so inbound connections arrive from a
/// rotating set of source addrs — the same-subnet LAN addr, a VPN / Tailscale
/// CGNAT addr (`100.x`), a carrier WAN addr. Storing an off-LAN one is exactly
/// what stalls multipath (`LastOpenPath`) and would thrash the stored addr on
/// every reconnect (the "mobile keeps flickering" bug). The relay already
/// covers cross-network reachability, so this adopts only a same-subnet IPv4 —
/// matching [`PeerEntry::iroh_endpoint_addr`]'s resolution filter. An off-LAN
/// or IPv6 inbound is a no-op that leaves the good stored addr intact.
pub fn refresh_peer_direct_addr(
    workspace_root: &Path,
    node_id: iroh::EndpointId,
    sock: SocketAddr,
) -> Result<bool> {
    refresh_peer_direct_addr_with_ifaces(workspace_root, node_id, sock, &local_v4_ifaces())
}

/// [`refresh_peer_direct_addr`] with an injectable interface list, so tests can
/// assert the on-LAN filter deterministically regardless of the host's real NICs.
pub(crate) fn refresh_peer_direct_addr_with_ifaces(
    workspace_root: &Path,
    node_id: iroh::EndpointId,
    sock: SocketAddr,
    ifaces: &[LocalV4],
) -> Result<bool> {
    // Reject off-LAN / IPv6 inbounds before touching disk — see the doc above.
    let std::net::IpAddr::V4(v4) = sock.ip() else {
        return Ok(false);
    };
    if !is_reachable_lan_ipv4(v4, ifaces) {
        return Ok(false);
    }

    let path = workspace_peers_path(workspace_root);
    let mut store = PeersStore::load_or_default(&path)?;
    let nid = node_id.to_string();

    let Some(existing) = store.list().iter().find(|p| p.node_id == nid).cloned() else {
        return Ok(false); // not a paired peer — never add one this way
    };

    let decoded = existing
        .endpoint_addr
        .as_deref()
        .and_then(|e| decode_endpoint_addr(e).ok());

    // Already current? (exactly this direct addr) → no write.
    if decoded
        .as_ref()
        .map(|a| {
            let ips: Vec<_> = a.ip_addrs().copied().collect();
            ips.len() == 1 && ips[0] == sock
        })
        .unwrap_or(false)
    {
        return Ok(false);
    }

    // Fresh addr: node id + the live direct socket + the known relay
    // (from the decoded addr, else the legacy `relay_url`). Dropping the
    // old direct addrs is the whole point — a stale one stalls multipath.
    let mut fresh = iroh::EndpointAddr::new(node_id).with_ip_addr(sock);
    if let Some(url) = decoded
        .as_ref()
        .and_then(|a| a.relay_urls().next().cloned())
        .or_else(|| existing.relay_url.as_deref().and_then(|u| u.parse().ok()))
    {
        fresh = fresh.with_relay_url(url);
    }

    let entry = PeerEntry {
        endpoint_addr: Some(encode_endpoint_addr(&fresh)?),
        ..existing
    };
    // `update_existing`, not `add`: this runs on every inbound connection, and
    // `add` would clear a tombstone written by a concurrent `peer remove`.
    store.update_existing(entry)?;
    Ok(true)
}

impl PeerEntry {
    /// Build a [`PeerEntry`] from a peer's full [`iroh::EndpointAddr`], stamping
    /// `added_at` with the current wall-clock time.
    ///
    /// Captures the full reachability info (relay + direct addrs) so a later
    /// dial does not depend on n0 discovery resolving a bare node id.
    pub fn from_endpoint_addr(addr: &iroh::EndpointAddr, alias: Option<String>) -> Result<Self> {
        Ok(Self {
            node_id: addr.id.to_string(),
            alias,
            relay_url: addr.relay_urls().next().map(|u| u.to_string()),
            endpoint_addr: Some(encode_endpoint_addr(addr)?),
            added_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    /// Parse the node id back to an iroh `EndpointId`.
    pub fn iroh_node_id(&self) -> Result<iroh::EndpointId> {
        self.node_id.parse().context("parse iroh EndpointId")
    }

    /// Build a full [`iroh::EndpointAddr`] for dialing this peer.
    ///
    /// Resolution order, most-reachable first:
    /// 1. The stored full `endpoint_addr` (node id + relay + **direct addrs**) —
    ///    captured at pairing time after the endpoint came online. Two devices
    ///    on the same LAN connect via the direct addrs without any relay or n0
    ///    discovery round-trip — this is what fixes the "offline dot" bug.
    /// 2. Node id + `relay_url` (legacy entries, or if the full addr won't
    ///    decode) — connecting via the relay still beats a bare id.
    /// 3. Bare node id — last resort, relies on n0 discovery resolving a route.
    ///
    /// Stored direct addrs are additionally filtered to the local machine's
    /// current LAN: an IPv4 addr on a subnet no local interface belongs to (a
    /// stale VPN/tunnel IP captured at pairing time) is dropped, because dialing
    /// it can never form a direct path yet stalls iroh's multipath. See
    /// `iroh_endpoint_addr_with_ifaces` for the injectable, unit-tested filter.
    pub fn iroh_endpoint_addr(&self) -> Result<iroh::EndpointAddr> {
        self.iroh_endpoint_addr_with_ifaces(&local_v4_ifaces())
    }

    /// [`iroh_endpoint_addr`](Self::iroh_endpoint_addr) with the local IPv4
    /// interface list injected, so the LAN-reachability filter is unit-testable
    /// without depending on the host's real network config.
    ///
    /// Batch callers (the catch-up resolver, `force_sync_all`) call
    /// [`local_v4_ifaces`] once and pass the result here for every peer, so the
    /// interface list is enumerated once per pass instead of once per peer.
    pub(crate) fn iroh_endpoint_addr_with_ifaces(
        &self,
        ifaces: &[LocalV4],
    ) -> Result<iroh::EndpointAddr> {
        let id = self.iroh_node_id()?;
        // Dial the relay AND the IPv4 direct addrs. On the same LAN the direct
        // IPv4 path connects without touching the relay — which is what saves
        // the sync when the public relay is flaky (its ping times out). The
        // relay stays as the cross-network fallback.
        //
        // We DROP IPv6 direct addrs: a dead global-IPv6 path stalls iroh's
        // multipath for minutes (`MultipathNotNegotiated` + timeout), while a
        // stale IPv4 fails fast (No route to host) and yields to the relay. The
        // IPv4-only endpoint bind means we rarely even store an IPv6 path; this
        // filter is belt-and-suspenders for older `peers.json` entries captured
        // before that bind. (The old code dialed relay-only to dodge the IPv6
        // stall, but that also threw away the LAN-direct path and made every
        // connect hostage to the relay.)
        //
        // We ALSO drop IPv4 direct addrs that are NOT on any local subnet: a
        // peer paired while on a VPN captures its tunnel IPs (`10.x`, `100.x`
        // CGNAT, a public WAN addr) into `endpoint_addr` alongside the real LAN
        // address. Those are unreachable from this machine's LAN, but iroh 1.0.0
        // opens a multipath path to each anyway and stalls the whole connect on
        // the dead paths — even when the real `192.168.x` addr is right there.
        // `is_reachable_lan_ipv4` keeps only addrs sharing a subnet with a local
        // interface; the relay still covers genuine cross-network peers.
        let relay = self
            .relay_url
            .as_ref()
            .and_then(|r| r.parse::<iroh::RelayUrl>().ok());

        if let Some(encoded) = &self.endpoint_addr {
            match decode_endpoint_addr(encoded) {
                Ok(stored) => {
                    let mut addr = iroh::EndpointAddr::new(id);
                    if let Some(url) = relay.clone() {
                        addr = addr.with_relay_url(url);
                    }
                    for sock in stored.ip_addrs() {
                        if matches!(sock, SocketAddr::V4(v4) if is_reachable_lan_ipv4(*v4.ip(), ifaces))
                        {
                            addr = addr.with_ip_addr(*sock);
                        }
                    }
                    return Ok(addr);
                }
                Err(e) => tracing::warn!("stored endpoint_addr won't decode, falling back: {e}"),
            }
        }
        // No full addr: dial node_id + relay only (iroh hole-punches current
        // direct addrs once the relay link is up). Bare id when there's no relay
        // either (loopback tests dial 127.0.0.1 via the stored addr above).
        match relay {
            Some(url) => Ok(iroh::EndpointAddr::new(id).with_relay_url(url)),
            None => Ok(iroh::EndpointAddr::new(id)),
        }
    }
}

/// A device this machine has explicitly removed.
///
/// Kept **after** removal because deleting the entry was not enough: the 5s
/// membership gossip re-added it from any peer that still knew it, so a
/// `peer remove` undid itself within a tick (issue #158). The entry the user
/// deleted is gone; this is only the memory that they deleted it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevokedPeer {
    /// The removed device's iroh node id.
    pub node_id: String,
    /// When this machine revoked it (RFC3339, wall clock).
    ///
    /// Informational — nothing compares it. A tombstone that expired would be
    /// a revocation that silently undoes itself, which is the bug.
    pub revoked_at: String,
}

/// Persisted list of trusted peers, plus what this device has revoked.
///
/// `revoked` is **device-local and never gossiped**: `build_membership_payload`
/// broadcasts [`PeersStore::list`], which returns only `peers`. That is a
/// deliberate limit, not an oversight — see [`PeersStore::revoked`].
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PeersFile {
    peers: Vec<PeerEntry>,
    /// `#[serde(default)]` so a `peers.json` written before this field existed
    /// loads unchanged.
    #[serde(default)]
    revoked: Vec<RevokedPeer>,
}

/// In-memory peer store backed by `<workspace>/.outl/peers.json`.
pub struct PeersStore {
    path: PathBuf,
    inner: PeersFile,
}

impl PeersStore {
    /// Load the peer store from `path`, or start with an empty list.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        let inner = if path.exists() {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("read peers.json from {}", path.display()))?;
            serde_json::from_str(&text).context("parse peers.json")?
        } else {
            PeersFile::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            inner,
        })
    }

    /// List all trusted peers.
    pub fn list(&self) -> &[PeerEntry] {
        &self.inner.peers
    }

    /// Path this store reads from / writes to
    /// (`<workspace>/.outl/peers.json` in prod).
    ///
    /// The transport keeps the path so its catch-up loop can reload peers added
    /// by pairing *after* the transport booted (pairing writes this same file).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Add a peer and persist to disk (dedup-replace by node_id).
    ///
    /// **Clears any tombstone for that node id.** Adding is the deliberate act
    /// of a user pairing the device again, and it has to outrank the earlier
    /// deliberate act of removing it — otherwise re-pairing a device you once
    /// removed would appear to succeed and then never sync, with nothing on
    /// screen explaining why.
    pub fn add(&mut self, entry: PeerEntry) -> Result<()> {
        self.mutate_full(|file| {
            file.peers.retain(|p| p.node_id != entry.node_id);
            file.revoked.retain(|r| r.node_id != entry.node_id);
            file.peers.push(entry);
            true
        })
    }

    /// Refresh an **already-known** peer's entry in place. Returns `false`
    /// when the peer is not in the list, and never adds one.
    ///
    /// Distinct from [`add`](Self::add), and the distinction is the point:
    /// `add` clears the tombstone, because it models a user deliberately
    /// pairing a device again. A background refresh is not that. Routing
    /// `refresh_peer_direct_addr` through `add` meant an inbound connection
    /// arriving just after `peer remove` would re-insert the peer **and erase
    /// the revocation** — permanently, since nothing re-creates a tombstone.
    ///
    /// The existence check runs inside the lock. Checking a snapshot taken
    /// before it is what left the window in the first place.
    pub fn update_existing(&mut self, entry: PeerEntry) -> Result<bool> {
        let mut updated = false;
        self.mutate_full(|file| {
            let Some(slot) = file.peers.iter_mut().find(|p| p.node_id == entry.node_id) else {
                return false;
            };
            *slot = entry;
            updated = true;
            true
        })?;
        Ok(updated)
    }

    /// Whether `node_id` has been revoked on this device.
    ///
    /// **Scope, stated plainly: `remove` revokes the peer *here*, not across
    /// the mesh.** Every other paired device keeps its own list, so a removed
    /// laptop still syncs with any device that has not also removed it. For a
    /// device the user no longer holds, the answer is
    /// [`crate::rotate_workspace_identity`], not a propagated tombstone — see
    /// [RFC 0155](../../../docs/rfcs/0155-peer-trust.md).
    pub fn is_revoked(&self, node_id: &str) -> bool {
        self.inner.revoked.iter().any(|r| r.node_id == node_id)
    }

    /// Merge a batch of peers, adding **only** node_ids not already present, and
    /// persist once if anything changed. Returns the number of peers added.
    /// Unlike [`add`](Self::add), a known node_id keeps its current `PeerEntry`
    /// (its locally-captured `endpoint_addr`, e.g. from direct pairing, may beat a
    /// gossiped one) — the ADD-only primitive membership auto-discovery uses.
    /// Returns `(added, refused)` — `refused` counts incoming peers dropped
    /// because this device revoked them.
    ///
    /// The second number is the one worth logging. Silently dropping a gossiped
    /// peer looks identical to never having been told about it, and "the
    /// removal keeps being undone" is exactly the failure that went unnoticed
    /// long enough to make it into a security issue.
    pub fn merge_unknown(
        &mut self,
        incoming: impl IntoIterator<Item = PeerEntry>,
    ) -> Result<(usize, usize)> {
        let mut incoming: Vec<PeerEntry> = incoming.into_iter().collect();
        let mut added = 0usize;
        let mut refused = 0usize;
        self.mutate_full(|file| {
            for entry in incoming.drain(..) {
                // Read the tombstones from the freshly-loaded file, not from
                // the caller's snapshot: another process may have revoked this
                // peer since we loaded, and re-adding it here would be the
                // same self-undoing removal one lock down.
                if file.revoked.iter().any(|r| r.node_id == entry.node_id) {
                    refused += 1;
                    continue;
                }
                if file.peers.iter().any(|p| p.node_id == entry.node_id) {
                    continue;
                }
                file.peers.push(entry);
                added += 1;
            }
            added > 0
        })?;
        Ok((added, refused))
    }

    /// Remove a peer by node_id prefix, and **remember that we removed it**.
    /// Returns true if a peer matched.
    ///
    /// Dropping the entry alone was not a removal. The membership gossip tick
    /// re-adds any peer it does not already know, and every other device in the
    /// mesh still knows this one — so the entry came back within ~5 seconds and
    /// the sync handler's authorization check (which reads this same file)
    /// waved it straight through. The user saw "removed" and the device kept
    /// syncing (issue #158).
    ///
    /// The tombstone is what makes the removal survive the next gossip tick.
    /// It never expires: an expiring tombstone is a revocation that undoes
    /// itself on a timer, which is the bug wearing a clock.
    pub fn remove(&mut self, node_id_prefix: &str) -> Result<bool> {
        let mut removed = false;
        let now = chrono::Utc::now().to_rfc3339();
        self.mutate_full(|file| {
            let doomed: Vec<String> = file
                .peers
                .iter()
                .filter(|p| p.node_id.starts_with(node_id_prefix))
                .map(|p| p.node_id.clone())
                .collect();
            removed = !doomed.is_empty();
            if !removed {
                return false;
            }
            file.peers
                .retain(|p| !p.node_id.starts_with(node_id_prefix));
            for node_id in doomed {
                if file.revoked.iter().any(|r| r.node_id == node_id) {
                    continue;
                }
                file.revoked.push(RevokedPeer {
                    node_id,
                    revoked_at: now.clone(),
                });
            }
            true
        })?;
        Ok(removed)
    }

    /// Run a read-modify-write against `peers.json` atomically against every
    /// other writer, in-process and cross-process (issue #160). Under the
    /// [`PeersWriteLock`] flock this **re-reads the current on-disk file**, hands
    /// the fresh list to `mutate`, and — if `mutate` reports a change — writes it
    /// back atomically. Re-reading inside the lock is what closes the lost update:
    /// the many writers (pairing, the 5s membership tick,
    /// [`refresh_peer_direct_addr`], cross-process GUI/MCP/`outl sync`) each
    /// `load_or_default` a possibly-stale snapshot, so applying to that snapshot
    /// and truncate-writing clobbered a concurrent add. `self.inner` is refreshed
    /// to the reconciled state either way.
    /// Both halves of a removal have to move under one lock, which is why this
    /// hands out the whole [`PeersFile`] rather than just `peers`: dropping the
    /// entry and recording the tombstone in two separate locked writes leaves a
    /// window in which the peer is absent and unrevoked — precisely when the 5s
    /// membership tick puts it back.
    fn mutate_full(&mut self, mutate: impl FnOnce(&mut PeersFile) -> bool) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _lock = PeersWriteLock::acquire(&self.path)?;

        let mut file = if self.path.exists() {
            let text = std::fs::read_to_string(&self.path)
                .with_context(|| format!("read peers.json from {}", self.path.display()))?;
            serde_json::from_str::<PeersFile>(&text).context("parse peers.json")?
        } else {
            PeersFile::default()
        };

        let changed = mutate(&mut file);
        self.inner = file;
        if !changed {
            return Ok(());
        }
        let text = serde_json::to_string_pretty(&self.inner)?;
        crate::peers_lock::atomic_write_json(&self.path, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a real, encodable `EndpointAddr` carrying a relay + a direct addr,
    /// so the stored `endpoint_addr` blob round-trips and we can prove the
    /// resolution order picks relay-only OVER the stored direct addr.
    fn addr_with_relay_and_direct() -> iroh::EndpointAddr {
        let id = iroh::SecretKey::generate().public();
        let relay: iroh::RelayUrl = "https://relay.example/".parse().expect("relay url");
        iroh::EndpointAddr::new(id)
            .with_relay_url(relay)
            .with_ip_addr("192.168.7.7:4242".parse().expect("direct addr"))
    }

    /// A fake local interface, so the LAN-reachability filter is deterministic
    /// regardless of the host running the test.
    fn iface(ip: &str, mask: &str) -> LocalV4 {
        LocalV4 {
            ip: ip.parse().expect("iface ip"),
            mask: mask.parse().expect("iface mask"),
        }
    }

    /// The `192.168.7.0/24` LAN the `addr_with_relay_and_direct` direct addr
    /// (`192.168.7.7`) lives on, so it survives the reachability filter.
    fn lan_192_168_7() -> Vec<LocalV4> {
        vec![iface("192.168.7.1", "255.255.255.0")]
    }

    /// Issue #3: `same_subnet_v4` / `is_reachable_lan_ipv4` — the pure filter
    /// that drops stale VPN/tunnel IPv4 while keeping same-LAN and loopback.
    #[test]
    fn is_reachable_lan_ipv4_matches_only_local_subnets() {
        let ifaces = vec![
            iface("192.168.1.50", "255.255.255.0"), // home WiFi
            iface("127.0.0.1", "255.0.0.0"),        // loopback
        ];
        // Same LAN + loopback: reachable.
        assert!(is_reachable_lan_ipv4(
            "192.168.1.83".parse().unwrap(),
            &ifaces
        ));
        assert!(is_reachable_lan_ipv4("127.0.0.1".parse().unwrap(), &ifaces));
        // VPN / CGNAT / WAN captured on another network: not reachable.
        assert!(!is_reachable_lan_ipv4(
            "10.71.22.9".parse().unwrap(),
            &ifaces
        ));
        assert!(!is_reachable_lan_ipv4(
            "100.78.230.122".parse().unwrap(),
            &ifaces
        ));
        assert!(!is_reachable_lan_ipv4(
            "188.37.137.132".parse().unwrap(),
            &ifaces
        ));
        // Empty iface list ⇒ fail-open (keep everything).
        assert!(is_reachable_lan_ipv4("10.71.22.9".parse().unwrap(), &[]));
        // Loopback is kept even when the iface list has NO loopback entry —
        // the allow-list is explicit, not dependent on enumeration.
        let wifi_only = vec![iface("192.168.1.50", "255.255.255.0")];
        assert!(is_reachable_lan_ipv4(
            "127.0.0.1".parse().unwrap(),
            &wifi_only
        ));
    }

    /// Bug #6 (reachability resolution, relay branch): with a relay present, the
    /// dial addr keeps the relay AND the IPv4 direct addrs — so a same-LAN peer
    /// connects directly without the (possibly flaky) relay — but DROPS global
    /// IPv6 direct addrs, because a dead one stalls iroh's multipath for minutes.
    /// (Earlier this was relay-only, which threw away the LAN-direct path and
    /// made every connect hostage to the relay.)
    #[test]
    fn iroh_endpoint_addr_keeps_relay_and_ipv4_but_drops_ipv6() {
        let id = iroh::SecretKey::generate().public();
        let relay: iroh::RelayUrl = "https://relay.example/".parse().expect("relay url");
        let full = iroh::EndpointAddr::new(id)
            .with_relay_url(relay)
            .with_ip_addr("192.168.7.7:4242".parse().expect("ipv4")) // kept
            .with_ip_addr("[2001:db8::1]:4242".parse().expect("ipv6")); // dropped
        let entry = PeerEntry::from_endpoint_addr(&full, None).expect("build entry");

        // Inject the peer's LAN as a local interface so the IPv4 direct addr is
        // reachable and the assertion isolates the IPv6-drop behaviour.
        let resolved = entry
            .iroh_endpoint_addr_with_ifaces(&lan_192_168_7())
            .expect("resolve");
        assert_eq!(resolved.id, id, "resolved must target the same node id");
        assert_eq!(
            resolved.relay_urls().count(),
            1,
            "resolved must keep the relay url"
        );
        let ips: Vec<_> = resolved.ip_addrs().copied().collect();
        assert_eq!(ips.len(), 1, "resolved keeps exactly the IPv4 direct addr");
        assert!(
            ips[0].is_ipv4(),
            "the surviving direct addr is the IPv4 one"
        );
    }

    /// Issue #3 (stale VPN/tunnel IPs in `peers.json`): a peer paired while on a
    /// VPN captured its tunnel IPs alongside the real LAN address. On resolution
    /// the dial must keep ONLY the same-LAN direct addr (`192.168.1.83`) and the
    /// relay, dropping every unreachable tunnel/CGNAT/WAN IPv4 — otherwise iroh's
    /// multipath stalls on the dead paths (`MultipathNotNegotiated`).
    #[test]
    fn iroh_endpoint_addr_drops_stale_vpn_ipv4_keeps_lan() {
        let id = iroh::SecretKey::generate().public();
        let relay: iroh::RelayUrl = "https://use1-1.relay.avelino.outl.iroh.link./"
            .parse()
            .expect("relay url");
        // The exact `endpoint_addr` payload from the issue: one real LAN addr
        // buried among five stale tunnel/CGNAT/WAN addrs.
        let full = iroh::EndpointAddr::new(id)
            .with_relay_url(relay)
            .with_ip_addr("10.71.22.9:62858".parse().unwrap())
            .with_ip_addr("100.78.230.122:62858".parse().unwrap())
            .with_ip_addr("100.85.18.51:62858".parse().unwrap())
            .with_ip_addr("188.37.137.132:62858".parse().unwrap())
            .with_ip_addr("192.0.0.6:62858".parse().unwrap())
            .with_ip_addr("192.168.1.83:62858".parse().unwrap());
        let entry = PeerEntry::from_endpoint_addr(&full, None).expect("build entry");

        // This machine is on the same home WiFi as the peer.
        let ifaces = vec![
            iface("192.168.1.50", "255.255.255.0"),
            iface("127.0.0.1", "255.0.0.0"),
        ];
        let resolved = entry
            .iroh_endpoint_addr_with_ifaces(&ifaces)
            .expect("resolve");

        let ips: Vec<_> = resolved.ip_addrs().copied().collect();
        assert_eq!(
            ips.len(),
            1,
            "only the same-LAN direct addr survives, got {ips:?}"
        );
        assert_eq!(
            ips[0],
            "192.168.1.83:62858".parse().unwrap(),
            "the survivor is the reachable LAN addr"
        );
        assert_eq!(
            resolved.relay_urls().count(),
            1,
            "the relay stays as the cross-network fallback"
        );
    }

    /// Bug #6 (reachability resolution, no-relay branch): with no relay url, fall
    /// back to the stored full `endpoint_addr` (the loopback case — direct addrs
    /// are all we have, e.g. the integration tests dialing 127.0.0.1). Those
    /// stored direct addrs MUST survive here, since there's no relay to prefer.
    #[test]
    fn iroh_endpoint_addr_falls_back_to_stored_addrs_when_no_relay() {
        let full = addr_with_relay_and_direct();
        let mut entry = PeerEntry::from_endpoint_addr(&full, None).expect("build entry");
        // No relay → the resolution order must drop through to endpoint_addr.
        entry.relay_url = None;

        let resolved = entry
            .iroh_endpoint_addr_with_ifaces(&lan_192_168_7())
            .expect("resolve");
        assert_eq!(resolved.id, full.id);
        assert!(
            resolved.ip_addrs().next().is_some(),
            "with no relay, the stored direct addrs must be used to dial"
        );
    }

    /// Bug #6 (last resort): no relay AND no stored `endpoint_addr` → dial the
    /// bare node id (relies on n0 discovery). Must still resolve, never error.
    #[test]
    fn iroh_endpoint_addr_falls_back_to_bare_node_id() {
        let id = iroh::SecretKey::generate().public();
        let entry = PeerEntry {
            node_id: id.to_string(),
            alias: None,
            relay_url: None,
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let resolved = entry.iroh_endpoint_addr().expect("resolve bare id");
        assert_eq!(resolved.id, id);
        assert_eq!(resolved.relay_urls().count(), 0);
        assert_eq!(resolved.ip_addrs().count(), 0);
    }

    /// Bug #6 (corrupt blob never fails the dial): a garbage `endpoint_addr` that
    /// won't decode must fall through to the bare node id with a warning, not
    /// propagate an error that would skip the peer entirely.
    #[test]
    fn iroh_endpoint_addr_recovers_from_corrupt_stored_addr() {
        let id = iroh::SecretKey::generate().public();
        let entry = PeerEntry {
            node_id: id.to_string(),
            alias: None,
            relay_url: None,
            endpoint_addr: Some("!!! not base64 json !!!".to_string()),
            added_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let resolved = entry
            .iroh_endpoint_addr()
            .expect("corrupt addr must still resolve to a dialable bare id");
        assert_eq!(resolved.id, id);
    }

    /// Bug #8 (membership merge is ADD-only): `merge_unknown` must NEVER clobber a
    /// locally-captured entry (e.g. a direct-pairing `endpoint_addr`) with a
    /// gossiped one for the same node_id. A known node keeps its existing entry.
    #[test]
    fn merge_unknown_never_clobbers_a_known_entry() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("peers.json");
        let node = iroh::SecretKey::generate().public().to_string();

        // Local entry captured at pairing: full endpoint_addr present.
        let local = addr_with_relay_and_direct();
        let mut local_entry =
            PeerEntry::from_endpoint_addr(&local, Some("local".into())).expect("local entry");
        local_entry.node_id = node.clone();

        let mut store = PeersStore::load_or_default(&path).expect("load");
        store.add(local_entry.clone()).expect("seed local entry");

        // A gossiped entry for the SAME node, but stripped of reachability.
        let gossiped = PeerEntry {
            node_id: node.clone(),
            alias: Some("gossiped".into()),
            relay_url: None,
            endpoint_addr: None,
            added_at: "2030-01-01T00:00:00Z".to_string(),
        };
        let (added, _refused) = store.merge_unknown([gossiped]).expect("merge");

        assert_eq!(added, 0, "a known node_id must not be re-added");
        let kept = store
            .list()
            .iter()
            .find(|p| p.node_id == node)
            .expect("entry still present");
        assert_eq!(
            kept.alias.as_deref(),
            Some("local"),
            "merge_unknown must keep the locally-captured entry, not the gossiped one"
        );
        assert!(
            kept.endpoint_addr.is_some(),
            "merge_unknown must NOT clobber the locally-captured endpoint_addr"
        );
    }

    /// The per-workspace path is `<root>/.outl/peers.json` — next to the
    /// workspace-id + config.toml the `.outl/` dir already holds.
    #[test]
    fn workspace_peers_path_is_under_dot_outl() {
        let root = std::path::Path::new("/tmp/ws");
        assert_eq!(
            workspace_peers_path(root),
            std::path::Path::new("/tmp/ws/.outl/peers.json")
        );
    }

    /// Migration is a no-op (idempotent) once the workspace already has a
    /// `peers.json`: it must NEVER clobber the workspace-local list with the
    /// global one. We seed a workspace file, then prove migrate leaves it byte
    /// for byte intact regardless of whatever the global file holds.
    #[test]
    fn migrate_is_noop_when_workspace_peers_exist() {
        let ws = tempfile::tempdir().expect("tempdir");
        let dest = workspace_peers_path(ws.path());
        std::fs::create_dir_all(dest.parent().unwrap()).expect("mkdir .outl");
        let sentinel = r#"{"peers":[{"node_id":"local-only","alias":null,"relay_url":null,"added_at":"2026-01-01T00:00:00Z"}]}"#;
        std::fs::write(&dest, sentinel).expect("seed ws peers");

        migrate_global_peers_if_absent(ws.path());

        let after = std::fs::read_to_string(&dest).expect("read ws peers");
        assert_eq!(
            after, sentinel,
            "migration must not overwrite an existing workspace peers.json"
        );
    }

    /// Auto-refresh (self-heal stale addrs): a peer stored with an old **on-LAN**
    /// direct addr is rewritten to the live socket observed on an inbound direct
    /// connection, keeping the relay and dropping the stale addr. Re-running with
    /// the same socket is a no-op, and an unknown node id is never added this way.
    #[test]
    fn refresh_peer_direct_addr_replaces_stale_keeps_relay_and_is_idempotent() {
        // This machine is on `192.168.18.0/24`, so the peer's LAN addr is on-LAN.
        let ifaces = vec![iface("192.168.18.50", "255.255.255.0")];
        let ws = tempfile::tempdir().expect("tempdir");
        let root = ws.path();
        let path = workspace_peers_path(root);

        // Seed a paired peer carrying a STALE LAN addr + a relay.
        let id = iroh::SecretKey::generate().public();
        let stale = iroh::EndpointAddr::new(id)
            .with_relay_url(
                "https://use1-1.relay.avelino.outl.iroh.link/"
                    .parse()
                    .expect("relay"),
            )
            .with_ip_addr("192.168.18.72:50009".parse().expect("stale addr"));
        let entry = PeerEntry::from_endpoint_addr(&stale, Some("iPhone".into())).expect("entry");
        let mut store = PeersStore::load_or_default(&path).expect("store");
        store.add(entry).expect("seed peer");

        // The live on-LAN socket seen when the peer dials us directly.
        let fresh: SocketAddr = "192.168.18.99:50009".parse().expect("fresh sock");
        assert!(
            refresh_peer_direct_addr_with_ifaces(root, id, fresh, &ifaces).expect("refresh"),
            "a stale on-LAN addr must be rewritten (returns true)"
        );

        // Exactly the fresh addr survives, the relay is preserved, stale is gone.
        let store = PeersStore::load_or_default(&path).expect("reload");
        let e = store
            .list()
            .iter()
            .find(|p| p.node_id == id.to_string())
            .expect("peer still present");
        let decoded =
            decode_endpoint_addr(e.endpoint_addr.as_deref().expect("addr")).expect("decode");
        let ips: Vec<_> = decoded.ip_addrs().copied().collect();
        assert_eq!(ips, vec![fresh], "only the live socket is kept");
        assert_eq!(decoded.relay_urls().count(), 1, "relay is preserved");

        // Idempotent: the same socket is already current → no write.
        assert!(
            !refresh_peer_direct_addr_with_ifaces(root, id, fresh, &ifaces).expect("second"),
            "an already-current addr must be a no-op (returns false)"
        );

        // An unknown peer is never added through this path.
        let stranger = iroh::SecretKey::generate().public();
        assert!(
            !refresh_peer_direct_addr_with_ifaces(root, stranger, fresh, &ifaces).expect("unknown"),
            "an unpaired peer must not be added (returns false)"
        );
        let store = PeersStore::load_or_default(&path).expect("reload after unknown");
        assert_eq!(store.list().len(), 1, "unknown peer was not persisted");
    }

    /// The flicker bug: a peer's iroh multipath dials in from a rotating set of
    /// source addrs (LAN, Tailscale CGNAT `100.x`, carrier WAN). Adopting an
    /// off-LAN one would thrash the stored addr and stall multipath, so the
    /// refresh must **reject** an off-LAN / IPv6 inbound and keep the good LAN
    /// addr untouched.
    #[test]
    fn refresh_peer_direct_addr_ignores_off_lan_inbound() {
        let ifaces = vec![iface("192.168.18.50", "255.255.255.0")];
        let ws = tempfile::tempdir().expect("tempdir");
        let root = ws.path();
        let path = workspace_peers_path(root);

        let id = iroh::SecretKey::generate().public();
        let good = iroh::EndpointAddr::new(id)
            .with_relay_url("https://relay.example/".parse().expect("relay"))
            .with_ip_addr("192.168.18.99:59313".parse().expect("good lan addr"));
        let entry = PeerEntry::from_endpoint_addr(&good, None).expect("entry");
        let mut store = PeersStore::load_or_default(&path).expect("store");
        store.add(entry).expect("seed peer");
        let before = std::fs::read_to_string(&path).expect("read peers");

        // Each of these off-LAN / IPv6 inbounds must be a no-op.
        for off in [
            "100.113.140.115:59313", // Tailscale CGNAT
            "138.94.127.156:55859",  // carrier WAN
            "[2001:db8::1]:59313",   // IPv6
        ] {
            let sock: SocketAddr = off.parse().expect("off-lan sock");
            assert!(
                !refresh_peer_direct_addr_with_ifaces(root, id, sock, &ifaces)
                    .expect("off-lan refresh"),
                "off-LAN inbound {off} must not be adopted (returns false)"
            );
        }

        // peers.json is byte-for-byte unchanged — the good LAN addr stayed.
        let after = std::fs::read_to_string(&path).expect("reread peers");
        assert_eq!(before, after, "off-LAN inbounds must not rewrite the entry");
    }
}
