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
//! These standalone [`host_pairing`] / [`join_pairing`] functions survive for
//! the **CLI** (`outl peer pair`), which has no running transport of its own,
//! so binding a one-shot endpoint here is the only option.
//!
//! It used to be the case that there was also no route to steal. That premise
//! is gone: `outl serve` is built to hold the endpoint for as long as the
//! machine is up, so on a box running it there IS a live route, and a pairing
//! endpoint takes it for the length of the handshake. The daemon's stand-down
//! with no paired peers covers only the FIRST device; pairing a second one
//! overlaps a running transport. That overlap is accepted for the same reason
//! the GUI accepts it — a device you cannot add is worse — and bounded the
//! same way: both paths still **close their endpoint**
//! (`endpoint.close().await`) before returning, and the daemon rebuilds its
//! transport when `peers.json` changes.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use base64::Engine as _;
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

/// How long one `connect` attempt to the pairing host may take, and how many
/// times the joiner retries before giving up.
///
/// `Endpoint::connect` has no timeout of its own and **can pend forever**. Every
/// other step of this handshake got a bound in issue #159 — the accept window,
/// each payload read — and this one was missed, so the failure it produces is
/// the worst-looking kind: `outl peer pair` sits there with no output and no
/// error, and the user has nothing to report but "it hung".
///
/// It is not hypothetical. It reproduces roughly half the time in
/// `a_refused_joiner_does_not_consume_the_pairing_window`, where the joiner
/// dials a host that has just closed another connection: the first attempt
/// never completes, and a second one succeeds immediately. That is a transient
/// path-selection stall, so a retry is the honest fix and the timeout is what
/// makes a retry possible at all.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Attempts before the joiner reports failure. Three bounded tries cost at most
/// 24s against a genuinely unreachable host — well inside the host's 2 minute
/// window, so a user who mistypes nothing still gets one shot per retry.
const CONNECT_ATTEMPTS: usize = 3;

/// Dial the pairing host, bounded and retried.
///
/// Returns the last error when every attempt fails, so the message names what
/// actually went wrong rather than "gave up".
async fn connect_to_host(
    endpoint: &Endpoint,
    remote_addr: EndpointAddr,
) -> Result<iroh::endpoint::Connection> {
    let mut last: Option<anyhow::Error> = None;
    for attempt in 1..=CONNECT_ATTEMPTS {
        match tokio::time::timeout(
            CONNECT_TIMEOUT,
            endpoint.connect(remote_addr.clone(), PAIRING_ALPN),
        )
        .await
        {
            Ok(Ok(conn)) => return Ok(conn),
            Ok(Err(e)) => {
                tracing::warn!(
                    "pairing connect attempt {attempt}/{CONNECT_ATTEMPTS} failed: {e:#}"
                );
                last = Some(anyhow::Error::new(e).context("connect to pairing host"));
            }
            Err(_) => {
                tracing::warn!(
                    "pairing connect attempt {attempt}/{CONNECT_ATTEMPTS} timed out after {CONNECT_TIMEOUT:?}"
                );
                last = Some(anyhow::anyhow!(
                    "connecting to the pairing host timed out after {CONNECT_TIMEOUT:?}"
                ));
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("could not connect to the pairing host")))
        .context("the other device did not answer — check it is still showing the pairing code, and that both devices are on a network that can reach each other")
}

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
    /// Joiner's proof that it holds the ticket the host issued: hex
    /// `blake3::keyed_hash(ticket_secret, our_node_id)` (issue #159).
    ///
    /// Only the joiner sends it — the host is already authenticated to the
    /// joiner by the node id baked into the ticket it dialled. `Option` so the
    /// host can name the failure ("that device is on an older outl") instead of
    /// reporting a decode error.
    #[serde(default)]
    pair_auth: Option<String>,
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
        // `Some` only on the joining side: the proof is what the host checks.
        ticket_secret: Option<&PairingSecret>,
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
            pair_auth: ticket_secret
                .map(|secret| hex_encode(secret.proof_for(identity.node_id()).as_bytes())),
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
/// This only proves the connecting device controls the key it claims. Proving
/// the ticket was issued to *this* joiner is a separate question, answered by
/// [`verify_pairing_proof`], and the two interlock: the proof is keyed to the
/// node id this function has just pinned to the authenticated identity.
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

/// The random secret carried inside a pairing ticket, proving that whoever
/// dials the host actually holds the invite the host issued.
///
/// ## Why the invite needs a secret at all
///
/// The host is armed for a couple of minutes, and before this existed the
/// **first** device to connect on the pairing ALPN was accepted and handed the
/// workspace identity. Anyone who learned the host's address during that window
/// — a photographed QR code, or any existing member of the mesh, which already
/// knows every peer's addr from membership gossip — could take the slot ahead of
/// the device the invite was meant for (issue #159).
///
/// Verifying the declared node id does not help here. It proves the dialer
/// controls the key it claims, which a stranger also does. Possession of the
/// invite is a different question and needs a different answer.
#[derive(Clone)]
pub struct PairingSecret([u8; 32]);

impl PairingSecret {
    /// Mint a fresh secret from the thread CSPRNG (seeded from the OS).
    pub fn generate() -> Self {
        Self(rand::random())
    }

    /// The proof a joiner sends: a MAC over its **own** node id, keyed by the
    /// ticket secret.
    ///
    /// Binding the MAC to the joiner's identity is what stops a replay. A bare
    /// `hash(secret)` would be a bearer token: anyone who watched one joiner use
    /// the ticket could resend the same bytes. Keyed to the node id, a captured
    /// proof only authenticates the device it was already issued for — and that
    /// device's node id is separately pinned to the connection's authenticated
    /// TLS identity by [`verify_declared_identity`], so the two checks
    /// interlock.
    pub(crate) fn proof_for(&self, node_id: iroh::EndpointId) -> blake3::Hash {
        // `keyed_hash` is a PRF/MAC, and `blake3::Hash`'s `PartialEq` is
        // constant-time — so the comparison in `verify_pairing_proof` does not
        // leak the expected value one byte at a time.
        blake3::keyed_hash(&self.0, node_id.to_string().as_bytes())
    }
}

impl std::fmt::Debug for PairingSecret {
    /// Redacted. A pairing secret in a log line or an error report is an invite
    /// anyone reading it can use for the rest of the window.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PairingSecret(<redacted>)")
    }
}

/// Wire form of a ticket: where to dial, plus the secret to prove you were
/// invited.
#[derive(Serialize, Deserialize)]
struct TicketBody {
    /// Base64 [`EndpointAddr`], the whole of what a v1 ticket carried.
    addr: String,
    /// Hex-encoded [`PairingSecret`].
    secret: String,
}

/// Version prefix on the ticket string.
///
/// A v1 ticket was a bare base64 blob with no secret, so a v1 joiner cannot
/// produce a proof and a v1 host would ignore one. Rather than accept a ticket
/// with no secret — which is a downgrade any observer could force — the prefix
/// makes the mismatch explicit so the user is told to update the other device
/// instead of silently pairing without the check.
const TICKET_PREFIX: &str = "outlpair1.";

/// Mint a ticket for `addr`, returning the string to display and the secret the
/// host must keep to verify the joiner.
pub fn mint_ticket(addr: &EndpointAddr) -> Result<(String, PairingSecret)> {
    let secret = PairingSecret::generate();
    let ticket = ticket_with_secret(addr, &secret)?;
    Ok((ticket, secret))
}

/// Encode a ticket for `addr` carrying an already-chosen `secret`.
///
/// Split out of [`mint_ticket`] for `test_support::retarget_ticket`, which has
/// to re-point a ticket at a deterministic address without invalidating the
/// secret the host is holding.
pub(crate) fn ticket_with_secret(addr: &EndpointAddr, secret: &PairingSecret) -> Result<String> {
    let body = TicketBody {
        addr: encode_endpoint_addr(addr)?,
        secret: hex_encode(&secret.0),
    };
    let json = serde_json::to_vec(&body).context("serialize pairing ticket")?;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json);
    Ok(format!("{TICKET_PREFIX}{encoded}"))
}

/// Decode a ticket produced by [`mint_ticket`].
pub fn decode_ticket(ticket: &str) -> Result<(EndpointAddr, PairingSecret)> {
    let ticket = ticket.trim();
    let Some(encoded) = ticket.strip_prefix(TICKET_PREFIX) else {
        // Be specific: "invalid ticket" sends the user hunting for a typo when
        // the actual problem is a version skew they can fix in a minute.
        anyhow::bail!(
            "this pairing code was made by an older version of outl, which paired without verifying the invite. Update outl on the other device and generate a new code."
        );
    };
    let json = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded)
        .context("base64-decode pairing ticket")?;
    let body: TicketBody = serde_json::from_slice(&json).context("decode pairing ticket")?;
    let addr = decode_endpoint_addr(&body.addr)?;
    let secret = hex_decode(&body.secret).context("decode pairing ticket secret")?;
    Ok((addr, PairingSecret(secret)))
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<[u8; 32]> {
    // `is_ascii()` is load-bearing, not belt-and-braces. `len()` counts bytes
    // and the loop below slices by byte offset, so a 64-**byte** string holding
    // any multi-byte char (`"a" + "é" + 61 ASCII`) makes `&s[0..2]` land inside
    // a char and **panic**. This input is `payload.pair_auth` off the wire,
    // read before the peer has proved anything, so a panic here is a remote
    // kill of the host's pairing session.
    anyhow::ensure!(
        s.len() == 64 && s.is_ascii(),
        "expected 64 ASCII hex chars, got {} bytes",
        s.len()
    );
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).context("non-hex character")?;
    }
    Ok(out)
}

/// Reject a joiner that cannot prove it holds the ticket the host issued.
///
/// Checked **before** the host sends its own payload, because that payload is
/// what carries the workspace identity — the thing an uninvited joiner is
/// after. A stranger that reaches the host now learns nothing beyond the fact
/// that something is listening.
fn verify_pairing_proof(
    conn: &iroh::endpoint::Connection,
    payload: &PairingPayload,
    secret: &PairingSecret,
) -> Result<()> {
    let Some(offered) = payload.pair_auth.as_deref() else {
        anyhow::bail!(
            "the joining device did not prove it holds this pairing code — it is probably running an older version of outl. Update it and try again."
        );
    };
    let offered = hex_decode(offered).context("decode the joiner's pairing proof")?;
    let expected = secret.proof_for(conn.remote_id());
    anyhow::ensure!(
        // `blake3::Hash: PartialEq` is constant-time.
        expected == blake3::Hash::from(offered),
        "the joining device could not prove it holds this pairing code — refusing to pair (issue #159)"
    );
    Ok(())
}

/// Encode an [`EndpointAddr`] into a copy-pasteable ticket string.
///
/// A pairing ticket IS a base64-JSON `EndpointAddr`, identical to what a
/// [`PeerEntry`] stores in its `endpoint_addr` field — so this delegates to the
/// one codec in [`crate::peers`] rather than carrying a parallel copy.
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
    let (ticket, secret) = mint_ticket(&addr)?;
    let qr = ticket_qr(&ticket)?;
    on_ticket(&ticket, &qr);

    // Keep accepting until the window closes or a joiner proves it holds the
    // ticket. Accepting exactly one connection made the window itself the
    // attack: any dial — a stranger's, or a stale probe — consumed the single
    // slot, and the device the user was actually pairing then found nothing
    // listening (issue #159). A failed attempt now costs the attacker nothing
    // and the user nothing either.
    let deadline = tokio::time::Instant::now() + HOST_ACCEPT_TIMEOUT;
    let (conn, entry) = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        anyhow::ensure!(
            !remaining.is_zero(),
            "timed out waiting for the other device to connect"
        );

        let incoming = tokio::time::timeout(remaining, endpoint.accept())
            .await
            .context("timed out waiting for the other device to connect")?
            .context("pairing endpoint closed before a connection arrived")?;

        let conn = match incoming
            .accept()
            .context("accept inbound pairing connection")
        {
            Ok(connecting) => match connecting.await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::warn!("inbound pairing connection failed to complete: {e:#}");
                    continue;
                }
            },
            Err(e) => {
                tracing::warn!("{e:#}");
                continue;
            }
        };

        // The host advertises its own id (`local_wid`) and keeps it; the joiner
        // is the side that adopts (see `join_pairing`). `_remote_wid` is the
        // joiner's id, which the host deliberately ignores.
        match accept_host_handshake(
            &conn,
            &identity,
            &addr,
            alias.clone(),
            Some(&local_wid),
            &secret,
        )
        .await
        {
            Ok((entry, _remote_wid)) => break (conn, entry),
            Err(e) => {
                // Log and re-arm rather than abort: the legitimate device may
                // still be about to dial, and telling the user "pairing failed"
                // because someone else's stale connection arrived first is the
                // failure this loop exists to prevent.
                tracing::warn!("rejected an inbound pairing attempt: {e:#}");
                conn.close(1u32.into(), b"pairing-refused");
            }
        }
    };
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
    ticket_secret: &PairingSecret,
) -> Result<(PeerEntry, Option<WorkspaceId>)> {
    // Bound the wait for the stream itself, not just the payload on it. A peer
    // that completes the QUIC handshake and never opens a bi stream would
    // otherwise hold this call forever — and because `host_pairing`'s accept
    // loop is serial and only re-checks its deadline at the top, one such
    // connection wedges the whole pairing window. That is the same one-packet
    // denial the re-arm loop exists to prevent, arriving one step earlier.
    let (mut send, mut recv) = tokio::time::timeout(HANDSHAKE_TIMEOUT, conn.accept_bi())
        .await
        .context("the joining device connected but never opened the pairing stream")?
        .context("accept pairing bi stream")?;

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

    // Then prove they were invited. Order matters: the identity check makes
    // `conn.remote_id()` and the declared id the same value, which is what the
    // proof is keyed to. Both run BEFORE we send our payload, so a caller that
    // fails either one has not yet disclosed the workspace id.
    verify_pairing_proof(conn, &remote, ticket_secret)?;

    let ours = PairingPayload::from_local(identity, our_addr, alias, workspace_id, None);
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
    let (remote_addr, ticket_secret) = decode_ticket(ticket)?;

    let conn = connect_to_host(endpoint, remote_addr).await?;

    let (mut send, mut recv) = conn.open_bi().await.context("open pairing bi stream")?;

    // We opened the stream, so we speak first: send ours, then read theirs (the
    // host's payload carries the workspace id this joiner ADOPTS).
    let ours = PairingPayload::from_local(
        identity,
        our_addr,
        alias,
        workspace_id,
        Some(&ticket_secret),
    );
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
    fn ticket_roundtrips_addr_and_secret() {
        // A bare EndpointAddr (id only) is enough to exercise the codec.
        let key = iroh::SecretKey::generate();
        let addr = EndpointAddr::new(key.public());
        let (ticket, secret) = mint_ticket(&addr).expect("mint");
        let (decoded_addr, decoded_secret) = decode_ticket(&ticket).expect("decode");
        assert_eq!(decoded_addr.id, addr.id);
        assert_eq!(
            decoded_secret.0, secret.0,
            "the secret the host keeps and the one the joiner reads must match",
        );
    }

    #[test]
    fn decode_ticket_rejects_garbage() {
        assert!(decode_ticket("not-a-real-ticket!!!").is_err());
    }

    /// Two tickets minted back to back must not share a secret. A constant (or
    /// workspace-derived) secret would make every past invite valid forever,
    /// which is worse than the no-secret state it replaced — the old window at
    /// least closed after two minutes.
    #[test]
    fn every_ticket_gets_a_fresh_secret() {
        let addr = EndpointAddr::new(iroh::SecretKey::generate().public());
        let (_, a) = mint_ticket(&addr).expect("mint");
        let (_, b) = mint_ticket(&addr).expect("mint");
        assert_ne!(a.0, b.0, "ticket secrets must not repeat");
    }

    /// A v1 ticket (bare base64, no secret) must be refused with an explanation,
    /// not silently accepted. Accepting one is a downgrade any observer of the
    /// old format could force, which would leave the check present but bypassed.
    #[test]
    fn a_pre_secret_ticket_is_refused_and_says_why() {
        let addr = EndpointAddr::new(iroh::SecretKey::generate().public());
        let v1 = encode_endpoint_addr(&addr).expect("v1 encode");
        let err = decode_ticket(&v1).expect_err("a v1 ticket must not be accepted");
        let msg = err.to_string();
        assert!(
            msg.contains("older version") && msg.contains("new code"),
            "the error must tell the user to update and re-generate, got: {msg}",
        );
    }

    /// The honest case: the joiner's proof is keyed to its own node id, and the
    /// host recomputes it from the connection's authenticated identity.
    #[test]
    fn proof_verifies_for_the_invited_device() {
        let secret = PairingSecret::generate();
        let joiner = iroh::SecretKey::generate().public();
        assert_eq!(
            secret.proof_for(joiner),
            secret.proof_for(joiner),
            "the proof must be deterministic for one (secret, node id) pair",
        );
    }

    /// Issue #159 (the exploit): someone who learned the host's address during
    /// the ~2 minute window — from a photographed QR, or from membership gossip,
    /// which hands every mesh member every peer's addr — dials in without the
    /// ticket. They authenticate honestly as themselves, so
    /// `check_identity_match` passes; only the proof stops them.
    #[test]
    fn a_device_without_the_ticket_cannot_produce_the_proof() {
        let issued = PairingSecret::generate();
        let attacker_guess = PairingSecret::generate();
        let attacker = iroh::SecretKey::generate().public();
        assert_ne!(
            issued.proof_for(attacker),
            attacker_guess.proof_for(attacker),
            "a proof under a different secret must not verify",
        );
    }

    /// A proof is bound to the device it was made for, so capturing one off the
    /// wire does not let a second device reuse it. Without the binding the proof
    /// would be a bearer token and the window would be back.
    #[test]
    fn a_captured_proof_does_not_transfer_to_another_device() {
        let secret = PairingSecret::generate();
        let invited = iroh::SecretKey::generate().public();
        let eavesdropper = iroh::SecretKey::generate().public();
        assert_ne!(
            secret.proof_for(invited),
            secret.proof_for(eavesdropper),
            "a replayed proof must not authenticate a different node id",
        );
    }

    /// A joiner on an older build sends no proof at all. That must be a named
    /// refusal, not a pass — `None` meaning "skip the check" is how a security
    /// check becomes optional in practice.
    #[test]
    fn a_payload_with_no_proof_is_refused() {
        let secret = PairingSecret::generate();
        let node = iroh::SecretKey::generate().public();
        let payload = payload_declaring(&node.to_string());
        assert!(payload.pair_auth.is_none());
        // `verify_pairing_proof` needs a live Connection for `remote_id()`, so
        // exercise the branch it delegates to: a missing proof can never equal
        // the expected one, and the caller must not treat absence as success.
        let expected = secret.proof_for(node);
        assert!(
            payload
                .pair_auth
                .as_deref()
                .and_then(|hex| hex_decode(hex).ok())
                .map(|offered| blake3::Hash::from(offered) == expected)
                .is_none(),
            "a missing proof must not be comparable to a valid one",
        );
    }

    /// A malformed proof is a refusal, not a panic or a coerced comparison.
    #[test]
    fn a_malformed_proof_is_refused() {
        assert!(hex_decode("").is_err(), "empty");
        assert!(hex_decode("zz".repeat(32).as_str()).is_err(), "non-hex");
        assert!(hex_decode(&"ab".repeat(31)).is_err(), "too short");
        assert!(hex_decode(&"ab".repeat(33)).is_err(), "too long");
    }

    /// The malformed case that is a **panic** rather than a refusal, and the
    /// reason the ASCII check exists.
    ///
    /// `len()` counts bytes; the decode loop slices by byte offset. A string
    /// that is 64 bytes but not 64 chars puts a slice boundary inside a
    /// character, and `&s[0..2]` panics rather than erroring. `pair_auth`
    /// arrives from the wire and is decoded **before** the peer has proved
    /// anything, so this was a remote, unauthenticated way to kill the host's
    /// pairing task.
    ///
    /// Every case here is exactly 64 bytes, so the length check alone lets all
    /// of them through.
    #[test]
    fn a_multi_byte_proof_is_refused_and_never_panics() {
        for (label, s) in [
            // 1 + 2 + 61 = 64 bytes, boundary inside `é` at offset 2.
            (
                "leading ascii then 2-byte",
                format!("a{}{}", "é", "b".repeat(61)),
            ),
            // 2 + 62 = 64 bytes, boundary lands cleanly but the char is not hex.
            ("leading 2-byte", format!("{}{}", "é", "c".repeat(62))),
            // 3-byte char, boundary inside it.
            ("3-byte char", format!("ab{}{}", "€", "d".repeat(59))),
            // 4-byte char.
            ("4-byte char", format!("a{}{}", "𝄞", "e".repeat(59))),
        ] {
            assert_eq!(
                s.len(),
                64,
                "{label}: fixture must be 64 bytes to bypass the length check"
            );
            assert!(
                hex_decode(&s).is_err(),
                "{label}: must be refused, and must not panic",
            );
        }
    }

    /// A pairing secret must never reach a log line or an error report — it is a
    /// live invite for the rest of the window.
    #[test]
    fn a_secret_is_redacted_in_debug_output() {
        let secret = PairingSecret::generate();
        let rendered = format!("{secret:?}");
        assert!(rendered.contains("redacted"), "got: {rendered}");
        assert!(
            !rendered.contains(&hex_encode(&secret.0)),
            "the secret leaked into Debug output",
        );
    }

    #[test]
    fn payload_roundtrips_through_length_prefix() {
        let payload = PairingPayload {
            node_id: "abcdef".into(),
            alias: Some("iPhone".into()),
            relay_url: None,
            endpoint_addr: None,
            workspace_id: None,
            pair_auth: None,
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
            pair_auth: None,
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
            pair_auth: None,
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
            pair_auth: None,
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
            pair_auth: None,
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
            pair_auth: None,
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
            pair_auth: None,
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
