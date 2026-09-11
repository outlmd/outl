# CLAUDE.md — outl-sync-iroh

iroh-based P2P transport for outl.
Implements `outl_actions::SyncTransport` using iroh QUIC + iroh-gossip.

## What this crate owns

- `IrohIdentity` — ed25519 keypair, stored at `~/.outl/identity.key` (per **device**, one node id per machine)
- `PeersStore` — known paired peers, stored at `<workspace>/.outl/peers.json` (per **graph**, the pair belongs to the workspace, not the OS).
  `workspace_peers_path(root)` builds the path; `migrate_global_peers_if_absent(root)` does a one-time best-effort copy of any legacy global `~/.outl/peers.json` into the workspace on first open (never deletes the global).
  Every client calls `migrate_*` then `PeersStore::load_or_default(workspace_peers_path(root))`.
- `build_transport(identity_path, workspace_root) -> TransportOutcome` (`device.rs`) — the **single owner** of "may this process bind an endpoint, and with what".
  It reads `[sync] transport` + `[sync] relay_url`, takes the endpoint lease, loads the identity + peer store, and returns `Ready` / `EndpointBusy` / `Disabled`.
  Every client calls it; the identity + peers + relay recipe used to be written out in the TUI, the shared Tauri backend and `outl sync` separately, and the MCP server skipped it entirely (issue #220).
  `build_default_transport(workspace_root)` is the form every client but mobile calls — it fills in `~/.outl/identity.key`; mobile passes its sandbox path to `build_transport` instead.
  `default_device_dir()` resolves that path and **logs one `WARN` per process when `$OUTL_DEVICE_DIR` redirects it** — a `cargo run` build is a *separate device*, and deleting its store voids every pairing.
  See [`docs/development.md` → Testing P2P sync from a source build](../../docs/development.md#testing-p2p-sync-from-a-source-build).
- `EndpointLease` (`lease.rs`) — the device-wide election backing it (see "One endpoint per identity, elected not assigned")
- `bind::bind_ipv4_only(relay_url, secret_key, alpns, advertise)` — the **single owner** of *how* an endpoint reaches the network, once `build_transport` has decided it may.
  Two things must hold for every endpoint and neither is optional per call site: the IPv4-only STOPGAP (dial and accept must agree or the iroh 1.0.0 multipath stall returns) and mDNS LAN discovery (issue #149).
  The STOPGAP is expressible on a shared builder; the mDNS attach is **not**, because iroh runs a registered `AddressLookupBuilder` inside `bind()` and a service that cannot start fails the whole bind — so it is attached *after* the bind and swallows its own error.
  A new endpoint calling `n0_builder_ipv4_only` + `.bind()` directly would compile and silently have no LAN discovery, which nothing would surface (sync still works over the relay).
  `every_endpoint_in_this_crate_binds_through_the_one_owner` (`bind.rs`) fails instead — **do not relax it**.
  On iOS `attach_mdns` routes to `lan::PlatformAddressLookup` instead: a socket-level multicast join needs an Apple entitlement, so discovery is driven by the platform's own mDNS daemon through a bridge in `outl-mobile` (`ios_bonjour.rs`). This crate is `#![forbid(unsafe_code)]` and stays that way — `lan` is a plain-Rust seam (`register_advertiser` out, `peer_discovered` in), and the FFI lives in the client.
  `lan::SERVICE_TYPE`, `lan::RELAY_TXT_KEY` and `lan::instance_label` are the **entire** agreement with the other clients, since `swarm-discovery` publishes plain DNS-SD. Drift there breaks discovery with no error on either side, so `the_lan_wire_constants_match_the_platform_bridge` reads the Swift and the `Info.plist` off disk, and two label tests pin the encoding.
  The label is **not** `EndpointId`'s `Display`: that is 64 hex chars, one over RFC 6763's 63-byte cap on a DNS label, so publishing it fails outright — and `from_str` accepts both encodings, so the breakage is one-directional and silent.
  `Advertise::No` is for a **transient** endpoint (the `outl peer status` probe): all endpoints share the device node id, so a short-lived one that publishes leaves a dead address for this device on every LAN peer, and multipath stalls on it — a diagnostic that degrades everyone's next dial.
  iOS never needs `com.apple.developer.networking.multicast` and **must not declare it** (a profile without it fails code signing); a fixed, declared Bonjour service type is exempt. What a user can hit there is declining the local-network prompt, which iOS reports to nobody; [`docs/sync.md`](../../docs/sync.md) says where to look.
- `authz` — the **single owner** of "is this dialer an approved device for this workspace?".
  One fail-closed `peers.json` read, called by every protocol that touches workspace content: `SYNC_ALPN`, `SNAPSHOT_ALPN`, `ASSET_ALPN`, and the gossip supervisor before it *dials* an announcer.
  It used to be inline in `SYNC_ALPN`'s handler.
  That is precisely why the other two shipped the whole materialized graph and every uploaded file to anyone who spoke the ALPN.
  It is also why an announce from a revoked device still got ops *in*: the authorization existed, and two of three protocols plus the outbound direction never called it.
  **Never write a second one.**
  A new protocol on this endpoint calls `authorize_or_close` before its first disk read; the module header says why, and `tests/revocation.rs` fails if one stops.
- `IrohSyncTransport` — implements `SyncTransport` trait, including the
  gossip-backed `announce_local_ops` hook (sync side → tokio task via an
  `mpsc` channel set up in `start()`) and the `peer_health()` reachability
  snapshot (see "One endpoint per identity, elected not assigned" below)
- Wire protocol — ALPN `b"outl-sync/3"`, vector-clock delta sync with per-actor `ActorClock { max, count }` gap detection (see "Sync invariants").
  v3 put the durable-ingest ack on the stream so a connection survives the exchange and can be pooled; a bump fails an old↔new dial cleanly at connect, no compat shim
- Pairing (`pairing` module, ALPN `b"outl-sync/pair/1"`) — the two-sided handshake.
  The "ticket" is a base64 `EndpointAddr` (id + relay + direct addrs).
  Both sides exchange one pairing payload (carrying their **full** `EndpointAddr`) over a single bi stream and persist the remote to `peers.json`.
  **Two drivers, one handshake:**
  - **CLI** (`outl peer pair`, no running transport) → `host_pairing` / `join_pairing` bind a one-shot endpoint.
    No relay route to steal.
  - **GUI** (mobile / desktop, transport already running) → `IrohSyncTransport::pair_host` / `pair_join` reuse the **live sync endpoint** (see "One endpoint per identity, elected not assigned").
    They never call `host_pairing` / `join_pairing`.
  The endpoint-agnostic handshake halves (`accept_host_handshake` / `run_join_handshake` in `pairing`) are shared by both paths.
  The GUI side is wired through `engine_pairing` (the `PairingHub` + `PairingProtocolHandler` mounted on the sync router).
  **The ticket carries a secret** (`mint_ticket` → `PairingSecret`); the joiner proves possession with `blake3::keyed_hash(secret, its own node id)`, checked before the host discloses its workspace id.
  Keyed to the node id on purpose — a bare hash would be a replayable bearer token — and interlocked with `verify_declared_identity`, which pins that node id to the authenticated TLS identity. Neither check stands alone.
  **A refused attempt must re-arm.** The CLI loops until the window closes; the GUI takes `arm_snapshot` and only `complete_arm`s on success. Consuming the arm on any inbound dial turns the proof into a one-packet denial of the pairing flow (issue #159).

## Workspace identity is a stable shared id, NOT the path (load-bearing)

**Two paired devices are "the same workspace" because they share one `outl_core::WorkspaceId`, never because their local paths match.**
Devices live at different paths (desktop `~/outl-p2p`, mobile `…/app.outl.mobile-app/outl`); deriving identity from the path made every device compute a different gossip topic, so cross-device gossip never connected and membership never propagated.

- **Home.**
  The id is a ULID persisted at `<root>/.outl/workspace-id` (plaintext, one line), read-or-generated on first transport `start()` via `WorkspaceId::read_or_create` (an existing workspace gets one on first open, stable thereafter).
  It lives in `.outl/` (never pollutes the clean markdown) as its **own** file, not a `config.toml` field.
  `config.toml` holds the **per-device** `actor_id` while the workspace id must be **identical** across devices; keeping them separate lets pairing-adoption rewrite the id without touching the actor.
- **Gossip topic.**
  `workspace_topic_id` = `blake3(workspace_id)`, so two devices on the same workspace land on the same topic regardless of path.
- **Sync request.**
  `SyncRequest.workspace_id` carries the id; `SyncProtocolHandler::serve` **validates it against the local id** (`workspace-mismatch` close) **and calls `authz::authorize_or_close`** (`peers.json` read fresh per connection; `unknown-peer` close).
The gossip supervisor applies the same verdict to the peer it is about to **dial**, since a sync is bidirectional and the initiator ingests the responder's ops.
  Issue #158: the id only proves the peer *thinks* it belongs; a removed device still knows it.
  **The check alone was not a revocation** — the add-only membership merge re-added the peer within one 5s tick and the check then passed honestly.
  `PeersStore::remove` leaves a tombstone (`peers.json` → `revoked`) and `merge_membership` honours it, so removal holds **on this device**; it is never gossiped, so other devices keep syncing with the peer until they remove it too.
  Never give the tombstone a TTL (a revocation that undoes itself on a timer) and never drop the `PeersStore::add` clear (re-pairing must work).
  **For a device the user no longer holds, the answer is `rotate_workspace_identity` (`revoke.rs`), not a propagated tombstone.**
  Gossiping a removal would let any paired device evict any other, and the stolen-laptop attacker *holds* a paired device — so they move first. Rotation has no race: the new id never leaves the devices being re-paired, and every mechanism enforcing it (topic = `blake3(id)`, the `workspace-mismatch` close, pairing adoption) already existed. [RFC 0155](../../docs/rfcs/0155-peer-trust.md).
- **Pairing makes the joiner adopt the host's id.**
  The handshake `PairingPayload` carries each side's id; the **host keeps** its id and the **joiner adopts** it *before* the immediate post-pair `delta_sync` fires.
  Adoption is **persist-first**: `adopt_workspace_id` writes the host's id to `.outl/workspace-id`, then flips the in-memory handle; a failed write does NOT adopt (retry-safe).
  Both sides then compute the same topic, validate as one workspace, and their op logs CRDT-merge (content from both converges — expected).
  **CLI `outl peer pair` adopts too** (issue #197): `join_pairing` writes the host's id to `<root>/.outl/workspace-id` and returns a `WorkspaceAdoption`.
  Without it the paired machine keeps a fresh id and every sync is refused `workspace-mismatch` (guard `pairing_roundtrip`).
- **Live handle.**
  The id lives behind a shared `RwLock` (`SharedWorkspaceId`) read at call time by `delta_sync` + serve, so an adopted id takes effect immediately for direct sync (boot connect, 8s catch-up, immediate post-pair dial — all carry the live id).
- **Gossip re-subscribes on id change (no restart).**
  Adoption fires a `tokio::sync::broadcast::Sender<WorkspaceId>` held in the `PairingHub` (`adopt_workspace_id` sends after persisting the file + updating the `RwLock`).
  The gossip supervisor (`engine_gossip::run_gossip`) holds one receiver and, on the signal, **drops its old-topic subscription and re-subscribes to `blake3(new id)`** — same `Gossip`/`Endpoint`, no second endpoint, no `start()` restart.
  Before this, the boot-time subscription stayed on the *pre-adoption* topic, so the two devices sat on different gossip topics and no real-time announce ever crossed (the "post-pair, nothing syncs again" bug).
- **Catch-up re-dials on id change.**
  A second receiver on the **same** broadcast channel (`PairingHub::subscribe_wid_changed`) makes the catch-up loop **clear its per-session `synced` dedup** and re-dial every peer under the adopted id.
  Without it, the post-pair `delta_sync` marked the peer synced forever and later host edits never pulled (gossip was the only live path, dead per the point above).
  A dropped signal is safe — the `RwLock` is the source of truth; the signal only fixes the *real-time* + *re-dial* gaps.

## One endpoint per identity, elected not assigned (load-bearing invariant)

A device binds **one** iroh endpoint, and which process gets it is decided by a **lease**, not by being the GUI.

"Only the GUI binds" was the old policy. It fixed a real collision and assumed a GUI exists, so a headless `outl mcp serve` box synced with nobody, silently ([issue #220](https://github.com/outlmd/outl/issues/220)).
Replacing it with a lease was right; what the change nearly missed was everyone leaning on "the GUI always wins" — desktop pairing read the transport out of app state, and `outl peer status` was documented as exempt on the grounds that the CLI has no running transport to conflict with, which stopped being true the moment the MCP server could hold the endpoint.
That is root `CLAUDE.md` invariant 10 in one paragraph: when you change who decides, enumerate who was standing on the old decision.

The lease lives on the **endpoint thread**, not on the transport. Too late strands the endpoint forever, too early admits a second owner, and both fail silently.

**Full mechanism, every failure mode, and what the lease refuses: [`docs/iroh-internals.md`](../../docs/iroh-internals.md#one-endpoint-per-identity-elected-not-assigned).**

## Status from the transport (`peer_health`)

`IrohSyncTransport` tracks per-peer reachability in an `Arc<Mutex<HashMap<EndpointId, PeerHealth>>>` (the `health` module).
The transport's own dials populate it: the **boot connect**, the **catch-up loop**, and **gossip-triggered sync** each record `record_success(nid, started)` / `record_failure(nid)` on their `delta_sync` outcome — no extra endpoint, no extra dials.

`SyncTransport::peer_health()` (a trait method in `outl-actions`, default `Vec::new()`) returns the snapshot as `Vec<outl_actions::PeerHealthSnapshot>` (`node_id`, `reachable`, `last_rtt_ms`).
`FileSyncTransport` uses the default (no peers).

The GUI status commands read it from the transport stored in Tauri state and merge it onto the full `peers.json` list, so a peer the transport hasn't dialed yet (or the file-transport case) shows offline.
The transport lives in desktop `state.iroh_transport` (`Arc<dyn SyncTransport>`, reached via the trait method) and mobile `state.iroh` (concrete `IrohSyncTransport`).
Desktop also keeps a **concrete** clone in `state.iroh_pairing` for `pair_host` / `pair_join` (not `SyncTransport` methods — the trait can't return `PeerEntry` without a dep cycle).
Both desktop slots are wired/cleared together in `wire_iroh_transport`; mobile reuses `state.iroh` for both status and pairing.
**Never** add a GUI status path that binds an endpoint with the device identity.

## Append-serialization invariant (load-bearing)

**Every op-log append the transport performs goes through one process-wide async append lock (`AppendLock`), held across open+`write_all`+`flush`+`sync_data` of each batch.**

`ingest_received_ops` is the single writer; it opens `ops-<actor>.jsonl` in append mode and writes the received batch.
Three concurrent paths reach it for the **same** file — the boot connect, the 8s catch-up loop, and gossip-triggered sync (all via `delta_sync`) — plus the inbound `serve` side.
Without serialization, two `write_all`s interleave at the syscall layer and glue two ops together with no separating newline (`…}}}{"ts":…`), corrupting the log.
That corruption is real: it was found on disk in an iCloud workspace (45 glued lines among 5261 valid ops).

Rules:

- `delta_sync` (initiator) and `SyncProtocolHandler::serve` (responder) both take/carry the **same** `AppendLock` clone and pass it to `ingest_received_ops`.
  Never add a new writer that appends to `ops-*.jsonl` without taking this lock.
- **A peer may not write OUR actor's file.**
  `write_deduped_batch` buckets on the untrusted `op.actor`, and had no guard.
  `outl_core`'s appender refuses a batch carrying a *foreign* actor's op.
  The ingest needs the mirror rule (anything but our own) and has to state it itself: that guard is `pub(super)`, reachable only through `Storage::append_ops`, which writes solely to `own_ops_path()`.
  Two things ride on it.
  `ops-<local>.jsonl` is serialized by `outl-core`'s `ops/.lock-<actor>`, a **different** lock from the one this path holds.
  And the dedup key is `(actor, ts)` and content-blind, so a forged op pre-claims an HLC slot and the user's real op is then dropped as a duplicate, silently.
  Refused per-op (the rest of the batch still lands) and logged.
- **Torn-tail heal.**
  A kill mid-`write_all` leaves a fragment with no terminator, and appending straight after glues the next op onto it — the glued-line recovery cannot split a torn prefix from a good op, so BOTH are lost.
  `outl-core`'s appender heals this; the ingest, a second writer to the same files, did not until `needs_tail_separator`.
- **Cross-process flock (`ops/.append.lock`).**
  The tokio mutex is per-process, but a device runs several transports at once (GUI + MCP server + `outl sync`), and their interleaved batches produced the timestamp retrocessions behind the watermark-gap bug.
  `write_deduped_batch` therefore also takes a blocking advisory flock on `ops/.append.lock` (same mechanism as `outl-core`'s `ops/.lock-<actor>`), acquired AFTER the in-process lock and held across the whole batch, on a `spawn_blocking` thread.
  The lockfile is ephemeral and recreated on demand, so a file transport dropping the dotfile is harmless (flock state is kernel-local and never syncs).
- Write the whole per-actor batch in one `write_all`, then `flush()` + `sync_data()` **before releasing the lock**, so a concurrent reader (or `outl-core`'s `reload`) can never observe a partial line.
- **In-flight guard (defense in depth).**
  A shared `InFlightPeers` set (`try_acquire_in_flight` → RAII `InFlightGuard`) stops boot + catch-up + gossip from launching a second `delta_sync` for a peer that already has one running.
  This cuts redundant relay traffic and the pile-up of writers behind the lock.
  The catch-up loop's per-session `synced` `HashSet` is a separate, complementary dedup (it skips peers already fully reconciled this session).
- Coverage: `tests/oplog_guards.rs` owns all three (the flock had **none** before — deleting its `acquire` failed nothing).
- Read-side safety net: `outl-core`'s `JsonlStorage::reload` recovers glued lines that already exist on disk (see `docs/storage.md`).
  That recovers historic corruption; the lock prevents new corruption.
  Both are needed.

## Force-sync trigger (`sync_now`)

`SyncTransport::sync_now()` (a trait method in `outl-actions`, default no-op so `FileSyncTransport` is unaffected) lets the GUI force an **immediate** delta-sync pass against every known peer instead of waiting for the 8s catch-up tick.
It backs the mobile pull-to-refresh / refresh button and the desktop Sync panel's Refresh.

Wiring mirrors the `announce_tx` / pairing-hub pattern exactly:

- `IrohSyncTransport` holds `sync_now_tx: Arc<Mutex<Option<UnboundedSender<()>>>>`, populated in `start()` (`None` before start / after shutdown = the "runtime down" guard).
- `sync_now()` sends a unit; a send error means the runtime is down, ignored (no-op), same contract as `announce_local_ops`.
- `run_iroh` moves the receiver into a `drain_sync_now` task (`engine_catchup`); each signal runs `force_sync_all`, dialing **every** peer in `peers.json` (reusing the append lock + in-flight guard + health recording).

**`force_sync_all` deliberately does NOT respect the catch-up loop's per-session `synced` `HashSet`** — the whole point of a manual sync is to re-dial even healthy peers the catch-up loop leaves to gossip.
`delta_sync` is a cheap no-op on matching vector clocks, so a forced re-dial of an already-synced peer just confirms convergence.
It still honors `try_acquire_in_flight` (skip a peer already being dialed) and the `AppendLock`.

The GUI commands are `outl_sync_now` in each client's `commands/peers.rs` (mobile reads `state.iroh`, desktop reads `state.iroh_transport` `Arc<dyn SyncTransport>`).
The shared wrapper is `syncNow()` in `@outl/shared/api/commands`.

**Observing completion — wait for YOUR pass, not for A pass.**
`completed_sync_passes()` is a monotonic counter bumped once per drained request (every peer dialed, succeeded *or* failed — no reachability promise).
`sync_now_seq()` returns the **sequence number** of the request it enqueued (`0` = runtime down, nothing to wait for), and the drain is FIFO with a 1:1 bump, so request *n* is done exactly when `completed_sync_passes() >= n`.

Polling "did the counter move" instead is a defect, not a style nit.
Mobile fires `sync_now()` on a 3s foreground timer, so the iOS background flush read that *foreground* pass as its own ~250ms in, dropped its `beginBackgroundTask` assertion, and let iOS suspend with its own request still queued.
Pinned by `every_queued_request_advances_the_counter_by_exactly_one` (`engine_catchup.rs`).

**A completed pass is not a settled device, and the two counters answer different questions.**
`force_sync_all` *skips* a peer that already has a dial running, so a pass can complete having dialed nobody.

`inbound_serves()` is the only in-flight count a background window may wait on: responder-side exchanges (`coordination::begin_inbound_serve`, RAII across the whole `serve`), which is someone else's ops seconds from the durable-ingest confirmation that stops them being re-sent.

**Never wait on the outbound dials.**
A version that summed both was unreachable exactly when the device was worst off: an unreachable peer costs 15s per dial (5s direct + 10s relay) while the catch-up loop starts another every 8s, so the outbound set never empties.
On device that read as `window elapsed before pass #107 settled` — the whole cap burned on a condition that could not become true, which iOS repays with fewer windows.
The outbound set therefore stays inside `run_iroh`, unreachable from outside, so the mistake cannot be made again.

## Module layout (delta-sync wire vs. orchestration)

Which file owns what, and why each split happened: [`docs/iroh-internals.md` → Module layout](../../docs/iroh-internals.md#module-layout).

The **gossip supervisor** lives in `engine_gossip.rs` (`run_gossip` + `GossipCtx`).
It is one task that `select!`s over the op-announce drain, the periodic membership broadcast, the inbound gossip stream, AND the `wid_changed` signal — re-subscribing to the new topic on an id change (see "Gossip re-subscribes on id change").
It runs even with zero peers at boot, so a device that pairs later still gets a live subscription via the id-change path (the old inline block only subscribed when boot-time peers existed).

## Mesh membership auto-discovery (gossip)

Without it, a full mesh needs **every pair** of devices hand-paired; ops only reach a non-adjacent device through **transitive propagation** (A↔B↔C reconciles).
Membership gossip closes that: when A pairs with B and B already knows C, A **auto-discovers** C's reachability and then syncs C **directly**.

It lives in `engine_membership.rs` (build / parse / merge) plus the send + receive glue in `engine::run_iroh`'s gossip block.

**Message kind (tagged, back-compat with op-announce).**
The op-announce is the untagged `"workspace_id\nactor\nhlc"` format.
A membership message carries a distinct first line — `MEMBERSHIP_TAG` (`"outl-membership/1"`, versioned) — followed by a JSON array of `PeerEntry` (the same node_id + relay/`endpoint_addr` reachability `peers.json` stores).
The receive side checks `parse_membership` **first**; a non-membership message falls through to the existing announce parser, so the announce path is untouched.

**Broadcast.**
A periodic task (`MEMBERSHIP_INTERVAL`, 5s) reloads `peers.json` and broadcasts the peer list (never an empty one) over the **same** gossip topic.
It reuses the **same** `Arc`-wrapped `GossipSender` as the announce drain — no second topic handle or endpoint.
It lives in the same `if !bootstrap_ids.is_empty()` block as the announce drain, so a zero-peer device neither subscribes nor gossips.

**Merge / persist flow.**
On receipt, `merge_membership` merges **unknown** peers into `peers.json` through `PeersStore::merge_unknown` (dedup by node_id, ADD-only — an existing entry's locally-captured addr, e.g. from direct pairing, is never clobbered).
The catch-up loop reloads `peers.json` each tick and dials the freshly-merged peer — **no new dialing machinery**; the append lock / in-flight guard / health map are reused as-is.

**Trust model — the assumption is FALSE, and the hole is open.**
The argument was: every device on the workspace gossip topic is already inside the trust domain, because the topic id is `blake3(workspace_id)` (`workspace_topic_id`), so only devices paired in by *someone* ever subscribe.
The workspace id is not a secret.
It is disclosed to every device that ever paired and sits in plaintext at `.outl/workspace-id`, so a **revoked** device still knows it, can still subscribe, and a gossip message carries no proof of authorship anyone checks.

That makes this merge a way to **manufacture** authorization, not just to discover reachability.
The tombstone stops a revoked device re-adding *itself*.
It does nothing about that device generating a fresh key pair and gossiping *that* node id, which has no tombstone, is merged, and is then honestly approved by `authz` for `SYNC_ALPN`, `SNAPSHOT_ALPN` and `ASSET_ALPN` alike.
Revocation is defeated by one `SecretKey::generate`.

**Pinned, not closed**, by `gossip_can_manufacture_an_authorized_peer` (`tests/membership_trust.rs`, `#[ignore]`d).
Every candidate fix is a protocol decision, not a local one.
Filtering on `Message::delivered_from` is partial (gossip relays, so that is the neighbour, not the author) and can starve discovery.
Signing membership entries needs a wire version and a key-distribution story.
Splitting "discovered, dialable" from "approved, authorized" changes the `peers.json` schema, `outl peer list` and revocation semantics.
The module header in `engine_membership.rs` is the long form.
**Do not close it by guessing** — and do not quietly delete the ignored test.

Conservative guards that ARE in place on the merge:

- **Never add self** (drop our own node_id from any incoming list).
- **Never add an unreachable peer** (skip an entry whose `PeerEntry::iroh_endpoint_addr` won't resolve — we don't store a peer we can't dial).
- **ADD-only dedup** (a known node_id keeps its current entry).
- **Never re-add a peer this device revoked** (the tombstone).
- **Refuse an oversized list whole** (`MAX_MEMBERSHIP_ENTRIES`, 256).
  This bounds what one unauthenticated message re-broadcast every 5s can write; it does not decide who may be merged.

## Phase-2 blob transfer (snapshot + asset)

Snapshot and asset bytes move over their own ALPNs, separate from the op-log exchange, and both are **authorized per request** — not per connection, which is what let a revoked peer keep draining `assets/` until it chose to hang up.

Protocol shape, framing, and the authorization boundary: [`docs/iroh-internals.md`](../../docs/iroh-internals.md#phase-2-blob-transfer-snapshot--asset).

## Test catalog (regression + chaos)

Every bug hand-found during the sync saga has a NAMED, permanent test — the name IS the bug, so a failure is self-explanatory.
Both catalogs (named guards + the chaos battery) live in [`docs/iroh-internals.md`](../../docs/iroh-internals.md#regression-suite-pilar-2).
**Do NOT delete a named guard without deleting the bug it guards**, and add the row when you add the test.

Three suites are organised by question rather than by bug:

- `tests/revocation.rs` — *may this dialer be served?*
  Every protocol on the endpoint, in both directions, and — for `ASSET_ALPN`, the one that serves many payloads over a single connection — mid-stream as well as at connect time.
- `tests/membership_trust.rs` — *how does a node id get INTO the `peers.json` those checks read?*
  The bounded merge, and the one `#[ignore]`d test pinning the open gossip hole (`gossip_can_manufacture_an_authorized_peer`).
- `tests/oplog_guards.rs` — *what may an already-authorized peer's bytes do to our `ops/` dir?*
  Bucketing, torn-tail heal, and the cross-process flock.

In all three, **deny cases outnumber allow cases on purpose**, and the allows are load-bearing: a check that refuses everything passes every deny test ever written.
`test_support::ingest_ops` exists so a test can hand the ingest what no honest peer would send, without writing a malicious responder.

## Sync invariants

- The op log (`ops-<actor>.jsonl`) IS the offline buffer.
- On reconnect: bidirectional vector-clock delta sync.
  Both sides exchange a per-actor `ActorClock { max: Hlc, count: u64 }` — max + DISTINCT-op count, derived from `all_ops` in `engine_sync::local_vector_clock` (the `Storage` trait is untouched) — and stream missing ops.
- **Gap detection (v2).**
  A bare max-HLC watermark assumes gapless delivery; an op landing ahead of a pending backlog made everything below the watermark permanently invisible (the Mac↔iPhone non-convergence).
  If the sender holds more distinct ops `<= max_r` than the receiver's `count`, it resends that actor's FULL log; the receiver's ingest dedup (present-set read under the append locks) absorbs the overlap and never appends an op twice.
  Accepted limit: equal counts over different subsets are indistinguishable — convergence still lands via each op's origin device, which always holds its own actor's complete log.
- Ops from actor C received via peer B are stored as `ops-<C>.jsonl` locally.
  A can get C's ops via A↔B sync even if A never connects to C directly.
- HLC sanity gate: ops with timestamps more than 24h in the future are
  logged as warnings and skipped (not applied).

## Catch-up loop (initial full sync on pairing)

`run_iroh` spawns a periodic **catch-up loop** (`catch_up_loop` → `run_catch_up`)
in addition to the boot-time connect, the gossip subscribe, and the announce
drain.
It exists for one bug: a device paired AFTER `start()` writes its `PeerEntry` to `peers.json`, but the boot connect read that list once and never re-reads it, so the new peer's history is never pulled.

- **Tick**: `CATCH_UP_INTERVAL` (8s), first tick immediate, so a freshly paired peer syncs right away.
- **Each tick**: reload `PeersStore` from the same `peers.json` path the transport started with, so peers paired after boot are picked up.
- **Dial**: `PeerEntry::iroh_endpoint_addr` — stored full `endpoint_addr` (id + relay + **direct addrs**) first, then id + `relay_url`, then the bare id.
  The direct addrs are what make same-LAN connect immediate instead of waiting on n0 discovery; the boot connect and `probe_peers` use the same builder.
- **Maintenance re-sync (the convergence safety net)**: each peer's last clean sync is timestamped, and a peer is re-dialed when new this session, when its last attempt failed, or when its last success is older than `MAINTENANCE_RESYNC` (10s).
  `delta_sync` no-ops on matching vector clocks and the in-flight guard collapses a slow re-dial into the previous one, so the short interval doesn't thunder.

**Those two numbers set the floor on how long a device stays busy.**
An unreachable peer costs 15s per dial against an 8s tick, which is why a background window must never wait on the outbound set (see "Force-sync trigger").
  **Load-bearing**: convergence must not depend on the real-time gossip path, since the announce may never cross (flaky cross-network iroh) or never be sent at all (the ephemeral CLI, see "Who ends up with the endpoint").
  The loop re-pulls every known peer within `MAINTENANCE_RESYNC` regardless.
  The earlier "synced once, never re-dial" design broke exactly there ("paired, first sync worked, then nothing propagates"); regression: `catch_up_resyncs_peer_after_interval`.
  **The map is cleared when the workspace id changes** (`run_catch_up` `select!`s on the `wid_changed` broadcast), forcing an immediate re-dial of every peer under the adopted id.
- `PeerEntry` carries the peer's **full** `EndpointAddr` in `endpoint_addr`
  (base64 JSON), captured at pairing time after the endpoint came online —
  see "Reachability: full `EndpointAddr`" below.

`run_catch_up` is parameterized over `period`, `resync_after`, and a `resolve_peers` closure so `test_support::run_catch_up_loop` drives it over loopback (regressions `catch_up_syncs_peer_paired_after_boot`, `catch_up_resyncs_peer_after_interval`).

## What breaks when this process has no endpoint

**Read this list before changing anything about who wins the lease.**
Five things need a live local endpoint, and every one of them fails *quietly* — a `None` slot, an empty `Vec`, a `let _ = tx.send(...)`.
Root `CLAUDE.md` invariant 10 exists because a change to the election shipped without checking them, and the desktop lost the ability to pair.

| Needs a live endpoint | Without one | Who covers it |
|---|---|---|
| `IrohSyncTransport::pair_host` / `pair_join` | GUI pairing has nothing to pair *through* | the client falls back to the one-shot `host_pairing` / `join_pairing`, the CLI's path |
| `sync_now()` | force-refresh is a no-op | nothing — the desktop reports it instead of returning `Ok(())` |
| `peer_health()` | empty, so every peer reads offline | nothing yet; the status dot cannot tell this from "all peers down" |
| `announce_local_ops` | peers wake on their next catch-up tick | `MAINTENANCE_RESYNC`, at a latency cost |
| inbound `serve` | this device answers no dials | the holder serves the same `ops-*.jsonl` from the same disk |

Only the last two degrade on their own.
The top three need a decision written down, because "the GUI always has a transport" stopped being true.

## Who ends up with the endpoint

Which process typically wins, and what each loser does instead, is one fact with one home: [`docs/sync.md` → Which process holds the endpoint](../../docs/sync.md#which-process-holds-the-endpoint).
It is a *consequence* of the lease (see "One endpoint per identity, elected not assigned"), never a policy anyone hard-codes — the table above ("What breaks when this process has no endpoint") is the part you must read before changing the election.

Correctness never depends on the announce: the `MAINTENANCE_RESYNC` catch-up is the safety net that converges any writer's ops, announced or not.
Holding the endpoint is a **latency** win (real-time vs next catch-up tick); losing it costs latency, never convergence.

## Reachability: full `EndpointAddr` in `PeerEntry` (load-bearing)

`PeerEntry` persists the peer's **full** `iroh::EndpointAddr` — node id + relay
URL + **direct socket addrs** — base64-JSON-encoded in the `endpoint_addr`
field, not just the bare node id.

**Why:** a bare node id makes every connect depend on n0 discovery resolving a route, unreliable between real devices (offline dot, first sync never pulled history).
Same-WiFi devices connect instantly via direct addrs — but only if captured; the old `PeerEntry` stored `relay_url: null` and no addrs.

**Capture (`pairing::ready_addr`):** `endpoint.addr()` right after `bind()` is typically empty, so both sides call `ready_addr` before minting the ticket / sending the payload.
It awaits `endpoint.online()` under a mandatory 5s timeout, after which the addr carries relay + LAN direct addrs; on timeout we proceed anyway (the local net report usually filled them).

**Exchange:** host mints the ticket from its ready addr (joiner stores a reachable host); joiner sends its ready `EndpointAddr` in the payload (host stores a reachable joiner).

**Resolution order** (`PeerEntry::iroh_endpoint_addr`): stored `endpoint_addr` → keep the relay + the **on-LAN IPv4** direct addrs, **drop IPv6** and **drop off-LAN IPv4**; else id + `relay_url`; else bare id.
A corrupt `endpoint_addr` logs a warning and falls through, never failing the dial.
Not relay-only (flaky relay breaks same-WiFi sync), not all-addrs (a dead IPv6/off-LAN direct stalls multipath) — on-LAN IPv4 + relay is the balance.

**Off-LAN IPv4 drop (issue #133):** a VPN-paired peer stores tunnel IPs (`10.x`, `100.x`, WAN) beside its LAN addr, stalling iroh's multipath on the dead ones.
`iroh_endpoint_addr` keeps only IPv4 on a **local** subnet (`is_reachable_lan_ipv4` + `if-addrs`; injectable `iroh_endpoint_addr_with_ifaces`), fail-open on error.

**Back-compat:** `endpoint_addr` is `#[serde(default)]`, so old `peers.json` entries (id + `relay_url` only) still deserialize + dial via the fallback.

Ticket codec == `endpoint_addr` codec (`encode_ticket` / `decode_ticket` delegate to `peers::{encode,decode}_endpoint_addr`) — a pairing ticket IS a `PeerEntry.endpoint_addr`.

**Self-heal on inbound connect (`peers::refresh_peer_direct_addr`):** a stored addr goes stale when a peer's DHCP lease moves (stalling multipath like an off-LAN one).
On an **accepted** inbound connection `serve()` reads the live remote socket off `Connection::paths()` and rewrites the stored `endpoint_addr` to *only* that socket + the known relay, so the next catch-up dial uses the fresh route without a re-pair.
Conservative: only an **already-paired** peer (unknown id → no-op), no write when unchanged.
Regression: `refresh_peer_direct_addr_replaces_stale_keeps_relay_and_is_idempotent`.

## STOPGAP: IPv4-only bind (iroh 1.0.0 multipath workaround)

Why `bind_pairing_endpoint` and the sync endpoint bind IPv4 only, and what has to be true before it is removed:
[`docs/iroh-internals.md`](../../docs/iroh-internals.md) → STOPGAP.
It is a workaround for an upstream bug, not a design decision — do not "clean it up" without reading that section.


## iroh 1.0.0 API notes (load-bearing)

The 1.0.0 surface differs from every older tutorial you will find.
The pinned list is in [`docs/iroh-internals.md`](../../docs/iroh-internals.md#iroh-100-api-notes-load-bearing) — check it before writing a call from memory, and update it when the pin moves.

## What this crate does NOT own

- CRDT logic — lives in `outl-core`
- Workspace reload / md projection — lives in `outl-actions::SyncEngine`
- iCloud / filesystem transport — lives in `outl-actions::FileSyncTransport`
