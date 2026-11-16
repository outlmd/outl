//! Per-peer reachability tracking, populated by every exchange this device
//! takes part in — the dials it makes **and** the dials it answers.
//!
//! The running [`crate::IrohSyncTransport`] dials every known peer at boot,
//! on every catch-up tick, and whenever gossip says a peer has new ops. Each
//! of those dials already knows whether it reached the peer. We record that
//! outcome here so a GUI status indicator can read reachability **without**
//! standing up a second iroh endpoint with the device identity.
//!
//! **The inbound direction counts, and leaving it out was a user-visible bug.**
//! Reachability is not symmetric: a phone behind carrier NAT is the side that
//! dials, which is the arrangement outl *designs for* (mobile forces a pass on
//! its own 3s timer precisely because waiting for a desktop to reach it is not
//! workable). So the desktop's dials to it fail, `record_failure` lands, and
//! the sync dot read "no peer reachable" while that same peer's ops were
//! arriving on an inbound serve seconds earlier — the dot's own tooltip said
//! "Last synced just now" directly under it. One exchange, two answers.
//! [`PeerHealthMap::record_reached`] is what both sides call the moment they
//! are in contact: the responder as soon as `authz` approves a dialer, the
//! initiator as soon as its connection opens. Waiting for the exchange to
//! finish is what made the dot flicker on a real workspace — a 500k-op resend
//! takes minutes, and a dial that opened and then broke mid-push
//! (`send our ops blob`) was recorded as "offline" four seconds after an
//! inbound serve from that same peer.
//!
//! That second-endpoint path is exactly the bug this module exists to kill:
//! iroh's relay keeps a single `node_id → endpoint` route, so a transient
//! probe endpoint sharing the device's `SecretKey` hijacks the relay route
//! from the long-lived sync endpoint, and inbound sync connections land on an
//! endpoint that doesn't accept `SYNC_ALPN` ("the server refused to accept a
//! new connection"). One endpoint per identity, elected not assigned, is the
//! invariant; status now
//! reads from this shared map instead of binding its own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iroh::EndpointId;
use outl_actions::PeerHealthSnapshot;

/// Reachability state for a single peer, keyed by its [`EndpointId`].
#[derive(Debug, Clone)]
pub(crate) struct PeerHealth {
    /// `true` if the most recent dial (or inbound serve) succeeded.
    pub reachable: bool,
    /// Duration of the last successful dial, in milliseconds.
    pub last_rtt_ms: Option<u64>,
}

/// Shared, thread-safe map of peer reachability.
///
/// Cloned into the boot connect, the catch-up loop, and the gossip-triggered
/// sync tasks so each can record its dial outcome; read by the transport's
/// `peer_health()` for the GUI status path.
#[derive(Clone, Default)]
pub(crate) struct PeerHealthMap {
    inner: Arc<Mutex<HashMap<EndpointId, PeerHealth>>>,
}

impl PeerHealthMap {
    /// Record a successful dial to `peer`, stamping the round-trip duration.
    pub(crate) fn record_success(&self, peer: EndpointId, started: Instant) {
        let rtt = started.elapsed().as_millis() as u64;
        if let Ok(mut map) = self.inner.lock() {
            map.insert(
                peer,
                PeerHealth {
                    reachable: true,
                    last_rtt_ms: Some(rtt),
                },
            );
        }
    }

    /// Record that this device and `peer` are in contact, without timing it.
    ///
    /// Two callers, one fact. The responder calls it when an approved dialer
    /// reaches us, and the initiator when its own connection opens — the
    /// exchange that follows may still fail, and a peer we are **talking to**
    /// is reachable whatever the outcome of that conversation.
    ///
    /// Reachable with **no** `last_rtt_ms` invented, since neither caller
    /// timed a round trip; a figure an earlier dial did measure is kept, for
    /// the same reason [`Self::record_failure`] keeps it.
    pub(crate) fn record_reached(&self, peer: EndpointId) {
        if let Ok(mut map) = self.inner.lock() {
            let prev_rtt = map.get(&peer).and_then(|h| h.last_rtt_ms);
            map.insert(
                peer,
                PeerHealth {
                    reachable: true,
                    last_rtt_ms: prev_rtt,
                },
            );
        }
    }

    /// Record a failed dial to `peer`. Keeps any prior `last_rtt_ms` so the
    /// UI can show "last seen Nms ago, now offline" if it wants to.
    pub(crate) fn record_failure(&self, peer: EndpointId) {
        if let Ok(mut map) = self.inner.lock() {
            let prev_rtt = map.get(&peer).and_then(|h| h.last_rtt_ms);
            map.insert(
                peer,
                PeerHealth {
                    reachable: false,
                    last_rtt_ms: prev_rtt,
                },
            );
        }
    }

    /// Project the current state into UI-agnostic [`PeerHealthSnapshot`]s.
    ///
    /// A peer the transport has never dialed this session is absent from the
    /// map and therefore absent from the result; the caller fills the gap
    /// with `reachable = false` from its own `peers.json` list.
    pub(crate) fn snapshot(&self) -> Vec<PeerHealthSnapshot> {
        let Ok(map) = self.inner.lock() else {
            return Vec::new();
        };
        map.iter()
            .map(|(id, h)| PeerHealthSnapshot {
                node_id: id.to_string(),
                reachable: h.reachable,
                last_rtt_ms: h.last_rtt_ms,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn some_id() -> EndpointId {
        iroh::SecretKey::generate().public()
    }

    #[test]
    fn empty_map_snapshots_empty() {
        let map = PeerHealthMap::default();
        assert!(map.snapshot().is_empty());
    }

    #[test]
    fn success_then_failure_keeps_last_rtt() {
        let map = PeerHealthMap::default();
        let id = some_id();
        map.record_success(id, Instant::now());
        let snap = map.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(snap[0].reachable);
        assert!(snap[0].last_rtt_ms.is_some());

        map.record_failure(id);
        let snap = map.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(!snap[0].reachable);
        // The prior rtt survives the failure so the UI can still show it.
        assert!(snap[0].last_rtt_ms.is_some());
    }

    #[test]
    fn contact_marks_the_peer_reachable_without_inventing_an_rtt() {
        let map = PeerHealthMap::default();
        let id = some_id();
        // The desktop's own dial to a phone behind NAT fails...
        map.record_failure(id);
        // ...and then the phone dials us and the exchange lands.
        map.record_reached(id);
        let snap = map.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(
            snap[0].reachable,
            "a peer whose ops we just ingested is reachable; reading it as \
             offline is what made the desktop dot orange while syncing"
        );
        assert!(
            snap[0].last_rtt_ms.is_none(),
            "a serve is not a round trip — no rtt was measured, so none is reported"
        );
    }

    #[test]
    fn contact_keeps_the_rtt_an_earlier_dial_measured() {
        let map = PeerHealthMap::default();
        let id = some_id();
        map.record_success(id, Instant::now());
        map.record_reached(id);
        let snap = map.snapshot();
        assert!(snap[0].reachable);
        assert!(
            snap[0].last_rtt_ms.is_some(),
            "the inbound report must not erase a figure an outbound dial did measure"
        );
    }

    #[test]
    fn failure_first_has_no_rtt() {
        let map = PeerHealthMap::default();
        let id = some_id();
        map.record_failure(id);
        let snap = map.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(!snap[0].reachable);
        assert!(snap[0].last_rtt_ms.is_none());
    }
}
