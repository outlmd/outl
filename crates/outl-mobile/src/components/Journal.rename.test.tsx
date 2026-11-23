/**
 * Renaming the page you are looking at, from its own title.
 *
 * The flow is `@outl/shared/pages/rename-merge`'s and runs for real
 * here, over stubbed `api/page-merge` commands — the same arrangement as
 * `PageSwitcher.page-actions.test.tsx`, because the whole point is that
 * the two doors reach one implementation. What these pin is what the
 * header owns: that the keyboard is armed before the sheet renders, that
 * a no-op name costs no IPC, that a taken name asks before it merges,
 * and that a journal never gets here.
 *
 * **Not covered, and not coverable in happy-dom:** whether the keyboard
 * actually comes up, and whether the document scrolls the header away.
 * Neither computes layout here. `armKeyboard` being called inside the
 * tap is a *proxy* for the first (`lib/keyboard-keeper.ts`), and the
 * second is addressed structurally — the field lives in a `position:
 * fixed` sheet, never inline in the chrome.
 */

import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const renamePage = vi.fn();
const mergePages = vi.fn();

vi.mock("@outl/shared/api/page-merge", () => ({
  renamePage: (...a: unknown[]) => renamePage(...a),
  mergePages: (...a: unknown[]) => mergePages(...a),
}));

/** Was the sheet already on the page when the keyboard was armed? */
let sheetWhenArmed: boolean | null = null;
const armKeyboard = vi.fn(() => {
  sheetWhenArmed = field() !== null;
  return true;
});
vi.mock("../lib/keyboard-keeper", () => ({ armKeyboard: () => armKeyboard() }));
vi.mock("../lib/haptics", () => ({ haptic: () => {} }));

import { mergeSummary, nameTakenMessage, renameSummary } from "@outl/shared/pages/rename-merge";
import { createPageRename, PageRenameOverlays, type PageRename } from "./Journal.rename";

let dispose: (() => void) | undefined;

const view = (kind: "page" | "journal", title = "Inbox", slug = "inbox") => ({
  page: { id: `id-${slug}`, slug, title, kind },
});

const field = () =>
  document.querySelector('input[aria-label="Rename page"]') as HTMLInputElement | null;

const dialog = () => document.querySelector('[role="alertdialog"]') as HTMLElement | null;

const button = (text: string, root: ParentNode = document) =>
  [...root.querySelectorAll("button")].find((b) => b.textContent?.trim() === text) as
    | HTMLButtonElement
    | undefined;

const settle = () => new Promise((r) => setTimeout(r, 0));

async function tap(el: HTMLElement | undefined) {
  if (!el) throw new Error("nothing to tap");
  el.click();
  await settle();
}

async function typeName(value: string) {
  const input = field();
  if (!input) throw new Error("the name sheet is not open");
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
  await settle();
}

function mount(initial: ReturnType<typeof view> | null = view("page")) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const [current, setCurrent] = createSignal(initial);
  const open = vi.fn();
  const notify = vi.fn();
  const onError = vi.fn();
  let rename!: PageRename;
  dispose = render(() => {
    rename = createPageRename({
      view: current as never,
      open,
      notify,
      onError,
    });
    return <PageRenameOverlays rename={rename} />;
  }, host);
  return { host, open, notify, onError, setCurrent, rename: () => rename };
}

beforeEach(() => {
  sheetWhenArmed = null;
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("page title rename: opening the sheet", () => {
  it("prefills the current title", () => {
    const { rename } = mount();
    rename().start();

    expect(field()?.value).toBe("Inbox");
  });

  /**
   * WKWebView raises the keyboard only for a focus inside the tap's own
   * gesture window, and `PageNameSheet` focuses a frame later. From the
   * switcher the search box is already focused and iOS sees an
   * input→input move; from the header the keyboard is down.
   */
  it("arms the keyboard before the sheet renders", () => {
    const { rename } = mount();
    rename().start();

    expect(armKeyboard).toHaveBeenCalledTimes(1);
    expect(sheetWhenArmed).toBe(false);
  });

  it("falls back to the slug when the page has no title", () => {
    const { rename } = mount(view("page", "", "reading-list"));
    rename().start();

    expect(field()?.value).toBe("reading-list");
  });

  it("refuses a journal: its slug is its date", () => {
    const { rename } = mount(view("journal", "Today", "2026-11-23"));
    rename().start();

    expect(field()).toBeNull();
    expect(armKeyboard).not.toHaveBeenCalled();
  });

  it("refuses when nothing has loaded yet", () => {
    const { rename } = mount(null);
    rename().start();

    expect(field()).toBeNull();
  });

  it("cancelling writes nothing and goes nowhere", async () => {
    const { rename, open, notify } = mount();
    rename().start();
    await tap(button("Cancel"));

    expect(field()).toBeNull();
    expect(renamePage).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
  });
});

describe("page title rename: committing", () => {
  const report = {
    old_slug: "inbox",
    old_title: "Inbox",
    rewritten_blocks: 2,
    skipped_namespace_refs: 0,
  };

  it("a free name renames, opens the result and shows the summary", async () => {
    renamePage.mockResolvedValue({ status: "renamed", view: view("page", "Archive", "archive"), report });
    const { rename, open, notify } = mount();
    rename().start();
    await typeName("Archive");
    await tap(button("Rename"));

    expect(renamePage).toHaveBeenCalledWith("inbox", "Archive");
    expect(mergePages).not.toHaveBeenCalled();
    expect(open).toHaveBeenCalledWith("archive", "page");
    expect(notify).toHaveBeenCalledWith(renameSummary(report));
    expect(field()).toBeNull();
  });

  /** The sheet disables the button; this is the belt that still holds
   *  when the view moved under an open sheet. */
  it("the unchanged name is not submittable and fires no IPC", async () => {
    const { rename, notify } = mount();
    rename().start();

    expect(button("Rename")?.disabled).toBe(true);
    await typeName("  Inbox  ");
    expect(button("Rename")?.disabled).toBe(true);
    await rename().commit("  Inbox  ");

    expect(renamePage).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
  });

  it("an empty name is not submittable", async () => {
    const { rename } = mount();
    rename().start();
    await typeName("   ");

    expect(button("Rename")?.disabled).toBe(true);
  });

  /** A rename commits against the page the user tapped, not whatever is
   *  on screen when they hit the button — a peer's op can land between. */
  it("renames the page the sheet was opened on", async () => {
    renamePage.mockResolvedValue({ status: "renamed", view: view("page", "Archive", "archive"), report });
    const { rename, setCurrent } = mount();
    rename().start();
    await typeName("Archive");
    setCurrent(view("page", "Beta", "beta"));
    await tap(button("Rename"));

    expect(renamePage).toHaveBeenCalledWith("inbox", "Archive");
  });

  it("a backend failure reaches the toast instead of being swallowed", async () => {
    renamePage.mockRejectedValue(new Error("invalid page name"));
    const { rename, open, onError, notify } = mount();
    rename().start();
    await typeName("???");
    await tap(button("Rename"));

    expect(onError).toHaveBeenCalledWith("invalid page name");
    expect(open).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
  });
});

describe("page title rename: the name is taken", () => {
  const exists = {
    status: "exists",
    existing: { id: "id-beta", slug: "beta", title: "Beta", kind: "page" },
  };
  const merged = {
    view: view("page", "Beta", "beta"),
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

  async function renameOnto(name: string) {
    renamePage.mockResolvedValue(exists);
    const mounted = mount();
    mounted.rename().start();
    await typeName(name);
    await tap(button("Rename"));
    return mounted;
  }

  it("asks before merging, in the shared wording", async () => {
    const { notify } = await renameOnto("Beta");

    expect(dialog()?.textContent).toContain(nameTakenMessage("Beta"));
    expect(mergePages).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
  });

  it("accepting merges this page INTO the existing one", async () => {
    mergePages.mockResolvedValue(merged);
    const { open, notify } = await renameOnto("Beta");
    await tap(button("Merge", dialog() as HTMLElement));

    expect(mergePages).toHaveBeenCalledWith("inbox", "beta");
    expect(open).toHaveBeenCalledWith("beta", "page");
    // The property conflict is part of what the user is told.
    expect(notify).toHaveBeenCalledWith(mergeSummary(merged.report).join("\n"));
  });

  it("refusing writes nothing and goes nowhere", async () => {
    const { open, notify } = await renameOnto("Beta");
    await tap(button("Cancel", dialog() as HTMLElement));

    expect(mergePages).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
    expect(notify).not.toHaveBeenCalled();
    expect(dialog()).toBeNull();
  });
});
