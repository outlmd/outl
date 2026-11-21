import { beforeEach, describe, expect, it, vi } from "vitest";

import type { PageView } from "@outl/shared/api/types";

/**
 * The inline `/` menu's own rows, and above all `/quote` — the first
 * caller `toggle_quote` has ever had.
 *
 * The command's body, its generated wrapper and its line in **both**
 * clients' `generate_handler!` shipped for releases with nothing
 * calling it, while `docs/desktop-ux.md` and `docs/mobile-ux.md` both
 * described their client as routing through it. So the assertion that
 * matters here is not "the reply is applied" but "the command is
 * called at all, with the block the slash was typed in".
 *
 * `toggleQuote` is mocked because the real one is a Tauri `invoke`,
 * and nothing about the prefix is decided on this side:
 * `outl_actions::quote::toggle_quote` owns both directions and the
 * `"> TODO foo"` normalisation, which `outl-actions`' own tests cover.
 */

vi.mock("@outl/shared/api/commands", () => ({
  instantiateTemplateAt: vi.fn(),
  toggleQuote: vi.fn(),
}));

import { instantiateTemplateAt, toggleQuote } from "@outl/shared/api/commands";

import {
  NATIVE_ASSET_PLUGIN_ID,
  NATIVE_BLOCK_PLUGIN_ID,
  NATIVE_TEMPLATE_PLUGIN_ID,
} from "../lib/slash-commands";
import {
  createNativeSlashOps,
  type NativeSlashOpsDeps,
} from "./outline-native-slash";

function makeView(): PageView {
  return {
    page: {
      id: "page-1",
      slug: "today",
      kind: "journal",
      title: "",
      icon: null,
    },
    outline: [],
    warnings: [],
    page_properties: [],
    backlinks: [],
  } as unknown as PageView;
}

async function handleError<T>(p: Promise<T>): Promise<T | undefined> {
  try {
    return await p;
  } catch {
    return undefined;
  }
}

function makeDeps(over: Partial<NativeSlashOpsDeps> = {}): NativeSlashOpsDeps {
  return {
    pageId: vi.fn(() => "page-1"),
    selectedBlockId: vi.fn(() => "blk-1"),
    applyView: vi.fn(),
    handleError,
    setLastError: vi.fn(),
    attachFile: vi.fn(async () => {}),
    ...over,
  };
}

describe("createNativeSlashOps", () => {
  // The module-level `vi.mock` factory is shared across the file, so
  // without this a `not.toHaveBeenCalled()` would read a call another
  // test made — the exact shape of a test that passes for the wrong
  // reason.
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("runs /quote against the block the slash was typed in", async () => {
    vi.mocked(toggleQuote).mockResolvedValue(makeView());
    const deps = makeDeps();
    const ops = createNativeSlashOps(deps);

    // The *before* state: nothing has called it, which is also what a
    // never-wired client looks like. Asserting only the after would
    // pass against a stub that fired on mount.
    expect(toggleQuote).not.toHaveBeenCalled();

    const owned = await ops.run(NATIVE_BLOCK_PLUGIN_ID, "quote");

    expect(owned).toBe(true);
    expect(toggleQuote).toHaveBeenCalledWith("page-1", "blk-1");
    expect(deps.applyView).toHaveBeenCalledTimes(1);
    expect(deps.setLastError).not.toHaveBeenCalled();
  });

  it("says what to do instead of quoting nothing when no block is selected", async () => {
    const deps = makeDeps({ selectedBlockId: vi.fn(() => null) });
    const ops = createNativeSlashOps(deps);

    expect(await ops.run(NATIVE_BLOCK_PLUGIN_ID, "quote")).toBe(true);

    expect(toggleQuote).not.toHaveBeenCalled();
    expect(deps.setLastError).toHaveBeenCalledWith("select a block to quote");
  });

  it("keeps the row rather than falling through when the command fails", async () => {
    vi.mocked(toggleQuote).mockRejectedValue(new Error("not in tree"));
    const deps = makeDeps();
    const ops = createNativeSlashOps(deps);

    // `true` even on a refusal: falling through would hand `/quote` to
    // `pluginRun`, which would then report "no such plugin" over the
    // real reason.
    expect(await ops.run(NATIVE_BLOCK_PLUGIN_ID, "quote")).toBe(true);
    expect(deps.applyView).not.toHaveBeenCalled();
  });

  it("owns an unknown command under its own sentinel without acting", async () => {
    const deps = makeDeps();
    const ops = createNativeSlashOps(deps);

    expect(await ops.run(NATIVE_BLOCK_PLUGIN_ID, "shout")).toBe(false);
    expect(toggleQuote).not.toHaveBeenCalled();
  });

  it("leaves a real plugin's command to the plugin path", async () => {
    const deps = makeDeps();
    const ops = createNativeSlashOps(deps);

    expect(await ops.run("@acme/stats", "stats")).toBe(false);
    expect(toggleQuote).not.toHaveBeenCalled();
    expect(instantiateTemplateAt).not.toHaveBeenCalled();
    expect(deps.attachFile).not.toHaveBeenCalled();
  });

  it("still routes /template and /upload after the extraction", async () => {
    vi.mocked(instantiateTemplateAt).mockResolvedValue(makeView());
    const deps = makeDeps();
    const ops = createNativeSlashOps(deps);

    expect(await ops.run(NATIVE_TEMPLATE_PLUGIN_ID, "meeting")).toBe(true);
    expect(instantiateTemplateAt).toHaveBeenCalledWith("meeting", "blk-1");

    expect(await ops.run(NATIVE_ASSET_PLUGIN_ID, "upload")).toBe(true);
    expect(deps.attachFile).toHaveBeenCalledTimes(1);
  });

  it("asks for a block before instantiating a template into nothing", async () => {
    const deps = makeDeps({ selectedBlockId: vi.fn(() => null) });
    const ops = createNativeSlashOps(deps);

    expect(await ops.run(NATIVE_TEMPLATE_PLUGIN_ID, "meeting")).toBe(true);
    expect(instantiateTemplateAt).not.toHaveBeenCalled();
    expect(deps.setLastError).toHaveBeenCalledWith(
      "select a block to insert a template",
    );
  });
});
