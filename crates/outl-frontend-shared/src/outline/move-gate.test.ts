import { describe, expect, it } from "vitest";

import { createMoveGate } from "./move-gate";

describe("createMoveGate", () => {
  it("calls run synchronously", () => {
    const gate = createMoveGate();
    let ran = false;
    void gate(async () => {
      ran = true;
    });
    expect(ran).toBe(true);
  });

  it("drops a call while one is in flight and releases afterwards", async () => {
    const gate = createMoveGate();
    let release!: () => void;
    const first = gate(() => new Promise<void>((r) => (release = r)));
    let second = 0;
    await gate(async () => {
      second++;
    });
    expect(second).toBe(0);
    release();
    await first;
    await gate(async () => {
      second++;
    });
    expect(second).toBe(1);
  });

  it("releases when run rejects", async () => {
    const gate = createMoveGate();
    await expect(
      gate(async () => {
        throw new Error("boom");
      }),
    ).rejects.toThrow("boom");
    let ran = false;
    await gate(async () => {
      ran = true;
    });
    expect(ran).toBe(true);
  });
});
