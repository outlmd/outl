/**
 * `<DevicesSheet />` — the mobile sync / device-pairing screen.
 *
 * Presented as a bottom sheet (same drag-to-dismiss family as
 * `Calendar` / `PageSwitcher`). It owns the peer data-fetching and
 * the pairing flow; the *presentation* of the device rows and the QR
 * code is delegated to the shared `<PeerList />` / `<PairingQR />`
 * components from `@outl/shared/peers` so mobile and desktop render
 * pairing identically.
 *
 * Primary flow on mobile is **scan**:
 *
 *   tap "Scan QR" → camera (tauri-plugin-barcode-scanner) → decoded
 *   ticket → `peerPairJoin(ticket)` → refresh device list.
 *
 * A secondary "Show my QR" path calls `peerPairHost()` and renders the
 * ticket via `<PairingQR />` so a *second* device can scan *this* one;
 * it's optional chrome — the scan path is the one that must work.
 *
 * Everything that touches `peers.json` goes through the typed
 * `@outl/shared/api/commands` wrappers; this component never calls
 * `invoke()` directly.
 */

import { Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { Portal } from "solid-js/web";
// Static import (NOT `await import(...)`): on iOS the WKWebView custom scheme
// can't fetch the separate chunk Vite emits for a dynamic plugin import, so the
// lazy form failed with "Importing a module script failed" the moment the user
// tapped Scan. This file is mobile-only, so pulling the plugin into the main
// bundle costs the desktop nothing.
import {
  scan,
  Format,
  cancel,
  checkPermissions,
  requestPermissions,
} from "@tauri-apps/plugin-barcode-scanner";
import { hostname } from "@tauri-apps/plugin-os";

import {
  peerList,
  peerStatus,
  peerRemove,
  peerPairHost,
  peerPairJoin,
} from "@outl/shared/api/commands";
import type { PeerDto, PeerStatusDto } from "@outl/shared/api/types";
import { PeerList, PairingQR, SyncProgressView, createSyncProgress } from "@outl/shared/peers";

import { createSheetDrag } from "../lib/sheet-drag";
import { haptic } from "../lib/haptics";
import { Toast } from "./Toast";

interface DevicesSheetProps {
  open: boolean;
  onClose: () => void;
}

/** localStorage key for this device's advertised pairing name. Per-install
 *  (a presentation label, not workspace state that must converge), so it
 *  lives in localStorage, not the op log — same key the desktop uses. */
const DEVICE_NAME_KEY = "outl.deviceName";

/** Whether the user explicitly named this device. Only `updateDeviceName`
 *  ever writes the key, so its presence means "user-chosen". */
function userNamedDevice(): boolean {
  return localStorage.getItem(DEVICE_NAME_KEY) !== null;
}

/** The user's saved device name, or "" when unset — the field then shows
 *  the OS device name, resolved on mount. */
function loadDeviceName(): string {
  return localStorage.getItem(DEVICE_NAME_KEY) ?? "";
}

/** Best-effort OS device name (hostname) with a platform fallback.
 *  Desktop returns the machine name; iOS returns a generic model name
 *  (Apple hides the real device name since iOS 16), which still beats a
 *  flat "mobile". Falls back when not in a Tauri webview. */
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

/** Scan a QR with the device camera, returning the decoded string.
 *  Resolves `null` when the user cancels the scanner. Loaded lazily so
 *  the (mobile-only) plugin bindings never enter the desktop bundle and
 *  the import cost is paid only when the user actually taps "Scan". */
async function scanQrTicket(): Promise<string | null> {
  // iOS only opens the camera once permission is granted. Ask explicitly so a
  // first-run (or previously-denied) device gets the prompt instead of a
  // silently dead scanner.
  let perm = await checkPermissions();
  if (perm !== "granted") {
    perm = await requestPermissions();
  }
  if (perm !== "granted") {
    throw new Error("Camera access is off — enable it in Settings › outl › Camera.");
  }

  // `windowed: true` renders the live camera feed *behind* the webview. The
  // app paints an opaque themed background, which would hide the feed — so we
  // flag <html> for the scan's lifetime and `styles.css` turns the app
  // transparent and reveals the camera. The `.scan-overlay` (framing + Cancel)
  // is portalled to <body>, outside #root, so it stays visible over the feed.
  document.documentElement.classList.add("barcode-scanning");
  try {
    const result = await scan({ windowed: true, formats: [Format.QRCode] });
    return result?.content ?? null;
  } finally {
    // Revert the UI *first* so a slow/hung native cancel() can never leave a
    // frozen camera frame on screen. Then stop the camera session, time-boxed:
    // on iOS the windowed scanner's cancel() has been seen to hang, and an
    // un-bounded await here would wedge the whole pairing flow.
    document.documentElement.classList.remove("barcode-scanning");
    await Promise.race([
      cancel().catch(() => {}),
      new Promise((resolve) => setTimeout(resolve, 1500)),
    ]);
  }
}

/** Abort an in-flight scan from outside `scanQrTicket` (the overlay's Cancel
 *  button). `cancel()` makes the pending `scan()` resolve, unwinding the
 *  `handleScan` flow and clearing the transparent-background class. */
async function cancelActiveScan(): Promise<void> {
  await cancel().catch(() => {});
}

export function DevicesSheet(props: DevicesSheetProps) {
  const [peers, setPeers] = createSignal<PeerDto[]>([]);
  const [statusMap, setStatusMap] = createSignal<Map<string, PeerStatusDto>>(
    new Map(),
  );
  const [busy, setBusy] = createSignal(false);
  const [scanning, setScanning] = createSignal(false);
  const [hostTicket, setHostTicket] = createSignal<string | null>(null);
  const [toast, setToast] = createSignal<string | null>(null);
  const [deviceName, setDeviceName] = createSignal(loadDeviceName());
  // Live sync progress (snapshot %, ops counts, "page X synced" feed). Cosmetic;
  // subscribes to the `sync-progress` event, unsubscribes on cleanup.
  const sync = createSyncProgress();

  function updateDeviceName(value: string) {
    setDeviceName(value);
    localStorage.setItem(DEVICE_NAME_KEY, value);
  }

  const drag = createSheetDrag(() => props.onClose());

  /** Re-pull the device list and (best-effort) live status. The list is
   *  the source of truth for the rows; status is decorative, so a
   *  status probe failure never blanks the list. */
  async function refresh() {
    try {
      const list = await peerList();
      setPeers(list);
    } catch (e) {
      setToast(`Could not load devices: ${String(e)}`);
      return;
    }
    try {
      const statuses = await peerStatus();
      setStatusMap(new Map(statuses.map((s) => [s.node_id, s])));
    } catch {
      // Status is best-effort; leave the dots in "unknown".
      setStatusMap(new Map());
    }
  }

  // Pairing succeeded on the host side — refresh so the new device shows
  // up. (Mobile is usually the joiner, but if the user used "Show my QR"
  // this is how the row appears once the other device connects.)
  let unlistenPaired: (() => void) | undefined;
  let unlistenFailed: (() => void) | undefined;

  onMount(async () => {
    // Pre-fill the name field with the OS device name when the user
    // hasn't chosen one. Re-check after the async resolve so a fast edit
    // mid-resolve isn't clobbered.
    if (!userNamedDevice()) {
      const suggested = await resolveDeviceName("mobile");
      if (!userNamedDevice()) setDeviceName(suggested);
    }
    const { listen } = await import("@tauri-apps/api/event");
    unlistenPaired = await listen("peer-paired", () => {
      setHostTicket(null);
      void refresh();
      setToast("Device paired");
    });
    unlistenFailed = await listen<string>("peer-pair-failed", (ev) => {
      setHostTicket(null);
      setToast(`Pairing failed: ${ev.payload}`);
    });
  });

  onCleanup(() => {
    unlistenPaired?.();
    unlistenFailed?.();
  });

  // Side effects on the sheet's open/close edges: refresh the device
  // list when it opens, drop any in-flight host ticket when it closes
  // so the camera/QR isn't left up on the next open.
  let wasOpen = false;
  createEffect(() => {
    const open = props.open;
    if (open && !wasOpen) void refresh();
    if (!open && wasOpen) setHostTicket(null);
    wasOpen = open;
  });

  async function handleScan() {
    if (scanning() || busy()) return;
    haptic("medium");
    setScanning(true);

    let ticket: string | null;
    try {
      ticket = await scanQrTicket();
    } catch (e) {
      setToast(`Could not scan: ${String(e)}`);
      return;
    } finally {
      // Drop the scanner overlay the instant the camera closes — never hold it
      // up while the (possibly slow) pairing handshake + status probe run, or a
      // hung probe leaves the user staring at a frozen scanner (had to force
      // quit the app). Teardown of the camera is owned by `scanQrTicket`.
      setScanning(false);
    }
    if (!ticket) return; // user cancelled — fall back to the sheet, no toast

    setBusy(true);
    try {
      const peer = await peerPairJoin(ticket, deviceName().trim() || "mobile");
      haptic("success");
      setToast(`Paired with ${peer.alias ?? "device"}`);
      // The status probe dials peers over iroh with per-peer timeouts and can
      // take seconds; refresh in the background so the sheet returns to normal
      // immediately after pairing instead of blocking on the probe.
      void refresh();
    } catch (e) {
      haptic("warning");
      setToast(`Could not pair: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function handleShowMyQr() {
    if (busy()) return;
    haptic("light");
    setBusy(true);
    try {
      const ticket = await peerPairHost(deviceName().trim() || "mobile");
      setHostTicket(ticket);
    } catch (e) {
      setToast(`Could not start hosting: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  async function handleRemove(nodeId: string) {
    haptic("light");
    try {
      await peerRemove(nodeId);
      await refresh();
    } catch (e) {
      setToast(`Could not remove device: ${String(e)}`);
    }
  }

  return (
    <Show when={props.open}>
      {/* Full-screen scan overlay. Portalled to <body> so the
          `barcode-scanning` transparency rules (which hide #root) don't hide
          it — it floats over the live camera feed with framing + Cancel. */}
      <Show when={scanning()}>
        <Portal mount={document.body}>
          <div class="scan-overlay">
            <div class="scan-frame" />
            <p class="scan-hint">
              Point your camera at the QR on your other device
            </p>
            <button
              type="button"
              class="scan-cancel"
              onClick={() => void cancelActiveScan()}
            >
              Cancel
            </button>
          </div>
        </Portal>
      </Show>
      <div
        class="fixed inset-0 z-50 bg-black/40 backdrop-blur-md outl-fade-in"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-50 flex max-h-[88vh] flex-col overflow-hidden rounded-t-2xl bg-(--color-outl-bg)/85 shadow-2xl backdrop-blur-2xl backdrop-saturate-150"
        style={{
          "padding-bottom": "env(safe-area-inset-bottom)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <header class="flex items-center px-4 py-3">
          <span
            class="mx-auto block h-3 w-16 cursor-grab py-1 active:cursor-grabbing"
            style={{ "touch-action": "none" }}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onPointerCancel={drag.onPointerCancel}
            aria-label="Drag to close"
            role="button"
          >
            <span
              aria-hidden="true"
              class="mx-auto block h-1 w-10 rounded-full bg-(--color-outl-border)"
            />
          </span>
        </header>

        <div class="ios-scroll flex-1 overflow-y-auto px-5 pb-6">
          <h2 class="mb-1 text-[22px] font-bold">Devices</h2>
          <p class="mb-4 text-[14px] text-(--color-outl-fg-dim)">
            Pair this device with another to sync your workspace
            peer-to-peer.
          </p>

          {/* This device's advertised name — what the other device shows in
              its paired list. Defaults to "mobile", editable, remembered. */}
          <label class="mb-3 block">
            <span class="mb-1 block text-[13px] text-(--color-outl-fg-dim)">
              This device's name
            </span>
            <input
              type="text"
              value={deviceName()}
              onInput={(e) => updateDeviceName(e.currentTarget.value)}
              placeholder="mobile"
              autocapitalize="none"
              autocorrect="off"
              class="w-full rounded-xl border border-(--color-outl-border) bg-(--color-outl-bg-elev)/60 px-3 py-2.5 text-[16px] text-(--color-outl-fg) outline-none focus:border-(--color-outl-accent)"
            />
          </label>

          {/* Primary action: scan the host's QR. */}
          <button
            type="button"
            onClick={handleScan}
            disabled={scanning() || busy()}
            class="mb-3 flex w-full items-center justify-center gap-2 rounded-xl bg-(--color-outl-accent) px-4 py-3 text-[16px] font-semibold text-white active:opacity-80 disabled:opacity-50"
          >
            <svg
              width="20"
              height="20"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M3 7V5a2 2 0 0 1 2-2h2M17 3h2a2 2 0 0 1 2 2v2M21 17v2a2 2 0 0 1-2 2h-2M7 21H5a2 2 0 0 1-2-2v-2" />
              <rect x="7" y="7" width="10" height="10" rx="1" />
            </svg>
            {scanning() ? "Scanning…" : "Scan QR"}
          </button>

          {/* Secondary: show this device's QR so another can scan it. */}
          <button
            type="button"
            onClick={handleShowMyQr}
            disabled={busy() || scanning()}
            class="mb-5 w-full rounded-xl border border-(--color-outl-border) px-4 py-2.5 text-[15px] font-medium text-(--color-outl-accent) active:opacity-60 disabled:opacity-50"
          >
            {hostTicket() ? "Hide my QR" : "Show my QR"}
          </button>

          <Show when={hostTicket()}>
            {(ticket) => (
              <div class="mb-6 flex flex-col items-center gap-3">
                <div class="rounded-2xl bg-white p-4 shadow-sm">
                  <PairingQR ticket={ticket()} />
                </div>
                <p class="text-center text-[13px] text-(--color-outl-fg-dim)">
                  Scan this code from your other device. Waiting for a
                  connection…
                </p>
                <button
                  type="button"
                  onClick={() => setHostTicket(null)}
                  class="text-[14px] font-medium text-(--color-outl-accent) active:opacity-60"
                >
                  Cancel
                </button>
              </div>
            )}
          </Show>

          <Show when={sync.current() || sync.feed().length > 0}>
            <div class="mb-2 text-[13px] font-semibold uppercase tracking-wider text-(--color-outl-fg-dim)">
              Syncing
            </div>
            <div class="mb-4">
              <SyncProgressView current={sync.current()} feed={sync.feed()} peers={peers()} />
            </div>
          </Show>

          <div class="mb-2 text-[13px] font-semibold uppercase tracking-wider text-(--color-outl-fg-dim)">
            Paired devices
          </div>
          <PeerList
            peers={peers()}
            statusByNodeId={statusMap()}
            onRemove={handleRemove}
            emptyState={
              <div class="rounded-xl bg-(--color-outl-bg-elev)/60 px-4 py-6 text-center text-[14px] text-(--color-outl-fg-dim)">
                No paired devices yet. Scan a QR to add one.
              </div>
            }
          />
        </div>
      </div>

      <Toast message={toast()} onDismiss={() => setToast(null)} />
    </Show>
  );
}
