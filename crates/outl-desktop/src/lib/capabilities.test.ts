/**
 * The main window's capability grants the clipboard in both directions.
 *
 * `lib/os-clipboard.ts` reads and writes through
 * `@tauri-apps/plugin-clipboard-manager`, and a permission missing here
 * fails only at runtime, on a real window: the plugin call rejects, and
 * `yy` / `p` report an error on a build every test passed. This client
 * shipped exactly that with `allow-read-text` alone.
 */
import { describe, expect, it } from "vitest";

import capability from "../../src-tauri/capabilities/default.json";

describe("capabilities/default.json", () => {
  it("grants the clipboard plugin both read and write of text", () => {
    const granted = capability.permissions.filter(
      (p): p is string => typeof p === "string",
    );
    expect(granted).toContain("clipboard-manager:allow-read-text");
    expect(granted).toContain("clipboard-manager:allow-write-text");
  });

  it("is scoped to the main window, where the clipboard is used", () => {
    expect(capability.windows).toEqual(["main"]);
  });
});
