/**
 * The page title is the rename door, so the tests are about the door:
 * it opens on one click, it writes through the one flow, it writes
 * nothing when the name did not change, and it is absent on a journal —
 * whose slug is its date and whose rename the backend refuses.
 *
 * The pending state is here too, because the regression it guards is a
 * layout one: a rename is a workspace-wide link rewrite, and the title
 * must stay on screen while it runs.
 */

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { PageMeta, PageView } from "@outl/shared/api/types";

import { PageTitleEditor } from "./PageTitleEditor";
import { renamePageFlow } from "../lib/page-actions";
import { setAppState } from "../lib/store";

vi.mock("../lib/page-actions", () => ({ renamePageFlow: vi.fn() }));

const meta = (over: Partial<PageMeta> = {}): PageMeta =>
  ({ id: "p1", slug: "inbox", title: "Inbox", kind: "page", ...over }) as PageMeta;

const view = (slug: string): PageView =>
  ({
    page: meta({ slug, title: slug }),
    outline: [],
    backlinks: [],
    backlinks_order: "newest",
  }) as unknown as PageView;

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  setAppState("todaySlug", null);
  vi.clearAllMocks();
});

function mount(page: PageMeta) {
  const onRenamed = vi.fn();
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <PageTitleEditor page={page} onRenamed={onRenamed} />, host);
  return { host, onRenamed };
}

const affordance = () => document.body.querySelector<HTMLElement>("[data-outl-page-title]");
const field = () => document.body.querySelector("input");

function type(text: string) {
  const el = field() as HTMLInputElement;
  el.value = text;
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

const press = (key: string) =>
  (field() as HTMLInputElement).dispatchEvent(
    new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }),
  );

/** The blur commit's grace, plus a tick. */
const settle = () => new Promise((r) => setTimeout(r, 150));

describe("PageTitleEditor", () => {
  it("opens on a single click, prefilled with the current title and selected", async () => {
    mount(meta());
    affordance()!.click();
    const el = field() as HTMLInputElement;
    expect(el.value).toBe("Inbox");
    // Focus and the selection land after render, so typing replaces the
    // name instead of appending to it.
    await vi.waitFor(() => {
      expect(el.selectionStart).toBe(0);
      expect(el.selectionEnd).toBe("Inbox".length);
    });
  });

  it("renames through the one flow and hands the reply up", async () => {
    vi.mocked(renamePageFlow).mockResolvedValue(view("archive"));
    const { onRenamed } = mount(meta());
    affordance()!.click();
    type("  Archive  ");
    press("Enter");
    await vi.waitFor(() => expect(onRenamed).toHaveBeenCalled());
    expect(renamePageFlow).toHaveBeenCalledWith(
      expect.objectContaining({ slug: "inbox" }),
      "Archive",
    );
    expect(onRenamed).toHaveBeenCalledWith(expect.objectContaining({ page: expect.anything() }));
  });

  it("writes nothing when the trimmed name is the title it already has", async () => {
    mount(meta());
    affordance()!.click();
    type("  Inbox  ");
    press("Enter");
    await settle();
    expect(renamePageFlow).not.toHaveBeenCalled();
    expect(field()).toBeNull();
  });

  it("writes nothing for an empty name", async () => {
    mount(meta());
    affordance()!.click();
    type("   ");
    press("Enter");
    await settle();
    expect(renamePageFlow).not.toHaveBeenCalled();
  });

  it("Escape restores the title and writes nothing, blur included", async () => {
    const { host } = mount(meta());
    affordance()!.click();
    type("Archive");
    press("Escape");
    expect(field()).toBeNull();
    expect(host.textContent).toContain("Inbox");
    await settle();
    expect(renamePageFlow).not.toHaveBeenCalled();
  });

  it("commits on blur, after the grace", async () => {
    vi.mocked(renamePageFlow).mockResolvedValue(view("archive"));
    mount(meta());
    affordance()!.click();
    type("Archive");
    (field() as HTMLInputElement).dispatchEvent(new FocusEvent("blur"));
    expect(renamePageFlow).not.toHaveBeenCalled();
    await settle();
    expect(renamePageFlow).toHaveBeenCalledWith(expect.anything(), "Archive");
  });

  /**
   * A journal's slug *is* its date: the backend refuses the rename and
   * `journalHeading` has no single editable string to put in a field.
   * Offering the door and then refusing is the defect `UX.md` section 3
   * names, so there is no door.
   */
  it("gives a journal no affordance at all", () => {
    setAppState("todaySlug", "2026-11-23");
    const { host } = mount(meta({ slug: "2026-11-23", title: "2026-11-23", kind: "journal" }));
    expect(affordance()).toBeNull();
    expect(host.querySelector("button")).toBeNull();
    expect(host.textContent).toContain("Today");
  });

  /** The input sits inside the outline region: the Normal-mode
   *  dispatcher must not read a typed title as chords. */
  it("keeps its keystrokes away from the Normal-mode dispatcher", () => {
    const seen = vi.fn();
    window.addEventListener("keydown", seen);
    try {
      mount(meta());
      affordance()!.click();
      press("d");
      expect(seen).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("keydown", seen);
    }
  });

  /**
   * The rename rewrites links across the workspace and returns a whole
   * `PageView`, so it is not instant — and the title must not go away
   * while it runs (`UX.md` section 2, and the `min-h-5` reservation the
   * header already carries for the same reason).
   */
  it("marks the title while the rename is in flight instead of replacing it", async () => {
    let land: (v: PageView) => void = () => {};
    vi.mocked(renamePageFlow).mockReturnValue(
      new Promise<PageView | null>((resolve) => {
        land = resolve;
      }),
    );
    const { host } = mount(meta());
    affordance()!.click();
    type("Archive");
    press("Enter");
    await vi.waitFor(() => expect(renamePageFlow).toHaveBeenCalled());
    expect(host.textContent).toContain("Inbox");
    expect(host.querySelector("[aria-busy='true']")).not.toBeNull();
    land(view("archive"));
    await vi.waitFor(() =>
      expect(host.querySelector("[aria-busy='true']")).toBeNull(),
    );
  });

  /**
   * The editor closes on commit, so the button is back under the
   * pointer while the round trip runs. Opening it again would seed the
   * draft from `props.page`, which still holds the **old** name until
   * the reply lands, and committing that sends a second rename against
   * the slug the first one is retiring: the user gets "page not found"
   * for the page they are looking at. A workspace-wide link rewrite is
   * long enough to click twice in.
   */
  it("refuses a second rename while the first is still in flight", async () => {
    let land: (v: PageView | null) => void = () => {};
    vi.mocked(renamePageFlow).mockReturnValue(
      new Promise<PageView | null>((resolve) => {
        land = resolve;
      }),
    );
    mount(meta());
    affordance()!.click();
    type("Archive");
    press("Enter");
    await vi.waitFor(() => expect(renamePageFlow).toHaveBeenCalledTimes(1));

    affordance()!.click();
    expect(field()).toBeNull();

    land(view("archive"));
    await vi.waitFor(() => expect(field()).toBeNull());
    expect(renamePageFlow).toHaveBeenCalledTimes(1);
  });

  /** A declined merge (or a refusal) produced no page: the caret comes
   *  back to the title the user was editing, not to nowhere. */
  it("returns focus to the title when the flow produced no page", async () => {
    vi.mocked(renamePageFlow).mockResolvedValue(null);
    mount(meta());
    affordance()!.click();
    type("Archive");
    press("Enter");
    await vi.waitFor(() => expect(document.activeElement).toBe(affordance()));
  });
});
