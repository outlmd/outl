/**
 * The phone's trash sheet: the logic is `createTrashRestore`'s (tested in
 * `@outl/shared`), so this pins what the sheet adds — it reads nothing
 * while closed, renders the verdicts it is handed, and closes only on a
 * restore that actually landed.
 */

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { listTrash, restoreTrashEntry, type TrashEntry } from "@outl/shared/api/trash";
import type { PageView } from "@outl/shared/api/types";

import { TrashSheet } from "./TrashSheet";

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

async function mount(rows: TrashEntry[], open = true) {
  vi.mocked(listTrash).mockImplementation(async () => rows.map((r) => ({ ...r })));
  const host = document.createElement("div");
  document.body.appendChild(host);
  const onClose = vi.fn();
  const onView = vi.fn();
  const onMessage = vi.fn();
  dispose = render(
    () => <TrashSheet open={open} onClose={onClose} onView={onView} onMessage={onMessage} />,
    host,
  );
  if (open) {
    await vi.waitFor(() => expect(listTrash).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 0));
  }
  return { host, onClose, onView, onMessage };
}

const restoreButton = (host: Element) =>
  [...host.querySelectorAll("button")].find((b) => /Restor/.test(b.textContent ?? ""));

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("TrashSheet", () => {
  it("does not read the trash while closed", async () => {
    await mount([entry()], false);
    expect(listTrash).not.toHaveBeenCalled();
  });

  it("lists rows with their age and block count", async () => {
    const { host } = await mount([entry({ blocks: 3 })]);
    expect(host.textContent).toContain("um bloco");
    expect(host.textContent).toContain("3 blocks");
    expect(host.textContent).toContain("1 deleted");
  });

  it("a refused row shows the refusal and no Restore button", async () => {
    const why = "cannot restore: its parent is in the trash";
    const { host } = await mount([entry({ refusal: why })]);
    expect(host.textContent).toContain(why);
    expect(host.textContent).toContain("Can't restore");
    expect(restoreButton(host)).toBeUndefined();
  });

  it("restoring applies the view and closes", async () => {
    const { host, onView, onClose } = await mount([entry()]);
    const landing = view("notas");
    vi.mocked(restoreTrashEntry).mockResolvedValue({ id: entry().id, blocks: 1, view: landing });

    restoreButton(host)!.click();

    await vi.waitFor(() => expect(onView).toHaveBeenCalledWith(landing));
    await vi.waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it("a refused restore keeps the sheet open and says why in the row", async () => {
    const { host, onClose } = await mount([entry()]);
    vi.mocked(restoreTrashEntry).mockRejectedValue(new Error("parent is gone"));

    restoreButton(host)!.click();

    await vi.waitFor(() => expect(host.textContent).toContain("parent is gone"));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("empty trash shows the explanation", async () => {
    const { host } = await mount([]);
    expect(host.textContent).toContain("Nothing has been deleted in this workspace");
  });
});
