/**
 * Sync panel — paired-devices list + "pair a new device" host flow.
 *
 * Rendered as the "Sync" section of {@link SettingsModal}. The desktop
 * is always the **host** in a pairing handshake (it shows the QR); the
 * joining device (mobile, or another desktop) scans / types the ticket.
 * There is no camera path here.
 *
 * Data + refresh policy live in this client (the shared `<PeerList />`
 * and `<PairingQR />` are pure presentational components — see
 * `@outl/shared/peers`). This panel:
 *
 * - loads `peerList()` + `peerStatus()` on mount and on Refresh,
 * - starts a host session with `peerPairHost()` and renders the ticket
 *   that arrives via the `peer-pairing-ticket` event as a `<PairingQR />`
 *   + a copyable string, with a "waiting…" spinner,
 * - listens for `peer-paired` to flip to a success state and refresh the
 *   list,
 * - tolerates the pairing call rejecting (timeout / cancel / error).
 *
 * Backend contract note: the desktop `outl_peer_pair_host` emits the
 * ticket early via `peer-pairing-ticket` (so the QR can render while the
 * handshake blocks), then resolves once a peer connects. We drive the UI
 * off the **events**, not the resolved value, so this panel works whether
 * the Rust command resolves with the ticket (aligned/mobile shape) or the
 * paired peer (current desktop shape).
 */

import { Show, createSignal, onCleanup, onMount } from "solid-js";

import {
  peerList,
  peerPairHost,
  peerRemove,
  peerStatus,
  reloadWorkspace,
  syncNow,
} from "@outl/shared/api/commands";
import {
  PairingQR,
  PeerList,
  SyncProgressView,
  createSyncProgress,
  peersOnline,
} from "@outl/shared/peers";
import type { PeerDto, PeerStatusDto } from "@outl/shared/api/types";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { hostname } from "@tauri-apps/plugin-os";

import {
  onPeerPaired,
  onPeerPairFailed,
  onPeerPairingTicket,
} from "../lib/events";
import { setAppState } from "../lib/store";

/** Pairing sub-flow state machine. */
type PairState =
  | { phase: "idle" }
  | { phase: "starting" }
  | { phase: "waiting"; ticket: string }
  | { phase: "paired"; alias: string | null; nodeId: string }
  | { phase: "error"; message: string };

function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** localStorage key for this device's advertised pairing name. Per-install
 *  (a name is a presentation label, not workspace state that must converge),
 *  so it lives in localStorage, not the op log. */
const DEVICE_NAME_KEY = "outl.deviceName";

/** Whether the user explicitly named this device. Only `updateDeviceName`
 *  ever writes the key, so its presence means "user-chosen". */
function userNamedDevice(): boolean {
  return localStorage.getItem(DEVICE_NAME_KEY) !== null;
}

/** The user's saved device name, or "" when unset — the field then shows
 *  the OS machine name (hostname), resolved on mount. */
function loadDeviceName(): string {
  return localStorage.getItem(DEVICE_NAME_KEY) ?? "";
}

/** Best-effort machine name (hostname) with a fallback. On desktop this is
 *  the real machine name; falls back when not in a Tauri host. */
async function resolveDeviceName(fallback: string): Promise<string> {
  try {
    const h = await hostname();
    const clean = (h ?? "").replace(/\.(local|lan|home)$/i, "").trim();
    if (clean) return clean;
  } catch {
    // Plugin unavailable / not a Tauri host — use the fallback.
  }
  return fallback;
}

/** Short, human-glanceable prefix of a long hex node id. */
function shortNodeId(nodeId: string): string {
  return nodeId.length <= 12 ? nodeId : `${nodeId.slice(0, 12)}…`;
}

export function SyncPanel() {
  const [peers, setPeers] = createSignal<PeerDto[]>([]);
  const [statuses, setStatuses] = createSignal<Map<string, PeerStatusDto>>(
    new Map(),
  );
  const [loading, setLoading] = createSignal(false);
  const [pair, setPair] = createSignal<PairState>({ phase: "idle" });
  const [copied, setCopied] = createSignal(false);
  const [deviceName, setDeviceName] = createSignal(loadDeviceName());
  // Live sync progress (snapshot %, ops counts, "page X synced" feed). Cosmetic;
  // subscribes to the `sync-progress` event, unsubscribes on cleanup.
  const sync = createSyncProgress();

  function updateDeviceName(value: string) {
    setDeviceName(value);
    localStorage.setItem(DEVICE_NAME_KEY, value);
  }

  let unlistenTicket: UnlistenFn | undefined;
  let unlistenPaired: UnlistenFn | undefined;
  let unlistenFailed: UnlistenFn | undefined;

  /**
   * Reload the device list + status probe. The list comes back fast;
   * the status probe is an async iroh connect per peer, so we paint the
   * list first and let the dots upgrade from "unknown" when it lands.
   */
  async function refresh() {
    setLoading(true);
    try {
      const list = await peerList();
      setPeers(list);
      // Status probe can fail (no network / no peers) — that's not fatal,
      // the dots just stay "unknown".
      try {
        const probed = await peerStatus();
        setStatuses(new Map(probed.map((s) => [s.node_id, s])));
      } catch {
        setStatuses(new Map());
      }
    } catch (e) {
      setAppState("lastError", errorMessage(e));
    } finally {
      setLoading(false);
    }
  }

  /**
   * The "Refresh" button: force an immediate P2P pull (dial every iroh
   * peer now instead of waiting for the catch-up tick), reload the local
   * op log so the workspace re-renders with whatever the peers delivered,
   * then re-read the device list + health for the dots. Best-effort — a
   * `syncNow` / reload failure surfaces on the status line but never
   * blocks the status read.
   */
  async function forceSync() {
    setLoading(true);
    try {
      await syncNow();
      await reloadWorkspace();
    } catch (e) {
      setAppState("lastError", errorMessage(e));
    } finally {
      setLoading(false);
    }
    await refresh();
  }

  async function remove(nodeId: string) {
    try {
      await peerRemove(nodeId);
      await refresh();
    } catch (e) {
      setAppState("lastError", errorMessage(e));
    }
  }

  /**
   * Start hosting a pairing session. The ticket arrives via the
   * `peer-pairing-ticket` event (wired in `onMount`), which flips us to
   * the "waiting" phase. `peerPairHost()` resolves once a peer connects
   * (or rejects on timeout / error); the `peer-paired` event flips us to
   * "paired" first when it succeeds.
   */
  async function startPairing() {
    setCopied(false);
    setPair({ phase: "starting" });
    try {
      await peerPairHost(deviceName().trim() || "desktop");
      // Success is handled by the `peer-paired` listener; if we got here
      // without that firing (aligned backend that resolves with the
      // ticket and never emits `peer-paired`), keep the panel as-is.
    } catch (e) {
      // Cancel or timeout. Only surface if we're still mid-flow — a
      // `peer-paired` may have already landed and flipped us to success.
      const p = pair();
      if (p.phase === "starting" || p.phase === "waiting") {
        setPair({ phase: "error", message: errorMessage(e) });
      }
    }
  }

  function cancelPairing() {
    // The host endpoint tears down when the command future is dropped;
    // there's no explicit cancel command, so we just reset the UI. A
    // late `peer-paired` for an already-cancelled session is harmless
    // (it refreshes the list).
    setPair({ phase: "idle" });
    setCopied(false);
  }

  async function copyTicket(ticket: string) {
    try {
      await navigator.clipboard.writeText(ticket);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      setAppState("lastError", errorMessage(e));
    }
  }

  onMount(async () => {
    void refresh();
    // Pre-fill the name field with the machine name when the user hasn't
    // chosen one. Re-check after the async resolve so a fast edit isn't
    // clobbered.
    if (!userNamedDevice()) {
      const suggested = await resolveDeviceName("desktop");
      if (!userNamedDevice()) setDeviceName(suggested);
    }
    unlistenTicket = await onPeerPairingTicket(({ ticket }) => {
      setPair({ phase: "waiting", ticket });
    });
    unlistenPaired = await onPeerPaired((peer) => {
      setPair({ phase: "paired", alias: peer.alias, nodeId: peer.node_id });
      void refresh();
    });
    unlistenFailed = await onPeerPairFailed((message) => {
      const p = pair();
      if (p.phase === "starting" || p.phase === "waiting") {
        setPair({ phase: "error", message });
      }
    });
  });

  onCleanup(() => {
    unlistenTicket?.();
    unlistenPaired?.();
    unlistenFailed?.();
  });

  return (
    <div class="space-y-3 border-t border-(--color-outl-fg)/10 pt-4">
      <div class="flex items-center justify-between">
        <div>
          <div class="flex items-center gap-1.5 text-sm font-medium">
            {/* Mesh status dot, derived identically to mobile via the
                shared `peersOnline` helper: green when at least one iroh
                peer is reachable, orange when none are (no peers paired,
                or all unreachable). */}
            <span
              role="status"
              aria-label={
                peersOnline(statuses())
                  ? "Sync status: connected"
                  : "Sync status: no peers reachable"
              }
              title={
                peersOnline(statuses())
                  ? "A peer is reachable"
                  : "No peer reachable"
              }
              class="inline-block h-2 w-2 rounded-full"
              style={{
                background: peersOnline(statuses())
                  ? "var(--color-outl-accent-alt)"
                  : "var(--color-outl-warn)",
              }}
            />
            <span>Sync</span>
          </div>
          <div class="text-xs opacity-60">
            Paired devices share this workspace over the local network.
          </div>
        </div>
        <button
          type="button"
          onClick={() => void forceSync()}
          disabled={loading()}
          class="rounded border border-(--color-outl-fg)/15 px-2 py-1 text-xs hover:bg-(--color-outl-fg)/10 disabled:opacity-50"
        >
          {loading() ? "Refreshing…" : "Refresh"}
        </button>
      </div>

      {/* Live sync progress (snapshot %, ops counts, "page X synced" feed).
          Only shows while a pass is running or has recent activity. */}
      <Show when={sync.current() || sync.feed().length > 0}>
        <div class="rounded border border-(--color-outl-fg)/10 bg-(--color-outl-fg)/[0.03] px-3 py-2">
          <SyncProgressView current={sync.current()} feed={sync.feed()} peers={peers()} />
        </div>
      </Show>

      {/* Paired devices. The shared <PeerList /> is pure; we feed it the
          list + the status map and handle remove here. */}
      <div class="rounded border border-(--color-outl-fg)/10 bg-(--color-outl-fg)/[0.03] px-3 py-2">
        <PeerList
          peers={peers()}
          statusByNodeId={statuses()}
          onRemove={(nodeId) => void remove(nodeId)}
          emptyState={
            <div class="py-2 text-xs opacity-60">
              No paired devices yet. Pair one below.
            </div>
          }
        />
      </div>

      {/* Pairing flow. */}
      <Show
        when={pair().phase !== "idle"}
        fallback={
          <div class="flex flex-col gap-2">
            <label class="flex flex-col gap-1 text-xs opacity-70">
              <span>This device's name (shown on your other devices)</span>
              <input
                type="text"
                value={deviceName()}
                onInput={(e) => updateDeviceName(e.currentTarget.value)}
                placeholder="desktop"
                class="rounded border border-(--color-outl-fg)/15 bg-(--color-outl-fg)/[0.03] px-2 py-1 text-sm text-(--color-outl-fg) outline-none focus:border-(--color-outl-fg)/30"
              />
            </label>
            <button
              type="button"
              onClick={() => void startPairing()}
              class="self-start rounded bg-(--color-outl-fg)/15 px-3 py-1.5 text-sm font-medium hover:bg-(--color-outl-fg)/25"
            >
              Pair a new device…
            </button>
          </div>
        }
      >
        <div class="rounded border border-(--color-outl-fg)/15 bg-(--color-outl-fg)/[0.03] p-4">
          <Show when={pair().phase === "starting"}>
            <div class="flex items-center gap-2 text-sm opacity-70">
              <Spinner />
              <span>Starting pairing session…</span>
            </div>
          </Show>

          <Show when={pair().phase === "waiting"}>
            {(() => {
              const ticket = (pair() as { phase: "waiting"; ticket: string })
                .ticket;
              return (
                <div class="flex flex-col items-center gap-3">
                  <PairingQR ticket={ticket} />
                  <div class="flex items-center gap-2 text-sm opacity-70">
                    <Spinner />
                    <span>Waiting for device to scan…</span>
                  </div>
                  <div class="w-full">
                    <div class="mb-1 text-xs opacity-60">
                      Or type this ticket on the other device:
                    </div>
                    <div class="flex items-stretch gap-2">
                      <code class="min-w-0 flex-1 truncate rounded bg-(--color-outl-fg)/10 px-2 py-1 font-mono text-xs">
                        {ticket}
                      </code>
                      <button
                        type="button"
                        onClick={() => void copyTicket(ticket)}
                        class="shrink-0 rounded border border-(--color-outl-fg)/15 px-2 py-1 text-xs hover:bg-(--color-outl-fg)/10"
                      >
                        {copied() ? "Copied" : "Copy"}
                      </button>
                    </div>
                  </div>
                  <button
                    type="button"
                    onClick={cancelPairing}
                    class="text-xs opacity-60 hover:opacity-100"
                  >
                    Cancel
                  </button>
                </div>
              );
            })()}
          </Show>

          <Show when={pair().phase === "paired"}>
            {(() => {
              const p = pair() as {
                phase: "paired";
                alias: string | null;
                nodeId: string;
              };
              const label = p.alias ?? shortNodeId(p.nodeId);
              return (
                <div class="flex flex-col items-center gap-3 text-center">
                  <div class="text-2xl">✓</div>
                  <div class="text-sm font-medium">Paired with {label}</div>
                  <button
                    type="button"
                    onClick={() => setPair({ phase: "idle" })}
                    class="rounded border border-(--color-outl-fg)/15 px-3 py-1 text-sm hover:bg-(--color-outl-fg)/10"
                  >
                    Done
                  </button>
                </div>
              );
            })()}
          </Show>

          <Show when={pair().phase === "error"}>
            {(() => {
              const message = (pair() as { phase: "error"; message: string })
                .message;
              return (
                <div class="flex flex-col gap-3">
                  <div class="text-sm text-(--color-outl-warn)">
                    Pairing failed: {message}
                  </div>
                  <div class="flex gap-2">
                    <button
                      type="button"
                      onClick={() => void startPairing()}
                      class="rounded bg-(--color-outl-fg)/15 px-3 py-1 text-sm font-medium hover:bg-(--color-outl-fg)/25"
                    >
                      Try again
                    </button>
                    <button
                      type="button"
                      onClick={() => setPair({ phase: "idle" })}
                      class="rounded px-3 py-1 text-sm opacity-70 hover:opacity-100"
                    >
                      Dismiss
                    </button>
                  </div>
                </div>
              );
            })()}
          </Show>
        </div>
      </Show>
    </div>
  );
}

/** Tiny inline spinner (Tailwind animate-spin ring). */
function Spinner() {
  return (
    <span
      class="inline-block h-3.5 w-3.5 animate-spin rounded-full border-2 border-(--color-outl-fg)/30 border-t-(--color-outl-fg)/80"
      aria-hidden="true"
    />
  );
}
