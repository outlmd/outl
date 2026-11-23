/**
 * The dialog is the only place a rename name or a merge target is
 * typed. It has to prefill, keep the source out of its own merge list,
 * and leave itself open when a confirm is declined.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { searchPages } from "@outl/shared/api/commands";
import type { PageMeta, PageView } from "@outl/shared/api/types";

import { PageActionDialog } from "./PageActionDialog";
import { mergePageFlow, renamePageFlow } from "../lib/page-actions";

vi.mock("@outl/shared/api/commands", () => ({ searchPages: vi.fn() }));
vi.mock("../lib/page-actions", () => ({
  renamePageFlow: vi.fn(),
  mergePageFlow: vi.fn(),
}));

const meta = (slug: string, title = slug): PageMeta => ({
  id: `page-${slug}`,
  slug,
  title,
  kind: "page",
});

const view = (slug: string): PageView => ({
  page: meta(slug),
  outline: [],
  backlinks: [],
  backlinks_order: "newest",
});

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

function mount(kind: "rename" | "merge", onDone = vi.fn(), onClose = vi.fn()) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <PageActionDialog
        action={{ kind, page: meta("a", "Alpha") }}
        onClose={onClose}
        onDone={onDone}
      />
    ),
    host,
  );
  return { onDone, onClose };
}

const field = () => document.body.querySelector("input") as HTMLInputElement;
const press = (key: string) =>
  field().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));

describe("PageActionDialog", () => {
  /** The field is the whole point of the dialog, so it has to be where
   *  the caret already is — including through the portal the shared
   *  `<ModalCard />` renders into. */
  it("puts the caret in the field, selected", async () => {
    mount("rename");
    await vi.waitFor(() => {
      expect(document.activeElement).toBe(field());
      expect(field().selectionEnd).toBe("Alpha".length);
    });
  });

  it("rename prefills the current title and submits the typed name", async () => {
    vi.mocked(renamePageFlow).mockResolvedValue(view("beta"));
    const { onDone } = mount("rename");
    expect(field().value).toBe("Alpha");
    field().value = "Beta";
    field().dispatchEvent(new Event("input", { bubbles: true }));
    press("Enter");
    await vi.waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(renamePageFlow).toHaveBeenCalledWith(expect.objectContaining({ slug: "a" }), "Beta");
  });

  it("rename with an unchanged name does nothing", () => {
    mount("rename");
    press("Enter");
    expect(renamePageFlow).not.toHaveBeenCalled();
  });

  it("stays open when the flow produced no page (confirm declined)", async () => {
    vi.mocked(renamePageFlow).mockResolvedValue(null);
    const { onDone, onClose } = mount("rename");
    field().value = "Beta";
    field().dispatchEvent(new Event("input", { bubbles: true }));
    press("Enter");
    await vi.waitFor(() => expect(renamePageFlow).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 0));
    expect(onDone).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("merge lists other pages only and hands the picked one to the flow", async () => {
    vi.mocked(searchPages).mockResolvedValue([meta("a", "Alpha"), meta("b", "Bravo")]);
    vi.mocked(mergePageFlow).mockResolvedValue(view("b"));
    const { onDone } = mount("merge");
    await vi.waitFor(() => expect(document.body.textContent).toContain("Bravo"));
    const rows = [...document.body.querySelectorAll("button")].map((b) => b.textContent);
    expect(rows.join(" ")).not.toContain("Alpha  a");
    expect(rows.some((r) => r?.includes("Bravo"))).toBe(true);
    press("Enter");
    await vi.waitFor(() => expect(onDone).toHaveBeenCalled());
    expect(mergePageFlow).toHaveBeenCalledWith(
      expect.objectContaining({ slug: "a" }),
      expect.objectContaining({ slug: "b" }),
    );
  });

  it("Escape closes without calling any flow", () => {
    vi.mocked(searchPages).mockResolvedValue([]);
    const { onClose } = mount("merge");
    press("Escape");
    expect(onClose).toHaveBeenCalled();
    expect(mergePageFlow).not.toHaveBeenCalled();
  });
});

/**
 * Where the door is mounted, read from the source.
 *
 * `appState.sidebarOpen` starts `false` on every fresh launch and is
 * never persisted, so a dialog rendered inside `<Sidebar />` did not
 * exist for most windows: there was no rename and no merge anywhere
 * until the user discovered `Cmd/Ctrl+Shift+E`. Read rather than
 * rendered, because both components need a Tauri bridge to mount and
 * the defect is in *which* component holds it.
 */
describe("the page-action dialog is mounted at the shell", () => {
  function source(file: string): string {
    return readFileSync(join(import.meta.dirname, file), "utf8");
  }

  it("AppShell renders it and drives it from the store", () => {
    const src = source("AppShell.tsx");
    expect(src).toContain("<PageActionDialog");
    expect(src).toContain("appState.pageAction");
  });

  it("Sidebar only posts the request", () => {
    const src = source("Sidebar.tsx");
    expect(src).not.toContain("<PageActionDialog");
    expect(src).toContain('setAppState("pageAction"');
  });
});
