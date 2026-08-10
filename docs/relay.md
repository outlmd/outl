# Relay & NAT traversal

This page explains the one piece of the [iroh P2P transport](sync.md) that *isn't* peer-to-peer: the relay.
It's the part people get nervous about — "wait, my notes go through a server?" — so it's worth being precise about what a relay does, what it can see, what it can't, and which relay outl uses by default.

If you just want the short version:

> The relay helps two devices behind NAT find each other and, when they can't connect directly, forwards their **already-encrypted** bytes.
> It never sees your notes.
> outl defaults to its own dedicated relay at `use1-1.relay.avelino.outl.iroh.link`; the public relays operated by [n0][n0] (iroh's authors) stay as the documented fallback, and the config lets you point anywhere. (`relay.outl.app` is a friendlier alias we're bringing online — see the roadmap note below.)

---

## Why a relay exists at all

> **Why iroh (and therefore a relay) is the default transport at all:** [RFC 0038](rfcs/0038-sync-transport-and-workspace-identity.md).

Two devices on the open internet with public IPs could just open a QUIC connection and sync.
That's never the real world.

Real devices sit behind NAT — your laptop on home wifi, your phone on LTE, a machine in an office behind a corporate firewall.
None of them has a stable, dialable address.
Two peers behind two different NATs can't simply "connect to each other" because neither knows a reachable address for the other, and the NAT won't accept an unsolicited inbound packet.

This is the problem every P2P system hits, and the solution is decades old (STUN/TURN, WebRTC's ICE, Tailscale's DERP).
iroh's flavor:

1. **A rendezvous point both peers *can* reach** — the relay.
   Every endpoint, on startup, registers its "home relay" with the relay server.
   You saw this in your own log:

   ```
   INFO endpoint{id=35c8fc38bf}:relay-actor: home is now relay https://use1-1.relay.avelino.outl.iroh.link./, was None
   ```

   That endpoint just told the relay — outl's own by default — "I'm reachable here."

2. **Hole punching** — using the relay to exchange addresses and timing, the two peers fire packets at each other's NATs simultaneously, punching a path open.
   When it works (it usually does), they get a **direct** QUIC connection and the relay drops out of the data path entirely.

3. **Fallback relaying** — when hole punching fails (symmetric NAT, strict firewall), the relay forwards packets between the two peers so sync still works.
   Slower, uses the relay's bandwidth, but never fails closed.

For outl this matters because the whole pitch is *no server*.
The relay is the asterisk on that claim, so the rest of this page is about shrinking that asterisk to exactly its true size — and no bigger.

---

## What the relay can see (and what it can't)

This is the part that decides whether a relay is a privacy problem.

**The relay cannot read your notes.**
Every iroh connection is end-to-end encrypted with QUIC + TLS 1.3, keyed to the peers' identities.
The relay forwards opaque ciphertext.
It has no key, so a relay — outl's, n0's, or yours — sees encrypted bytes and nothing else.
This is true on any relay, with no extra work.

**The relay can see metadata.**
Specifically, when traffic is relayed it observes:

| Visible to the relay | Not visible to the relay |
|----------------------|--------------------------|
| The two endpoint IDs (public keys) talking through it | The content of any op, block, or page |
| Connection timing — when devices sync, how often | Which pages/blocks changed |
| Packet sizes and traffic volume | Page titles, tags, backlinks, anything semantic |
| The IP each endpoint connects *from* | The op log, the CRDT state, the `.md` |

So the honest threat model is: a relay operator can learn **that** two specific devices sync, **when**, and roughly **how much** — never **what**.

And after a successful hole punch, even that shrinks: the data goes peer-to-peer and the relay only ever saw the coordination handshake, not the sync traffic.

---

## The default: outl's dedicated relay

Out of the box, with `[sync] transport = "iroh"`, outl registers against a **dedicated** relay at `use1-1.relay.avelino.outl.iroh.link` — outl's own hostname under the `*.iroh.link` namespace, not the shared n0 public pool.
Zero setup, works immediately, and it's a named endpoint we control instead of best-effort shared infra.

Why a dedicated relay rather than the shared pool: **whoever runs the relay can observe sync metadata** — not content, metadata (see the table above).
A dedicated endpoint is the first step toward outl's reason for existing (*your data, your machines, no provider in the loop*); the roadmap below closes the rest of the gap.
n0's shared relays remain the documented fallback, not the front door.

---

## Roadmap: `relay.outl.app` on our own box

Today's default relay is **hosted on n0's relay infrastructure** (the `*.iroh.link` domain), just under an outl-scoped hostname.
That already buys a dedicated, named endpoint — but the coordination metadata still transits n0's servers.
The end state is a relay **outl runs**, fronted by the vanity `relay.outl.app` with our own TLS cert, so the metadata path leaves third-party infra entirely.

**What owning the box buys**

- **Metadata sovereignty.**
  The "who syncs with whom, and when" metadata stops transiting anyone else's servers — it goes to a box we run, logging (ideally) nothing persistent.
- **Independence + SLA.**
  A product that ships sync can't lean forever on infra it doesn't control being up, unthrottled, and free.
- **The domain is already ours.**
  `outl.app` is owned; `relay.outl.app` is a DNS record and a box away.

**What it does *not* buy**

- **Not more content privacy.**
  Content is already E2E encrypted on *any* relay (ours, n0's, or yours).
  Owning the box changes *who sees metadata*, not *whether content is private*.
- **Not full self-hosting on its own.**
  iroh has *two* services: the **relay** (NAT traversal + fallback) and **discovery** (endpoint ID → current addresses).
  A self-hosted `relay.outl.app` covers the relay; endpoint discovery is a separate, later step.
  Relay is the 80/20.

TLS is the current blocker for the vanity name: `relay.outl.app` must present a cert valid for `relay.outl.app`, and a CNAME to the n0 host serves the wrong cert (→ `NotValidForName`).
Until that's sorted (self-host + Let's Encrypt, or a Cloudflare-proxied cert), the default stays the working `*.iroh.link` hostname.

---

## Relay config

The relay URL is a config value; the iroh transport reads it and binds the long-lived sync endpoint against it:

```toml
[sync]
transport = "iroh"
relay_url = "https://use1-1.relay.avelino.outl.iroh.link"   # this is the default; empty / omitted uses it too
```

Leave `relay_url` empty (or omit it) and you get outl's dedicated relay — that's the built-in default.
Point it at any other `iroh-relay` URL to override, and that endpoint uses your relay for home registration, hole-punch coordination, and fallback.

How it threads through: `outl_sync_iroh::build_transport` (`crates/outl-sync-iroh/src/device.rs`) is the single reader of `[sync] relay_url`.
Every client (TUI, the shared Tauri backend, `outl sync`, the MCP server) calls it instead of reading the config itself.
`build_transport` passes the value to `IrohSyncTransport::new`, which hands it to the endpoint builder (`bind::n0_builder_ipv4_only`).
An empty / `None` value resolves to the built-in `DEFAULT_RELAY_URL` (`https://use1-1.relay.avelino.outl.iroh.link`); a non-empty value swaps in that `RelayMode::Custom` relay.
A malformed URL logs a warning and falls back to iroh's `presets::N0` (n0) relay rather than failing the bind, so a typo degrades gracefully instead of taking sync down.
Every endpoint (the long-lived **sync** endpoint plus the short-lived pairing / status / test endpoints) resolves the same default, so a device coordinates through one relay end to end.

iroh also supports **mixing**: an endpoint can register a relay *and* keep another as a fallback.
outl's current wiring sets a single relay (`RelayMode::Custom` with one URL — the dedicated `*.iroh.link` host by default).
Broadening that to a relay-plus-n0 list is a one-line change in `bind::n0_builder_ipv4_only` if we want it, so the default relay doesn't have to be a single point of failure.

---

## Self-hosting your own relay

The default is outl's dedicated relay, but nothing stops you from running your own — point `[sync] relay_url` at it and that device coordinates through your box instead.
This is also the path to a fully outl-owned `relay.outl.app` (see the roadmap above).
The shape of standing one up:

1. **Run `iroh-relay`** on a small always-on VPS.
   It's the relay server binary from the iroh project; one process, modest resources for op-log-sized traffic.
2. **DNS.**
   Point your relay hostname (A / AAAA) at the box.
3. **TLS.**
   The relay terminates HTTPS; use ACME / Let's Encrypt for certs so the relay URL is `https://<your-host>`.
   (Confirm the exact `iroh-relay` flags against the current iroh release — the relay server's config surface moves faster than this doc.)
4. **Config.**
   Set `relay_url = "https://<your-host>"` in `[sync]` on every device that should use it.
5. **(Later) Discovery.**
   Stand up endpoint discovery (pkarr publisher / DNS) for full independence from n0's name resolution, not just its relays.

---

## Troubleshooting: restrictive networks (VPN, TLS-inspection proxy)

Corporate / deep-inspection networks and VPNs break relay connectivity in three distinct ways.
The first is fixed in code; the other two are environmental and need a config or pairing change.

**TLS handshake fails with `UnknownIssuer` (fixed).**
A network with a custom root CA in the OS keychain (a TLS-inspection proxy) used to have every relay handshake rejected.
iroh trusted only Mozilla's bundled roots, not your OS trust store, even though macOS / `curl` / Safari accepted the same cert.
outl now delegates relay-TLS trust to the OS keychain (`rustls-platform-verifier`, wired in `bind::n0_builder_ipv4_only`), so a keychain-trusted proxy cert is accepted.
No action needed.

**Relay WebSocket upgrade returns `502` (self-host the workaround).**
Some proxies allow plain HTTPS but block or rewrite the HTTP `Upgrade: websocket` the relay needs, so the connection fails with `expected HTTP 101 Switching Protocols, got status code 502`.
iroh 1.0.0 has no non-WebSocket relay transport to fall back to, so there is no code fix.
Point `[sync] relay_url` at a relay on a domain/port the proxy doesn't intercept (see "It's already a config flip" above) — a self-hosted `iroh-relay` on your own domain sidesteps the interception.

**Direct LAN sync stalls after pairing on a VPN (`MultipathNotNegotiated`).**
If you pair two devices while one is on a VPN, the captured peer address in `peers.json` picks up the VPN's tunnel IPs (`10.x`, `100.x` CGNAT, a public WAN addr) alongside the real `192.168.x` LAN address.
outl now drops direct addresses that aren't on any local subnet before dialing, so stale tunnel IPs no longer stall the connect.
If you still hit it on an old `peers.json`, **re-pair with the VPN off** for the cleanest capture, or manually strip the non-LAN `"Ip"` entries (keep only `192.168.x` and the `Relay` entry).

---

## See also

- [Sync, done right](sync.md) — the transport layer, the op log, the CRDT, and how the iroh transport plugs into `outl-actions::SyncTransport`.
- [Privacy](privacy.md) — what leaves your device and what never does.
- [Configuration](config.md) — the full `[sync]` config surface.

[n0]: https://n0.computer
[iroh]: https://www.iroh.computer
