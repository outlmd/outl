//! Long-lived QUIC connections to paired peers.
//!
//! # Why this exists
//!
//! Every sync used to open a connection and close it. That was not a choice
//! anyone made; it fell out of the confirmation living in the close code, so
//! the connection *had* to die for the exchange to be confirmed.
//!
//! The bill, measured between two devices on one LAN moving two ops:
//!
//! | | |
//! |---|---|
//! | direct connect to a stale address | ~5s (timeout) |
//! | relay connect fallback | ~10s |
//! | waiting on `conn.closed()` | ~5s |
//! | the actual exchange | ~3s |
//! | **total** | **~23s** |
//!
//! Roughly 20 of those 23 seconds were connection setup and teardown, repeated
//! every 8s catch-up tick and every forced pass. Real-time announce over gossip
//! was no better off: it triggers a `delta_sync`, and that paid the same toll.
//!
//! With the confirmation moved onto the stream (`protocol::ACK_DURABLE`), the
//! connection survives the exchange, so it can be kept. A sync on a hot
//! connection is one bi stream: a round trip, not a handshake.
//!
//! # What it does NOT do
//!
//! It does not keep a connection alive that the peer has dropped, and it does
//! not retry. A dead entry is discovered on next use and replaced. That is
//! deliberate — a background reconnect loop would dial a phone that is asleep
//! every few seconds, which is exactly the battery drain P2P sync is accused
//! of.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use iroh::endpoint::Connection;
use tracing::debug;

/// Pool of live connections to paired peers, keyed by node id.
///
/// `pub` only so `test_support` can hand one to an out-of-crate integration
/// test; every method that mutates it stays `pub(crate)`.
///
/// Cloning is cheap and shares the pool: every task that syncs holds the same
/// one, which is the point — two tasks dialing the same peer at the same
/// moment should end up on one connection.
#[derive(Clone)]
pub struct PeerConnections {
    endpoint: iroh::Endpoint,
    live: Arc<Mutex<HashMap<iroh::EndpointId, Connection>>>,
    /// Who this device is in contact with, as the GUI status dot reads it.
    ///
    /// It lives here because **connecting is the only step that answers the
    /// question**: everything after it is about the exchange. Threading it
    /// through `delta_sync` and its five callers instead put the verdict in
    /// the hands of code that only knew whether the *pass* worked, which is
    /// how an exchange that broke after connecting came to be recorded as
    /// "offline" (see `crate::health`).
    health: crate::health::PeerHealthMap,
}

impl PeerConnections {
    pub(crate) fn new(endpoint: iroh::Endpoint) -> Self {
        Self::with_health(endpoint, crate::health::PeerHealthMap::default())
    }

    /// A pool that records contact into `health` — the transport's own map.
    ///
    /// `new` gives a pool whose records go nowhere, which is what the tests
    /// that only care about the exchange want.
    pub(crate) fn with_health(
        endpoint: iroh::Endpoint,
        health: crate::health::PeerHealthMap,
    ) -> Self {
        Self {
            endpoint,
            live: Arc::new(Mutex::new(HashMap::new())),
            health,
        }
    }

    /// The reachability map this pool writes into.
    pub(crate) fn health(&self) -> &crate::health::PeerHealthMap {
        &self.health
    }

    /// The endpoint these connections are made from.
    ///
    /// Exposed because the snapshot and asset transfers still dial on their own
    /// ALPNs. Those are rare, large, one-shot transfers where a fresh
    /// connection is the honest shape, unlike a delta sync that runs every few
    /// seconds.
    pub(crate) fn endpoint(&self) -> &iroh::Endpoint {
        &self.endpoint
    }

    /// A connection to `peer`, reusing the live one when there is one.
    ///
    /// `close_reason()` is the liveness check: it is `None` while the
    /// connection is usable and `Some` the moment either side has closed it or
    /// it has timed out. Checking it here rather than tracking state ourselves
    /// means we never disagree with QUIC about whether a connection is alive.
    pub(crate) async fn get_or_connect(&self, peer_addr: iroh::EndpointAddr) -> Result<Connection> {
        let node_id = peer_addr.id;

        if let Some(conn) = self.live_connection(node_id) {
            debug!("reusing live connection to {}", node_id.fmt_short());
            // A connection QUIC still calls usable is contact, so say so: a
            // pooled peer would otherwise keep whatever the last completed
            // exchange left behind, including a stale failure.
            self.health.record_reached(node_id);
            return Ok(conn);
        }

        let conn = match crate::engine_sync::connect_with_fallback(&self.endpoint, peer_addr).await
        {
            Ok(conn) => {
                self.health.record_reached(node_id);
                conn
            }
            // The one outcome that means offline. Everything that can go wrong
            // after this point went wrong *with* a reachable device.
            Err(e) => {
                self.health.record_failure(node_id);
                return Err(e);
            }
        };
        // A concurrent caller may have connected first. Keep whichever is
        // already in the map so both callers converge on one connection rather
        // than leaving an orphan nobody closes.
        let mut live = self
            .live
            .lock()
            .map_err(|_| anyhow::anyhow!("peer connection pool mutex poisoned"))?;
        Ok(live.entry(node_id).or_insert(conn).clone())
    }

    /// Drop a peer's cached connection.
    ///
    /// Called when an exchange fails: the connection may be half-open, and
    /// reusing it would fail the next sync the same way. Cheap to be wrong —
    /// the next call reconnects.
    pub(crate) fn invalidate(&self, node_id: iroh::EndpointId) {
        if let Ok(mut live) = self.live.lock() {
            if live.remove(&node_id).is_some() {
                debug!("dropped cached connection to {}", node_id.fmt_short());
            }
        }
    }

    /// Close every pooled connection. Called on transport shutdown so peers
    /// see a clean close instead of waiting out an idle timeout.
    pub(crate) fn close_all(&self) {
        let Ok(mut live) = self.live.lock() else {
            return;
        };
        for (_, conn) in live.drain() {
            conn.close(0u32.into(), b"shutdown");
        }
    }

    /// The cached connection for `peer`, if there is one and QUIC still
    /// considers it usable. Prunes the entry when it does not.
    ///
    /// `pub(crate)` so `test_support` can hand it to an out-of-crate test that
    /// asserts the pooling contract on connection identity rather than on a
    /// stopwatch, which over loopback would measure nothing.
    pub(crate) fn live_connection(&self, node_id: iroh::EndpointId) -> Option<Connection> {
        let mut live = self.live.lock().ok()?;
        let conn = live.get(&node_id)?;
        if conn.close_reason().is_none() {
            return Some(conn.clone());
        }
        live.remove(&node_id);
        None
    }
}
