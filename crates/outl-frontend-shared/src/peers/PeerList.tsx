/**
 * `<PeerList />` — pure presentational list of paired devices, shared by
 * mobile + desktop.
 *
 * Strictly **dumb**: it takes the peer rows and an optional status map
 * as data, and emits a `onRemove(nodeId)` callback. It never calls a
 * Tauri command — the host fetches `peerList()` / `peerStatus()` (from
 * `@outl/shared/api/commands`), holds them in its own store, and passes
 * them down here. That keeps the data-fetching + refresh policy in the
 * client (which owns the event listeners for `peer-paired`) and this
 * component reusable in tests with plain fixtures.
 *
 * Markup is minimal and class-named (`outl-peer-list__*`); import
 * `@outl/shared/peers/styles` for a neutral baseline each client can
 * override.
 */

import { For, Show, type JSX } from "solid-js";

import type { PeerDto, PeerStatusDto } from "../api/types";

interface PeerListProps {
  /** Paired devices, e.g. from `peerList()`. */
  peers: PeerDto[];
  /**
   * Live status per peer, keyed by `node_id`. Optional — when omitted
   * (or a peer is missing from the map) the status dot renders in an
   * "unknown" neutral state instead of online/offline. Build it from
   * `peerStatus()` results: `new Map(statuses.map(s => [s.node_id, s]))`.
   */
  statusByNodeId?: Map<string, PeerStatusDto>;
  /**
   * Invoked when the user clicks remove on a row, with the peer's full
   * `node_id`. The host calls `peerRemove(nodeId)` then refreshes its
   * list. Omit to hide the remove button (read-only list).
   *
   * When set, the list renders a note saying what removal actually
   * does. That note is not decoration: removal is per-device, and a
   * bare "Remove" button reads as "cut this device off", which is a
   * promise neither client can keep (issue #158). The CLI says the
   * same thing in words; saying it in only one of the two would just
   * move the misunderstanding.
   */
  onRemove?: (nodeId: string) => void;
  /** Rendered when `peers` is empty. Defaults to a neutral hint. */
  emptyState?: JSX.Element;
}

/** Short, human-glanceable prefix of a long hex node id. */
function shortNodeId(nodeId: string): string {
  return nodeId.length <= 12 ? nodeId : `${nodeId.slice(0, 12)}…`;
}

export function PeerList(props: PeerListProps): JSX.Element {
  return (
    <Show
      when={props.peers.length > 0}
      fallback={props.emptyState ?? <div class="outl-peer-list__empty">No paired devices yet</div>}
    >
      <ul class="outl-peer-list" data-peer-count={props.peers.length}>
        <For each={props.peers}>
          {(peer) => {
            const status = () => props.statusByNodeId?.get(peer.node_id);
            // "unknown" until a status probe lands; then online/offline.
            const dotState = () => {
              const s = status();
              if (!s) return "unknown";
              return s.online ? "online" : "offline";
            };
            return (
              <li class="outl-peer-list__item">
                <span
                  class="outl-peer-list__dot"
                  data-state={dotState()}
                  aria-label={`Status: ${dotState()}`}
                />
                <span class="outl-peer-list__body">
                  <span class="outl-peer-list__alias">{peer.alias ?? "Unnamed device"}</span>
                  <span class="outl-peer-list__node-id" title={peer.node_id}>
                    {shortNodeId(peer.node_id)}
                  </span>
                  <Show when={status()?.online && status()?.rtt_ms != null}>
                    <span class="outl-peer-list__rtt">{status()!.rtt_ms}ms</span>
                  </Show>
                </span>
                <Show when={props.onRemove}>
                  <button
                    type="button"
                    class="outl-peer-list__remove"
                    aria-label={`Remove ${peer.alias ?? shortNodeId(peer.node_id)}`}
                    onClick={() => props.onRemove?.(peer.node_id)}
                  >
                    Remove
                  </button>
                </Show>
              </li>
            );
          }}
        </For>
      </ul>
      <Show when={props.onRemove}>
        <p class="outl-peer-list__scope-note">
          Removing a device only stops <em>this</em> device syncing with it. Your
          other devices keep their own list — remove it there too. It also keeps
          the copy of your notes it already has.
        </p>
      </Show>
    </Show>
  );
}
