import { beforeEach, describe, expect, it, vi } from "vitest";
import { createRoot, createSignal } from "solid-js";

import type { TrashEntry } from "../api/trash";
import type { PageView } from "../api/types";

const listTrash = vi.fn();
const restoreTrashEntry = vi.fn();
vi.mock("../api/trash", () => ({
  listTrash: (...a: unknown[]) => listTrash(...a),
  restoreTrashEntry: (...a: unknown[]) => restoreTrashEntry(...a),
}));

import { createTrashRestore, deletedAgo } from "./trash";

const entry = (id: string, over: Partial<TrashEntry> = {}): TrashEntry => ({
  id,
  text: `block ${id}`,
  page: null,
  blocks: 1,
  deleted_at_ms: null,
  refusal: null,
  ...over,
});

const tick = () => new Promise((r) => setTimeout(r, 0));
const view = { slug: "p" } as unknown as PageView;

function setup(initialOpen = true) {
  const onView = vi.fn();
  const onError = vi.fn();
  const [open, setOpen] = createSignal(initialOpen);
  const dispose = { fn: () => {} };
  const t = createRoot((d) => {
    dispose.fn = d;
    return createTrashRestore({ open, onView, onError });
  });
  return { t, setOpen, onView, onError, dispose: () => dispose.fn() };
}

beforeEach(() => {
  listTrash.mockReset();
  restoreTrashEntry.mockReset();
});

describe("createTrashRestore", () => {
  it("does not list until opened", async () => {
    listTrash.mockResolvedValue([]);
    const { setOpen, dispose } = setup(false);
    await tick();
    expect(listTrash).not.toHaveBeenCalled();
    setOpen(true);
    await tick();
    expect(listTrash).toHaveBeenCalledTimes(1);
    dispose();
  });

  it("restore applies the view and resolves true", async () => {
    const a = entry("a");
    listTrash.mockResolvedValue([a]);
    restoreTrashEntry.mockResolvedValue({ id: "a", blocks: 1, view });
    const { t, onView, dispose } = setup();
    await tick();
    expect(await t.restore(a)).toBe(true);
    expect(onView).toHaveBeenCalledWith(view);
    expect(t.busy()).toBeNull();
    dispose();
  });

  it("restore with a null view resolves true without onView", async () => {
    const a = entry("a");
    listTrash.mockResolvedValue([a]);
    restoreTrashEntry.mockResolvedValue({ id: "a", blocks: 1, view: null });
    const { t, onView, dispose } = setup();
    await tick();
    expect(await t.restore(a)).toBe(true);
    expect(onView).not.toHaveBeenCalled();
    dispose();
  });

  it("restore resolves false, records the notice and refetches", async () => {
    const a = entry("a");
    listTrash.mockResolvedValue([a]);
    restoreTrashEntry.mockRejectedValue(new Error("parent is gone"));
    const { t, dispose } = setup();
    await tick();
    expect(await t.restore(a)).toBe(false);
    await tick();
    expect(t.notice(a)).toBe("parent is gone");
    expect(listTrash).toHaveBeenCalledTimes(2);
    dispose();
  });

  it("the row's own refusal wins over the notice", async () => {
    const a = entry("a");
    listTrash.mockResolvedValueOnce([a]);
    listTrash.mockResolvedValue([entry("a", { refusal: "cannot restore: parent gone" })]);
    restoreTrashEntry.mockRejectedValue(new Error("parent is gone"));
    const { t, dispose } = setup();
    await tick();
    await t.restore(a);
    await tick();
    const row = t.rows()![0];
    expect(row.refusal).toBe("cannot restore: parent gone");
    expect(t.notice(row)).toBeNull();
    dispose();
  });

  it("notice is per row", async () => {
    const a = entry("a");
    const b = entry("b");
    listTrash.mockResolvedValue([a, b]);
    restoreTrashEntry.mockRejectedValue(new Error("nope"));
    const { t, dispose } = setup();
    await tick();
    await t.restore(a);
    expect(t.notice(a)).toBe("nope");
    expect(t.notice(b)).toBeNull();
    dispose();
  });

  it("reopening clears the attempt notice", async () => {
    const a = entry("a");
    listTrash.mockResolvedValue([a]);
    restoreTrashEntry.mockRejectedValue(new Error("nope"));
    const { t, setOpen, dispose } = setup();
    await tick();
    await t.restore(a);
    expect(t.notice(a)).toBe("nope");
    setOpen(false);
    setOpen(true);
    await tick();
    expect(t.notice(a)).toBeNull();
    dispose();
  });

  it("a listing failure reaches onError and yields []", async () => {
    listTrash.mockRejectedValue(new Error("not open"));
    const { t, onError, dispose } = setup();
    await tick();
    expect(onError).toHaveBeenCalledWith("not open");
    expect(t.rows()).toEqual([]);
    dispose();
  });
});

describe("deletedAgo", () => {
  const now = 1_700_000_000_000;
  it("formats each band", () => {
    expect(deletedAgo(null, now)).toBe("");
    expect(deletedAgo(now - 30_000, now)).toBe("just now");
    expect(deletedAgo(now - 5 * 60_000, now)).toBe("5m ago");
    expect(deletedAgo(now - 3 * 3_600_000, now)).toBe("3h ago");
    expect(deletedAgo(now - 2 * 86_400_000, now)).toBe("2d ago");
    const old = now - 40 * 86_400_000;
    expect(deletedAgo(old, now)).toBe(new Date(old).toLocaleDateString());
  });
});
