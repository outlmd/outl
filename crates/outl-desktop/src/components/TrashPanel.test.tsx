/**
 * What the trash panel has to get right, which is all about the
 * refusal: a row the backend says cannot come back must say so and not
 * offer the action, and a row the *listing* thought was fine must still
 * handle the mutation declining — the verdict is computed against a
 * live tree and the list is a snapshot.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { listTrash, restoreTrashEntry, type TrashEntry } from "@outl/shared/api/trash";
import type { PageView } from "@outl/shared/api/types";

import { TrashPanel } from "./TrashPanel";
import { appState, setAppState } from "../lib/store";

vi.mock("@outl/shared/api/trash", () => ({
  listTrash: vi.fn(),
  restoreTrashEntry: vi.fn(),
}));

let dispose: (() => void) | undefined;

function entry(over: Partial<TrashEntry> = {}): TrashEntry {
  return {
    id: "01JQ0000000000000000000001",
    text: "um bloco",
    page: null,
    blocks: 1,
    deleted_at_ms: Date.now() - 60_000,
    refusal: null,
    ...over,
  };
}

function view(slug: string): PageView {
  return {
    page: { id: `page-${slug}`, slug, title: slug, kind: "page" },
    outline: [],
    backlinks: [],
    backlinks_order: "newest",
  };
}

async function mount(rows: TrashEntry[], applyView = vi.fn()) {
  // Fresh objects per call, like the real command, so a refetch really
  // replaces the rows.
  vi.mocked(listTrash).mockImplementation(async () => rows.map((r) => ({ ...r })));
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => <TrashPanel applyView={applyView} setError={vi.fn()} />,
    host,
  );
  setAppState("trashOpen", true);
  await vi.waitFor(() => expect(listTrash).toHaveBeenCalled());
  await new Promise((r) => setTimeout(r, 0));
  return { host, applyView };
}

const restoreButton = (host: Element) =>
  [...host.querySelectorAll("button")].find((b) => /Restor/.test(b.textContent ?? ""));

beforeEach(() => {
  setAppState({ trashOpen: false });
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("TrashPanel", () => {
  it("does not read the trash until it is opened", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    vi.mocked(listTrash).mockResolvedValue([]);
    dispose = render(
      () => <TrashPanel applyView={vi.fn()} setError={vi.fn()} />,
      host,
    );
    // The component is mounted for the app's whole life; folding every
    // deletion's parent trail at boot is the cost this gate avoids.
    expect(listTrash).not.toHaveBeenCalled();
  });

  it("restores a row and shows the page the block landed on", async () => {
    const { host, applyView } = await mount([entry()]);
    const landing = view("notas");
    vi.mocked(restoreTrashEntry).mockResolvedValue({
      id: entry().id,
      blocks: 1,
      view: landing,
    });

    restoreButton(host)!.click();
    await vi.waitFor(() => expect(applyView).toHaveBeenCalledWith(landing));
    // Applying the landing page *is* the navigation, so the panel gets
    // out of the way rather than sitting over the block it just restored.
    expect(appState.trashOpen).toBe(false);
  });

  it("shows a refused row's reason verbatim and offers no restore", async () => {
    const why = "cannot restore 01JQ…: the block it sat under is in the trash too";
    const { host } = await mount([entry({ refusal: why })]);

    expect(host.textContent).toContain(why);
    expect(restoreButton(host)).toBeUndefined();
    // Hidden would be worse than refused: a user hunting for a block
    // needs to find it and be told why it cannot come back.
    expect(host.textContent).toContain("um bloco");
  });

  it("keeps a refusal that only the mutation knew about, in the row", async () => {
    // The listing said restorable; by the time the user clicked, a peer
    // had trashed the parent. The mutation is the one that finds out.
    const why = "cannot restore 01JQ…: the block it sat under is in the trash too";
    const { host, applyView } = await mount([entry()]);
    vi.mocked(restoreTrashEntry).mockRejectedValue(new Error(why));

    restoreButton(host)!.click();

    await vi.waitFor(() => expect(host.textContent).toContain(why));
    // Not a toast, which scrolls away from the list the user is reading;
    // and the panel stays open so they can see what happened.
    expect(appState.trashOpen).toBe(true);
    expect(applyView).not.toHaveBeenCalled();
    // And it re-lists, so the listing — the one owner of whether a row
    // can be restored — gets the last word rather than this component.
    await vi.waitFor(() => expect(vi.mocked(listTrash).mock.calls.length).toBeGreaterThan(1));
  });

  it("states a refusal once when the refetch has caught up", async () => {
    // The mutation's error is the bare diagnosis; the row's refusal is
    // worded for this surface and carries the way out. They are no
    // longer one string, so the dedup cannot be a string compare — and
    // without a working dedup every refused restore prints its reason
    // twice, in two voices. Here the refetch comes back refused, so the
    // row's verdict is the one that shows.
    const diagnosis = "cannot restore 01JQ…: the block it was deleted from (01JR…) is not in the tree";
    const worded = `${diagnosis} — check whether ops for it are missing — run this in the workspace folder: \`outl doctor\``;
    const rows = [entry()];
    const { host } = await mount(rows);
    vi.mocked(restoreTrashEntry).mockRejectedValue(new Error(diagnosis));
    // The peer's change is visible to the next listing.
    rows[0] = entry({ refusal: worded });

    restoreButton(host)!.click();

    await vi.waitFor(() => expect(host.textContent).toContain(worded));
    const occurrences = host.textContent!.split(diagnosis).length - 1;
    expect(occurrences, "the diagnosis appears once, not once per surface").toBe(1);
  });

  it("drops a stale attempt notice once the condition clears", async () => {
    // The regression this pins: the failure signal was merged into the
    // row's verdict and nothing cleared it, so a peer restoring the
    // parent left the row reading refused and button-less on every
    // reopen, for the life of the process.
    const why = "cannot restore 01JQ…: the block it sat under is in the trash too";
    const { host } = await mount([entry()]);
    vi.mocked(restoreTrashEntry).mockRejectedValue(new Error(why));

    restoreButton(host)!.click();
    await vi.waitFor(() => expect(host.textContent).toContain(why));

    // The peer puts the parent back, so the next listing says the row is
    // restorable again.
    setAppState("trashOpen", false);
    await new Promise((r) => setTimeout(r, 0));
    setAppState("trashOpen", true);
    await vi.waitFor(() => expect(host.textContent).not.toContain(why));

    expect(restoreButton(host), "the row is actionable again").toBeDefined();
  });

  it("arrows from row to row, past the controls inside them", async () => {
    // **Restorable** rows on purpose. An all-refused fixture has no
    // Restore button, so it cannot see the defect this is written for:
    // `FOCUSABLE` is a selector *list*, `button` matches on its own tag,
    // and no `tabindex` on the button removes it. With a generic item
    // selector a row holding a button is two arrow stops, and ArrowDown
    // from row 0 lands on row 0's own button.
    const { host } = await mount([entry({ id: "a" }), entry({ id: "b" })]);
    expect(restoreButton(host), "both rows offer a restore").toBeDefined();

    const list = host.querySelector<HTMLElement>("[data-trash-list]")!;
    const rows = [...host.querySelectorAll<HTMLElement>("[data-trash-row]")];
    expect(rows).toHaveLength(2);

    rows[0].focus();
    expect(document.activeElement).toBe(rows[0]);
    list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(document.activeElement, "the arrow has to land on the next row").toBe(rows[1]);
  });

  it("arrows out of a focused Restore button to the next row, not to row 0", async () => {
    // Clicking Restore focuses the button on WebKitGTK (the Linux
    // desktop runtime), and a refused mutation leaves the row listed
    // with focus still on it — a path that already has its own test.
    // `document.activeElement` is then the button, which does not match
    // the row selector, so `indexOf` answered -1 and ArrowDown wrapped
    // to row 0 while ArrowUp landed on row n-2. Three rows, so a jump to
    // row 0 and a jump to the button's own row are both distinguishable
    // from the right answer.
    const { host } = await mount([
      entry({ id: "a", text: "bloco a" }),
      entry({ id: "b", text: "bloco b" }),
      entry({ id: "c", text: "bloco c" }),
    ]);

    const list = host.querySelector<HTMLElement>("[data-trash-list]")!;
    const rows = [...host.querySelectorAll<HTMLElement>("[data-trash-row]")];
    const buttons = [...host.querySelectorAll<HTMLButtonElement>("button")].filter((b) =>
      /Restor/.test(b.textContent ?? ""),
    );
    expect(rows).toHaveLength(3);
    expect(buttons).toHaveLength(3);

    // The rows render identically unless they say different things, and
    // an `toBe(rows[2])` between two identical `<div>`s fails with
    // "serializes to the same string" — which does not say where focus
    // actually went. Reporting the index is what makes the failure
    // readable.
    const landed = () => rows.indexOf(document.activeElement as HTMLElement);

    buttons[1].focus();
    expect(document.activeElement).toBe(buttons[1]);
    list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(landed(), "ArrowDown off row 1's button goes to row 2").toBe(2);

    buttons[1].focus();
    list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    expect(landed(), "ArrowUp off row 1's button goes to row 0").toBe(0);
  });

  it("still has rows to arrow through when every one is refused", async () => {
    // The other half, and the hole the item was originally filed for: an
    // all-refused trash (18 rows on the reference workspace) must still
    // put focus somewhere, or the list's own Esc handler — which needs
    // focus inside it — never fires.
    const { host } = await mount([
      entry({ id: "a", refusal: "cannot restore: the page it came from is in the trash too" }),
      entry({ id: "b", refusal: "cannot restore: the block it sat under is in the trash too" }),
    ]);
    expect(restoreButton(host), "no row offers a restore").toBeUndefined();

    const list = host.querySelector<HTMLElement>("[data-trash-list]")!;
    const rows = [...host.querySelectorAll<HTMLElement>("[data-trash-row]")];
    rows[0].focus();
    list.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(document.activeElement).toBe(rows[1]);
  });

  it("restores from the focused row without reaching the button", async () => {
    // The row is the roving item, so the row carries the action — a
    // keyboard user never has to reach the button, which is what lets
    // the button leave the arrow walk.
    const { host, applyView } = await mount([entry()]);
    const landing = view("notas");
    vi.mocked(restoreTrashEntry).mockResolvedValue({ id: entry().id, blocks: 1, view: landing });

    const row = host.querySelector<HTMLElement>("[data-trash-row]")!;
    row.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

    await vi.waitFor(() => expect(applyView).toHaveBeenCalledWith(landing));
  });

  it("says the trash is empty rather than showing nothing", async () => {
    const { host } = await mount([]);
    expect(host.textContent).toContain("Nothing has been deleted");
  });
});
