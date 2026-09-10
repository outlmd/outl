import { describe, expect, it } from "vitest";
import { MFU_STORAGE_KEY } from "./mfu";
import { LOCK_STORAGE_KEY } from "./lock";

/**
 * Both keys are a cross-language contract: `OutlToolbar.swift`
 * interpolates them into the JS it evaluates to read the iOS bar's
 * counts and lock out of this `localStorage`.
 *
 * Nothing fails at build time when one side is renamed. The bar simply
 * stops seeing the user's taps and their lock, silently, which is the
 * exact failure these two lines exist to make loud. Every other test
 * in this directory uses the constant, so only a literal catches it.
 *
 * The Swift half is `ToolbarStoreTests.testStorageKeysMatchTheTypeScriptContract`.
 */
describe("toolbar storage keys — cross-language contract", () => {
  it("pins the MFU counts key", () => {
    expect(MFU_STORAGE_KEY).toBe("outl.toolbar.mfu.v1");
  });

  it("pins the order-lock key", () => {
    expect(LOCK_STORAGE_KEY).toBe("outl.toolbar.lock.v1");
  });
});
