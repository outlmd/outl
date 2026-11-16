/**
 * Pairing + peer primitives shared by every GUI client.
 *
 * Components ({@link PairingQR}, {@link PeerList}) are pure / stateless —
 * data + callbacks flow through props, no Tauri command runs inside.
 * The typed `peer*` command wrappers live in `@outl/shared/api/commands`;
 * the `PeerDto` / `PeerStatusDto` DTOs in `@outl/shared/api/types`.
 *
 * Import the baseline chrome with `@import "@outl/shared/peers/styles"`.
 */

export { PairingQR } from "./PairingQR";
export { PeerList } from "./PeerList";
export { ticketToSvg, type TicketQrOptions } from "./qr";
export { peersAfterFailedProbe, peersOnline } from "./status";
export {
  SYNC_STATES,
  syncState,
  syncStateColor,
  syncStateLabel,
  type SyncState,
  type SyncStateInput,
} from "./status";
export { SyncDot } from "./SyncDot";
export { SyncProgressView } from "./SyncProgressView";
export { createSyncProgress, type SyncFeedEntry, type SyncProgressState } from "./sync-progress";
