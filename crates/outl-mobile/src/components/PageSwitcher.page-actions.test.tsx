/**
 * What a long-press on a page row offers beyond delete: rename, and
 * merge into another page. The flow (rename, ask when the name is
 * taken, merge) is `@outl/shared/pages/rename-merge`'s and runs for real here, over
 * stubbed backend commands; these tests pin what this sheet owns: the
 * menu, the prompts, that nothing is written on cancel, that a merge is
 * never reached without a confirmation, and that journals get none of it.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const deletePage = vi.fn();
const listPages = vi.fn();
const searchPages = vi.fn();
const renamePage = vi.fn();
const mergePages = vi.fn();

vi.mock("@outl/shared/api/commands", () => ({
  listPages: (...a: unknown[]) => listPages(...a),
  searchPages: (...a: unknown[]) => searchPages(...a),
  deletePage: (...a: unknown[]) => deletePage(...a),
  searchBlocks: () => Promise.resolve([]),
  togglePin: vi.fn(),
}));
vi.mock("@outl/shared/api/page-merge", () => ({
  renamePage: (...a: unknown[]) => renamePage(...a),
  mergePages: (...a: unknown[]) => mergePages(...a),
}));

import { mergeConfirmMessage, nameTakenMessage, renameSummary } from "@outl/shared/pages/rename-merge";
import { PageSwitcher } from "./PageSwitcher";

let dispose: (() => void) | undefined;
let host: HTMLElement;

const page = (slug: string, extra: object = {}) => ({
  id: `id-${slug}`,
  slug,
  title: slug,
  kind: "page",
  ...extra,
});
const view = (slug: string) => ({ page: { slug, kind: "page" } });

function mount() {
  host = document.createElement("div");
  document.body.appendChild(host);
  const onPick = vi.fn();
  const onNotice = vi.fn();
  dispose = render(
    () =>
      PageSwitcher({
        open: true,
        currentSlug: null,
        onClose: vi.fn(),
        onPick,
        onJumpToBlock: vi.fn(),
        onNotice,
      }) as never,
    host,
  );
  return { onPick, onNotice };
}

const settle = () => new Promise((r) => setTimeout(r, 0));

const button = (text: string, root: ParentNode = document): HTMLButtonElement | undefined =>
  [...root.querySelectorAll("button")].find((b) => b.textContent?.trim() === text) as
    | HTMLButtonElement
    | undefined;

function row(title: string): HTMLButtonElement {
  const hit = [...document.querySelectorAll("button")].find((b) => b.textContent?.includes(title));
  if (!hit) throw new Error(`no row containing "${title}"`);
  return hit as HTMLButtonElement;
}

/** Hold a finger on a row past the long-press threshold. */
async function longPress(el: HTMLElement) {
  el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 1, clientY: 1 }));
  await new Promise((r) => setTimeout(r, 560));
  el.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
}

/** The confirm dialog's own buttons (the sheet underneath also has a "Cancel"). */
const dialog = () => document.querySelector('[role="alertdialog"]') as HTMLElement | null;

async function tap(el: HTMLElement | undefined) {
  if (!el) throw new Error("nothing to tap");
  el.click();
  await settle();
}

async function typeName(value: string) {
  const input = document.querySelector('input[aria-label="Rename page"]') as HTMLInputElement;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
}

beforeEach(() => {
  const pages = [page("inbox", { title: "Inbox" }), page("beta", { title: "Beta" })];
  listPages.mockResolvedValue(pages);
  searchPages.mockResolvedValue(pages);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("PageSwitcher: page actions menu", () => {
  it("a long-press offers rename, merge and delete, and opens nothing", async () => {
    const { onPick } = mount();
    await settle();
    await longPress(row("Inbox"));
    await settle();

    expect(button("Rename page...")).toBeDefined();
    expect(button("Merge into...")).toBeDefined();
    expect(button("Delete page")).toBeDefined();
    expect(onPick).not.toHaveBeenCalled();
  });

  it("a journal row gets no actions menu at all", async () => {
    listPages.mockResolvedValue([page("2026-09-01", { title: "Sep 1", kind: "journal" })]);
    mount();
    await settle();
    await longPress(row("Sep 1"));
    await settle();

    expect(button("Rename page...")).toBeUndefined();
    expect(button("Merge into...")).toBeUndefined();
    expect(button("Delete page")).toBeUndefined();
  });

  it("Delete page still asks before deleting", async () => {
    const confirm = vi.fn(() => false);
    window.confirm = confirm;
    mount();
    await settle();
    await longPress(row("Inbox"));
    await settle();
    await tap(button("Delete page"));

    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("Inbox"));
    expect(deletePage).not.toHaveBeenCalled();
  });
});

describe("PageSwitcher: rename", () => {
  async function openRename() {
    const mounted = mount();
    await settle();
    await longPress(row("Inbox"));
    await settle();
    await tap(button("Rename page..."));
    return mounted;
  }

  it("opens a prompt prefilled with the current title", async () => {
    await openRename();
    const input = document.querySelector('input[aria-label="Rename page"]') as HTMLInputElement;
    expect(input.value).toBe("Inbox");
  });

  it("will not submit the unchanged or an empty name", async () => {
    await openRename();
    expect(button("Rename")?.disabled).toBe(true);
    await typeName("   ");
    expect(button("Rename")?.disabled).toBe(true);
    await typeName("Archive");
    expect(button("Rename")?.disabled).toBe(false);
  });

  it("a free name renames, navigates to the page and shows the summary", async () => {
    const report = { old_slug: "inbox", old_title: "Inbox", rewritten_blocks: 2, skipped_namespace_refs: 0 };
    renamePage.mockResolvedValue({ status: "renamed", view: view("archive"), report });
    const { onPick, onNotice } = await openRename();
    await typeName("Archive");
    await tap(button("Rename"));

    expect(renamePage).toHaveBeenCalledWith("inbox", "Archive");
    expect(mergePages).not.toHaveBeenCalled();
    expect(dialog()).toBeNull();
    expect(onPick).toHaveBeenCalledWith("archive", "page");
    expect(onNotice).toHaveBeenCalledWith(renameSummary(report));
  });

  describe("when the name is taken", () => {
    const exists = { status: "exists", existing: page("beta", { title: "Beta" }) };
    const merged = {
      view: view("beta"),
      report: {
        source_slug: "inbox",
        source_title: "Inbox",
        target_slug: "beta",
        target_title: "Beta",
        moved_blocks: 3,
        rewritten_blocks: 1,
        skipped_namespace_refs: 0,
        property_conflicts: [{ key: "status", kept: "open", dropped: "done" }],
        aliases_added: [],
        already_merged: false,
      },
    };

    it("asks first, and accepting merges into the existing page", async () => {
      renamePage.mockResolvedValue(exists);
      mergePages.mockResolvedValue(merged);
      const { onPick, onNotice } = await openRename();
      await typeName("Beta");
      await tap(button("Rename"));

      expect(dialog()?.textContent).toContain(nameTakenMessage("Beta"));
      expect(mergePages).not.toHaveBeenCalled();

      await tap(button("Merge", dialog() as HTMLElement));

      expect(mergePages).toHaveBeenCalledWith("inbox", "beta");
      expect(onPick).toHaveBeenCalledWith("beta", "page");
      // The property conflict is part of what the user is shown.
      expect(onNotice.mock.calls[0][0]).toContain("status");
      expect(onNotice.mock.calls[0][0]).toContain("done");
    });

    it("cancelling writes nothing and goes nowhere", async () => {
      renamePage.mockResolvedValue(exists);
      const { onPick, onNotice } = await openRename();
      await typeName("Beta");
      await tap(button("Rename"));
      await tap(button("Cancel", dialog() as HTMLElement));

      expect(mergePages).not.toHaveBeenCalled();
      expect(onPick).not.toHaveBeenCalled();
      expect(onNotice).not.toHaveBeenCalled();
      expect(dialog()).toBeNull();
    });
  });

  it("an error from the backend is surfaced, not swallowed", async () => {
    const alert = vi.fn();
    window.alert = alert;
    renamePage.mockRejectedValue(new Error("invalid page name"));
    const { onPick } = await openRename();
    await typeName("???");
    await tap(button("Rename"));

    expect(alert).toHaveBeenCalledWith("invalid page name");
    expect(onPick).not.toHaveBeenCalled();
  });
});

describe("PageSwitcher: merge into", () => {
  const merged = {
    view: view("beta"),
    report: {
      source_slug: "inbox",
      source_title: "Inbox",
      target_slug: "beta",
      target_title: "Beta",
      moved_blocks: 1,
      rewritten_blocks: 0,
      skipped_namespace_refs: 0,
      property_conflicts: [],
      aliases_added: [],
      already_merged: false,
    },
  };

  async function pickMerge() {
    const mounted = mount();
    await settle();
    await longPress(row("Inbox"));
    await settle();
    await tap(button("Merge into..."));
    return mounted;
  }

  it("turns the list into a target picker that leaves the source out", async () => {
    await pickMerge();

    expect(host.textContent).toContain("into which page?");
    const titles = [...host.querySelectorAll("button")].map((b) => b.textContent ?? "");
    expect(titles.some((t) => t.includes("Beta"))).toBe(true);
    expect(titles.some((t) => t.includes("Inbox"))).toBe(false);
    // No pin toggles while picking: a tap must mean one thing.
    expect(host.querySelector('button[aria-label="Pin page"]')).toBeNull();
  });

  it("always confirms; cancelling writes nothing and keeps the picker open", async () => {
    const { onPick } = await pickMerge();
    await tap(row("Beta"));

    expect(dialog()?.textContent).toContain(mergeConfirmMessage("Inbox", "Beta"));
    await tap(button("Cancel", dialog() as HTMLElement));

    expect(mergePages).not.toHaveBeenCalled();
    expect(onPick).not.toHaveBeenCalled();
    expect(host.textContent).toContain("into which page?");
  });

  it("accepting merges the source into the chosen page and opens it", async () => {
    mergePages.mockResolvedValue(merged);
    const { onPick, onNotice } = await pickMerge();
    await tap(row("Beta"));
    await tap(button("Merge", dialog() as HTMLElement));

    expect(mergePages).toHaveBeenCalledTimes(1);
    expect(mergePages).toHaveBeenCalledWith("inbox", "beta");
    expect(onPick).toHaveBeenCalledWith("beta", "page");
    expect(onNotice).toHaveBeenCalledTimes(1);
    expect(host.textContent).not.toContain("into which page?");
  });

  it("the banner's Cancel leaves picking without writing", async () => {
    const { onPick } = await pickMerge();
    const banner = [...host.querySelectorAll("button")].find(
      (b) => b.textContent?.trim() === "Cancel" && b.closest("div")?.textContent?.includes("into which"),
    );
    await tap(banner);

    expect(host.textContent).not.toContain("into which page?");
    expect(mergePages).not.toHaveBeenCalled();
    expect(onPick).not.toHaveBeenCalled();
  });
});
