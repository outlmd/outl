/**
 * Peer / device pairing wrappers (iroh sync transport).
 *
 * Split out of `commands.ts` under the file-size ratchet: these touch
 * `peers.json` and the iroh transport, never the workspace lock, so
 * they share nothing with the page / block commands beyond `invoke`.
 * Re-exported from `commands.ts`, which stays the import path every
 * client uses.
 */

import { invoke } from "@tauri-apps/api/core";

import type { PeerDto, PeerStatusDto } from "./types";

// ---------------------------------------------------------------------------
// Peer / device pairing (iroh sync transport)
// ---------------------------------------------------------------------------
//
// These wrap the `outl_peer_*` Tauri commands both clients register in
// `src-tauri/src/commands/peers.rs`. They touch the iroh `peers.json`,
// not the workspace lock — peer pairing is sync-transport state, not
// workspace state. See `outl-mobile/CLAUDE.md` / `outl-desktop/CLAUDE.md`
// § "Peers".

/**
 * List every paired device. Mirrors `outl_peer_list`. Reads the iroh
 * `peers.json` (empty list when the file is absent).
 */
export function peerList(): Promise<PeerDto[]> {
  return invoke<PeerDto[]>("outl_peer_list");
}

/**
 * Remove paired peers whose `node_id` starts with `id` (prefix match).
 * Resolves `true` when at least one peer matched and was removed.
 * Mirrors `outl_peer_remove`.
 */
export function peerRemove(id: string): Promise<boolean> {
  return invoke<boolean>("outl_peer_remove", { id });
}

/**
 * Live reachability + RTT for each paired peer. Mirrors
 * `outl_peer_status`. Reads the running iroh transport's own dial
 * outcomes (`peer_health()`) — no fresh probe endpoint — and merges
 * them onto the full `peers.json` list, so a peer the transport hasn't
 * dialed yet (or the file-transport case) comes back `online: false`.
 * Returns one {@link PeerStatusDto} per paired peer.
 */
export function peerStatus(): Promise<PeerStatusDto[]> {
  return invoke<PeerStatusDto[]>("outl_peer_status");
}

/**
 * Force an immediate P2P (iroh) sync pass against every paired peer.
 * Mirrors `outl_sync_now`.
 *
 * Backs the GUI's pull-to-refresh / "sync now" affordance: instead of
 * waiting for the iroh transport's ~8s catch-up tick, this dials every
 * peer right now to pull the freshest state. Callers typically chain it
 * with {@link reloadWorkspace} (sync, then re-render):
 *
 * ```ts
 * await syncNow();
 * await reloadWorkspace();
 * ```
 *
 * Resolves with no value. A no-op on the backend when no iroh transport
 * is wired (the iCloud file transport has no peer to dial) or its runtime
 * is down — it never rejects for "nothing to sync", so a missing peer
 * mesh is silent rather than an error.
 */
export function syncNow(): Promise<void> {
  return invoke<void>("outl_sync_now");
}

/**
 * Host a pairing session and resolve with the **ticket string** the
 * other device scans / types to join. Mirrors `outl_peer_pair_host`.
 *
 * The ticket comes back as soon as the iroh endpoint is bound — long
 * before a peer actually connects — so the caller can render it (e.g.
 * via {@link import("../peers").PairingQR}) while the handshake runs in
 * the background. The completed pairing surfaces through the backend's
 * `peer-paired` Tauri event (payload: {@link PeerDto}); listen for it
 * to refresh the device list. `peer-pair-failed` (payload: error
 * string) fires if the handshake times out or errors.
 *
 * `alias` is this device's own human label. We advertise it to the joining
 * device, which stores it under *our* node id in its `peers.json`. Defaults
 * to the platform name ("desktop" / "mobile") when omitted.
 *
 * Backend note: the desktop's `outl_peer_pair_host` currently resolves
 * with the paired peer object instead of the ticket and emits the
 * ticket early via a `peer-pairing-ticket` event; the mobile command
 * resolves with the ticket directly. This wrapper follows the mobile
 * contract (ticket string) — the desktop Rust command is being aligned
 * to it so both clients share this surface.
 */
export function peerPairHost(alias?: string | null): Promise<string> {
  return invoke<string>("outl_peer_pair_host", { alias: alias ?? null });
}

/**
 * Join a pairing session from a `ticket` produced by a host's
 * {@link peerPairHost}. Connects, completes the handshake, persists the
 * host to `peers.json`, and resolves with the newly paired
 * {@link PeerDto}. Mirrors `outl_peer_pair_join`.
 *
 * `alias` is this device's own human label, advertised to and stored by
 * the host (it persists under *our* node id in the host's `peers.json`).
 * The returned {@link PeerDto} carries the *host's* alias, not this one.
 */
export function peerPairJoin(ticket: string, alias?: string | null): Promise<PeerDto> {
  return invoke<PeerDto>("outl_peer_pair_join", { ticket, alias: alias ?? null });
}
