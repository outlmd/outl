import { describe, expect, it } from "vitest";

import type { PeerStatusDto } from "../api/types";
import {
  SYNC_STATES,
  peersAfterFailedProbe,
  syncState,
  syncStateColor,
  syncStateLabel,
} from "./status";

const peer = (online: boolean): PeerStatusDto => ({
  node_id: "n1",
  alias: "iPhone",
  online,
  rtt_ms: null,
  pending_ops: null,
});

describe("syncState", () => {
  it("says nothing is known until the first probe answers", () => {
    expect(syncState({ syncing: false, peers: null })).toBe("checking");
    expect(syncState({ syncing: false, peers: [] })).toBe("offline");
  });

  it("is why mobile used to claim offline on launch", () => {
    // Desktop started at `null` (dim) and mobile at `false` (orange), so the
    // phone reported a verdict on a measurement nobody had taken yet.
    expect(syncState({ syncing: false, peers: null })).not.toBe("offline");
  });

  it("reads a reachable device as online", () => {
    expect(syncState({ syncing: false, peers: [peer(true)] })).toBe("online");
    expect(syncState({ syncing: false, peers: [peer(false)] })).toBe("offline");
  });

  it("takes the Map the desktop sync panel already holds", () => {
    const byId = new Map([["n1", peer(true)]]);
    expect(syncState({ syncing: false, peers: byId })).toBe("online");
  });

  it("lets a pass in flight win over everything", () => {
    expect(syncState({ syncing: true, peers: null })).toBe("syncing");
    expect(syncState({ syncing: true, peers: [peer(false)] })).toBe("syncing");
    expect(
      syncState({ syncing: true, peers: [peer(true)], deviceOnline: false }),
    ).toBe("syncing");
  });

  it("floors on the device's own radio, without inventing one", () => {
    // Mobile passes navigator.onLine; a client with nothing to report omits it
    // and must not be read as offline for staying quiet.
    expect(
      syncState({ syncing: false, peers: [peer(true)], deviceOnline: false }),
    ).toBe("offline");
    expect(syncState({ syncing: false, peers: [peer(true)] })).toBe("online");
    // No radio and no probe yet is still offline: no peer can be up.
    expect(syncState({ syncing: false, peers: null, deviceOnline: false })).toBe(
      "offline",
    );
  });
});

describe("syncStateLabel", () => {
  it("gives every state one sentence, written once", () => {
    const said = SYNC_STATES.map(syncStateLabel);
    expect(new Set(said).size).toBe(SYNC_STATES.length);
    for (const sentence of said) expect(sentence.length).toBeGreaterThan(0);
  });

  it("speaks of devices, which is the word the rest of the app uses", () => {
    expect(syncStateLabel("online")).toMatch(/device/i);
    expect(syncStateLabel("offline")).toMatch(/device/i);
    expect(syncStateLabel("online")).not.toMatch(/peer/i);
  });
});

describe("syncStateColor", () => {
  it("resolves to palette tokens, never a hex literal", () => {
    for (const state of SYNC_STATES) {
      expect(syncStateColor(state)).toMatch(/^var\(--color-outl-[a-z-]+\)$/);
    }
  });

  it("keeps the success hue for online and the warning hue for offline", () => {
    expect(syncStateColor("online")).toBe("var(--color-outl-accent-alt)");
    expect(syncStateColor("offline")).toBe("var(--color-outl-warn)");
    expect(syncStateColor("checking")).toBe("var(--color-outl-fg-dim)");
  });
});

describe("peersAfterFailedProbe", () => {
  it("keeps a verdict a probe never disproved", () => {
    const known = [peer(true)];
    expect(peersAfterFailedProbe(known)).toBe(known);
  });

  it("does not strand a client that has never had one on checking", () => {
    expect(peersAfterFailedProbe(null)).toEqual([]);
  });
});
