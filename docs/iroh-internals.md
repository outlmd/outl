# iroh transport internals

Contributor-facing internals of `outl-sync-iroh`, the crate behind the default `iroh` transport.
Two things live here, both extracted from `crates/outl-sync-iroh/CLAUDE.md` so the crate file can stay focused on invariants:
the **pinned iroh 1.0.0 API surface** (the calls that differ from every tutorial you will find) and the **named test catalog** (regression + chaos).

The architecture, the load-bearing invariants ("one endpoint per identity", the append-serialization rule, workspace identity), and the module layout stay in [`crates/outl-sync-iroh/CLAUDE.md`](../crates/outl-sync-iroh/CLAUDE.md).
The user-facing sync story is [sync.md](sync.md); the relay threat model is [relay.md](relay.md).

---

## iroh 1.0.0 API notes (load-bearing)

The 1.0.0 surface differs from older tutorials — pin these:

- `iroh::SecretKey`, not `iroh::key::SecretKey`. `SecretKey::from_bytes(&[u8;32])`,
  `to_bytes() -> [u8;32]`, `public() -> PublicKey`, `generate(&mut rand::rng())`.
- `EndpointId = PublicKey` is the node identifier type. `iroh::PublicKey` is the
  concrete struct; parse from string with `.parse()`.
- `Endpoint::builder(presets::N0)` — the builder takes a discovery preset arg.
  `presets` lives at `iroh::endpoint::presets`.
- `ProtocolHandler::accept(&self, conn: Connection) -> Result<(), AcceptError>`
  is a **native async fn** and receives an already-accepted `Connection`
  (not `Connecting`, no manually-boxed future).
- `endpoint.connect(id, b"alpn")` takes the ALPN bytes directly.
- `SendStream::finish()` is sync and returns `Result`; `write_all`/`read_to_end`
  are async.
- Gossip: `gossip.subscribe(topic, peers).await?` returns a topic handle;
  `.split()` yields `(GossipSender, GossipReceiver)`.
  Events are `iroh_gossip::api::Event::Received(message)` with
  `message.content` and `message.delivered_from`.
  `StreamExt` comes from `n0_future`.
- `GossipSender::broadcast(&self, msg: bytes::Bytes) -> Result<(), ApiError>`
  takes `bytes::Bytes` and `&self` (not `&mut`), so the sender can live in a
  drain task that `announce_local_ops` feeds via an `mpsc` channel.
- **No `NodeTicket` type ships in iroh 1.0.0 / iroh-base 1.0.0.**
  The pairing "ticket" is a base64 of `serde_json(EndpointAddr)`.
  `EndpointAddr` is `Serialize`/`Deserialize` with public `id: EndpointId`
  and `addrs: BTreeSet<TransportAddr>`; `endpoint.addr()` returns the current
  one, and `endpoint.connect(addr, alpn)` takes `impl Into<EndpointAddr>`, so
  the decoded value feeds straight back into `connect`.
- Accept loop (host side): `endpoint.accept().await -> Option<Incoming>`,
  then `incoming.accept()? -> Accepting`, then `.await -> Connection`.
  `Connection::accept_bi()` / `open_bi()` / `close(VarInt, &[u8])` as usual.

---

## Module layout

Every split below was forced by the file-size guard, but each one landed on a seam that was already there.

| File | Owns | Why it is not in `engine.rs` |
|---|---|---|
| `engine_sync.rs` | The delta-sync wire protocol: `delta_sync` (initiator), `SyncProtocolHandler` (responder), the framing helpers (`read_frame` + typed `read_*`), and the op-log read/write helpers (`local_vector_clock`, `ops_missing_for`, `ingest_received_ops` — which owns the `AppendLock` write path). | "On the wire" and "stand it up" are different jobs, and only one of them changes when the protocol version moves. |
| `engine.rs` | Boot orchestration: the `IrohSyncTransport` struct + channel wiring, `run_iroh`, the boot/catch-up/gossip task spawns, the router setup. | — |
| `coordination.rs` | The handles concurrent tasks meet on: `AppendLock`, `InFlightPeers` (+ `InFlightGuard`, `try_acquire_in_flight`), `InboundServes`, `SharedWorkspaceId`. | Four dial paths and the inbound `serve` all reach for these, and none of them care how `run_iroh` works. |
| `oplog.rs` | What the wire reads from and writes to disk: `local_vector_clock`, `ops_missing_for`, `ingest_received_ops`, and the append-serialization invariant. | Those durability rules hold regardless of which wire version calls them, so they are not protocol code. |
| `peer_conn.rs` | `PeerConnections` — one live QUIC connection per peer, reused across syncs, invalidated on failure. | Only possible once the durable-ingest ack moved off the close code (v3); before that a connection could not outlive one exchange. |
| `protocol.rs` | What the bytes mean: ALPNs, encode/decode, the close codes, and `classify_close` / `CloseVerdict`. | A misclassified close is invisible at runtime (both non-success verdicts return the same error and re-push), so the decision table has to be a value a test can enumerate, not a `match` reachable only over real QUIC. |

`engine.rs` re-exports `delta_sync`, `SyncProtocolHandler` and the `coordination` types, so `crate::engine::delta_sync` and `crate::engine::AppendLock` keep resolving for `engine_catchup`, `engine_gossip`, `engine_pairing` and `test_support`.

## Regression suite (Pilar 2)

Every bug hand-found during the sync saga has a NAMED, permanent test — the name IS the bug, so a failure is self-explanatory.
Pure guards live in `#[cfg(test)]` next to the code.
Over-the-wire (real QUIC, loopback) guards live in four suites: `tests/regression.rs` (one row per saga bug), plus three organised by question instead — `tests/revocation.rs` (*may this dialer be served?*), `tests/membership_trust.rs` (*how does a node id get into `peers.json`?*) and `tests/oplog_guards.rs` (*what may an authorized peer's bytes do to `ops/`?*).
The three question-suites keep **deny cases outnumbering allow cases on purpose**, and their allows are load-bearing: a check that refuses everything passes every deny test ever written.
Shared seed/read/wait helpers stay in `tests/common/mod.rs` (read-only); saga-specific helpers live inside `regression.rs`.

| Saga bug | Guard test | Where |
|----------|-----------|-------|
| 1. Op-log corruption from concurrent appends (glued `…}}}{`) — append lock serializes inbound batches | `concurrent_appends_never_glue_ops_on_the_responder` (asserts no `}{` on disk + every op parses) | `tests/regression.rs` |
| 1. (parser-recovery half) a hand-crafted glued `}}}{` line still loads both ops | `recovers_glued_ops_on_one_line` (pre-existing, core-side) | `outl-core` `storage/jsonl.rs` |
| 2. HLC far-future op skipped on ingest (±24h gate) | `far_future_hlc_op_is_skipped_on_ingest` (B sends a ~48h-ahead op + a valid op; only the valid one lands on A) | `tests/regression.rs` |
| 3. Workspace identity = stable id, not path (topic) | `same_workspace_id_yields_same_topic_across_paths` (pre-existing) | `tests/integration.rs` |
| 3. Workspace identity = stable id, not path (END-TO-END sync) | `different_paths_same_workspace_id_sync_as_one` (two devices at different paths, same id, converge) | `tests/regression.rs` |
| 3. Mismatched ids are rejected | `delta_sync_rejects_mismatched_workspace_id` (pre-existing) | `tests/integration.rs` |
| 3b. Removed/unknown peer denied (issue #158) | `removed_peer_is_denied_sync` | `tests/regression.rs` |
| 3c. Revocation reaches every protocol on the endpoint, not just `SYNC_ALPN` — snapshot and asset shipped the whole graph and every upload to any dialer (denies: revoked, stranger, malformed list, absent list, asset manifest; allows: an approved peer still pulls both) | `a_revoked_peer_gets_no_snapshot` / `an_unknown_dialer_gets_no_snapshot` / `a_malformed_peer_list_denies_the_snapshot` / `an_absent_peer_list_denies_the_snapshot` / `a_revoked_peer_gets_no_assets` / `an_unknown_dialer_gets_no_asset_manifest` / `an_approved_peer_still_pulls_the_snapshot` / `an_approved_peer_still_pulls_assets` | `tests/revocation.rs` |
| 3d. Asset authorization is per REQUEST, not per connection — one connection serves unbounded blobs at the initiator's pace, so a connect-time-only check let a peer keep draining `assets/` after `outl peer remove` | `a_peer_revoked_mid_connection_gets_no_further_assets` + allow `an_approved_peer_gets_every_asset_on_one_connection` | `tests/revocation.rs` |
| 3e. A forged snapshot body with an empty `cutoff` skips `outl-core`'s adoption guard, so the transport must never cache it | `a_snapshot_with_no_cutoff_is_never_cached` | `tests/revocation.rs` |
| 3f. One verdict, one scan — the blocking and async authorization forms must agree case for case, including "the peer list is unreadable" ≠ "that device was revoked" | `the_blocking_verdict_matches_the_async_one_case_for_case` / `both_refusals_look_identical_on_the_wire` | `src/authz.rs` `#[cfg(test)]` |
| 4. Pairing adoption — joiner adopts host id (GUI + CLI, #197) | `gui_pairing_over_live_sync_endpoint` + `pairing_roundtrip` (CLI: shared on-disk id, returns `Adopted`) | `tests/integration.rs` |
| 5. Single endpoint per identity (pair AND sync over the live sync endpoint, no relay hijack) | `gui_pairing_over_live_sync_endpoint` (pre-existing; pairing rides the live sync endpoint, no second bind) | `tests/integration.rs` |
| 5b. Endpoint lease — one process binds, the loser is told to stay off the wire, not silently offline (issue #220) | `one_process_binds_the_device_endpoint_and_the_next_one_is_told_to_stay_off_the_wire` | `tests/endpoint_lease.rs` |
| 5c. A lease file that cannot be opened denies the endpoint (no arbiter must not mean everyone binds) | `a_lease_file_that_cannot_be_opened_denies_the_endpoint_instead_of_granting_it` | `src/lease.rs` |
| 5d. The status probe stands down instead of stealing the route it was run to diagnose | `the_probe_stands_down_when_another_process_holds_the_endpoint` | `src/status.rs` |
| 6. Reachability resolution + off-LAN/IPv6 direct-addr filter (issue #133) | `iroh_endpoint_addr_*` + `is_reachable_lan_ipv4_*` (keep on-LAN IPv4, drop IPv6 + stale VPN IPs, fall back stored/bare/corrupt) | `src/peers.rs` |
| 7. Bidirectional push materializes on BOTH sides AND fires BOTH reload signals | `bidirectional_sync_fires_reload_signal_on_both_sides` (set convergence + `peer_ready_tx` on initiator AND responder) | `tests/regression.rs` |
| 7. (set-convergence half) both sides hold all ops | `bidirectional_delta_sync` (pre-existing) | `tests/integration.rs` |
| 8. Membership merge is ADD-only (never clobber a local entry, drop self, drop undialable) | `merge_unknown_never_clobbers_a_known_entry` + `merge_skips_self` / `merge_adds_unknown_and_dedups_known` / `merge_skips_unreachable_peer` | `src/peers.rs`, `src/engine_membership.rs` `#[cfg(test)]` |
| 8b. Membership gossip is unauthenticated, so it can MANUFACTURE an authorization (OPEN protocol hole, pinned not closed); the bounded merge is the half that IS closed | `gossip_can_manufacture_an_authorized_peer` (`#[ignore]`d — read its body before deleting it) / `an_oversized_membership_broadcast_is_refused_whole` / `a_membership_list_at_the_cap_still_merges` | `tests/membership_trust.rs` |
| 9. Watermark gap — ops below a receiver's max-HLC stayed permanently invisible after out-of-order ingest; the v2 `ActorClock` count detects the gap, the full-log fallback + ingest dedup converge without duplicating | `backlog_below_watermark_crosses_after_gap_detected` / `ingest_dedups_already_present_ops` / `full_actor_resend_converges_and_dedups` | `tests/regression.rs` |
| 10. Snapshot sync — peer snapshot on pair (byte-identical, reload fired); absent harmless | `snapshot_transfers_from_peer_on_pair` / `snapshot_pull_absent_is_harmless` | `tests/regression.rs` |
| 11. Asset sync — peer asset on pull (byte-identical, held file skipped); absent harmless | `assets_transfer_from_peer` / `asset_pull_from_peer_without_assets_is_harmless` | `tests/regression.rs` |
| 12. Forced-pass completion is per REQUEST, not global — a waiter on sequence `n` must not stop when somebody else's pass lands (the iOS background flush released its OS window on the foreground timer's 3s pass and let the device suspend mid-exchange) | `every_queued_request_advances_the_counter_by_exactly_one` | `src/engine_catchup.rs` `#[cfg(test)]` |
| 13. Op-log ingest guards — batch bucketing on the untrusted `op.actor` (a peer must not write OUR actor's file), torn-tail heal on the ingest's own append path, and the cross-process `ops/.append.lock` that had zero coverage | `an_op_carrying_our_own_actor_id_is_refused_on_ingest` / `a_forged_op_cannot_pre_claim_our_hlc_slot` / `a_torn_tail_never_glues_an_incoming_op_onto_a_fragment` / `the_ingest_waits_for_the_cross_process_append_flock` / `ops_from_other_actors_still_land` | `tests/oplog_guards.rs` |

Names map 1:1 to the saga checklist; do NOT delete one without deleting the bug it guards.

### Chaos/concurrency tests (Pilar 3)

The regression suite pins one named bug per row; the **chaos battery** (`tests/chaos.rs`) instead *hammers* the same wire code (real `delta_sync` + `SyncProtocolHandler` via `test_support`) with STRESS loads (N writers × M ops).
Every test runs over real QUIC on loopback under `#[tokio::test(flavor = "multi_thread")]`.

| Failure mode | Chaos test | Asserts |
|--------------|-----------|---------|
| Concurrent writers gluing the op log | `concurrent_writers_never_corrupt_op_log` | 8 initiators push one actor's ops at one responder under the shared `AppendLock`; every line is one JSON value + exact union |
| Reordered + duplicated delivery | `reordered_and_duplicated_delivery_converges` | 3 nodes, seeded-shuffled passes, ~half twice; all converge |
| Partition + heal under load | `partition_then_heal_under_load` | B offline while A/C edit; B rejoins and converges, no glued line |
| Fan-out + redundant dials | `fan_out_to_many_peers_converges_without_double_dial` | 5 peers dial one hub, each dial twice; exact union, every op once |
| Single-endpoint invariant under concurrency | `concurrent_inbound_dials_on_single_endpoint_stay_clean` | 6 inbound dials on one hub endpoint while it dials out; converges both ways, no corruption |

**Determinism** (a flaky chaos test is worse than none): randomness is a seeded xorshift (`chaos_helpers::Rng`); the only true nondeterminism is network timing, so every wait uses `common::STEP_TIMEOUT` + `wait_until`, never a fixed sleep.
Sizes are bounded (≤ 64 ops, ≤ 8 tasks).

**Why raw bytes, not `all_ops`:** `JsonlStorage::reload` recovers glued `…}}}{…` lines on read, so `all_ops` would MASK an append-lock failure.
`chaos_helpers::assert_every_line_is_one_json_value` reads the `ops-<actor>.jsonl` bytes directly — the only thing that reveals whether the lock held.

**Helpers** live in `tests/chaos_helpers/mod.rs`, not `tests/common/mod.rs` (clippy `duplicate_mod` allows one `common` loader per test binary).

---

## STOPGAP: IPv4-only bind (iroh 1.0.0 multipath workaround)

**All four endpoints bind IPv4-only** through `bind::n0_builder_ipv4_only` — `run_iroh`, `bind_pairing_endpoint`, `probe_peers`, `bind_sync_endpoint`.
The `bind` module owns the bug, the fix and the revert condition; dial and accept must both go through it, because dropping IPv6 on one side only lets the other advertise a dead path.

**It narrows the bug, it does not close it.**
Multipath opens paths to **all** of a peer's candidate addrs at once and one unreachable addr stalls the whole connect/accept (`MultipathNotNegotiated`, ~30s) rather than converging on a working path.
Binding IPv4-only removes the usual offender (a global IPv6 addr that is "No route to host") but an unreachable **IPv4** addr stalls it identically — a VM bridge or VPN `utun` addr in our own ticket, or a peer's stale DHCP lease in theirs.
Signature: `sendmsg error: … HostUnreachable` / `Host is down` toward one addr, then a connect timeout with the relay up the whole time.

### Configurable relay (default: outl's own)

`n0_builder_ipv4_only(relay_url: Option<&str>)` picks the relay on top of the IPv4-only STOPGAP.
Default is outl's own dedicated relay, `DEFAULT_RELAY_URL` (`use1-1.relay.avelino.outl.iroh.link`, via `RelayMode::custom`) — the n0 public relay proved slow/unreachable on some networks.
A non-empty `[sync] relay_url` overrides it; a parse error falls back to `presets::N0` with a warning.
Pairing / status / test pass `None`, which is **not** "the n0 preset" — `None` resolves to `DEFAULT_RELAY_URL` too, so every endpoint rides outl's relay by default.
Only the long-lived **sync** endpoint threads the *configured* one (`run_iroh` ← `IrohSyncTransport::new` ← `outl_config::load().sync.relay_url()`), so only a deployment that overrides `[sync] relay_url` sees a split.
See `docs/relay.md` / `docs/config.md`.

**Revert condition:** delete the `bind` module once iroh > 1.0.0 ships the multipath fallback fix, and let every call site go back to the plain dual-stack `Endpoint::builder(presets::N0)` builder (details in the module docs).

## One endpoint per identity, elected not assigned

The detail behind `crates/outl-sync-iroh/CLAUDE.md` → "One endpoint per identity, elected not assigned", which keeps the rule; this is the mechanism, the failure modes, and what the lease refuses.

**A device binds at most ONE iroh endpoint at a time, and which process gets it is decided by a lease, not by what kind of client it is.**

**Why the route is single:** a second endpoint registering the same secret key *replaces* the active client in the relay's `DashMap<EndpointId, ClientState>`.
All inbound datagrams then route to the newcomer and the original silently stops receiving (`endpoint.rs::same_endpoint_id_relay` asserts this).
The demoted endpoint's *outbound* catch-up stalls too for any relay-only peer, because that peer's QUIC return traffic is addressed to the node id and lands on whoever is ACTIVE.
So a second endpoint breaks the first's sync in **both** directions.
This is not the "stable, benign hijack" an earlier version of this document claimed; believing it is what let `outl mcp serve` silently kill the desktop's sync to an off-LAN iPhone.
If the newcomer doesn't accept `SYNC_ALPN` at all, the dialer additionally gets `quinn` `CONNECTION_REFUSED` — the older "connection refused, nothing syncs" bug (a transient status-probe or the GUI's old pairing endpoint stealing the route).

**The lease (`lease::EndpointLease`).**
An advisory `flock` on `endpoint.lock`, a **sibling of the identity key**, so the arbitration scope follows the node id automatically (desktop / TUI / CLI / MCP share `~/.outl/`; mobile's sandbox identity never contends).
`build_transport` (`device.rs`) is the one place that takes it, so no client has to remember to.
Released by the kernel when the holder exits: no TTL, no stale lease, no daemon.

**The endpoint thread owns the lease, and the two ways of getting that wrong are opposite.**
`start()` moves it into the `outl-iroh-sync` thread, where it is bound first and therefore dropped last, so the claim ends exactly when `run_iroh` returns.
Leave it on the struct instead and a failed `.bind()` kills the thread while the transport keeps the claim, locking every other process on the device out of an endpoint forever — issue #220 again, this time with a padlock.
Drop it any earlier and you reopen the reverse.
`shutdown()` only sends a oneshot.
A client that drops the transport right after (the desktop, on a workspace swap) would free the lease while `run_iroh` is still closing the endpoint, and a second endpoint could bind onto the same node id in that window.
A transport that was built but never started still holds it, which is what `outl sync` needs when it exits early.

Two failure modes at acquire time, deliberately opposite (`lease.rs`).
The lease file failing to **open** (permission, read-only mount) is **fail-closed**: there is no arbiter and no way to know whether someone is already bound, so granting would grant to everyone.
The file opening but refusing to **lock** (`ENOLCK`, a mount with no locking) is **fail-open** with a warning.
The file is ours, only the locking is missing, and refusing everyone leaves the device with no endpoint at all, which is the failure the lease exists to remove.

**A refusal says which one it is.**
`try_acquire` returns `Result<EndpointLease, LeaseDenied>`, and `LeaseDenied` is `HeldByAnotherProcess` or `LeaseFileUnusable { path, error }`; `TransportOutcome::EndpointBusy` and `PeerProbe::EndpointBusy` carry it through to the client.
The degradation is identical either way (file transport, stay off the wire), so a caller that only degrades ignores the payload.
Every caller that words this for a **human** must read it.
"Another outl process holds the endpoint" sends a user whose `~/.outl` is read-only hunting for an `outl mcp serve` that is not running, and no process exiting will ever free a lease nobody took.

**Why a lease and not a policy.**
The rule used to be "only the GUI binds; the MCP server and the CLI are passive writers".
That kept two endpoints apart, but it assumed a GUI exists.
On a headless machine (an agent driving `outl mcp serve`) *nobody* bound an endpoint, so the device's ops never left and no peer's ops ever arrived — silently, with `outl peer status` on the other device just showing "offline" (issue #220).
The constraint was never "only the GUI"; it is "one live endpoint per identity", and that is a question about **who got here first**, which only a lock can answer.
Losing the election is a working state, not a failure: the loser runs `outl_actions::FileSyncTransport` and converges through the shared `ops/` dir, which the holder pushes out on its `MAINTENANCE_RESYNC` pass.

**Known limitation: the lease is per device, so it is also per *workspace holder*.**
The lock is a sibling of `identity.key`, not of the workspace, because the thing being arbitrated is the node id and there is exactly one of those per device.
A process holding the endpoint for workspace A therefore keeps a process on workspace B off the wire.
B's ops only leave the machine when a process that *does* hold the endpoint opens B — the shared `ops/` fallback converges B across local processes, not across devices.
Scoping the lease per workspace would not fix this; it would let two endpoints bind the same node id, which is the exact failure this section exists to prevent.
The real fix is one endpoint multiplexing every open workspace (the sync protocol already carries `WorkspaceId` per request), and that is a redesign of `engine::run_iroh`, not a change to the lease.
Until then, a user running two workspaces at once P2P-syncs the one whose process got there first.

Pinned by `tests/endpoint_lease.rs` (`one_process_binds_the_device_endpoint_and_the_next_one_is_told_to_stay_off_the_wire`) plus the unit tests in `lease.rs`.

**Non-sync endpoints are the sharper case, and they take the lease too.**
An endpoint that does NOT serve `SYNC_ALPN` is worse than a competing one: a dialer routed to it gets `CONNECTION_REFUSED` instead of a working peer.
The status probe (`status::probe_peers`) is the only one left, and it now asks for the lease like everything else, returning `PeerProbe::EndpointBusy` instead of binding when it loses.
It used to be exempt on the grounds that "the CLI has no running transport to conflict with".
That stopped being true the moment `outl mcp serve` could hold the endpoint, and `outl peer status` is precisely the command a user runs to diagnose sync, so it must not be the thing that breaks it.

**Three call sites, three rules:**

1. **Sync endpoint (`engine::run_iroh`)** — the one allowed long-lived endpoint.
   Router accepts `SYNC_ALPN` + gossip ALPN **+ `PAIRING_ALPN`** (rule 3) **+ `SNAPSHOT_ALPN`** (see "Phase-2 blob transfer"), all advertised in its `.alpns()`.
   All catch-up / boot / gossip / pairing dials go out through *this* endpoint (the one bound in `run_iroh`); no helper spins up a second.
2. **Status (`status::probe_peers`)** — binds a transient endpoint, and **only if it wins the lease**.
   It returns `PeerProbe::EndpointBusy` rather than binding when it loses, so it can never demote the transport it was run to inspect.
   A client that has its own running transport reads `peer_health()` instead and never calls this at all.
3. **Pairing** — the split is about **holding an endpoint**, not about being a GUI:
   - **Transport running** → `IrohSyncTransport::pair_host` / `pair_join` reuse the **live sync endpoint**.
     The host (accept) side is the `PAIRING_ALPN` router handler (`engine_pairing::PairingProtocolHandler`), armed by `pair_host` via a shared `PairingHub`; the join side dials out on the same endpoint.
     After a successful pair the new peer is persisted to `peers.json` and an **immediate** `delta_sync` is fired against it (`engine_pairing::drain_pair_completions`) — no app restart, no 8s catch-up wait.
   - **No transport of our own** (the ephemeral CLI, or a GUI that lost the lease) → `pairing::host_pairing` / `join_pairing` bind a one-shot endpoint and **close it** (`endpoint.close().await`) before returning.
     This is the **one** sanctioned exception to the lease.
     It does take the route from the holder for the length of the handshake, and it is worth it because pairing is rare, explicit and short, while the alternative is a user who cannot add a device.
     Nothing else may bind around the lease.

## Phase-2 blob transfer (snapshot + asset)

Two ALPNs ship binary blobs that are NOT ops.
Both mount on the one sync endpoint's router, hold no workspace lock, never write the op log, and are best-effort (failure = logged no-op).

**Both authorize the dialer through `authz` before reading a byte off disk**, the same fail-closed `peers.json` verdict `SYNC_ALPN` uses.
They did not, for two releases, and that is the whole of what `outl peer remove` failed to revoke: a removed device kept a complete read of the graph and of every uploaded file.
These two carry no request body, so they have no `workspace_id` line to validate — and need none: `peers.json` is per workspace, so a device paired into a different one is refused by node id alone.

**Snapshot** — `SNAPSHOT_ALPN` `outl-snapshot/1`, `engine_snapshot.rs`.
A freshly-paired device pulls a peer's `snap-<actor>.bin` and boots from settled state, not the full op log (`pull_snapshot_from_peer`).
Responder `SnapshotProtocolHandler` authorizes, then sends one length-prefixed frame (empty when absent); the puller writes `snap-<peer-actor>.bin` under `.outl/snapshots/` and fires `peer_ready_tx`.

**Asset** — `ASSET_ALPN` `outl-asset/1`, `engine_assets.rs`.
Uploaded files live at `<root>/assets/<hash>.<ext>` (content-addressed by SHA-256); their bytes NEVER enter the op log (`outl_actions::asset`).
Since a device holds N assets, the protocol negotiates a manifest: responder `AssetProtocolHandler` authorizes, then sends its `assets/` basenames (`protocol::encode_asset_manifest`).
The refusal has to land **before** the manifest — the names are content hashes and their count is the size of the user's upload history, so shipping it and refusing the blobs still leaks.
It also has to be re-asked **per request**, not once per connection: this is the only protocol here that serves an unbounded number of payloads over one connection, at the initiator's pace, so a connect-time-only check left a peer approved a moment before `outl peer remove` draining `assets/` for as long as it kept asking.
`SYNC_ALPN` is per exchange on a pooled connection and has never had that shape; the next frame is this protocol's next exchange.
The initiator pulls each file it lacks as a blob frame (atomic tmp+rename).
`is_safe_asset_name` (both sides, anti-traversal) blocks any non-basename; the initiator re-hashes each file (`outl_md::asset::hash_bytes`) and drops a name mismatch.
Fires after the post-pair snapshot pull and every catch-up `delta_sync` (`run_catch_up`'s `pull_assets`).
