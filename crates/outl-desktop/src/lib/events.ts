/**
 * Tauri event listeners for the desktop client.
 *
 * - `workspace-ready` — emitted by the Rust backend after
 *   [`setWorkspace`](./api.ts) finishes opening a directory (and
 *   after the boot-time background opener completes when
 *   `settings.last_workspace` is already set).
 * - `peer-ops-changed` — fired by the cross-platform FS watcher
 *   (`fs_watcher.rs`) when a peer writes its `ops-*.jsonl` or an
 *   external editor touches a `.md` under the workspace. Debounced
 *   to ~100ms by the backend, so the listener can call
 *   `reload_workspace` straight through without extra throttling.
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ProjectionWriteFailed } from "@outl/shared/api/types";

/**
 * Register a handler for the `workspace-ready` event. Returns an
 * unlisten function — call it on component cleanup to avoid leaks.
 */
export function onWorkspaceReady(handler: () => void): Promise<UnlistenFn> {
  return listen("workspace-ready", () => handler());
}

/**
 * Register a handler for the `peer-ops-changed` event. Returns an
 * unlisten function — call it on component cleanup to avoid leaks.
 */
export function onPeerOpsChanged(handler: () => void): Promise<UnlistenFn> {
  return listen("peer-ops-changed", () => handler());
}

/** Report a projection failure without turning its committed mutation into an error. */
export function onProjectionWriteFailed(
  handler: (payload: ProjectionWriteFailed) => void,
): Promise<UnlistenFn> {
  return listen<ProjectionWriteFailed>("projection-write-failed", (event) =>
    handler(event.payload),
  );
}

/**
 * Payload emitted with `ref-projection-failed` when `open_ref` resolved
 * the target (the page now lives in the op log) but writing the
 * resulting `pages/<slug>.md` + sidecar failed. The op log is still
 * the source of truth; the next save / orphan scanner retries the
 * projection. The frontend surfaces a toast so the user knows the
 * link they just inserted may not be visible to peers yet.
 */
export interface RefProjectionFailedPayload {
  target: string;
  error: string;
}

/**
 * Register a handler for the `ref-projection-failed` event.
 */
export function onRefProjectionFailed(
  handler: (payload: RefProjectionFailedPayload) => void,
): Promise<UnlistenFn> {
  return listen<RefProjectionFailedPayload>("ref-projection-failed", (e) =>
    handler(e.payload),
  );
}

/**
 * Payload emitted with `deep-link://navigate` when the backend receives
 * an `outl://` URL (issue #98). The Rust side parses the URL through the
 * shared `outl_actions::parse_deep_link` (so the scheme contract has one
 * owner) and emits one of three shapes the frontend maps onto its
 * existing `open*` commands:
 *
 * - `{ kind: "today" }` — open today's journal.
 * - `{ kind: "daily", date }` — open the journal for that ISO date.
 * - `{ kind: "page", slug }` — open the page by slug.
 */
export type DeepLinkNavigate =
  | { kind: "today" }
  | { kind: "daily"; date: string }
  | { kind: "page"; slug: string };

/**
 * Register a handler for `deep-link://navigate`. Fires when the user
 * opens an `outl://` URL while the app is running (warm start). Returns
 * an unlisten function — call it on cleanup to avoid leaks.
 */
export function onDeepLinkNavigate(
  handler: (payload: DeepLinkNavigate) => void,
): Promise<UnlistenFn> {
  return listen<DeepLinkNavigate>("deep-link://navigate", (e) =>
    handler(e.payload),
  );
}

// ---------------------------------------------------------------------------
// Peer pairing (iroh sync transport)
// ---------------------------------------------------------------------------
//
// The host's `outl_peer_pair_host` command (`commands/peers.rs`) emits the
// pairing ticket **early** — the moment the transient iroh endpoint binds,
// before it blocks on the inbound connection — via `peer-pairing-ticket`.
// Once a peer completes the handshake and is persisted to the workspace's
// `.outl/peers.json`, the backend emits `peer-paired` (a `PeerDto`).
//
// The Sync panel listens to both: `peer-pairing-ticket` to render the QR
// + copyable ticket while we wait, and `peer-paired` to flip to the
// "paired" state and refresh the device list.

import type { PeerDto } from "@outl/shared/api/types";

/** Payload for the early `peer-pairing-ticket` event (`{ ticket }`). */
export interface PeerPairingTicketPayload {
  ticket: string;
}

/**
 * Register a handler for `peer-pairing-ticket`. Fires once per
 * `outl_peer_pair_host` call, before the handshake completes, carrying
 * the ticket the joining device scans / types. Returns an unlisten fn.
 */
export function onPeerPairingTicket(
  handler: (payload: PeerPairingTicketPayload) => void,
): Promise<UnlistenFn> {
  return listen<PeerPairingTicketPayload>("peer-pairing-ticket", (e) =>
    handler(e.payload),
  );
}

/**
 * Register a handler for `peer-paired`. Fires after a pairing handshake
 * completes and the peer is persisted (payload: the new `PeerDto`).
 * Returns an unlisten fn.
 */
export function onPeerPaired(
  handler: (peer: PeerDto) => void,
): Promise<UnlistenFn> {
  return listen<PeerDto>("peer-paired", (e) => handler(e.payload));
}

/**
 * Register a handler for `peer-pair-failed`. Fires if the host
 * handshake times out or errors (payload: an error string). Returns an
 * unlisten fn.
 *
 * The current backend resolves/rejects the `outl_peer_pair_host`
 * promise on failure rather than emitting this event, so the panel
 * relies on the rejected promise too; the listener is here for the
 * aligned-backend path where the long-running host blocks and surfaces
 * failures out-of-band.
 */
export function onPeerPairFailed(
  handler: (error: string) => void,
): Promise<UnlistenFn> {
  return listen<string>("peer-pair-failed", (e) => handler(e.payload));
}
