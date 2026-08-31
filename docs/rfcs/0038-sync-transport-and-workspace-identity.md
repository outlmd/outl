# RFC 0038 — iroh is the default transport, and a workspace is an id the joiner adopts

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#38](https://github.com/outlmd/outl/issues/38), [#133](https://github.com/outlmd/outl/issues/133), [#197](https://github.com/outlmd/outl/issues/197) (supporting: [#120](https://github.com/outlmd/outl/issues/120)) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [sync.md](../sync.md), [relay.md](../relay.md) |
| **Invariant** | root `CLAUDE.md` invariant 7 and its two transport rows in "Decisions you don't get to revisit"; `outl-sync-iroh/CLAUDE.md` → Workspace identity |
| **Guarded by** | `pairing_roundtrip`, `different_paths_same_workspace_id_sync_as_one`, `iroh_endpoint_addr_drops_stale_vpn_ipv4_keeps_lan` — full list under [How it cannot regress](#how-it-cannot-regress) |

## Why

Three reports, filed months apart by three different people, all read like "sync is broken" and none of them are about the choice of transport.

**A new user could not pair a second machine at all** ([#197](https://github.com/outlmd/outl/issues/197), outl 0.9.0-beta.141, Debian sid).
They ran the desktop app on machine A, installed the CLI on machine B in a fresh empty directory, paired with `outl peer pair --ticket …`, both sides accepted, the command exited clean.
Every sync afterwards was refused:

```
rejecting sync from peer on a different workspace local=01KYPKR... remote=01KYPQ...
```

The two devices were permanently separate graphs, and nothing in the successful pairing output said so.
The GUI joiner adopted the host's workspace id; the CLI joiner kept its own freshly minted one.
So the setup most likely to *be* a second device — a headless machine paired from a terminal — was the one path that could never converge.

**Two devices on the same WiFi could not reach each other** ([#133](https://github.com/outlmd/outl/issues/133), Mac TUI ↔ iOS, both on `192.168.1.x`).
One trace-level debug session surfaced three independent failures stacked on top of each other.
All four n0 relays plus pkarr failed TLS with `invalid peer certificate: UnknownIssuer`, because the machine had a corporate root CA that macOS, `curl` and Safari all trust and rustls' bundled Mozilla roots do not.
With TLS fixed, the relay WebSocket upgrade came back `502 Bad Gateway` from an intercepting proxy.
With the relay unavailable, the direct LAN path stalled on `MultipathNotNegotiated` and `BrokenPipe`.
The devices had been paired while on a VPN, so `peers.json` had captured **seven** candidate addresses, of which exactly one was reachable from the current network.

**An iPhone synced the future and not the past** ([#120](https://github.com/outlmd/outl/issues/120), outl 0.7.0-beta.90).
Pairing a phone into an existing Mac workspace delivered every note written *after* pairing and none written before.

The common subject is not QUIC versus iCloud.
It is: what is a workspace, how does a second device prove it belongs to one, and which of a peer's advertised addresses are worth dialing.

## What we chose

**Transport is a config key over one trait, and iroh is the default.**
`[sync] transport` in `outl-config` (`SyncTransportKind`) is `"iroh"` unless the user writes `transport = "file"`.
Both sit behind `outl_actions::SyncTransport` (`crates/outl-actions/src/sync.rs:147`), whose entire contract is that `ops-<peer>.jsonl` files land on disk — `SyncEngine::reload_workspace` then cannot tell which transport delivered the bytes.
That thinness is what lets the default flip without touching the CRDT.

**Workspace identity is a `WorkspaceId` in its own file, `<root>/.outl/workspace-id`.**
Not a `config.toml` field, because `config.toml` holds the per-**device** `actor_id` while the workspace id must be **identical** across devices; keeping them apart lets pairing rewrite the id without touching the actor.
The gossip topic is `blake3(workspace_id)`, so two devices on the same workspace land on the same topic regardless of where the folder lives on disk.
`SyncProtocolHandler::serve` validates the incoming `SyncRequest.workspace_id` against the local one and closes `workspace-mismatch` otherwise.

**`PairingHub::adopt_workspace_id` is the single owner of "the joiner takes the host's id"** (`crates/outl-sync-iroh/src/engine_pairing.rs:159`).
The host keeps its id, the joiner adopts it, and adoption happens *before* the immediate post-pair `delta_sync` fires.
Adoption is **persist-first**: the host's id is written to `<root>/.outl/workspace-id`, then the in-memory `SharedWorkspaceId` handle flips, so a failed write does not adopt and the operation is retry-safe.
Adoption also fires a `broadcast::Sender<WorkspaceId>`, which is what fixes the second half of #197.
The gossip supervisor drops its old-topic subscription and re-subscribes to `blake3(new id)` without a restart.
A second receiver on the same channel makes the catch-up loop clear its per-session dedup and re-dial every peer under the adopted id.
The CLI joiner (`join_pairing`) goes through the same owner and returns a `WorkspaceAdoption` the command prints, so the user can see that it worked.

**`bind::n0_builder_ipv4_only` is the single owner of every endpoint in the crate**, and `PeerEntry::iroh_endpoint_addr` the single owner of which addresses get dialed.
The builder passes `CaTlsConfig::system()` explicitly, so relay TLS is validated against the OS keychain like every other client on the machine.
Enabling iroh's `platform-verifier` feature alone is not enough: without the explicit call the default stays `EmbeddedWebPki`.
Address resolution keeps the relay plus **on-LAN IPv4** direct addresses and drops IPv6 and off-LAN IPv4 (`is_reachable_lan_ipv4` over `if-addrs`), which is what turns #133's seven candidates back into one working path.
`peers::refresh_peer_direct_addr` then self-heals a stale entry: on an accepted inbound connection, the live remote socket read off `Connection::paths()` replaces the stored address, so a peer whose DHCP lease moved does not need a re-pair.

#120 is the same identity story seen from the op log.
With both devices validated as one workspace, catch-up ships a peer snapshot on pair, and the `ActorClock` gap detection falls back to a full-actor resend when ops sit below the receiver's watermark.

## Why not the alternatives

**Path as workspace identity.**
The obvious cheap answer, and the one the pre-#197 shape leaned toward.
It costs the actual use case: the same workspace lives at `/Users/x/notes` on the Mac and inside an app container on iOS, so identical graphs would compute different gossip topics and never meet, while moving a folder would silently orphan a device.

**Workspace id inside `config.toml`.**
One less file, and it collides head-on with `actor_id` living in the same file: pairing would have to rewrite a file whose other field must stay per-device forever.
The bug that produces is a joiner that adopts the host's actor id and starts appending to the host's op-log file.

**Keep iCloud as the default and leave iroh opt-in.**
This was #38's own stated non-goal ("replace iCloud immediately"), and it held for a while.
It costs every non-Apple user: the file transport is last-write-wins at the OS layer, macOS and iOS only, with opaque cadence and `.icloud` ghost files.
Defaulting to iroh and keeping `transport = "file"` as a supported opt-out is the same set of capabilities with the platform lock moved off the default path.

**`iroh-docs` instead of `iroh::Endpoint`.**
iroh ships a multi-writer KV store, and using it would mean running a second CRDT underneath the tree CRDT this project is built on.
Strictly worse: two convergence models to reason about, and the Kleppmann move-op guarantees stop being the only thing that decides the tree.

**libp2p, Hypercore/Holepunch, the Automerge sync protocol, raw `quinn`.**
libp2p is DHT-heavy and shaped for IPFS-adjacent workloads, and Hypercore is JS-first with Rust as a secondary path.
Automerge's sync protocol solves sync for Automerge documents, which outl does not have.
Raw `quinn` means writing discovery and hole punching by hand.
`iroh::Endpoint` does QUIC plus discovery plus hole punching and stops, which is exactly the size of the hole.

**Dial only the relay, or dial every advertised address.**
Relay-only is simple and breaks same-WiFi sync the moment the relay is slow or blocked — which is #133 issue 2 in practice.
All-addresses is what iroh 1.0.0 does by default, and a single dead IPv6 or VPN address stalls multipath for ~30s instead of converging on the working path.
On-LAN IPv4 plus relay is the balance, and it is a filter rather than a preference because iroh 1.0.0 exposes no knob to rank paths.

**Downgrade iroh to escape the multipath stall.**
Blocked, not rejected on taste: `iroh-gossip 0.101.0` requires `iroh = "1"`.
The IPv4-only bind is documented in `bind.rs` as a stopgap with an explicit revert condition.

**Leave relay TLS on the bundled Mozilla roots.**
Cheapest possible non-fix.
It makes outl the only app on a TLS-inspected network that cannot reach its own relay, and the failure surfaces as `UnknownIssuer` deep in a trace log rather than as anything a user can act on.

## The opposite direction

**The mirror of "the joiner adopts the host's id" is a merge nobody asked for.**
#197 is the case where adoption did *not* happen and two graphs stayed apart.
The mirrored case is a joiner that already had content: adoption keeps the joiner's ops, per-actor files land on both sides, and the two graphs **union**.
The crate's `CLAUDE.md` calls this expected, and it is — but there is no un-merge.
A user who pairs the wrong folder gets both graphs interleaved and re-pairing does not undo it, because the ops are already in the log on both devices.

**Pairing is not symmetric, and the CLI does not say so up front.**
Whoever runs `outl peer pair` without `--ticket` keeps their identity; whoever passes `--ticket` gives theirs up.
The joiner learns this *after the fact* from the printed `WorkspaceAdoption`.
Nothing warns the device with the content that it is about to become the joiner.

**Making the LAN path work made the remote path relay-only.**
Dropping IPv6 and off-LAN IPv4 is what fixed same-WiFi sync; the cost is that a genuinely remote peer on a routable address is now reachable *only* through the relay.
On a network where the relay WebSocket upgrade returns 502 — #133 issue 2, still unfixed — that is no path at all, and the user sees an empty peer list with no explanation.
`[sync] relay_url` is the escape hatch and it requires operating a relay.

**`is_reachable_lan_ipv4` fails open on purpose.**
If interface enumeration errors, every stored address is kept and dialed.
That is the right default (a filter that cannot see the network should not block sync) and it means a machine where `if-addrs` fails re-enters the exact stall #133 reported, silently.

**The workspace id is a shared-name file, which invariant 7 says to model as an `Op`.**
Stated plainly because it is the one deliberate exception in this RFC.
`.outl/workspace-id` has last-write-wins semantics and bypasses the op log entirely.
It is exempt because it is the *precondition* for exchanging ops rather than state that converges through them — two devices must already agree on identity before a single op can cross.
The cost is real: two concurrent adoptions on one device race on that file, and the loser sits on the wrong gossip topic until something re-reads it.
Not observed in the wild, and not guarded by a test.

**Persist-first adoption trades one silent failure for another.**
A failed write means no adoption, which is retry-safe and correct.
It also means a workspace whose `.outl/` is read-only pairs successfully and never adopts, and the user is back to reading `workspace-mismatch` from a log line.

## How it cannot regress

1. **Invariants.**
   The root `CLAUDE.md` "Decisions you don't get to revisit" table carries both transport rows: iroh as the default, `transport = "file"` as the explicit opt-out.
   The same table's "one `ops-<actor>.jsonl` per device, never shared" row is what makes the `SyncTransport` contract meaningful.
   Invariant 7 states the op-log rule that the workspace id is the documented exception to.
   `outl-sync-iroh/CLAUDE.md` → Workspace identity carries adoption, persist-first ordering, the broadcast re-subscribe and the address-resolution order.
   `outl-cli/CLAUDE.md:134` carries the CLI adoption rule where a contributor editing `outl peer pair` will read it.

2. **Tests.**
   - `pairing_roundtrip` and `gui_pairing_over_live_sync_endpoint` (`crates/outl-sync-iroh/tests/integration.rs`) pin adoption for the CLI and the GUI path.
     `pairing_roundtrip` asserts both sides end up on one on-disk id and the call returns `Adopted`, which is #197 exactly.
   - `same_workspace_id_yields_same_topic_across_paths` and `delta_sync_rejects_mismatched_workspace_id` (`crates/outl-sync-iroh/tests/integration.rs`) pin the two halves of identity.
     Same id at different paths is one workspace; different ids never merge.
   - `different_paths_same_workspace_id_sync_as_one` (`crates/outl-sync-iroh/tests/regression.rs`) is the end-to-end version — two devices at different paths converge.
   - Six `iroh_endpoint_addr_*` tests plus `is_reachable_lan_ipv4_matches_only_local_subnets` (`crates/outl-sync-iroh/src/peers.rs`) pin #133 issue 3.
     They cover keeping relay + on-LAN IPv4, dropping IPv6, dropping stale VPN IPv4, and the three fallbacks that must survive a missing relay, a bare id and a corrupt stored address.
   - `refresh_peer_direct_addr_replaces_stale_keeps_relay_and_is_idempotent` (`crates/outl-sync-iroh/src/peers.rs`) pins the inbound self-heal.
   - `empty_config_defaults_to_iroh_transport` and `sync_relay_url_is_returned_when_set` (`crates/outl-config/src/schema.rs`) pin the default and the relay override.
   - `snapshot_transfers_from_peer_on_pair`, `backlog_below_watermark_crosses_after_gap_detected` and `full_actor_resend_converges_and_dedups` (`crates/outl-sync-iroh/tests/regression.rs`) pin #120.
     History has to reach a device that joined late, and reach it exactly once.

   **Two gaps, named rather than papered over.**
   The relay selection around it is pinned: `an_empty_or_absent_relay_url_falls_back_to_outls_own` and `a_configured_relay_url_wins_and_a_typo_degrades_rather_than_failing` (`crates/outl-sync-iroh/src/bind.rs`) cover `None` / empty / whitespace resolving to outl's own relay rather than the n0 preset, and a malformed url degrading instead of failing the bind.
   `the_bind_address_is_ipv4_only` in the same module pins the IPv4-only STOPGAP, whose removal reads as "sync got slow" rather than as a config change.

   `CaTlsConfig::system()` itself still has **no test — none found, gap**: iroh exposes no way to read the choice back off a `Builder`, and reproducing the failure needs a custom root CA in the OS trust store.
   What guards it instead is the compiler — `CaTlsConfig::system()` only exists with iroh's `platform-verifier` feature, so dropping the feature fails the build rather than silently reverting to the bundled Mozilla roots.
   Removing the `.ca_tls_config(...)` call while keeping the feature would still pass: **that specific regression is unguarded**.

   The relay WebSocket 502 path has **no test — none found, gap**, because there is no fix to guard.

## Scope

**Not covered — the relay WebSocket upgrade (#133 issue 2).**
An intercepting proxy that allows HTTPS but rewrites `Upgrade: websocket` still returns 502 and still kills relay connectivity.
The only mitigation today is pointing `[sync] relay_url` at a relay the proxy leaves alone.
Open on [#133](https://github.com/outlmd/outl/issues/133).

**Not covered — peer trust, pairing authentication, and revocation.**
Whether the connecting identity is on the approved list, and what `peer remove` actually revokes, is [RFC 0155](0155-peer-trust.md) and issues [#158](https://github.com/outlmd/outl/issues/158) / [#159](https://github.com/outlmd/outl/issues/159).

**Not covered — op signing.**
Ops are unsigned; a paired device can claim another actor's id.
#38's open question 5, unresolved.

**Not covered — multi-user shared workspaces.**
Everything here assumes "your devices, your workspace".

**Not covered — the IPv4-only bind.**
It is a stopgap for iroh 1.0.0's multipath stall, with its revert condition written into `crates/outl-sync-iroh/src/bind.rs`.
Dropping it is a behaviour change that needs its own RFC.

**Not covered — background delivery on iOS.**
iOS suspends sockets on backgrounding, so P2P runs in `BGAppRefreshTask` / `BGProcessingTask` windows.
See [`docs/sync.md`](../sync.md) → Background sync on iOS.
