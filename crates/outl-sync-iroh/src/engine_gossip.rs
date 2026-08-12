//! Gossip supervisor: real-time op-announce + mesh-membership over iroh-gossip,
//! with **live re-subscription when the workspace id changes**.
//!
//! Extracted from `engine.rs` so that module stays focused on boot
//! orchestration. This module owns the one gossip task `run_iroh` spawns.
//!
//! ## Why a supervisor (not a fire-and-forget subscribe)
//!
//! The gossip topic is `blake3(workspace_id)` ([`workspace_topic_id`]). A joiner
//! that pairs at runtime ADOPTS the host's workspace id (see `engine_pairing`),
//! which changes the [`SharedWorkspaceId`] handle — but the boot-time gossip
//! subscription is still on the OLD topic. The two devices then sit on different
//! gossip topics, so no real-time announce ever propagates between them and the
//! desktop's edits never reach the mobile (the catch-up loop's per-session
//! `synced` dedup compounds this: after one immediate post-pair sync it stops
//! re-dialing, trusting gossip — which is dead).
//!
//! The fix: a single supervisor task that `select!`s over the announce drain,
//! the membership tick, the gossip receiver, AND a `wid_changed_rx` signal. When
//! adoption fires `wid_changed_rx`, the supervisor **drops the old topic handle
//! and re-subscribes** to `blake3(new id)`, with no transport restart and no
//! second endpoint (one endpoint per identity stays intact — re-subscribe reuses
//! the same `Gossip`/`Endpoint`).

use std::path::PathBuf;
use std::time::Instant;

use iroh_gossip::api::{Event, GossipReceiver, GossipSender};
use iroh_gossip::Gossip;
use n0_future::StreamExt;
use outl_core::id::ActorId;
use outl_core::WorkspaceId;
use tracing::{debug, info, warn};

use crate::engine::{
    delta_sync, try_acquire_in_flight, workspace_topic_id, Announce, AppendLock, InFlightPeers,
    SharedWorkspaceId,
};
use crate::health::PeerHealthMap;
use crate::peers::PeersStore;

/// Everything the gossip supervisor needs, grouped so `run_iroh`'s spawn site
/// stays readable (the alternative is a 12-arg function).
pub(crate) struct GossipCtx {
    pub(crate) gossip: Gossip,
    pub(crate) endpoint: iroh::Endpoint,
    /// Shared connection pool, so a gossip-triggered sync reuses the same hot
    /// connection the catch-up loop and forced passes use.
    pub(crate) conns: crate::peer_conn::PeerConnections,
    pub(crate) workspace_root: PathBuf,
    pub(crate) workspace_id: SharedWorkspaceId,
    pub(crate) actor: ActorId,
    pub(crate) peer_ready_tx: std::sync::mpsc::Sender<()>,
    pub(crate) health: PeerHealthMap,
    pub(crate) append_lock: AppendLock,
    pub(crate) in_flight: InFlightPeers,
    pub(crate) peers_path: PathBuf,
    pub(crate) progress: crate::progress::ProgressSink,
}

/// Resolve the current bootstrap peer ids from `peers.json` (so a re-subscribe
/// after an id change still seeds the swarm with the known peers).
fn bootstrap_ids(peers_path: &std::path::Path) -> Vec<iroh::EndpointId> {
    match PeersStore::load_or_default(peers_path) {
        Ok(store) => store
            .list()
            .iter()
            .filter_map(|p| p.iroh_node_id().ok())
            .collect(),
        Err(e) => {
            debug!("gossip: reload peers.json for bootstrap failed: {e}");
            Vec::new()
        }
    }
}

/// Subscribe to the topic for `wid`, returning the split sender/receiver.
///
/// Returns `None` (skipping real-time gossip until the next id change) if the
/// subscribe fails — direct delta-sync still converges content meanwhile.
async fn subscribe(
    gossip: &Gossip,
    wid: &WorkspaceId,
    peers_path: &std::path::Path,
) -> Option<(GossipSender, GossipReceiver)> {
    let topic_id = workspace_topic_id(wid);
    let peers = bootstrap_ids(peers_path);
    match gossip.subscribe(topic_id, peers).await {
        Ok(topic) => Some(topic.split()),
        Err(e) => {
            debug!("gossip subscribe failed for topic {topic_id}: {e}");
            None
        }
    }
}

/// Handle one inbound gossip message: a membership broadcast merges unknown
/// peers into `peers.json`; an op-announce triggers a delta sync from the sender.
fn handle_message(ctx: &GossipCtx, msg: iroh_gossip::api::Message) {
    let content = std::str::from_utf8(&msg.content).unwrap_or("");

    // Membership broadcast? Merge unknown peers into peers.json; the catch-up
    // loop reloads it each tick and dials them. Tagged kind, so it never
    // collides with the untagged op-announce below.
    if let Some(parsed) = crate::engine_membership::parse_membership(content) {
        match parsed {
            Ok(incoming) => {
                let self_node_id = ctx.endpoint.id().to_string();
                match crate::engine_membership::merge_membership(
                    &ctx.peers_path,
                    &self_node_id,
                    incoming,
                ) {
                    Ok(added) if added > 0 => {
                        info!(
                            "membership gossip: discovered {added} new peer(s) from {}",
                            msg.delivered_from.fmt_short()
                        );
                    }
                    Ok(_) => {}
                    Err(e) => debug!("membership merge failed: {e}"),
                }
            }
            Err(e) => debug!("membership decode failed: {e}"),
        }
        return;
    }

    // Otherwise: op-announce "workspace_id\nactor_id\nhlc". A peer's announcement
    // means it has new ops — pull from it.
    let parts: Vec<&str> = content.splitn(3, '\n').collect();
    if parts.len() < 2 {
        return;
    }
    let peer_node_id = msg.delivered_from;
    let conns = ctx.conns.clone();
    let wr = ctx.workspace_root.clone();
    let wid = ctx.workspace_id.clone();
    let actor = ctx.actor;
    let tx = ctx.peer_ready_tx.clone();
    let health = ctx.health.clone();
    let lock = ctx.append_lock.clone();
    let in_flight = ctx.in_flight.clone();
    let prog = ctx.progress.clone();
    tokio::spawn(async move {
        let Some(_in_flight) = try_acquire_in_flight(&in_flight, peer_node_id) else {
            debug!(
                "gossip: sync from {} already in flight, skipping",
                peer_node_id.fmt_short()
            );
            return;
        };
        let started = Instant::now();
        let wid_snapshot = wid.read().expect("workspace id rwlock poisoned").clone();
        match delta_sync(
            &conns,
            peer_node_id,
            &wr,
            &wid_snapshot,
            actor,
            tx,
            &lock,
            &prog,
        )
        .await
        {
            Ok(()) => health.record_success(peer_node_id, started),
            Err(e) => {
                warn!(
                    "gossip-triggered sync from {} failed: {e}",
                    peer_node_id.fmt_short()
                );
                health.record_failure(peer_node_id);
            }
        }
    });
}

/// Broadcast this device's local-op announcement so peers know to pull.
async fn announce(
    sender: &GossipSender,
    actor: ActorId,
    workspace_id: String,
    hlc: outl_core::hlc::Hlc,
) {
    let hlc_json = match serde_json::to_string(&hlc) {
        Ok(s) => s,
        Err(e) => {
            warn!("serialize hlc for gossip announce: {e}");
            return;
        }
    };
    let payload = format!("{workspace_id}\n{actor}\n{hlc_json}");
    if let Err(e) = sender.broadcast(bytes::Bytes::from(payload)).await {
        debug!("gossip broadcast failed: {e}");
    }
}

/// Broadcast this device's known peer list (mesh membership auto-discovery).
async fn broadcast_membership(sender: &GossipSender, peers_path: &std::path::Path) {
    match crate::engine_membership::build_membership_payload(peers_path) {
        Ok(Some(payload)) => {
            if let Err(e) = sender.broadcast(payload).await {
                debug!("membership broadcast failed: {e}");
            }
        }
        Ok(None) => {} // no peers yet, nothing to share
        Err(e) => debug!("build membership payload failed: {e}"),
    }
}

/// The gossip supervisor task `run_iroh` spawns once.
///
/// Owns BOTH the announce drain and the membership broadcast (one
/// [`GossipSender`] shared between them) plus the receive stream, all in one
/// `select!` so a workspace-id change can swap the topic atomically: on a
/// `wid_changed_rx` signal it drops the current topic handle and re-subscribes
/// to `blake3(new id)`. Reuses the same `Gossip`/`Endpoint` — no second endpoint,
/// no restart.
///
/// Returns when `announce_rx` closes (transport shutdown drops the sender), or
/// when the task is aborted.
pub(crate) async fn run_gossip(
    ctx: GossipCtx,
    mut announce_rx: tokio::sync::mpsc::UnboundedReceiver<Announce>,
    mut wid_changed_rx: tokio::sync::broadcast::Receiver<WorkspaceId>,
) {
    let mut membership_tick = tokio::time::interval(crate::engine_membership::MEMBERSHIP_INTERVAL);

    // Subscribe to the boot-time topic. A `None` (subscribe failed) still lets
    // the supervisor run: it drains announce + membership into the void and
    // waits for a `wid_changed` signal to retry, while direct delta-sync carries
    // content. We model "no live topic" as `topic: Option<(sender, receiver)>`.
    let initial = {
        let wid = ctx
            .workspace_id
            .read()
            .expect("workspace id rwlock poisoned")
            .clone();
        subscribe(&ctx.gossip, &wid, &ctx.peers_path).await
    };
    let mut topic = initial;

    loop {
        // Re-derive the receiver each iteration: when there's no live topic we
        // park the gossip-receive arm on `pending()` so the other arms still run.
        let recv_next = async {
            match topic.as_mut() {
                Some((_, recv)) => recv.next().await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            // ── Op-announce drain (local edits → broadcast so peers pull) ──
            announce_msg = announce_rx.recv() => {
                let Some((_hint, hlc)) = announce_msg else {
                    // Sender dropped: transport shutting down.
                    return;
                };
                if let Some((sender, _)) = topic.as_ref() {
                    // Announce under the CANONICAL workspace id, not whatever the
                    // client passed in (clients pass the page slug, which is the
                    // wrong key — the announce payload should carry the gossip
                    // topic key). The broadcast already targets the subscribed
                    // topic, so peers are woken regardless; using the real id here
                    // keeps the payload honest if the receive side ever validates
                    // it.
                    let wid = ctx
                        .workspace_id
                        .read()
                        .expect("workspace id rwlock poisoned")
                        .as_str()
                        .to_string();
                    announce(sender, ctx.actor, wid, hlc).await;
                }
            }

            // ── Periodic mesh-membership broadcast ──
            _ = membership_tick.tick() => {
                if let Some((sender, _)) = topic.as_ref() {
                    broadcast_membership(sender, &ctx.peers_path).await;
                }
            }

            // ── Inbound gossip (announce → pull, membership → merge) ──
            event = recv_next => {
                if let Some(Ok(Event::Received(msg))) = event {
                    handle_message(&ctx, msg);
                }
                // A `None` (stream ended) or error leaves `topic` as-is; the next
                // id change re-subscribes. We don't drop it here to avoid a busy
                // loop on a transiently-erroring receiver.
            }

            // ── Workspace id adopted at runtime → re-subscribe to new topic ──
            changed = wid_changed_rx.recv() => {
                match changed {
                    Ok(new_wid) => {
                        info!(
                            adopted = %new_wid,
                            "gossip: workspace id changed; re-subscribing to new topic"
                        );
                        // Drop the old topic handle (unsubscribes) before binding
                        // the new one. Same endpoint, new topic.
                        drop(topic.take());
                        topic = subscribe(&ctx.gossip, &new_wid, &ctx.peers_path).await;
                    }
                    // `Lagged`: we missed an id change. Re-subscribe to whatever
                    // the live handle says now (it's the source of truth).
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        let wid = ctx
                            .workspace_id
                            .read()
                            .expect("workspace id rwlock poisoned")
                            .clone();
                        drop(topic.take());
                        topic = subscribe(&ctx.gossip, &wid, &ctx.peers_path).await;
                    }
                    // Sender dropped (transport shutting down). Keep serving the
                    // current topic until announce_rx closes too.
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                }
            }
        }
    }
}
