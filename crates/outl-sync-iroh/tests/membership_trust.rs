//! Membership gossip: what an unauthenticated broadcast is allowed to do to
//! `peers.json`.
//!
//! Split from `revocation.rs` because it answers a different question. That
//! file asks whether a *dialer* may be served; this one asks how a node id ends
//! up in the file every one of those checks reads. The two failure modes are
//! opposite in kind: one leaks the workspace to a device you removed, this one
//! **manufactures** authorization for a device you never approved.
//!
//! The important test here is the `#[ignore]`d one. It is an OPEN protocol
//! hole, not a regression, and it is why `outl peer remove` is documented as
//! retiring a device you still control rather than as a security boundary.
//! Read its body before deleting it; `engine_membership.rs`'s module header has
//! the long form and the three candidate fixes.

use outl_sync_iroh::{test_support, PeerEntry, PeersStore};

fn reachable_entry() -> PeerEntry {
    PeerEntry {
        node_id: iroh::SecretKey::generate().public().to_string(),
        alias: None,
        relay_url: Some("https://relay.example/".to_string()),
        endpoint_addr: None,
        added_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// DENY. One gossip message may not write an unbounded number of peers.
///
/// This does not decide WHO may be merged (see the ignored test below — that
/// is still open); it stops one unauthenticated message, re-broadcast every 5
/// seconds, from deciding it for thousands at a time.
#[test]
fn an_oversized_membership_broadcast_is_refused_whole() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("peers.json");
    let me = iroh::SecretKey::generate().public().to_string();

    let flood: Vec<PeerEntry> = (0..600).map(|_| reachable_entry()).collect();
    let added = test_support::membership_merge(&path, &me, flood);

    assert_eq!(
        added, 0,
        "an oversized list is refused whole, not truncated"
    );
    let store = PeersStore::load_or_default(&path).expect("load");
    assert!(
        store.list().is_empty(),
        "not one entry from the refused message may land — a half-applied \
         peer list is worse to debug than a rejected one"
    );
}

/// OPEN HOLE, pinned. Membership gossip can still manufacture authorization.
///
/// The gossip topic is `blake3(workspace_id)` and the workspace id is not a
/// secret — a revoked device knows it and can still subscribe. A message on
/// that topic carries no proof of authorship, so the revoked device generates a
/// fresh keypair, gossips THAT node id, and the merge accepts it: no tombstone
/// applies to a key that has never been seen. Every check that reads
/// `peers.json` — including the one this file exists to test — then authorizes
/// it honestly. Revocation is defeated by one `SecretKey::generate`.
///
/// `#[ignore]`d rather than deleted, and NOT converted into an assertion of the
/// current behaviour: the fix is a protocol decision (signed membership
/// entries, or splitting "discovered, dialable" from "approved, authorized"),
/// which is bigger than this change. Run it with `--ignored` to watch the hole.
/// Delete it when the hole closes, not before.
///
/// See `engine_membership.rs` → "The premise that is STILL false".
#[test]
#[ignore = "open protocol hole: gossip membership is unauthenticated (see the doc comment)"]
fn gossip_can_manufacture_an_authorized_peer() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("peers.json");
    let me = iroh::SecretKey::generate().public().to_string();

    // A device the user revoked. The tombstone holds for THIS key...
    let revoked = reachable_entry();
    assert_eq!(
        test_support::membership_merge(&path, &me, vec![revoked.clone()]),
        1
    );
    let mut store = PeersStore::load_or_default(&path).expect("load");
    assert!(store.remove(&revoked.node_id).expect("revoke"));
    assert_eq!(
        test_support::membership_merge(&path, &me, vec![revoked.clone()]),
        0,
        "the tombstone holds for the revoked key itself"
    );

    // ...and does nothing about a fresh one the same device gossips.
    let reborn = reachable_entry();
    let added = test_support::membership_merge(&path, &me, vec![reborn.clone()]);

    assert_eq!(
        added, 0,
        "a device with no pairing must not be able to write itself into \
         peers.json — it is now authorized for SYNC, SNAPSHOT and ASSET"
    );
}
/// ALLOW. A membership list at the cap still merges.
///
/// Guards the guard: an off-by-one that refused at the cap rather than past it
/// would pass the oversized deny test and silently shrink real meshes.
#[test]
fn a_membership_list_at_the_cap_still_merges() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let path = tmp.path().join("peers.json");
    let me = iroh::SecretKey::generate().public().to_string();

    let full: Vec<PeerEntry> = (0..256).map(|_| reachable_entry()).collect();
    let added = test_support::membership_merge(&path, &me, full);
    assert_eq!(added, 256, "a list exactly at the cap is still merged");
}
