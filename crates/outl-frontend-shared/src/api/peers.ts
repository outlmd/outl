/**
 * Peering and sync-progress wire shapes.
 *
 * Split out of `./types` as a size boundary, not a contract boundary: it
 * is re-exported from there, so `@outl/shared/api/types` stays the one
 * import path every client already uses. Same arrangement `./plugins`,
 * `./tokens` and `./table` have.
 *
 * Every shape here mirrors a `serde`-serialized Rust type and is pinned
 * by `outl-tauri-shared/tests/` — this file is in
 * `ts_parser::MIRROR_FILES`, which is what makes the pin see it.
 */

/**
 * A paired device, as returned by `outl_peer_list` and
 * `outl_peer_pair_join`. Mirrors the Rust `PeerDto` in both clients'
 * `src-tauri/src/commands/peers.rs` (and `outl_sync_iroh::PeerEntry`).
 *
 * - `node_id` is the iroh node id (hex), the stable identity of the
 *   remote device. Clients show a short prefix; `outl_peer_remove`
 *   matches on a prefix of this.
 * - `alias` is the optional human label set during pairing (`null`
 *   when the user didn't name the device).
 * - `added_at` is an RFC3339 timestamp string of when the pairing was
 *   persisted to `peers.json`.
 */
export interface PeerDto {
  node_id: string;
  alias: string | null;
  added_at: string;
}

/**
 * A paired peer's live reachability, as returned by `outl_peer_status`.
 * Mirrors the Rust `PeerStatusDto` in both clients' `commands/peers.rs`.
 * On GUI clients this reads the running transport's `peer_health()` snapshot
 * (populated by the transport's own boot / catch-up / gossip dials), NOT a
 * transient probe endpoint — a second endpoint with the device identity would
 * hijack the relay route (see outl-sync-iroh "One endpoint per identity,
 * elected not assigned").
 * Only the CLI `outl peer status`, which has no running transport, probes.
 *
 * - `online`: reachable now. `rtt_ms`: last round trip in ms, `null` offline.
 * - `pending_ops`: ops the peer hasn't confirmed, `null` before any exchange.
 */
export interface PeerStatusDto {
  node_id: string;
  alias: string | null;
  online: boolean;
  rtt_ms: number | null;
  pending_ops: number | null;
}

/**
 * A sync-progress update pushed on the `sync-progress` Tauri event while a
 * pass runs. Mirrors the Rust `outl_actions::SyncProgress` enum (serialized
 * with an internal `phase` tag, kebab-case). Purely informational — it drives
 * the pairing-screen progress feed; the load-bearing reload signal is separate.
 *
 * `peer` is the peer's short node id; the UI resolves it to a friendly alias
 * against the peer list it already holds. `snapshot` and `asset` are the phases
 * with an honest percentage (`received` / `total` in bytes, total known up
 * front — for `asset`, the bytes of the file currently transferring); op counts
 * surface as a live number, not a bar. `received-ops.nodes` carries the (capped)
 * block ids touched, resolved to page slugs via `resolvePageLabels`; empty on a
 * bulk pass (the initial pair).
 */
export type SyncProgress =
  | { phase: "connecting"; peer: string }
  | { phase: "snapshot"; peer: string; received: number; total: number }
  | { phase: "asset"; peer: string; received: number; total: number }
  | { phase: "received-ops"; peer: string; count: number; nodes: string[] }
  | { phase: "pushed-ops"; peer: string; count: number }
  | { phase: "synced"; peer: string }
  /**
   * The exchange was cut off before the peer confirmed, and this device will
   * retry on its next pass. A phone that locked its screen, a laptop that
   * slept, a dropped carrier-NAT flow — expected, transient, not a defect.
   *
   * Kept apart from `failed` because a responder only confirms durable ingest
   * by closing cleanly, so a suspended peer produces an unconfirmed pass every
   * single time; rendering that red made locking your phone look like breakage.
   * Styled amber and deliberately NOT pushed into the feed — it updates
   * `current` only, the way `connecting` does.
   */
  | { phase: "interrupted"; peer: string; reason: string }
  | { phase: "failed"; peer: string; error: string };
