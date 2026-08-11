/**
 * Shared sync-progress state for the pairing screen.
 *
 * `createSyncProgress()` subscribes to the `sync-progress` Tauri event (emitted
 * by the iroh transport bridge in `outl-tauri-shared`), keeps the latest phase
 * for the progress bar/pill, and accumulates a short activity feed. It resolves
 * the block ids a `received-ops` event carries to page/journal slugs via
 * `resolvePageLabels`, so the feed can name what synced ("MacBook → journals/…").
 *
 * Both GUI clients (`DevicesSheet` mobile, `SyncPanel` desktop) call this and
 * feed the result to the pure {@link SyncProgressView}, so the behaviour can't
 * drift. Cosmetic only — the load-bearing reload signal is a separate event.
 */

import { listen } from "@tauri-apps/api/event";
import { createSignal, onCleanup } from "solid-js";

import { resolvePageLabels } from "../api/commands";
import type { SyncProgress } from "../api/types";

/** One line in the activity feed: the raw event plus any resolved page slugs. */
export interface SyncFeedEntry {
  /** Monotonic id (list key + resolution target). */
  id: number;
  /** The progress event this line was built from. */
  event: SyncProgress;
  /** Page/journal slugs this batch touched (resolved async; may fill in late). */
  pages: string[];
}

/** Reactive handles returned by {@link createSyncProgress}. */
export interface SyncProgressState {
  /** Latest phase — drives the bar (snapshot %) / pill / live count. */
  current: () => SyncProgress | null;
  /** Recent "milestone" events (ops received/pushed, synced, failed), newest first. */
  feed: () => SyncFeedEntry[];
}

/** How many feed lines to keep. Older ones fall off. */
const FEED_CAP = 24;

/**
 * Subscribe to `sync-progress` and expose the reactive state. Must run inside a
 * Solid reactive root (component body) so `onCleanup` unsubscribes the listener.
 */
export function createSyncProgress(): SyncProgressState {
  const [current, setCurrent] = createSignal<SyncProgress | null>(null);
  const [feed, setFeed] = createSignal<SyncFeedEntry[]>([]);
  let nextId = 1;

  const pushEntry = (event: SyncProgress): number => {
    const id = nextId++;
    setFeed((f) => [{ id, event, pages: [] }, ...f].slice(0, FEED_CAP));
    return id;
  };

  const unlisten = listen<SyncProgress>("sync-progress", (e) => {
    const p = e.payload;
    setCurrent(p);
    switch (p.phase) {
      case "received-ops": {
        const id = pushEntry(p);
        // Name the pages this batch touched. Best-effort + async: an id not yet
        // materialized resolves to nothing and the line just keeps its count.
        if (p.nodes.length > 0) {
          resolvePageLabels(p.nodes)
            .then((pages) => {
              if (pages.length === 0) return;
              setFeed((f) =>
                f.map((entry) => (entry.id === id ? { ...entry, pages } : entry)),
              );
            })
            .catch(() => {});
        }
        break;
      }
      case "pushed-ops":
      case "synced":
      case "failed":
        pushEntry(p);
        break;
      // `connecting` + `snapshot` + `asset` update `current` only (spinner /
      // progress bar); flooding the feed with per-chunk byte ticks would bury it.
      //
      // `interrupted` is deliberately in that group too. It fires every time a
      // paired phone locks its screen, so feeding it would fill the panel with
      // permanent rows about a sync that is working — the exact noise this
      // phase was split off `failed` to stop.
    }
  });

  onCleanup(() => {
    unlisten.then((off) => off()).catch(() => {});
  });

  return {
    current,
    feed,
  };
}
