/**
 * Pure derivations over peer reachability, shared by every GUI client so
 * the sync indicator reads identically on mobile and desktop.
 */

import type { PeerStatusDto } from "../api/types";

/**
 * `true` when at least one paired peer is currently reachable over iroh.
 *
 * This is the single source of truth for "is the P2P mesh up right now?"
 * that both clients feed into their sync dot: a green/synced dot means
 * `peersOnline(statuses) === true`. When it's `false` there is nothing to
 * sync with (no peers paired, or every paired peer is unreachable), so the
 * dot reads offline regardless of the device's own internet connection —
 * `navigator.onLine` says the phone has WiFi, not that a peer answered.
 *
 * Accepts either the raw `peerStatus()` array or the `Map<node_id, …>`
 * the desktop's `SyncPanel` already builds, so neither client has to
 * reshape its data to call it.
 */
export function peersOnline(
  statuses: readonly PeerStatusDto[] | ReadonlyMap<string, PeerStatusDto>,
): boolean {
  const list = statuses instanceof Map ? [...statuses.values()] : (statuses as readonly PeerStatusDto[]);
  return list.some((s) => s.online === true);
}

/**
 * What a peer still has to receive from this device, as a short label.
 * `null` before the first exchange this session: the panel shows nothing
 * rather than a number nobody measured.
 */
export function pendingLabel(s: PeerStatusDto | undefined): string | null {
  const n = s?.pending_ops;
  if (n == null) return null;
  if (n === 0) return "Up to date";
  return `${n} ${n === 1 ? "change" : "changes"} waiting`;
}

/**
 * What a sync indicator is showing right now.
 *
 * `checking` is the honest answer before the first probe lands, and it is a
 * state rather than a default because the two clients used to disagree about
 * it: the desktop started at `null` (a dim dot) and mobile at `false` (an
 * orange one), so a phone reported "offline" over a measurement nobody had
 * taken yet.
 */
export type SyncState = "checking" | "online" | "offline" | "syncing";

/** Every state, so a test (or a legend) can walk them without listing them. */
export const SYNC_STATES: readonly SyncState[] = [
  "checking",
  "online",
  "offline",
  "syncing",
];

/** What a client knows when it asks what to paint. */
export interface SyncStateInput {
  /** A pass this client started is still running. */
  syncing: boolean;
  /** The last `peerStatus()` answer, or `null` before the first one. */
  peers: readonly PeerStatusDto[] | ReadonlyMap<string, PeerStatusDto> | null;
  /**
   * The device's own radio, as a floor: with no network at all no device can
   * be reachable, whatever the last probe said. Mobile passes
   * `navigator.onLine`; a client with nothing to report omits it and is not
   * read as offline for staying quiet.
   */
  deviceOnline?: boolean;
}

/**
 * The single owner of what a sync dot is showing, on every client.
 *
 * `peersOnline` was already shared and the state built on top of it was not,
 * so the desktop and mobile each derived their own and had already drifted on
 * the first-probe case above ([#575](https://github.com/outlmd/outl/issues/575)).
 * The reachability it reads is the running transport's own evidence, in both
 * directions: the dials this device made, and the dials it answered.
 */
export function syncState(input: SyncStateInput): SyncState {
  // A pass in flight is the one thing the user started, so it outranks a
  // reachability snapshot taken before it.
  if (input.syncing) return "syncing";
  if (input.deviceOnline === false) return "offline";
  if (input.peers === null) return "checking";
  return peersOnline(input.peers) ? "online" : "offline";
}

const SYNC_STATE_LABEL: Record<SyncState, string> = {
  checking: "Sync: checking for devices…",
  online: "Sync: a device is reachable",
  offline: "Sync: no device reachable",
  syncing: "Syncing…",
};

/**
 * The sentence for a state, written once.
 *
 * "Device" and not "peer": the user pairs devices, opens "Devices and sync",
 * and never meets the word actor or peer anywhere else in the app.
 */
export function syncStateLabel(state: SyncState): string {
  return SYNC_STATE_LABEL[state];
}

const SYNC_STATE_COLOR: Record<SyncState, string> = {
  // `accent_alt` is the palette's success hue and `warn` its warning one. A
  // hex here would be a second definition of a colour with no `Palette` field
  // behind it (root CLAUDE.md invariant 13).
  checking: "var(--color-outl-fg-dim)",
  online: "var(--color-outl-accent-alt)",
  offline: "var(--color-outl-warn)",
  // The spinner paints its own border in the accent; this is what a client
  // falls back to if it draws the state as a plain dot.
  syncing: "var(--color-outl-accent)",
};

/** The palette token a state paints with, picked once for every client. */
export function syncStateColor(state: SyncState): string {
  return SYNC_STATE_COLOR[state];
}

/**
 * What to hold when a `peerStatus()` probe throws.
 *
 * The clients answered this differently and neither answer was written down:
 * the desktop read a failed probe as offline ("so the dot stays meaningful"),
 * mobile kept the previous value ("the next tick retries"). Both are half
 * right. A probe that failed measured nothing, so it must not *change* a
 * verdict we already have — and it must not leave a client that has never had
 * one stuck on "checking…" forever either.
 *
 * So: keep what we had, and fall back to "nothing reachable" only when there
 * was nothing.
 */
export function peersAfterFailedProbe(
  previous: readonly PeerStatusDto[] | null,
): readonly PeerStatusDto[] {
  return previous ?? [];
}
