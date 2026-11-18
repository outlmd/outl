import { createSignal, onCleanup, onMount } from "solid-js";

import type { PeerStatusDto } from "@outl/shared/api/types";
import {
  peerStatus,
  reloadWorkspace,
  syncNow,
} from "@outl/shared/api/commands";
import {
  SyncDot,
  peersAfterFailedProbe,
  syncState,
  syncStateLabel,
} from "@outl/shared/peers";

import { setAppState } from "../lib/store";
import { onPeerOpsChanged } from "../lib/events";

/**
 * Always-visible sync status dot for the desktop chrome cluster.
 *
 * It **is** the mobile dot: the state comes from `syncState` and the glyph
 * from `<SyncDot>`, both in `@outl/shared/peers`, so green / orange / dim /
 * spinner mean the same thing in both clients and are decided once.
 *
 * Clicking it syncs now: dial every peer, reload the op log so the page
 * re-renders with whatever arrived. That is the refresh the chrome used
 * to need a separate button for. Clicks while a pass is in flight are
 * ignored, so mashing the dot never stacks dials. Right-click opens
 * Settings → Sync, where the full peer list + pairing live
 * (`<SyncPanel>` is the Sync section of `<SettingsModal>`).
 *
 * The reachability read is the running transport's own evidence — the dials
 * it made and, since #575, the dials it answered — which is the SAME source
 * the Settings Sync panel uses, so the dot and the panel can never disagree.
 * We re-probe on a slow interval and immediately whenever a peer delivers ops
 * (`peer-ops-changed`): a delivery is proof the mesh is up, so the dot
 * should flip green without waiting for the next tick.
 */
export function SyncIndicator() {
  // `null` = the first probe has not answered yet (dim), never "offline".
  const [peers, setPeers] = createSignal<readonly PeerStatusDto[] | null>(null);
  const [syncing, setSyncing] = createSignal(false);
  // When this window last saw a sync land: a pass it ran finished, or a
  // peer delivered ops. `null` until the first one, the tooltip says so.
  const [lastSynced, setLastSynced] = createSignal<number | null>(null);
  // Ticks the "2 min ago" text forward while the tooltip may be read.
  const [now, setNow] = createSignal(Date.now());

  async function refresh() {
    try {
      setPeers(await peerStatus());
    } catch {
      // A probe that threw measured nothing. `peersAfterFailedProbe` owns
      // what to hold, for both clients.
      setPeers((prev) => peersAfterFailedProbe(prev));
    }
  }

  onMount(async () => {
    void refresh();
    // Re-probe on the catch-up cadence. Cheap because `peerStatus()`
    // defaults to leaving `pending_ops` out: reachability is the
    // transport's in-memory dial record, and the count is the only part
    // that reads the op log. This comment used to claim the whole call
    // was cheap, which is how a 38-100ms census ended up running every
    // 8s plus once per delivered op for a number this dot never renders.
    const id = setInterval(() => {
      setNow(Date.now());
      void refresh();
    }, 8000);
    onCleanup(() => clearInterval(id));
    const unlisten = await onPeerOpsChanged(() => {
      setLastSynced(Date.now());
      void refresh();
    });
    onCleanup(() => unlisten());
  });

  async function syncNowFromDot() {
    if (syncing()) return;
    setSyncing(true);
    try {
      await syncNow();
      await reloadWorkspace();
      setLastSynced(Date.now());
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    } finally {
      setSyncing(false);
    }
    await refresh();
  }

  const state = () => syncState({ syncing: syncing(), peers: peers() });

  function label(): string {
    return syncStateLabel(state());
  }

  function tooltip(): string {
    const last = lastSynced();
    const when =
      last === null ? "Not synced yet this session" : `Last synced ${ago(now() - last)}`;
    if (syncing()) return `${label()}\n${when}`;
    return `${label()}\n${when}\nClick to sync now, right-click for Sync settings`;
  }

  return (
    <button
      type="button"
      // The button says what pressing it does; `<SyncDot>` announces what
      // the state is, so a screen reader is not told both on every change.
      aria-label="Sync now"
      aria-busy={syncing()}
      title={tooltip()}
      onMouseEnter={() => setNow(Date.now())}
      onClick={() => void syncNowFromDot()}
      onContextMenu={(e) => {
        e.preventDefault();
        setAppState("settingsOpen", true);
      }}
      class="flex h-8 w-8 items-center justify-center rounded-full transition-colors hover:bg-(--color-outl-border)/40"
    >
      <SyncDot state={state()} />
    </button>
  );
}

/** "just now" / "3 min ago" / "2 h ago". Coarse on purpose: the tooltip
 *  answers "is this fresh?", not "at which second". */
export function ago(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
}
