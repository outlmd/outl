/**
 * The desktop half of rename / merge: which question gets asked, with
 * whose words, and what runs after the answer.
 *
 * The order of events and every sentence belong to
 * `@outl/shared/pages/rename-merge`, so those are imported real and
 * asserted against — a copy here is the drift that module exists to
 * prevent. Only the bridge is faked.
 */

import { afterAll, afterEach, describe, expect, it, vi } from "vitest";

import { invoke } from "@tauri-apps/api/core";
import type { PageMeta } from "@outl/shared/api/types";
import {
  mergeConfirmMessage,
  nameTakenMessage,
} from "@outl/shared/pages/rename-merge";

import { mergePageFlow, renamePageFlow } from "./page-actions";
import { answerConfirm, pendingConfirm, registerConfirmSurface } from "./confirm";
import { appState, setAppState } from "./store";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

// `<ConfirmDialog />` is what answers in the app; here the tests answer
// in its place, so the surface has to be declared or `askConfirm`
// refuses on the spot (by design — see `lib/confirm.ts`).
const unregister = registerConfirmSurface();
afterAll(unregister);

const meta = (slug: string, title = slug): PageMeta =>
  ({ id: `page-${slug}`, slug, title, kind: "page" }) as PageMeta;

const view = (slug: string) => ({
  page: meta(slug),
  outline: [],
  backlinks: [],
  backlinks_order: "newest",
});

const renameReport = {
  old_slug: "a",
  old_title: "Alpha",
  rewritten_blocks: 2,
  skipped_namespace_refs: 0,
};

const mergeReport = {
  source_slug: "a",
  source_title: "Alpha",
  target_slug: "b",
  target_title: "Bravo",
  moved_blocks: 3,
  rewritten_blocks: 1,
  skipped_namespace_refs: 0,
  property_conflicts: [],
  aliases_added: [],
  already_merged: false,
};

afterEach(() => {
  vi.clearAllMocks();
  setAppState("notice", null);
  setAppState("lastError", null);
  answerConfirm(false);
});

/** Resolve whichever question is open, once it is. */
async function answer(ok: boolean) {
  await vi.waitFor(() => expect(pendingConfirm()).not.toBeNull());
  const message = pendingConfirm()!.message;
  answerConfirm(ok);
  return message;
}

describe("renamePageFlow", () => {
  it("renames without asking anything when the name is free", async () => {
    vi.mocked(invoke).mockResolvedValue({
      status: "renamed",
      view: view("bravo"),
      report: renameReport,
    });
    const result = await renamePageFlow(meta("a", "Alpha"), "  Bravo  ");
    expect(invoke).toHaveBeenCalledWith("rename_page", { page: "a", newName: "Bravo" });
    expect(result?.page.slug).toBe("bravo");
    // Blocks, not links: one block can carry several mentions, and the
    // noun is `outl_actions::page_summary`'s on every client.
    expect(appState.notice?.join(" ")).toContain("Links updated in 2 blocks");
    // Nothing destructive happened, so nothing was asked.
    expect(pendingConfirm()).toBeNull();
  });

  it("offers a merge in the shared words when the name is taken, and merges on accept", async () => {
    vi.mocked(invoke).mockImplementation(async (cmd) => {
      if (cmd === "rename_page") return { status: "exists", existing: meta("b", "Bravo") };
      return { view: view("b"), report: mergeReport };
    });

    const run = renamePageFlow(meta("a", "Alpha"), "Bravo");
    expect(await answer(true)).toBe(nameTakenMessage("Bravo"));
    const result = await run;

    expect(invoke).toHaveBeenCalledWith("merge_pages", { source: "a", target: "b" });
    expect(result?.page.slug).toBe("b");
    expect(appState.notice?.join(" ")).toContain("3 blocks moved");
  });

  it("writes nothing when the merge offer is declined", async () => {
    vi.mocked(invoke).mockResolvedValue({ status: "exists", existing: meta("b", "Bravo") });

    const run = renamePageFlow(meta("a", "Alpha"), "Bravo");
    await answer(false);

    expect(await run).toBeNull();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(appState.notice).toBeNull();
  });

  it("puts a refusal on the error surface and nothing on the toast", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("cannot rename a journal"));
    expect(await renamePageFlow(meta("2026-11-23"), "Monday")).toBeNull();
    expect(appState.lastError).toBe("cannot rename a journal");
    expect(appState.notice).toBeNull();
  });
});

describe("mergePageFlow", () => {
  it("always asks first, in the shared words", async () => {
    vi.mocked(invoke).mockResolvedValue({ view: view("b"), report: mergeReport });

    const run = mergePageFlow(meta("a", "Alpha"), meta("b", "Bravo"));
    expect(await answer(true)).toBe(mergeConfirmMessage("Alpha", "Bravo"));

    expect((await run)?.page.slug).toBe("b");
    expect(invoke).toHaveBeenCalledWith("merge_pages", { source: "a", target: "b" });
  });

  it("invokes nothing when the question is refused", async () => {
    const run = mergePageFlow(meta("a", "Alpha"), meta("b", "Bravo"));
    await answer(false);
    expect(await run).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  /** What a merge dropped has to be on screen afterwards: the target's
   *  value won, and the source's is gone from every device. */
  it("shows the property conflicts a merge resolved", async () => {
    vi.mocked(invoke).mockResolvedValue({
      view: view("b"),
      report: {
        ...mergeReport,
        property_conflicts: [{ key: "status", kept: "open", dropped: "done" }],
      },
    });
    const run = mergePageFlow(meta("a", "Alpha"), meta("b", "Bravo"));
    await answer(true);
    await run;

    const shown = appState.notice?.join("\n") ?? "";
    expect(shown).toContain("status");
    expect(shown).toContain("open");
    expect(shown).toContain("done");
  });

  it("puts a refusal on the error surface", async () => {
    vi.mocked(invoke).mockRejectedValue(new Error("cannot merge a page into itself"));
    const run = mergePageFlow(meta("a", "Alpha"), meta("b", "Bravo"));
    await answer(true);

    expect(await run).toBeNull();
    expect(appState.lastError).toBe("cannot merge a page into itself");
    expect(appState.notice).toBeNull();
  });
});

/**
 * `window.confirm` is a native dialog: it takes no palette token, no
 * destructive signal and no focus rule, which is why the most
 * destructive action in the product cannot be asked through it.
 */
describe("the questions never reach the OS", () => {
  it("does not call window.confirm", async () => {
    const native = vi.fn(() => true);
    const original = window.confirm;
    window.confirm = native as unknown as typeof window.confirm;
    try {
      vi.mocked(invoke).mockResolvedValue({ status: "exists", existing: meta("b", "Bravo") });
      const run = renamePageFlow(meta("a", "Alpha"), "Bravo");
      await answer(false);
      await run;

      const merge = mergePageFlow(meta("a", "Alpha"), meta("b", "Bravo"));
      await answer(false);
      await merge;

      expect(native).not.toHaveBeenCalled();
    } finally {
      window.confirm = original;
    }
  });
});
