/**
 * The one owner of OS clipboard I/O on the desktop.
 *
 * Two properties nothing upstream of this module can see:
 *
 * - **A read waits for every write queued before it.** A yank is an IPC
 *   round-trip (`copy_markdown`) followed by a write; a `p` pressed
 *   while the first half is in flight must paste the new markdown, not
 *   whatever the clipboard held before the `yy`.
 * - **A failure reaches the caller.** The old writes ended in
 *   `.catch(() => {})`, so a refused write read as a copy that worked.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const plugin = vi.hoisted(() => ({
  readText: vi.fn<() => Promise<string>>(),
  writeText: vi.fn<(text: string) => Promise<void>>(),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => plugin);

import { readOsClipboard, writeOsClipboard } from "./os-clipboard";

/** A promise the test settles by hand. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

describe("os-clipboard", () => {
  let osClipboard = "";

  beforeEach(() => {
    vi.clearAllMocks();
    osClipboard = "old";
    plugin.writeText.mockImplementation(async (t) => {
      osClipboard = t;
    });
    plugin.readText.mockImplementation(async () => osClipboard);
  });

  it("a read waits for a write whose text is still being computed", async () => {
    const md = deferred<string | undefined>();
    const written = writeOsClipboard(md.promise);
    const read = readOsClipboard();

    md.resolve("- new\n");
    await written;
    await expect(read).resolves.toBe("- new\n");
  });

  it("writes land in the order they were asked for", async () => {
    const first = deferred<string | undefined>();
    const a = writeOsClipboard(first.promise);
    const b = writeOsClipboard("second");

    first.resolve("first");
    await Promise.all([a, b]);
    expect(plugin.writeText.mock.calls.map((c) => c[0])).toEqual([
      "first",
      "second",
    ]);
    expect(osClipboard).toBe("second");
  });

  it("a text that resolved to nothing writes nothing", async () => {
    await writeOsClipboard(Promise.resolve(undefined));
    expect(plugin.writeText).not.toHaveBeenCalled();
  });

  it("a refused write rejects, and does not poison the next one", async () => {
    plugin.writeText.mockRejectedValueOnce("denied by capability");
    await expect(writeOsClipboard("x")).rejects.toThrow(/denied by capability/);

    await writeOsClipboard("y");
    expect(osClipboard).toBe("y");
  });

  it("an empty or image-only clipboard reads as 'Clipboard has no text'", async () => {
    plugin.readText.mockRejectedValueOnce("The clipboard contents were not available");
    await expect(readOsClipboard()).rejects.toThrow("Clipboard has no text");

    plugin.readText.mockResolvedValueOnce("");
    await expect(readOsClipboard()).rejects.toThrow("Clipboard has no text");
  });

  it("a read still runs after an earlier write failed", async () => {
    plugin.writeText.mockRejectedValueOnce(new Error("nope"));
    await writeOsClipboard("x").catch(() => {});
    await expect(readOsClipboard()).resolves.toBe("old");
  });
});
