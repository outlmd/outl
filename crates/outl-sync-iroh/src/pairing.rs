//! Device pairing handshake over the [`PAIRING_ALPN`] protocol.
//!
//! Pairing is a one-shot, two-sided exchange that teaches each device the
//! other's identity so future op-sync (over [`crate::SYNC_ALPN`]) can find it.
//!
//! ## Ticket
//!
//! The "ticket" the generating side prints is a base64-encoded JSON
//! [`iroh::EndpointAddr`] — the node id plus the relay URL and direct
//! addresses iroh currently knows about. iroh 1.0.0 ships no `NodeTicket`
//! type, so we serialize the `EndpointAddr` ourselves; it is `Serialize`
//! and `connect` takes `impl Into<EndpointAddr>`, so the joining side feeds
//! the decoded value straight back into `endpoint.connect`.
//!
//! ## Handshake
//!
//! Both sides exchange one [`PeerEntry`] payload (length-prefixed JSON) over a
//! single bidirectional stream, then persist the remote entry to `peers.json`:
//!
//! - **Host** (`outl peer pair`, no ticket): binds an endpoint, prints the
//!   ticket + an ASCII QR, accepts exactly one inbound connection, *reads*
//!   the joiner's entry first, then *writes* its own.
//! - **Join** (`outl peer pair --ticket …`): parses the ticket, connects,
//!   *writes* its entry first, then *reads* the host's.
//!
//! The asymmetric read/write order keeps the single stream from deadlocking
//! (the joiner, which opened the stream, speaks first).
//!
//! ## One endpoint per identity, elected not assigned (load-bearing)
//!
//! The pairing endpoint binds the **device identity** (same `SecretKey` as the
//! long-lived sync endpoint). In iroh the relay keeps a single
//! `node_id → endpoint` route, so two endpoints with the same key compete for
//! it; the newest registration wins and the other stops receiving inbound
//! traffic.
//!
//! **A client whose own sync transport is running never calls [`host_pairing`]
//! / [`join_pairing`].** Binding a second endpoint would hijack the relay route
//! and silently kill that transport's sync (the "Another endpoint connected
//! with the same endpoint id" relay error). It pairs through the *live* sync
//! endpoint instead: [`crate::IrohSyncTransport::pair_host`] /
//! [`crate::IrohSyncTransport::pair_join`] reuse the sync endpoint and its
//! [`PAIRING_ALPN`] router handler (see [`accept_host_handshake`] /
//! [`run_join_handshake`], the endpoint-agnostic handshake halves both paths
//! share).
//!
//! The rule is about **holding an endpoint**, not about being a GUI. A client
//! that lost the device endpoint lease ([`crate::EndpointLease`]) has no live
//! endpoint to pair through, so it uses these one-shot helpers exactly like the
//! CLI does — a desktop coexisting with an `outl mcp serve` that got the lease
//! first is the case that matters. That one-shot bind *does* take the relay
//! route from the lease holder for the seconds the handshake runs, and the
//! holder gets it back when the endpoint closes. Accepted deliberately:
//! pairing is rare, explicit and short, and the alternative is a user who
//! cannot add a device at all. It is not licence to bind an endpoint on any
//! other path.
//!
//! These standalone [`host_pairing`] / [`join_pairing`] functions survive only
//! for the **CLI** (`outl peer pair`), which has *no* running transport — so
//! binding a one-shot endpoint here is the only option and there is no route to
//! steal. To keep the overlap with any concurrent endpoint bounded, both still
//! **close their endpoint** (`endpoint.close().await`) before returning.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::{Endpoint, EndpointAddr};
use outl_core::WorkspaceId;
use serde::{Deserialize, Serialize};

use crate::identity::IrohIdentity;
use crate::peers::{decode_endpoint_addr, encode_endpoint_addr, PeerEntry, PeersStore};
use crate::protocol::PAIRING_ALPN;

/// What happened to the joiner's [`WorkspaceId`] during CLI pairing.
///
/// The joiner joins the host's workspace, so it normally [`Adopted`] the host's
/// id. The other two are the "sync won't converge yet" and "nothing to do"
/// cases the CLI reports differently.
///
/// [`Adopted`]: WorkspaceAdoption::Adopted
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceAdoption {
    /// The host's id was written to `<root>/.outl/workspace-id`; sync will now
    /// converge. Carries the adopted id.
    Adopted(WorkspaceId),
    /// This device already had the host's id (a re-pair, or the two were seeded
    /// from the same workspace). Nothing changed.
    AlreadyMatched,
    /// The host advertised no id — it predates workspace-id pairing. We kept our
    /// own id, so sync with this host can't converge until it upgrades.
    HostSentNone,
}

/// How long the host waits for an inbound pairing connection before giving up.
const HOST_ACCEPT_TIMEOUT: Duration = Duration::from_secs(120);

/// How long a single handshake half (read the remote payload, exchange ours) may
/// take once a connection is up, before we abort it (issue #159).
///
/// Without this, a peer that connects on [`PAIRING_ALPN`] and then sends nothing
/// (or a truncated length prefix) parks `read_payload`'s `read_exact` forever,
/// holding the pairing session — on the GUI path that means the armed host is
/// stuck until [`HOST_ACCEPT_TIMEOUT`], never able to serve the real joiner. The
/// exchange is a couple of small frames over an already-established QUIC stream,
/// so a generous 30s is far more than a healthy peer needs and still bounds a
/// stalled one.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long we wait for the endpoint to come "online" (a relay handshake
/// completed) before generating the ticket / payload. `Endpoint::online`
/// pends forever with no WAN/relay, so it MUST be wrapped in a timeout.
///
/// We still proceed if it times out: the endpoint's `addr()` already carries
/// the discovered direct (LAN) addresses, which is exactly what two devices on
/// the same WiFi need — the relay is a bonus for the cross-network case.
const ONLINE_TIMEOUT: Duration = Duration::from_secs(5);

/// The wire payload exchanged during pairing.
///
/// A trimmed projection of [`PeerEntry`] — the receiving side fills in its own
/// `added_at` timestamp, so the sender never dictates it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PairingPayload {
    /// Sender's iroh node id (string form of [`iroh::EndpointId`]).
    node_id: String,
    /// Optional human-readable label the sender advertises for itself.
    alias: Option<String>,
    /// Sender's home relay URL, if any (a hint for the first reconnect).
    relay_url: Option<String>,
    /// Sender's **full** [`iroh::EndpointAddr`] (id + relay + direct addrs),
    /// base64-encoded JSON. Captured after the sender's endpoint came online so
    /// it carries reachable direct (LAN) addresses. `None` only if encoding
    /// failed; the receiver then falls back to node id + relay url.
    #[serde(default)]
    endpoint_addr: Option<String>,
    /// Sender's stable, shared workspace id (see [`outl_core::WorkspaceId`]).
    /// The JOINER adopts the HOST's id so both sides derive the same gossip topic
    /// and validate sync requests as one workspace. `#[serde(default)]` keeps it
    /// back-compatible: a peer on an older build (no id) sends `None`, and the
    /// joiner simply keeps its own id (no adoption) instead of failing pairing.
    #[serde(default)]
    workspace_id: Option<String>,
}

impl PairingPayload {
    /// Build our own payload from the local node id + a *ready* endpoint addr.
    ///
    /// `addr` must come from an endpoint that has already discovered its
    /// addresses (see [`ready_addr`]); otherwise the direct addrs / relay are
    /// empty and the remote stores an unreachable peer (the original bug).
    fn from_local(
        identity: &IrohIdentity,
        addr: &EndpointAddr,
        alias: Option<String>,
        workspace_id: Option<&WorkspaceId>,
    ) -> Self {
        let endpoint_addr = match encode_endpoint_addr(addr) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!("encode our endpoint addr for pairing payload: {e}");
                None
            }
        };
        Self {
            node_id: identity.node_id().to_string(),
            alias,
            relay_url: addr.relay_urls().next().map(|u| u.to_string()),
            endpoint_addr,
            workspace_id: workspace_id.map(|w| w.as_str().to_string()),
        }
    }

    /// The remote's advertised workspace id, if it sent one. `None` when the peer
    /// is on an older build that predates workspace-id pairing.
    fn remote_workspace_id(&self) -> Option<WorkspaceId> {
        self.workspace_id
            .as_ref()
            .map(|s| WorkspaceId::from_raw(s.clone()))
    }

    /// Convert a received payload into a persistable [`PeerEntry`], stamping
    /// the local wall-clock time as `added_at`.
    ///
    /// Validates the sender's full `endpoint_addr` (drops it if it won't decode)
    /// so a corrupt field degrades to id + relay url instead of poisoning the
    /// store.
    fn into_peer_entry(self) -> PeerEntry {
        let endpoint_addr = self.endpoint_addr.filter(|encoded| {
            decode_endpoint_addr(encoded)
                .inspect_err(|e| tracing::warn!("peer sent an undecodable endpoint addr: {e}"))
                .is_ok()
        });
        PeerEntry {
            node_id: self.node_id,
            alias: self.alias,
            relay_url: self.relay_url,
            endpoint_addr,
            added_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Serialize a [`PairingPayload`] with a 4-byte big-endian length prefix.
fn encode_payload(payload: &PairingPayload) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(payload)?;
    let len = u32::try_from(json.len())?.to_be_bytes();
    let mut buf = Vec::with_capacity(4 + json.len());
    buf.extend_from_slice(&len);
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Read one length-prefixed [`PairingPayload`] off a stream.
async fn read_payload(recv: &mut iroh::endpoint::RecvStream) -> Result<PairingPayload> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .context("read payload length prefix")?;
    let len = u32::from_be_bytes(len_buf) as usize;
    anyhow::ensure!(len <= 64 * 1024, "pairing payload too large ({len} bytes)");
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body)
        .await
        .context("read payload body")?;
    serde_json::from_slice(&body).context("decode pairing payload")
}

/// Reject a pairing payload whose self-declared `node_id` does not match the
/// connection's **authenticated** TLS identity (issue #159).
///
/// iroh authenticates the remote's [`iroh::EndpointId`] as part of the QUIC/TLS
/// handshake (`conn.remote_id()` is not spoofable), but the `PairingPayload`
/// carries a self-declared `node_id` that we would otherwise persist verbatim
/// via [`PairingPayload::into_peer_entry`]. A malicious joiner could declare the
/// node id of an *already-paired* device and, through `PeersStore::add`'s
/// dedup-replace, overwrite that peer's stored address — making the legit device
/// unreachable. Binding the persisted id to the authenticated one closes that.
///
/// TODO(#159): this only proves the connecting device controls the key it
/// claims. It does NOT prove the ticket was issued to *this* joiner — a stronger
/// guarantee needs a shared secret / proof-of-possession baked into the ticket,
/// which is a larger protocol change tracked as a follow-up.
fn verify_declared_identity(
    conn: &iroh::endpoint::Connection,
    payload: &PairingPayload,
) -> Result<()> {
    check_identity_match(conn.remote_id(), payload)
}

/// Pure core of [`verify_declared_identity`]: compare a payload's self-declared
/// `node_id` against an already-authenticated [`iroh::EndpointId`]. Split from
/// the `Connection` so it is unit-testable without standing up QUIC.
fn check_identity_match(authenticated: iroh::EndpointId, payload: &PairingPayload) -> Result<()> {
    let declared: iroh::EndpointId = payload
        .node_id
        .parse()
        .context("pairing payload carried an unparseable node_id")?;
    anyhow::ensure!(
        declared == authenticated,
        "pairing identity mismatch: peer declared node_id {} but the connection is authenticated as {} — aborting to prevent a peer-hijack (issue #159)",
        declared.fmt_short(),
        authenticated.fmt_short(),
    );
    Ok(())
}

/// Encode an [`EndpointAddr`] into a copy-pasteable ticket string.
///
/// A pairing ticket IS a base64-JSON `EndpointAddr`, identical to what a
/// [`PeerEntry`] stores in its `endpoint_addr` field — so this delegates to the
/// one codec in [`crate::peers`] rather than carrying a parallel copy.
pub fn encode_ticket(addr: &EndpointAddr) -> Result<String> {
    encode_endpoint_addr(addr)
}

/// Decode a ticket string back into an [`EndpointAddr`].
pub fn decode_ticket(ticket: &str) -> Result<EndpointAddr> {
    decode_endpoint_addr(ticket)
}

/// Render an [`EndpointAddr`]'s ticket as a block-character QR for the terminal.
pub fn ticket_qr(ticket: &str) -> Result<String> {
    use qrcode::render::unicode;
    use qrcode::QrCode;

    let code = QrCode::new(ticket.as_bytes()).context("build QR code")?;
    Ok(code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .quiet_zone(true)
        .build())
}

/// Bind a fresh endpoint on the pairing ALPN with the given identity.
async fn bind_pairing_endpoint(identity: &IrohIdentity) -> Result<Endpoint> {
    // STOPGAP: IPv4-only bind (iroh 1.0.0 multipath stalls on unreachable IPv6
    // direct paths). Binding IPv4-only here means the `EndpointAddr` captured by
    // `ready_addr` and baked into the pairing ticket / payload carries no global
    // IPv6 direct addr, so the peer never stores (and later dials) a dead path.
    // Revert to the plain dual-stack builder when iroh > 1.0.0 ships the
    // multipath fallback fix. See `crate::bind`.
    crate::bind::n0_builder_ipv4_only(None)
        .secret_key(identity.secret_key().clone())
        .alpns(vec![PAIRING_ALPN.to_vec()])
        .bind()
        .await
        .context("bind pairing endpoint")
}

/// Wait (bounded) for the endpoint to discover its addresses, then snapshot a
/// **ready** [`EndpointAddr`] carrying relay + direct addrs.
///
/// `endpoint.addr()` right after `bind()` is typically empty — no relay
/// handshake, no net report — so a ticket/payload built from it stores an
/// unreachable peer (the root cause of the offline-dot bug). `Endpoint::online`
/// resolves once a relay handshake completes (and a net report has run, which
/// populates the LAN direct addrs); we cap it with [`ONLINE_TIMEOUT`] because
/// `online` pends forever with no relay/WAN. On timeout we still return the
/// current addr — by then the local net report has usually filled in the direct
/// addrs, which is all two devices on the same WiFi need.
pub(crate) async fn ready_addr(endpoint: &Endpoint) -> EndpointAddr {
    if tokio::time::timeout(ONLINE_TIMEOUT, endpoint.online())
        .await
        .is_err()
    {
        tracing::warn!(
            "endpoint not online within {}s; pairing with direct addrs only (no relay yet)",
            ONLINE_TIMEOUT.as_secs()
        );
    }
    let addr = endpoint.addr();
    tracing::info!(
        node_id = %addr.id.fmt_short(),
        relays = addr.relay_urls().count(),
        direct_addrs = addr.ip_addrs().count(),
        "pairing endpoint ready"
    );
    addr
}

/// The generating side of pairing.
///
/// Binds an endpoint, hands its ticket (string + QR) to `on_ticket`, then waits
/// for exactly one inbound connection, completes the handshake, persists the
/// peer to `peers_path`, and returns the entry that was stored.
///
/// `workspace_root` is the graph the pairing belongs to; the host reads (or
/// creates) its stable [`WorkspaceId`] from `<root>/.outl/workspace-id` and
/// **advertises** it so the joiner can adopt it. The host keeps its own id — a
/// joiner joins the host's workspace, never the other way around.
pub async fn host_pairing<F>(
    identity: Arc<IrohIdentity>,
    peers_path: &Path,
    workspace_root: &Path,
    alias: Option<String>,
    on_ticket: F,
) -> Result<PeerEntry>
where
    F: FnOnce(&str, &str),
{
    // Advertise our stable workspace id so the joiner adopts it and both sides
    // land on the same gossip topic + pass the `serve` workspace-id check.
    // Without this the joiner keeps a fresh id and every later sync is rejected
    // as `workspace-mismatch` (issue #197).
    let local_wid = WorkspaceId::read_or_create(workspace_root)
        .context("read or create local workspace id for pairing")?;
    let endpoint = bind_pairing_endpoint(&identity).await?;

    // Wait for the endpoint to discover its relay + direct addresses before
    // snapshotting the addr — a bare-bound `addr()` would mint a ticket the
    // joiner can't dial.
    let addr = ready_addr(&endpoint).await;
    let ticket = encode_ticket(&addr)?;
    let qr = ticket_qr(&ticket)?;
    on_ticket(&ticket, &qr);

    // Accept exactly one inbound pairing connection (with a sane timeout).
    let incoming = tokio::time::timeout(HOST_ACCEPT_TIMEOUT, endpoint.accept())
        .await
        .context("timed out waiting for the other device to connect")?
        .context("pairing endpoint closed before a connection arrived")?;

    let conn = incoming
        .accept()
        .context("accept inbound pairing connection")?
        .await
        .context("complete pairing handshake")?;

    // The host advertises its own id (`local_wid`) and keeps it; the joiner is
    // the side that adopts (see `join_pairing`). `_remote_wid` is the joiner's
    // id, which the host deliberately ignores.
    let (entry, _remote_wid) =
        accept_host_handshake(&conn, &identity, &addr, alias, Some(&local_wid)).await?;
    persist_peer(peers_path, entry.clone())?;

    // The host sends its payload LAST, so it must not slam the connection (or
    // its endpoint) shut before the joiner has read it — that truncates the
    // joiner's `read_payload` ("connection lost / closed by peer"). Wait for the
    // joiner to close the connection itself (it does so right after reading our
    // payload); `closed()` returns once that close arrives, or on the accept
    // timeout's connection drop.
    conn.closed().await;
    endpoint.close().await;
    Ok(entry)
}

/// The host (accept) side of the pairing handshake over an already-accepted
/// [`iroh::endpoint::Connection`].
///
/// Reads the joiner's [`PairingPayload`], replies with ours (built from
/// `our_addr`, which must be a *ready* addr — relay + direct addrs), and
/// returns the joiner's [`PeerEntry`]. **Does not** persist or close the
/// connection — the caller owns both, because the close timing differs between
/// the CLI ([`host_pairing`], which closes its one-shot endpoint) and the GUI
/// (the router handler, which leaves the live sync endpoint up).
///
/// Endpoint-agnostic on purpose: the CLI feeds a one-shot pairing endpoint's
/// connection; the GUI feeds the live sync endpoint's `PAIRING_ALPN` connection.
/// One handshake, two transports.
pub(crate) async fn accept_host_handshake(
    conn: &iroh::endpoint::Connection,
    identity: &IrohIdentity,
    our_addr: &EndpointAddr,
    alias: Option<String>,
    workspace_id: Option<&WorkspaceId>,
) -> Result<(PeerEntry, Option<WorkspaceId>)> {
    let (mut send, mut recv) = conn.accept_bi().await.context("accept pairing bi stream")?;

    // The joiner speaks first: read their entry, then send ours (advertising our
    // workspace id, which the joiner adopts). Bound the read so a peer that opens
    // the stream and then stalls can't hold the pairing session (issue #159).
    let remote = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_payload(&mut recv))
        .await
        .context("timed out reading the joiner's pairing payload")??;

    // Reject a joiner whose declared node_id doesn't match its authenticated TLS
    // identity BEFORE persisting — otherwise it could overwrite an existing
    // peer's stored address and make that device unreachable (issue #159).
    verify_declared_identity(conn, &remote)?;

    let ours = PairingPayload::from_local(identity, our_addr, alias, workspace_id);
    send.write_all(&encode_payload(&ours)?)
        .await
        .context("send our pairing payload")?;
    send.finish().context("finish pairing send")?;

    let remote_wid = remote.remote_workspace_id();
    Ok((remote.into_peer_entry(), remote_wid))
}

/// The joining side of pairing.
///
/// Parses `ticket`, connects to the host, completes the handshake, persists the
/// peer to `peers_path`, **adopts the host's [`WorkspaceId`]**, and returns the
/// stored entry plus a [`WorkspaceAdoption`] describing what happened to our id.
///
/// Adoption is the load-bearing half: a CLI machine that pairs into an existing
/// workspace must take on the host's id, or every later sync is refused as
/// `workspace-mismatch` (issue #197). It is **persist-first** — the host's id is
/// written to `<root>/.outl/workspace-id` before this returns, so the next
/// `outl` / `outl sync` on this machine reads the adopted id instead of the
/// fresh one `read_or_create` would otherwise mint.
pub async fn join_pairing(
    identity: Arc<IrohIdentity>,
    ticket: &str,
    peers_path: &Path,
    workspace_root: &Path,
    alias: Option<String>,
) -> Result<(PeerEntry, WorkspaceAdoption)> {
    // Our current id (advertised to the host; a host on the GUI path ignores it,
    // but a CLI host would keep its own and we still send ours for symmetry).
    let local_wid = WorkspaceId::read_or_create(workspace_root)
        .context("read or create local workspace id for pairing")?;
    let endpoint = bind_pairing_endpoint(&identity).await?;

    // Snapshot a *ready* addr (relay + direct addrs) so the payload we send the
    // host stores a reachable joiner, not a bare node id.
    let our_addr = ready_addr(&endpoint).await;

    let (entry, remote_wid) = run_join_handshake(
        &endpoint,
        &identity,
        ticket,
        &our_addr,
        alias,
        Some(&local_wid),
    )
    .await?;

    // Adopt the host's id. Persist-first: write to disk, and only report success
    // once it's durable, so a failed write leaves us on our old id (retry-safe)
    // rather than half-adopted.
    let adopted = match remote_wid {
        Some(host_wid) if host_wid != local_wid => {
            host_wid
                .write(workspace_root)
                .context("adopt host workspace id")?;
            WorkspaceAdoption::Adopted(host_wid)
        }
        Some(_) => WorkspaceAdoption::AlreadyMatched,
        None => WorkspaceAdoption::HostSentNone,
    };

    persist_peer(peers_path, entry.clone())?;

    endpoint.close().await;
    Ok((entry, adopted))
}

/// The joiner side of the pairing handshake, dialing out over `endpoint`.
///
/// Decodes the host's `ticket`, connects on [`PAIRING_ALPN`], sends our
/// [`PairingPayload`] (built from `our_addr`, which must be a ready addr), reads
/// the host's, and returns the host's [`PeerEntry`]. Closes the *connection*
/// (`conn.close`) but **not** the `endpoint` — the caller owns the endpoint
/// lifetime (the CLI closes its one-shot endpoint; the GUI keeps the live sync
/// endpoint up).
pub(crate) async fn run_join_handshake(
    endpoint: &Endpoint,
    identity: &IrohIdentity,
    ticket: &str,
    our_addr: &EndpointAddr,
    alias: Option<String>,
    workspace_id: Option<&WorkspaceId>,
) -> Result<(PeerEntry, Option<WorkspaceId>)> {
    let remote_addr = decode_ticket(ticket)?;

    let conn = endpoint
        .connect(remote_addr, PAIRING_ALPN)
        .await
        .context("connect to pairing host")?;

    let (mut send, mut recv) = conn.open_bi().await.context("open pairing bi stream")?;

    // We opened the stream, so we speak first: send ours, then read theirs (the
    // host's payload carries the workspace id this joiner ADOPTS).
    let ours = PairingPayload::from_local(identity, our_addr, alias, workspace_id);
    send.write_all(&encode_payload(&ours)?)
        .await
        .context("send our pairing payload")?;
    send.finish().context("finish pairing send")?;

    // Bound the read so a host that accepts the dial but never replies can't hang
    // the joiner (issue #159).
    let remote = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_payload(&mut recv))
        .await
        .context("timed out reading the host's pairing payload")??;

    // The host is the one we're persisting into peers.json here; verify its
    // declared node_id matches the connection's authenticated identity so a
    // man-in-the-middle host can't get us to store someone else's node id
    // (issue #159).
    verify_declared_identity(&conn, &remote)?;

    let remote_wid = remote.remote_workspace_id();
    let entry = remote.into_peer_entry();

    conn.close(0u32.into(), b"paired");
    Ok((entry, remote_wid))
}

/// Append a freshly-paired peer to `peers.json` (deduplicating by node id).
fn persist_peer(peers_path: &Path, entry: PeerEntry) -> Result<()> {
    let mut store = PeersStore::load_or_default(peers_path)?;
    store.add(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_roundtrips_through_base64() {
        // A bare EndpointAddr (id only) is enough to exercise the codec.
        let secret = iroh::SecretKey::generate();
        let addr = EndpointAddr::new(secret.public());
        let ticket = encode_ticket(&addr).expect("encode");
        let decoded = decode_ticket(&ticket).expect("decode");
        assert_eq!(decoded.id, addr.id);
    }

    #[test]
    fn decode_ticket_rejects_garbage() {
        assert!(decode_ticket("not-a-real-ticket!!!").is_err());
    }

    #[test]
    fn payload_roundtrips_through_length_prefix() {
        let payload = PairingPayload {
            node_id: "abcdef".into(),
            alias: Some("iPhone".into()),
            relay_url: None,
            endpoint_addr: None,
            workspace_id: None,
        };
        let encoded = encode_payload(&payload).expect("encode");
        // First 4 bytes are the big-endian length.
        let len = u32::from_be_bytes(encoded[..4].try_into().unwrap()) as usize;
        assert_eq!(len, encoded.len() - 4);
        let decoded: PairingPayload = serde_json::from_slice(&encoded[4..]).expect("decode");
        assert_eq!(decoded.node_id, "abcdef");
        assert_eq!(decoded.alias.as_deref(), Some("iPhone"));
    }

    #[test]
    fn payload_into_peer_entry_stamps_added_at() {
        let payload = PairingPayload {
            node_id: "node".into(),
            alias: None,
            relay_url: Some("https://relay.example".into()),
            endpoint_addr: None,
            workspace_id: None,
        };
        let entry = payload.into_peer_entry();
        assert_eq!(entry.node_id, "node");
        assert!(!entry.added_at.is_empty());
    }

    #[test]
    fn payload_carries_full_endpoint_addr_into_peer_entry() {
        // A pairing payload built from a real (id-only) addr roundtrips the
        // encoded full addr into the persisted entry, and that entry's
        // `iroh_endpoint_addr()` decodes back to the same id.
        let secret = iroh::SecretKey::generate();
        let addr = EndpointAddr::new(secret.public());
        let encoded = encode_endpoint_addr(&addr).expect("encode addr");
        let payload = PairingPayload {
            node_id: addr.id.to_string(),
            alias: None,
            relay_url: None,
            endpoint_addr: Some(encoded),
            workspace_id: None,
        };
        let entry = payload.into_peer_entry();
        assert!(entry.endpoint_addr.is_some());
        let decoded = entry.iroh_endpoint_addr().expect("decode entry addr");
        assert_eq!(decoded.id, addr.id);
    }

    #[test]
    fn payload_drops_corrupt_endpoint_addr() {
        let payload = PairingPayload {
            node_id: "node".into(),
            alias: None,
            relay_url: None,
            endpoint_addr: Some("!!!not-base64!!!".into()),
            workspace_id: None,
        };
        let entry = payload.into_peer_entry();
        // Corrupt addr is dropped rather than persisted.
        assert!(entry.endpoint_addr.is_none());
    }

    #[test]
    fn payload_roundtrips_workspace_id() {
        let wid = WorkspaceId::from_raw("WS00000000000000000000000000");
        let payload = PairingPayload {
            node_id: "node".into(),
            alias: None,
            relay_url: None,
            endpoint_addr: None,
            workspace_id: Some(wid.as_str().to_string()),
        };
        let encoded = encode_payload(&payload).expect("encode");
        let decoded: PairingPayload = serde_json::from_slice(&encoded[4..]).expect("decode");
        assert_eq!(decoded.remote_workspace_id(), Some(wid));
    }

    #[test]
    fn payload_without_workspace_id_yields_none() {
        // Back-compat: a peer on an older build sends no id, so adoption is a
        // no-op (the joiner keeps its own id).
        let payload = PairingPayload {
            node_id: "node".into(),
            alias: None,
            relay_url: None,
            endpoint_addr: None,
            workspace_id: None,
        };
        assert_eq!(payload.remote_workspace_id(), None);
    }

    /// Build a `PairingPayload` that self-declares `node_id`.
    fn payload_declaring(node_id: &str) -> PairingPayload {
        PairingPayload {
            node_id: node_id.to_string(),
            alias: None,
            relay_url: None,
            endpoint_addr: None,
            workspace_id: None,
        }
    }

    /// Issue #159: a payload that self-declares the SAME node_id as the
    /// connection's authenticated identity passes — the honest handshake.
    #[test]
    fn identity_match_accepts_when_declared_equals_authenticated() {
        let authenticated = iroh::SecretKey::generate().public();
        let payload = payload_declaring(&authenticated.to_string());
        assert!(
            check_identity_match(authenticated, &payload).is_ok(),
            "a matching declared node_id must be accepted"
        );
    }

    /// Issue #159 (the exploit): a malicious joiner authenticates as one key but
    /// declares the node_id of an ALREADY-PAIRED device, aiming to overwrite that
    /// peer's stored address via `PeersStore::add`'s dedup-replace and make it
    /// unreachable. The mismatch MUST be rejected before any persist.
    #[test]
    fn identity_match_rejects_a_spoofed_peer_node_id() {
        let authenticated = iroh::SecretKey::generate().public();
        let victim = iroh::SecretKey::generate().public(); // an already-paired device
        assert_ne!(authenticated, victim);
        let payload = payload_declaring(&victim.to_string());
        let err = check_identity_match(authenticated, &payload)
            .expect_err("a spoofed node_id must be rejected");
        assert!(
            err.to_string().contains("identity mismatch"),
            "error must name the identity mismatch, got: {err}"
        );
    }

    /// A payload whose `node_id` isn't even a valid EndpointId is rejected (not
    /// silently coerced) — the pre-#159 code fed it straight into `peers.json`.
    #[test]
    fn identity_match_rejects_an_unparseable_node_id() {
        let authenticated = iroh::SecretKey::generate().public();
        let payload = payload_declaring("not-a-real-node-id");
        assert!(
            check_identity_match(authenticated, &payload).is_err(),
            "an unparseable declared node_id must be rejected"
        );
    }
}
