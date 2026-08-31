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
Pure guards live in `#[cfg(test)]` next to the code; over-the-wire (real QUIC, loopback) guards in `tests/regression.rs`.
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
| 4. Pairing adoption — joiner adopts host id (GUI + CLI, #197) | `gui_pairing_over_live_sync_endpoint` + `pairing_roundtrip` (CLI: shared on-disk id, returns `Adopted`) | `tests/integration.rs` |
| 5. Single endpoint per identity (pair AND sync over the live sync endpoint, no relay hijack) | `gui_pairing_over_live_sync_endpoint` (pre-existing; pairing rides the live sync endpoint, no second bind) | `tests/integration.rs` |
| 5b. Endpoint lease — one process binds, the loser is told to stay off the wire, not silently offline (issue #220) | `one_process_binds_the_device_endpoint_and_the_next_one_is_told_to_stay_off_the_wire` | `tests/endpoint_lease.rs` |
| 5c. A lease file that cannot be opened denies the endpoint (no arbiter must not mean everyone binds) | `a_lease_file_that_cannot_be_opened_denies_the_endpoint_instead_of_granting_it` | `src/lease.rs` |
| 5d. The status probe stands down instead of stealing the route it was run to diagnose | `the_probe_stands_down_when_another_process_holds_the_endpoint` | `src/status.rs` |
| 6. Reachability resolution + off-LAN/IPv6 direct-addr filter (issue #133) | `iroh_endpoint_addr_*` + `is_reachable_lan_ipv4_*` (keep on-LAN IPv4, drop IPv6 + stale VPN IPs, fall back stored/bare/corrupt) | `src/peers.rs` |
| 7. Bidirectional push materializes on BOTH sides AND fires BOTH reload signals | `bidirectional_sync_fires_reload_signal_on_both_sides` (set convergence + `peer_ready_tx` on initiator AND responder) | `tests/regression.rs` |
| 7. (set-convergence half) both sides hold all ops | `bidirectional_delta_sync` (pre-existing) | `tests/integration.rs` |
| 8. Membership merge is ADD-only (never clobber a local entry, drop self, drop undialable) | `merge_unknown_never_clobbers_a_known_entry` + `merge_skips_self` / `merge_adds_unknown_and_dedups_known` / `merge_skips_unreachable_peer` | `src/peers.rs`, `src/engine_membership.rs` `#[cfg(test)]` |
| 9. Watermark gap — ops below a receiver's max-HLC stayed permanently invisible after out-of-order ingest; the v2 `ActorClock` count detects the gap, the full-log fallback + ingest dedup converge without duplicating | `backlog_below_watermark_crosses_after_gap_detected` / `ingest_dedups_already_present_ops` / `full_actor_resend_converges_and_dedups` | `tests/regression.rs` |
| 10. Snapshot sync — peer snapshot on pair (byte-identical, reload fired); absent harmless | `snapshot_transfers_from_peer_on_pair` / `snapshot_pull_absent_is_harmless` | `tests/regression.rs` |
| 11. Asset sync — peer asset on pull (byte-identical, held file skipped); absent harmless | `assets_transfer_from_peer` / `asset_pull_from_peer_without_assets_is_harmless` | `tests/regression.rs` |
| 12. Forced-pass completion is per REQUEST, not global — a waiter on sequence `n` must not stop when somebody else's pass lands (the iOS background flush released its OS window on the foreground timer's 3s pass and let the device suspend mid-exchange) | `every_queued_request_advances_the_counter_by_exactly_one` | `src/engine_catchup.rs` `#[cfg(test)]` |

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
