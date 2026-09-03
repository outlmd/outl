import { describe, expect, it } from "vitest";

import { pickSide } from "./install";

describe("pickSide", () => {
  it("follows the OS in auto", () => {
    expect(pickSide("auto", true)).toBe("light");
    expect(pickSide("auto", false)).toBe("dark");
  });

  it("ignores the OS when the mode is explicit", () => {
    expect(pickSide("light", false)).toBe("light");
    expect(pickSide("dark", true)).toBe("dark");
  });
});
