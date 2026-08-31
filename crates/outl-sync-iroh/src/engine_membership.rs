//! Mesh membership auto-discovery over the existing gossip topic.
//!
//! ## Why this module exists
//!
//! Without it, the mesh only converges through **transitive op propagation**
//! (A↔B↔C reconciles ops) plus **manually pairing every pair** of devices to
//! get direct links. Item 5 closes that gap: when A pairs with B and B already
//! knows C, A should learn C's reachability automatically, so the user never
//! has to hand-pair every pair to get a full mesh.
//!
//! ## How
//!
//! Each device periodically broadcasts its **known peer list** (the same
//! node_id + relay/endpoint_addr reachability `peers.json` stores) over the
//! existing workspace gossip topic, as a message kind *distinct* from the
//! op-announcement. On receiving a membership message, a device merges any
//! **unknown** peers into its local [`PeersStore`] and persists `peers.json`.
//! The existing catch-up loop (which reloads `peers.json` every tick) then dials
//! the newly-merged peers — no extra dialing machinery here.
//!
//! ## Message kind (tagged, back-compat with op-announce)
//!
//! The op-announce message is the untagged `"workspace_id\nactor\nhlc"` format
//! parsed in [`crate::engine::run_iroh`]. Membership messages carry a distinct
//! first line — [`MEMBERSHIP_TAG`] — so the receive side routes them before
//! falling through to the announce parser. An announce's first token is a
//! workspace slug (a directory name), which never equals the literal
//! `"outl-membership/1"`, so the two kinds never collide.
//!
//! Wire format:
//!
//! ```text
//! outl-membership/1\n<json array of PeerEntry>
//! ```
//!
//! ## Trust model (load-bearing)
//!
//! **Every device subscribed to the workspace gossip topic is already inside the
//! trust domain.** The topic id is `blake3(workspace_id)` (see
//! [`crate::engine::workspace_topic_id`]) — only devices that were paired into
//! this mesh by *someone* ever subscribe to it. Membership gossip therefore only
//! ever ADDS reachability for peers that are *already mesh members*; it never
//! invites a stranger. A device that isn't on the topic can't inject a peer, and
//! a peer we merge was already trusted by the device that gossiped it.
//!
//! ### The premise that was false: "already a mesh member"
//!
//! That argument holds for a peer nobody has removed. It does **not** hold for
//! one somebody has: a device the user revoked is no longer a member, every
//! *other* device still lists it, and this module re-added it from their gossip
//! within one 5s tick.
//!
//! Which made the removal cosmetic twice over. `engine_sync`'s authorization
//! check reads the same `peers.json` fresh per connection and correctly refuses
//! an unlisted peer — so the fix that landed for issue #158 was real, and this
//! module put the peer back before it could ever fire. A guard and its undo,
//! shipped in the same binary.
//!
//! So [`PeersStore::remove`] leaves a tombstone and [`merge_membership`] honours
//! it. **This revokes the peer on this device only** — the tombstone is
//! deliberately not gossiped, because propagating "B is out" between devices
//! that disagree is convergent state, which
//! [invariant 7](../../../CLAUDE.md) puts in the op log rather than in a
//! last-write-wins file. See [RFC 0155](../../../docs/rfcs/0155-peer-trust.md)
//! → Scope and [issue #158](https://github.com/outlmd/outl/issues/158).
//!
//! Conservative guards on the merge:
//!
//! - **Never add self.** A device drops its own node_id from any incoming list.
//! - **Never add an unreachable peer.** An entry without a usable
//!   relay/endpoint_addr (its [`PeerEntry::iroh_endpoint_addr`] won't resolve) is
//!   skipped — we don't store a peer we can't dial.
//! - **Never re-add a peer this device revoked.** See above.
//! - **Dedup by node_id; only ADD unknown peers.** A node_id already in
//!   `peers.json` is left untouched (its locally-captured addr, e.g. from direct
//!   pairing, may be fresher than the gossiped one). See
//!   [`PeersStore::merge_unknown`].

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::peers::{PeerEntry, PeersStore};

/// First-line tag marking a gossip message as a membership broadcast (as opposed
/// to the untagged op-announce). Versioned so the format can evolve.
pub(crate) const MEMBERSHIP_TAG: &str = "outl-membership/1";

/// How often a device re-broadcasts its known peer list over gossip.
///
/// Short enough that a device paired into the mesh learns the rest of the mesh
/// within a couple of ticks, long enough that the chatter is negligible (the
/// payload is a handful of small JSON entries). The catch-up loop's 8s tick then
/// dials whatever this merges, so end-to-end discovery settles in well under a
/// catch-up cycle plus a membership tick.
pub(crate) const MEMBERSHIP_INTERVAL: Duration = Duration::from_secs(5);

/// Build the membership broadcast payload from the current peer list on disk.
///
/// Reloads `peers.json` so the broadcast always reflects the latest known set
/// (including peers paired after boot). Returns the tagged bytes ready for
/// `GossipSender::broadcast`. Returns `Ok(None)` when there are no peers to share
/// (nothing to gossip — don't spam an empty list).
pub(crate) fn build_membership_payload(peers_path: &Path) -> Result<Option<bytes::Bytes>> {
    let store = PeersStore::load_or_default(peers_path).context("reload peers.json for gossip")?;
    let peers = store.list();
    if peers.is_empty() {
        return Ok(None);
    }
    let json = serde_json::to_string(peers).context("serialize membership peer list")?;
    let payload = format!("{MEMBERSHIP_TAG}\n{json}");
    Ok(Some(bytes::Bytes::from(payload)))
}

/// If `content` is a membership message, return the decoded peer list it carries.
///
/// Returns `None` when `content` is **not** a membership message (so the caller
/// falls through to the op-announce parser). A membership message with a
/// malformed body returns `Some(Err(..))` so the caller can log it.
pub(crate) fn parse_membership(content: &str) -> Option<Result<Vec<PeerEntry>>> {
    let body = content.strip_prefix(MEMBERSHIP_TAG)?;
    // The tag must be a full first line: either the whole message is just the
    // tag (no body → empty list) or the tag is followed by a newline + body.
    let body = match body.strip_prefix('\n') {
        Some(rest) => rest,
        None if body.is_empty() => return Some(Ok(Vec::new())),
        // Tag is a prefix of a longer token (e.g. a workspace slug that happens
        // to start with the tag) — not a membership message.
        None => return None,
    };
    Some(serde_json::from_str::<Vec<PeerEntry>>(body).context("decode membership peer list"))
}

/// Merge a gossiped peer list into the local store, persisting `peers.json`.
///
/// Conservative by design (see the module-level trust model): drops self, drops
/// peers with no usable reachability, drops peers this device revoked, and only
/// ADDS node_ids not already known (an existing entry's locally-captured addr is
/// left untouched).
///
/// Returns the number of peers newly added (0 means nothing changed, so the
/// caller can skip logging / re-dial hints).
pub(crate) fn merge_membership(
    peers_path: &Path,
    self_node_id: &str,
    incoming: Vec<PeerEntry>,
) -> Result<usize> {
    // Drop self and any peer we can't actually reach before touching the store.
    let candidates: Vec<PeerEntry> = incoming
        .into_iter()
        .filter(|p| p.node_id != self_node_id)
        .filter(|p| p.iroh_endpoint_addr().is_ok())
        .collect();
    if candidates.is_empty() {
        return Ok(0);
    }
    let mut store = PeersStore::load_or_default(peers_path)
        .context("reload peers.json for membership merge")?;
    let (added, refused) = store.merge_unknown(candidates)?;
    if refused > 0 {
        // Worth a line every tick: the mesh keeps offering a device this user
        // removed, and a silent drop is indistinguishable from never being
        // told. It also tells them the removal only holds here — the other
        // devices are still gossiping this peer as a member.
        tracing::debug!(
            refused,
            "membership gossip offered {refused} peer(s) revoked on this device — not re-adding (they remain paired with the devices still gossiping them)"
        );
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(node_id: &str) -> PeerEntry {
        PeerEntry {
            node_id: node_id.to_string(),
            alias: None,
            // A bare relay url so `iroh_endpoint_addr` resolves (reachable).
            relay_url: Some("https://relay.example/".to_string()),
            endpoint_addr: None,
            added_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    /// A real node id (valid public key) so `iroh_endpoint_addr` parsing passes
    /// even without a relay url.
    fn real_node_id() -> String {
        iroh::SecretKey::generate().public().to_string()
    }

    #[test]
    fn parse_membership_ignores_op_announce() {
        // The untagged op-announce format must NOT parse as membership.
        let announce = "my-workspace\nsome-actor\n{\"physical_ms\":1}";
        assert!(parse_membership(announce).is_none());
    }

    #[test]
    fn parse_membership_decodes_tagged_payload() {
        let peers = vec![entry("abc")];
        let json = serde_json::to_string(&peers).unwrap();
        let msg = format!("{MEMBERSHIP_TAG}\n{json}");
        let decoded = parse_membership(&msg)
            .expect("is membership")
            .expect("decodes");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].node_id, "abc");
    }

    #[test]
    fn parse_membership_tag_prefix_of_longer_token_is_not_membership() {
        // A workspace slug that merely starts with the tag text (no newline) is
        // an op-announce, not a membership message.
        let msg = format!("{MEMBERSHIP_TAG}-not-really\nactor\nhlc");
        assert!(parse_membership(&msg).is_none());
    }

    #[test]
    fn merge_skips_self() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let me = real_node_id();
        let mut e = entry(&me);
        e.relay_url = None; // force resolution via node id only
        let added = merge_membership(&path, &me, vec![e]).unwrap();
        assert_eq!(added, 0, "must never add self");
    }

    #[test]
    fn merge_adds_unknown_and_dedups_known() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let me = real_node_id();

        let p1 = real_node_id();
        let added = merge_membership(&path, &me, vec![entry(&p1)]).unwrap();
        assert_eq!(added, 1, "p1 is new");

        // Re-merging p1 plus a new p2: only p2 is added.
        let p2 = real_node_id();
        let added = merge_membership(&path, &me, vec![entry(&p1), entry(&p2)]).unwrap();
        assert_eq!(added, 1, "p1 already known, only p2 added");

        let store = PeersStore::load_or_default(&path).unwrap();
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn a_revoked_peer_is_never_re_added_by_gossip() {
        // Issue #158, the exact sequence a user performs:
        //
        //   1. A and B are paired; C is also in the mesh and knows B.
        //   2. On A, the user runs `outl peer remove B`.
        //   3. C's membership tick gossips its peer list, which still has B.
        //
        // Before the tombstone, step 3 put B back within ~5s and `engine_sync`
        // then authorized it, because that check reads this same file. The
        // removal was undone by the product, not defeated by an attacker.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let me = real_node_id();
        let b = real_node_id();

        assert_eq!(merge_membership(&path, &me, vec![entry(&b)]).unwrap(), 1);

        let mut store = PeersStore::load_or_default(&path).unwrap();
        assert!(
            store.remove(&b).unwrap(),
            "B was paired, so remove finds it"
        );
        assert!(store.list().is_empty());

        // C gossips its list, which still contains B.
        let added = merge_membership(&path, &me, vec![entry(&b)]).unwrap();
        assert_eq!(added, 0, "a revoked peer must not come back from gossip");

        let store = PeersStore::load_or_default(&path).unwrap();
        assert!(
            store.list().iter().all(|p| p.node_id != b),
            "B is back in peers.json — engine_sync would authorize it again",
        );
    }

    #[test]
    fn re_pairing_a_revoked_peer_clears_the_tombstone() {
        // The mirror case, and the one a tombstone gets wrong by default. If
        // revocation were permanent, re-pairing a device the user had removed
        // would report success and then never sync, with nothing on screen
        // saying why — a worse failure than the one being fixed, because it
        // looks like the pairing itself is broken.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let b = real_node_id();

        let mut store = PeersStore::load_or_default(&path).unwrap();
        store.add(entry(&b)).unwrap();
        store.remove(&b).unwrap();
        assert!(store.is_revoked(&b));

        // `add` is what pairing calls. It must outrank the earlier removal.
        store.add(entry(&b)).unwrap();
        assert!(!store.is_revoked(&b), "re-pairing must clear the tombstone");

        // And gossip may now maintain it again like any other member.
        let store = PeersStore::load_or_default(&path).unwrap();
        assert_eq!(store.list().len(), 1);
        assert!(
            !store.is_revoked(&b),
            "the cleared tombstone must stay cleared on reload"
        );
    }

    #[test]
    fn removing_a_peer_that_was_never_paired_leaves_no_tombstone() {
        // `remove` returns false here, and a tombstone for a device that was
        // never a member would quietly block a *first* pairing later — a
        // removal that pre-emptively revokes a stranger is a footgun, not a
        // safety measure.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let stranger = real_node_id();

        let mut store = PeersStore::load_or_default(&path).unwrap();
        assert!(!store.remove(&stranger).unwrap());
        assert!(!store.is_revoked(&stranger));

        // A later gossip about that device is merged normally.
        let me = real_node_id();
        assert_eq!(
            merge_membership(&path, &me, vec![entry(&stranger)]).unwrap(),
            1,
        );
    }

    #[test]
    fn an_address_refresh_never_resurrects_a_revoked_peer() {
        // `refresh_peer_direct_addr` runs on every inbound connection and used
        // to go through `add`, which clears the tombstone. So a device removed
        // moments earlier could connect once, get its entry re-inserted **and
        // its revocation erased** — permanently, because nothing writes a
        // tombstone back.
        //
        // `update_existing` is the fix: it refreshes a peer that is still
        // listed and refuses to add one that is not.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let b = real_node_id();

        let mut store = PeersStore::load_or_default(&path).unwrap();
        store.add(entry(&b)).unwrap();
        store.remove(&b).unwrap();
        assert!(store.is_revoked(&b));

        // The removed device connects; the refresh path fires.
        let mut fresher = entry(&b);
        fresher.relay_url = Some("https://relay.moved/".to_string());
        assert!(
            !store.update_existing(fresher).unwrap(),
            "a peer that is not listed must not be re-added by a refresh",
        );

        let store = PeersStore::load_or_default(&path).unwrap();
        assert!(store.list().is_empty(), "the removed device stayed removed");
        assert!(
            store.is_revoked(&b),
            "the revocation survived an inbound connection from the revoked device",
        );
    }

    #[test]
    fn an_address_refresh_still_updates_a_peer_that_is_still_paired() {
        // Guards the guard: a fix that made `update_existing` never write would
        // pass the test above and silently break self-healing of a moved peer's
        // address, which shows up as "sync got slow" months later.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let b = real_node_id();

        let mut store = PeersStore::load_or_default(&path).unwrap();
        store.add(entry(&b)).unwrap();

        let mut fresher = entry(&b);
        fresher.relay_url = Some("https://relay.moved/".to_string());
        assert!(
            store.update_existing(fresher).unwrap(),
            "a paired peer refreshes"
        );

        let store = PeersStore::load_or_default(&path).unwrap();
        assert_eq!(
            store.list()[0].relay_url.as_deref(),
            Some("https://relay.moved/"),
            "the refreshed address must be what landed on disk",
        );
    }

    #[test]
    fn a_tombstone_is_never_gossiped_to_other_devices() {
        // The scope limit, pinned. `build_membership_payload` broadcasts
        // `PeersStore::list`, and if a tombstone ever leaked into that payload
        // it would let any device revoke any other by gossip alone — the
        // protocol change issue #158 still has to think through, arriving by
        // accident instead.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let b = real_node_id();
        let c = real_node_id();

        let mut store = PeersStore::load_or_default(&path).unwrap();
        store.add(entry(&b)).unwrap();
        store.add(entry(&c)).unwrap();
        store.remove(&b).unwrap();

        let payload = build_membership_payload(&path).unwrap().expect("has peers");
        let text = String::from_utf8(payload.to_vec()).unwrap();
        assert!(text.contains(&c), "C is still a member and is gossiped");
        assert!(
            !text.contains(&b),
            "a revoked peer must not appear in the broadcast at all",
        );
        assert!(
            !text.contains("revoked_at"),
            "tombstones are device-local and must never reach the wire",
        );
    }

    #[test]
    fn merge_skips_unreachable_peer() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("peers.json");
        let me = real_node_id();
        // node_id that won't parse as a public key AND no relay → unreachable.
        let mut bad = entry("not-a-real-key");
        bad.relay_url = None;
        let added = merge_membership(&path, &me, vec![bad]).unwrap();
        assert_eq!(added, 0, "an unreachable peer is never stored");
    }
}
