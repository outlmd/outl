# RFC 0155 — A paired peer is not a trusted peer

| | |
|---|---|
| **Status** | Accepted (all four holes closed; op signing remains out of scope — see Scope) |
| **Issue** | [#155](https://github.com/outlmd/outl/issues/155), [#160](https://github.com/outlmd/outl/issues/160), [#158](https://github.com/outlmd/outl/issues/158), [#159](https://github.com/outlmd/outl/issues/159) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [sync.md](../sync.md), [privacy.md](../privacy.md) |
| **Invariant** | root `CLAUDE.md` invariant 7 (`peers.json` is deliberately outside the op log — see The opposite direction) and invariant 8's general rule (a fix in one direction, undone in the other); `outl-sync-iroh/CLAUDE.md` → Sync request, Membership merge |
| **Guarded by** | `frame_body_length_is_capped` (`crates/outl-sync-iroh/src/engine_sync.rs`), `concurrent_saves_never_lose_an_entry_or_tear_the_file` (`crates/outl-sync-iroh/src/peers_lock.rs`) |

## Why

The sync layer was written as if pairing were a security boundary.
It is not, and two of the ways it is not were exploitable from a single paired device.

**One peer could kill the other's app, repeatedly** ([#155](https://github.com/outlmd/outl/issues/155)).
Every sync frame starts with a 4-byte length header.
The reader trusted that number and sized its buffer from it — before reading a byte of body, and before checking whether the sender was even on the same workspace.
A header of `0xFFFFFFFF` asked the receiver to reserve ~4 GiB.
On mobile that is an immediate OS kill; on desktop it is an OOM crash.
The device on the other end does not have to be malicious, only buggy, and the crash repeats on every reconnect.
The sharpest detail is that the pairing handshake in the *same crate* already capped its payload at 64 KiB (`pairing.rs:212`) — the sync read path simply never got the same guard.

**The peer list silently lost entries** ([#160](https://github.com/outlmd/outl/issues/160)).
`peers.json` was written with a plain overwrite: no atomic replace, no lock.
Four writers race for it in normal operation — pairing persistence, the ~5s membership gossip tick, the inbound address refresh, and, across processes, the GUI plus the MCP server plus `outl sync` running against one workspace.
Two of them doing read-modify-write means one saves its stale copy over the other's change.
A freshly paired device disappears, or a removal undoes itself, and nothing reports it.
A reader catching the file mid-write gets truncated JSON and drops that cycle's work.
The op log solved exactly this for itself years earlier with serialized atomic appends; `peers.json` was the one piece of persistent state left out.

Two further holes were found in the same audit: `peer remove` does not revoke ([#158](https://github.com/outlmd/outl/issues/158)) and pairing accepts an unverified identity ([#159](https://github.com/outlmd/outl/issues/159)).
Both are now closed **on the device that acts** — see *What we chose*.
Revoking a device across the whole mesh is still open, and is why this RFC is `Accepted` and not `Shipped`.

**A note on how #158 read from the outside.**
For a while the receiver-side check existed, `removed_peer_is_denied_sync` passed, and the removal still did not hold — because membership gossip re-added the peer within ~5 seconds and the check then waved it through.
A guard and its undo, shipped in the same binary, each individually defensible.
That is worth stating plainly because it is the failure mode this repo keeps finding: not a missing fix, but a fix whose opposite direction nobody stated (root `CLAUDE.md` invariant 8's general rule).

## What we chose

**`checked_frame_body_len` is the single owner of "is this declared length believable?"** (`crates/outl-sync-iroh/src/engine_sync.rs:360`).
It validates the 4 header bytes against `MAX_FRAME_BODY` (256 MiB) and returns an error before any allocation happens.
Every frame read in the crate goes through it, so there is no second opinion about the ceiling.

The allocation itself is also no longer sized from the header.
`Vec::with_capacity(4 + body_len.min(64 * 1024))` reserves at most 64 KiB up front and grows as bytes actually arrive.
That is the load-bearing half: the cap stops the absurd claim, and incremental growth means even a *legal* 256 MiB header cannot be used to make the receiver reserve 256 MiB for a body the sender never sends.

**`PeersStore::mutate_locked` is the single owner of every `peers.json` write** (`crates/outl-sync-iroh/src/peers.rs:486`).
The sequence is: take a cross-process `PeersWriteLock` flock on a sibling `.peers.lock`, **re-read the current file from disk inside the lock**, apply the mutation to that fresh copy, then write it back through `atomic_write_json`.
Re-reading inside the lock is what closes the lost update — an in-memory copy taken before the lock is stale by definition, and serializing writes on a stale copy still loses data.
`atomic_write_json` (`crates/outl-sync-iroh/src/peers_lock.rs:56`) mirrors `outl_core::snapshot::write_to_disk`, so a crash leaves either the old file or a stale `.tmp`, never a torn target.

**Membership merge is add-only.**
Gossip-learned entries may add a peer and may never clobber a known one, and the merge drops self and undialable entries.
That is what keeps a concurrent gossip tick from being an unlogged mutation of the trust set.

**Two halves make a revocation, and only one of them was a check** (#158).
`SyncProtocolHandler::serve` checks the connection's authenticated `remote_id()` against `peers.json`, read fresh per connection, and closes `unknown-peer` when it is absent.
That is the receiver-side half.
The other half is that the entry has to *stay* removed: `PeersStore::remove` now records a tombstone in `peers.json`, and `merge_membership` refuses to re-add a node id this device revoked.
Without it the add-only merge put the peer back on the next 5s tick and the check passed honestly.

Three properties of the tombstone are load-bearing, and each is pinned by a test:

- **It never expires.** A tombstone with a TTL is a revocation that undoes itself on a timer.
- **Re-pairing clears it.** `PeersStore::add` drops the tombstone, because pairing a device again is a later deliberate act than removing it. Without this, re-pairing a device you once removed reports success and then never syncs — a worse failure than the one being fixed, and one that looks like broken pairing rather than an intact revocation.
- **It is never gossiped.** `build_membership_payload` broadcasts `list()`, which is peers only. A gossiped tombstone would let any device revoke any other by broadcast alone, which is the protocol change #158 still has to think through — arriving by accident.

**Possession of the invite is a different question from control of a key** (#159).
`verify_declared_identity` proves the dialer controls the key it claims. A stranger also controls their key, so it never addressed the first hole in the issue: while the host is armed, the first device to connect was accepted and handed the workspace identity.

The ticket now carries a 32-byte secret, and the joiner proves it holds the ticket with `blake3::keyed_hash(secret, its own node id)`.
Two details make it work rather than merely exist:

- **The proof is keyed to the joiner's node id**, not a bare hash of the secret. A bare hash is a bearer token — anyone who watched one joiner use the ticket could resend the bytes. Keyed, a captured proof only authenticates the device it was issued for, and *that* id is pinned to the TLS identity by `verify_declared_identity`. The two checks interlock; either alone is bypassable.
- **Both checks run before the host sends its payload**, which is what carries the workspace identity. A refused joiner learns only that something is listening.

**The dial is bounded too.**
`connect` was the one step of this handshake with no timeout, which only mattered once the host could handle more than one connection.
Three bounded attempts, then an error saying what to check — rather than a command that hangs.

**A failed attempt re-arms, on both drivers.**
This is the half that was easy to miss. `host_pairing` accepted exactly one connection, and the GUI's `take_arm` consumed the arm the moment *any* connection arrived.
Adding a proof to check would have turned both into a one-packet denial of the pairing flow: dial once, fail the proof, and the user's pairing screen silently stops accepting the device they are actually trying to pair.
The CLI now loops until the window closes or a joiner passes; the GUI takes an `arm_snapshot` and only calls `complete_arm` after a successful handshake.

**The old ticket format is refused, not tolerated.**
A v1 ticket carried no secret. Accepting one would be a downgrade any observer could force, leaving the check present and bypassed, so `decode_ticket` refuses it with a message telling the user to update the other device and generate a new code.

## Why not the alternatives

**Grow the buffer incrementally and skip the cap.**
Half the fix, and it was tempting because it needs no constant to argue about.
It leaves the receiver reading an unbounded stream from a peer that never stops sending, so the OOM moves from one allocation to a slow one.
A ceiling makes the refusal immediate and legible in a log.

**Cap at the pairing handshake's 64 KiB.**
Consistent with the existing guard and wrong for this path: a first-sync op batch from a real workspace exceeds it easily, so the cap would refuse legitimate traffic and the guard would be turned off within a release.
256 MiB is chosen to be far above any plausible batch and far below "kills the process".

**Validate the workspace id before reading the frame, and rely on that.**
Appealing because it looks like authentication.
It does not help — the frame is read *in order* to learn the workspace id, and #155's premise is a peer that already passed pairing, so it would pass this check too.
Trust does not remove the need for a bound.

**Put `peers.json` behind an in-process mutex.**
Cheapest correct-looking fix, and it does not survive the actual topology.
The GUI, the MCP server and `outl sync` are separate processes against one workspace, so an in-process lock serializes three of the four writers and leaves the interesting race intact.
A flock is the only thing that spans processes.

**Model the peer set as an `Op` and let it converge through the log.**
The default position under invariant 7, and it was considered.
It costs correctness in the opposite direction: the op log is the thing the peer set gates access to, so making membership converge through it means a peer that should be refused can hand you the ops that say it is trusted.
Bootstrapping has to sit outside the thing it bootstraps.
The consequence is stated below rather than hidden.

**Ship #155 and #160 only after #158 and #159 are designed.**
The tidy option: one RFC, one coherent trust model.
It costs a live denial-of-service and a live data-loss bug in production for however long the design takes.
#155 was labelled P1 and marked active in production; #158 and #159 are P2 and `status:needs-design`.
Shipping the two mechanical fixes and writing down the two open holes is the honest split.

## The opposite direction

**Refusing a frame is now a way to break sync, not only to survive one.**
Before the cap, an oversized declaration crashed the receiver.
Now it errors, and the *mirrored* case matters: a legitimate frame above 256 MiB is refused with exactly the same error as an attack.
Nothing distinguishes them in the log, and nothing tells the user that sync is failing because a batch got too big rather than because a peer is hostile.
Today no code path produces such a batch — but nothing enforces that, and the first feature that does will surface as "sync stopped working".

**A lock is a new way to hang.**
`PeersWriteLock::acquire` blocks until the lock is free, on the caller's thread, in a synchronous `save`.
It is short by construction, and a stale lock from a killed process is now a way for a peer write to stall where the old plain overwrite would simply have proceeded — and corrupted.
The trade is deliberate; the failure mode is different, not absent.

**The receiver-side peer check made removal look like it works.**
This was the sharpest inversion in this RFC, and it stood for two releases.
`removed_peer_is_denied_sync` passed, so `peer remove` genuinely refused the next inbound connection — and then membership gossip re-added the peer within seconds and it worked again.
The half-fix was *more* misleading than the original behaviour, because the user could observe a denial and reasonably conclude they were protected.
The tombstone closes it **on the device that ran the command**, and `outl peer remove` now says exactly that instead of printing `Removed peer <id>` and letting the user infer a mesh-wide revocation it never performed.

**A gossiped tombstone, so removal propagates.**
The obvious next step, and rejected here rather than in a follow-up, because rejecting it is the reason this RFC stops where it does.
It makes membership convergent state living in a last-write-wins file, which invariant 7 puts in the op log.
It also hands every mesh member the power to evict every other by broadcast, with no way to tell a legitimate revocation from a malicious one — the peer set has no signing story.
And it does not actually lock anyone out: a revoked device still holds the whole op log and the workspace id, so real revocation needs identity rotation as well.
Three separate problems, one of which contradicts a standing invariant. That is an RFC of its own, not a paragraph in this one.

**A TTL on the tombstone, to keep `peers.json` from growing.**
The growth is one short record per device the user has ever removed, which is not a problem worth a mechanism.
The mechanism, meanwhile, is a revocation that silently reverses itself on a timer — the exact bug being fixed, wearing a clock.

**A PIN the user types, instead of a secret inside the ticket.**
Better against a shoulder-surfed QR code, and worse everywhere else: it adds a step to every pairing to defend a case where the attacker is already in the room.
The secret rides in the ticket the user is already transferring, so the honest path costs nothing. If the ticket itself leaks, a PIN would help — that is a real gap, named in Scope.

**`peers.json` stays outside the op log, against invariant 7's default.**
Stated explicitly because the invariant says the opposite is the default position.
The peer set is a shared file with last-write-wins semantics between devices, which is exactly what the invariant warns about.
It is exempt because it gates access to the log, and the exemption has a cost: two devices can disagree about the peer set forever, and there is no convergence story for that disagreement.
Add-only merge keeps it from destroying entries, and add-only merge is also precisely why a removal cannot propagate — which is #158.

**Fail-open in the merge.**
Dropping undialable entries keeps the set clean and means a peer that is temporarily unreachable can be skipped rather than kept.
Combined with add-only, the peer set drifts toward "everything anyone has ever seen", which is the shape #158 has to fix.

## How it cannot regress

1. **Invariants.**
   `outl-sync-iroh/CLAUDE.md` → Sync request states the two-part receiver check (workspace id **and** `remote_id()` in `peers.json`, read fresh per connection) and names #158 as the reason the peer check exists at all.
   The same file's Membership merge row states the add-only rule (never clobber a known entry, drop self, drop undialable).
   Root `CLAUDE.md` invariant 7 is the rule this RFC documents an exception to, and the exception is written here rather than in the invariant so the invariant stays absolute for everything else.

2. **Tests.**
   - `frame_body_length_is_capped` (`crates/outl-sync-iroh/src/engine_sync.rs`) is the #155 guard.
     Its doc comment says the declared length is attacker-controlled and must be rejected *before* it can size an allocation, and it asserts the `0xFFFFFFFF` claim, one byte over the ceiling, the ceiling itself, an empty body and a small body.
     Testing the exact boundary is what stops a future "simplify" from turning the check into a loose sanity heuristic.
   - `concurrent_saves_never_lose_an_entry_or_tear_the_file` (`crates/outl-sync-iroh/src/peers_lock.rs`) is the #160 guard.
     It runs many threads through the full read-modify-write the production writers do, and asserts every add survives and every observer parses the file whole.
     Its doc comment records that the old plain `std::fs::write` failed it both ways, so a reader cannot mistake it for a redundant test.
   - `merge_unknown_never_clobbers_a_known_entry` (`crates/outl-sync-iroh/src/peers.rs`) pins the "never clobber" half of the add-only merge.
     `merge_skips_self`, `merge_adds_unknown_and_dedups_known` and `merge_skips_unreachable_peer` (`crates/outl-sync-iroh/src/engine_membership.rs`) pin the rest.
   - `removed_peer_is_denied_sync` (`crates/outl-sync-iroh/tests/regression.rs`) pins the receiver-side half of #158 — a peer absent from `peers.json` is refused `unknown-peer` on a fresh connection.
   - `a_revoked_peer_is_never_re_added_by_gossip` (`crates/outl-sync-iroh/src/engine_membership.rs`) pins the other half, and is the one that fails if someone re-simplifies `merge_unknown` back to "add anything not already present".
     Its doc comment walks the exact user sequence, because the failure it guards was not an attack — it was the product undoing the user's own command.
   - `re_pairing_a_revoked_peer_clears_the_tombstone` pins the escape hatch. Delete it and a permanent tombstone ships, which breaks re-pairing in a way that looks like broken pairing.
   - `a_tombstone_is_never_gossiped_to_other_devices` pins the scope limit at the wire, so the mesh-wide protocol change cannot arrive by accident.
   - `removing_a_peer_that_was_never_paired_leaves_no_tombstone` stops `peer remove` becoming a way to pre-emptively block a stranger you have not met.
   - `rotation_changes_the_id_and_unpairs_everything`, `a_revoked_device_keeps_its_tombstone_after_rotation`, `rotating_twice_locks_out_both_previous_identities`, `rotating_an_unpaired_workspace_is_harmless` (`crates/outl-sync-iroh/src/revoke.rs`) pin `revoke-all`.
     The second is the one to keep: rotation must not wipe an existing tombstone, or a device removed just before rotating could be re-added by a peer still gossiping about it during the re-pairing window.
   - For #159 (`crates/outl-sync-iroh/src/pairing.rs`): `a_device_without_the_ticket_cannot_produce_the_proof` is the exploit from the issue;
     `a_captured_proof_does_not_transfer_to_another_device` pins the node-id binding that stops a replay;
     `a_pre_secret_ticket_is_refused_and_says_why` pins the downgrade refusal;
     `a_payload_with_no_proof_is_refused` pins that a missing proof is not a passing one;
     `every_ticket_gets_a_fresh_secret` stops a constant secret making every past invite eternal;
     `a_secret_is_redacted_in_debug_output` keeps a live invite out of the logs.
     Six deny cases against one allow case (`proof_verifies_for_the_invited_device`), per the security-testing rule in the user's Rust guidelines.

   - The re-arm is pinned on both drivers.
     `a_refused_joiner_does_not_consume_the_pairing_window` (`crates/outl-sync-iroh/tests/integration.rs`) runs it end to end over real QUIC: an attacker that knows the host's address but not the invite dials first and is refused, the invited device dials afterwards and pairs, and the host's `peers.json` ends up with the second and never the first.
     `reading_the_arm_never_disarms_it`, `a_refused_joiner_leaves_the_next_one_able_to_pair`, `completing_the_handshake_disarms` and `re_arming_replaces_the_previous_session` (`crates/outl-sync-iroh/src/engine_pairing.rs`) pin the GUI half without a network, against `ArmSlot` — extracted from `PairingHub` for exactly that reason, since the hub needs an endpoint and a runtime to construct and the policy needs neither.

   **Writing that test found a second bug, and it was not in the new code.**
   `Endpoint::connect` had no timeout and can pend forever.
   Every other step of this handshake got a bound in #159 — the accept window, each payload read — and the dial was missed, so `outl peer pair` on the joining side could sit with no output and no error.
   It is now `CONNECT_TIMEOUT` (8s) × `CONNECT_ATTEMPTS` (3), with a message naming what to check.

   Worth recording because of *how* it surfaced.
   The old code accepted exactly one connection, so a joiner never dialled a host that had already handled one, and the stall had nowhere to appear.
   Making the host survive a refusal created the first situation that dials twice.
   **A fix that removes a limit exposes whatever that limit was hiding** — invariant 10 seen from the other side.

   The test also had to pin both dials to the host's loopback address (`test_support::retarget_ticket` + `loopback_only`).
   A host advertises one direct address per local interface, and with no relay reachable iroh 1.0.0's path selection settles on a dead one about half the time on the second dial.
   Left in, the test would have been a flaky measurement of path selection rather than of the accept loop — and a flaky security test gets muted, which is worse than not having it.

## Scope

**Covered — pairing authentication ([#159](https://github.com/outlmd/outl/issues/159)).**
All three holes named in the issue are closed: the joiner proves possession of the ticket, its declared id is checked against the connection's authenticated identity on both sides, and the handshake is bounded and re-arms on failure.

**Covered — revocation ([#158](https://github.com/outlmd/outl/issues/158)), in two halves that answer different questions.**

`outl peer remove` answers *"retire a device I still have"*: the entry goes, a tombstone stops membership gossip putting it back, and the receiver check refuses the next connection — **on the machine that ran it**.

`outl peer revoke-all` answers *"lock out a device I no longer have"* by rotating the workspace identity (`rotate_workspace_identity`, `crates/outl-sync-iroh/src/revoke.rs`). Every pairing drops, the workspace gets a fresh `WorkspaceId`, and re-pairing spreads it — a device that is not re-paired keeps the old id, lands on a different gossip topic (`blake3(workspace_id)`) and is refused `workspace-mismatch` on any direct dial.

It is a dozen lines because every mechanism it needs was already built and tested for other reasons. That is the argument for it, not a coincidence: the rejected alternative would have added a wire format, a signature scheme and a trust question to arrive somewhere weaker.

**Why not a signed, gossiped tombstone.**
It is the obvious design and it loses to the threat it is for. Propagating a removal means any paired device can evict any other; in the stolen-laptop case the attacker *holds* a paired device, so they revoke your devices first. That trades "cannot revoke" for "whoever moves first wins", which is not an improvement against someone paying more attention than you are. Rotation has no race — the new id never leaves the devices physically in your hands.

**What neither half does.**
The revoked device keeps the copy of the graph it already synced. Rotation stops it receiving anything **new**; nothing can un-send history. Every surface says so in those words, because a user who believes their notes came back is worse off than one who knows they did not.

Still open: ops are unsigned (below), so a device that is still paired can forge an op claiming another actor's id. Rotation does not address that and does not need to — it removes the device, not the device's ability to lie while it is present.

**Superseded — the earlier scope note.**
Before `revoke-all` existed, this section read: *"`outl peer remove` now holds on the device that ran it"*, and listed propagation, authenticity and rotation as three prerequisites for real revocation.

Rotation dissolved the first two rather than solving them: with a new identity there is nothing to propagate and nothing to authenticate, because the old id simply stops working. The third *was* the answer.

Worth keeping as a note on method — the three prerequisites were all real, and two of them were only prerequisites of the design being assumed.

**Not covered — a leaked ticket.**
The secret defends the pairing window against someone who knows the host's address but not the invite. It does nothing if the invite itself is captured — a photographed QR *code* rather than a photographed *address*.
A user-typed PIN would help there and is rejected above for cost; the honest limit is that a pairing ticket is a bearer credential for its two-minute life.

**Not covered — op signing.**
Ops are unsigned, so a paired device can forge ops claiming another device's actor id.
[#38](https://github.com/outlmd/outl/issues/38) open question 5, and out of scope for both open issues above.

**Not covered — transport, workspace identity and address resolution.**
Those are [RFC 0038](0038-sync-transport-and-workspace-identity.md).
