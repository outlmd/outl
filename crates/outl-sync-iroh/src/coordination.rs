//! The handles the transport's concurrent tasks coordinate through.
//!
//! Four paths run `delta_sync` against the same peers and the same op log at
//! once — the boot connect, the 8s catch-up loop, gossip-triggered sync, and
//! the forced `sync_now` pass — plus the inbound `serve` side. Everything in
//! this module exists because two of them overlapping produces a wrong result
//! rather than a slow one.
//!
//! Extracted from `engine.rs` (which owns boot orchestration) because these
//! types have no relationship to standing an endpoint up: `engine_sync`,
//! `engine_catchup`, `engine_gossip` and `engine_pairing` all reach for them
//! and none of them care how `run_iroh` works.

use std::collections::HashSet;
use std::sync::Arc;

use outl_core::WorkspaceId;

/// Process-wide guard serializing every op-log append performed by the iroh
/// transport.
///
/// `ingest_received_ops` opens `ops-<actor>.jsonl` in append mode and writes a
/// batch. Three concurrent paths reach it for the *same* file — boot connect,
/// the 8s catch-up loop, gossip-triggered sync (all via `delta_sync`), plus the
/// inbound `serve` side. Without serialization two `write_all`s interleave at
/// the syscall layer and glue two ops together with no separating newline
/// (`…}}}{"ts":…`), corrupting the log. A single global async mutex held across
/// the open+write+flush of each batch closes that race. Batches are small and
/// infrequent, so a global lock costs nothing measurable and correctness wins.
pub(crate) type AppendLock = Arc<tokio::sync::Mutex<()>>;

/// Process-wide set of peers with a `delta_sync` currently in flight.
///
/// Defense in depth on top of [`AppendLock`]: boot + catch-up + gossip can all
/// launch a `delta_sync` for the same peer at once. Each redundant run dials,
/// re-exchanges the full delta, and queues another writer behind the append
/// lock. Skipping a dial when one is already running for that peer cuts the
/// redundant relay traffic and the pile-up of writers.
pub(crate) type InFlightPeers = Arc<std::sync::Mutex<HashSet<iroh::EndpointId>>>;

/// Process-wide count of inbound responder-side `serve` exchanges running.
///
/// The outbound set above is not the whole in-flight picture: the responder
/// (`SyncProtocolHandler::serve`) reads, ingests and confirms a peer's push
/// without ever appearing in it. A caller holding an OS runtime assertion
/// open until the device settles (the mobile background flush) must not read
/// "nothing in flight" while an inbound exchange is still short of its
/// durable-ingest confirmation — suspending there tears down the exact
/// exchange the flush exists to finish.
///
/// A plain counter, NOT a per-peer entry in [`InFlightPeers`]: that set
/// doubles as an exclusion guard (skip a peer already being dialed), and an
/// inbound serve must not participate in the exclusion — a simultaneous
/// bidirectional exchange with one peer is legal and routine.
pub(crate) type InboundServes = Arc<std::sync::atomic::AtomicUsize>;

/// RAII guard that decrements the inbound-serve count on drop, so an early
/// return, an error, or task cancellation inside `serve` never leaves the
/// count stuck above zero (which would pin every background flush to its
/// full cap for the life of the process).
pub(crate) struct InboundServeGuard {
    serves: InboundServes,
}

impl Drop for InboundServeGuard {
    fn drop(&mut self) {
        self.serves
            .fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Mark one inbound serve as running until the returned guard drops.
pub(crate) fn begin_inbound_serve(serves: &InboundServes) -> InboundServeGuard {
    serves.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    InboundServeGuard {
        serves: serves.clone(),
    }
}

/// Shared, mutable handle to this workspace's stable [`WorkspaceId`].
///
/// Read at call time by every `delta_sync` / serve so the value reflects the
/// **current** workspace identity, and written by pairing adoption (the joiner
/// overwrites its id with the host's — see `engine_pairing`). Because the
/// initiator dials and the responder validates against this same live value, an
/// adopted id takes effect for the immediate post-pair sync and every later sync
/// without a transport restart. (The gossip *topic* is subscribed once at boot
/// from the boot-time id; an adopted id reaches real-time gossip on the next
/// start, but direct delta-sync — boot connect, 8s catch-up, immediate post-pair
/// dial — carries it live, so content still converges immediately. See the
/// crate `CLAUDE.md`.)
pub(crate) type SharedWorkspaceId = Arc<std::sync::RwLock<WorkspaceId>>;

/// RAII guard that removes a peer from the in-flight set on drop, so an early
/// return or an error inside `delta_sync` never leaves a peer stuck "in flight".
pub(crate) struct InFlightGuard {
    peers: InFlightPeers,
    nid: iroh::EndpointId,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut set) = self.peers.lock() {
            set.remove(&self.nid);
        }
    }
}

/// Try to mark `nid` in flight. Returns `Some(guard)` if it was free (caller
/// proceeds and the guard clears it on drop), `None` if a sync is already
/// running for that peer (caller skips).
pub(crate) fn try_acquire_in_flight(
    peers: &InFlightPeers,
    nid: iroh::EndpointId,
) -> Option<InFlightGuard> {
    let mut set = peers.lock().ok()?;
    if set.insert(nid) {
        Some(InFlightGuard {
            peers: peers.clone(),
            nid,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> iroh::EndpointId {
        iroh::SecretKey::generate().public()
    }

    /// The whole contract in one test: one holder at a time, and the slot frees
    /// itself on drop.
    ///
    /// The drop half is the load-bearing one. `delta_sync` has a dozen `?`
    /// early returns between acquiring and finishing, and a peer left marked
    /// in flight is never dialed again for the life of the process — the
    /// device goes quiet with nothing logged.
    #[test]
    fn one_peer_at_a_time_and_the_slot_frees_itself() {
        let peers: InFlightPeers = Arc::new(std::sync::Mutex::new(HashSet::new()));
        let (a, b) = (peer(), peer());

        let guard_a = try_acquire_in_flight(&peers, a).expect("a is free");
        assert!(
            try_acquire_in_flight(&peers, a).is_none(),
            "a second dial for the same peer must be refused"
        );
        assert!(
            try_acquire_in_flight(&peers, b).is_some(),
            "a different peer is unaffected"
        );

        drop(guard_a);
        assert!(
            try_acquire_in_flight(&peers, a).is_some(),
            "dropping the guard must release the peer, including on an early return"
        );
    }

    /// Inbound accounting mirrors the outbound guard's contract: concurrent
    /// serves stack, and every guard — including one dropped on an early
    /// return — gives its count back. A count stuck above zero pins the
    /// mobile background flush to its full cap forever.
    #[test]
    fn inbound_serves_count_stacks_and_frees_itself() {
        let serves: InboundServes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let load = |s: &InboundServes| s.load(std::sync::atomic::Ordering::Acquire);

        let first = begin_inbound_serve(&serves);
        let second = begin_inbound_serve(&serves);
        assert_eq!(load(&serves), 2, "concurrent inbound serves both count");

        drop(first);
        assert_eq!(load(&serves), 1, "each guard returns exactly its own count");
        drop(second);
        assert_eq!(load(&serves), 0, "an idle responder reads as settled");
    }
}
