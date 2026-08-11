/**
 * `<SyncProgressView />` — pure presentational sync-progress panel for the
 * pairing screen, shared by mobile + desktop.
 *
 * Strictly **dumb**: it takes the current phase, the activity feed, and the
 * peer list (to resolve short node ids to friendly aliases) as data. It never
 * calls a Tauri command or subscribes to an event — the host runs
 * {@link createSyncProgress} (`@outl/shared/peers`) and passes the signals down,
 * exactly like `<PeerList />` takes `peerList()` results.
 *
 * Renders the current phase as a pill + (for `snapshot`) a real progress bar or
 * (for ops) a live count, then a feed of "device → what synced" lines with the
 * resolved page/journal slugs. Markup is class-named (`outl-syncprog__*`);
 * import `@outl/shared/peers/styles` for the neutral baseline.
 */

import { For, Show, type JSX } from "solid-js";

import type { PeerDto, SyncProgress } from "../api/types";
import type { SyncFeedEntry } from "./sync-progress";

interface SyncProgressViewProps {
  /** Latest phase (from `createSyncProgress().current()`). `null` = idle. */
  current: SyncProgress | null;
  /** Activity feed (from `createSyncProgress().feed()`), newest first. */
  feed: SyncFeedEntry[];
  /** Paired devices, to resolve a short node id to its alias. */
  peers: PeerDto[];
}

/** Short, human-glanceable prefix of a long hex node id. */
function shortNodeId(nodeId: string): string {
  return nodeId.length <= 10 ? nodeId : nodeId.slice(0, 10);
}

/** Friendly name for a peer's short node id, falling back to the short id. */
function aliasFor(peers: PeerDto[], shortId: string): string {
  const match = peers.find((p) => p.node_id.startsWith(shortId));
  return match?.alias ?? shortNodeId(shortId);
}

/** Bytes → a compact `8.4 MB` / `912 KB` label. */
function fmtBytes(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)} MB`;
  if (n >= 1_000) return `${Math.round(n / 1_000)} KB`;
  return `${n} B`;
}

/** Integer with thousands separators (locale-aware). */
function fmtCount(n: number): string {
  return n.toLocaleString();
}

/** One-line human label for the current phase (drives the pill + status text). */
function phaseLabel(p: SyncProgress, peers: PeerDto[]): string {
  const who = aliasFor(peers, p.peer);
  switch (p.phase) {
    case "connecting":
      return `Connecting to ${who}…`;
    case "snapshot":
      return `Downloading snapshot from ${who}`;
    case "asset":
      return `Downloading file from ${who}`;
    case "received-ops":
      return `Receiving ${fmtCount(p.count)} changes from ${who}`;
    case "pushed-ops":
      return `Sending ${fmtCount(p.count)} changes to ${who}`;
    case "synced":
      return "Synced";
    case "interrupted":
      return `${who} went away — will retry`;
    case "failed":
      return "Sync failed";
  }
}

/**
 * Coarse state used for styling the pill.
 *
 * `interrupted` is its own state, not an error: the peer was suspended
 * mid-exchange and the next pass re-sends. Folding it into `error` is what
 * made a locked phone read as a broken sync.
 */
function pillState(
  phase: SyncProgress["phase"],
): "running" | "done" | "error" | "interrupted" {
  if (phase === "synced") return "done";
  if (phase === "failed") return "error";
  if (phase === "interrupted") return "interrupted";
  return "running";
}

/** The trailing detail text for a feed line. */
function feedDetail(event: SyncProgress): string {
  switch (event.phase) {
    case "received-ops":
      return `${fmtCount(event.count)} ops`;
    case "pushed-ops":
      return `${fmtCount(event.count)} ops sent`;
    case "synced":
      return "synced";
    case "failed":
      return event.error;
    default:
      return "";
  }
}

export function SyncProgressView(props: SyncProgressViewProps): JSX.Element {
  const hasContent = () => props.current !== null || props.feed.length > 0;
  return (
    <Show when={hasContent()}>
      <div class="outl-syncprog">
        <Show when={props.current}>
          {(p) => (
            <div class="outl-syncprog__current" data-phase={p().phase}>
              <span class="outl-syncprog__pill" data-state={pillState(p().phase)}>
                {phaseLabel(p(), props.peers)}
              </span>
              <Show
                when={
                  p().phase === "snapshot" || p().phase === "asset"
                    ? (p() as Extract<SyncProgress, { phase: "snapshot" | "asset" }>)
                    : undefined
                }
              >
                {(snap) => {
                  const pct = () =>
                    snap().total > 0 ? Math.min(100, (snap().received / snap().total) * 100) : 0;
                  return (
                    <div class="outl-syncprog__bar-row">
                      <div
                        class="outl-syncprog__bar"
                        role="progressbar"
                        aria-valuenow={Math.round(pct())}
                      >
                        <i style={{ width: `${pct()}%` }} />
                      </div>
                      <span class="outl-syncprog__bytes">
                        {fmtBytes(snap().received)} / {fmtBytes(snap().total)}
                      </span>
                    </div>
                  );
                }}
              </Show>
            </div>
          )}
        </Show>

        <Show when={props.feed.length > 0}>
          <ul class="outl-syncprog__feed">
            <For each={props.feed}>
              {(entry) => (
                <li class="outl-syncprog__item" data-phase={entry.event.phase}>
                  <span class="outl-syncprog__from">{aliasFor(props.peers, entry.event.peer)}</span>
                  <span class="outl-syncprog__arrow" aria-hidden="true">
                    →
                  </span>
                  <Show
                    when={entry.pages.length > 0}
                    fallback={<span class="outl-syncprog__detail">{feedDetail(entry.event)}</span>}
                  >
                    <span class="outl-syncprog__pages">
                      <For each={entry.pages}>
                        {(slug) => <span class="outl-syncprog__page">{slug}</span>}
                      </For>
                    </span>
                  </Show>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </div>
    </Show>
  );
}
