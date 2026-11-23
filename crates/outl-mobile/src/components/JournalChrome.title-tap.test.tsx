/**
 * The page title is the rename door, and the long-press is still page
 * properties. Two gestures on one element, so what these pin is the
 * split rather than either half.
 *
 * The sharp edge is `consumedClick()`: it clears the flag as it answers
 * (`lib/long-press.ts`), so a second reader in the same click sees
 * `false`. Two readers means the rename sheet opens on top of the
 * properties sheet the hold just opened — which is why the guard and the
 * tap live in ONE handler, and why that is asserted here.
 *
 * A journal is exempt on both counts: its tap is already "back to
 * today", and its slug is its date, so it must never be renameable.
 */

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("../lib/haptics", () => ({ haptic: () => {} }));

import { JournalChrome } from "./JournalChrome";

let dispose: (() => void) | undefined;

const noop = () => {};

function pageView(kind: "page" | "journal", title = "Reading list") {
  return {
    page: { id: "p1", slug: kind === "journal" ? "2026-11-23" : "reading-list", title, kind },
  };
}

interface MountOpts {
  view?: ReturnType<typeof pageView> | null;
  /** What a completed hold would answer; read-once is the point. */
  consumed?: boolean;
}

function mount(opts: MountOpts = {}) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const onRenameTitle = vi.fn();
  const onJumpToday = vi.fn();
  // Counts how many times the click handler asked. The real recogniser
  // clears on read, so more than one reader is a bug the stub can see.
  let reads = 0;
  const lp = {
    onPointerDown: noop,
    onPointerMove: noop,
    onPointerUp: noop,
    cancel: noop,
    consumedClick: () => {
      reads += 1;
      return reads === 1 ? (opts.consumed ?? false) : false;
    },
  };
  dispose = render(
    () => (
      <JournalChrome
        view={(opts.view === undefined ? pageView("page") : opts.view) as never}
        online
        peers={[]}
        syncing={false}
        refreshing={false}
        todaySlug="2026-11-23"
        toolbarButtons={[]}
        titleLongPress={lp}
        onJumpToday={onJumpToday}
        onPrevDay={noop}
        onNextDay={noop}
        onFoldAll={noop}
        onUnfoldAll={noop}
        onRefresh={noop}
        onRunToolbarButton={noop}
        onOpenPlugins={noop}
        onOpenDevices={noop}
        onOpenSettings={noop}
        onRenameTitle={onRenameTitle}
      />
    ),
    host,
  );
  return { host, onRenameTitle, onJumpToday, reads: () => reads };
}

/** The title region: the element carrying both gestures. */
function titleRegion(host: HTMLElement): HTMLElement {
  const h1 = host.querySelector("h1");
  if (!h1) throw new Error("no title rendered");
  const region = h1.closest("[data-outl-title]");
  if (!region) throw new Error("the title region is not marked");
  return region as HTMLElement;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

describe("JournalChrome: tapping a page title", () => {
  it("opens the rename flow", () => {
    const { host, onRenameTitle } = mount();
    titleRegion(host).click();

    expect(onRenameTitle).toHaveBeenCalledTimes(1);
  });

  it("asks the recogniser exactly once per click", () => {
    const { host, reads } = mount();
    titleRegion(host).click();

    expect(reads()).toBe(1);
  });

  /** The hold already opened the properties sheet; the click it produced
   *  must not then open the rename sheet over it. */
  it("swallows the click a completed hold produced", () => {
    const { host, onRenameTitle, reads } = mount({ consumed: true });
    titleRegion(host).click();

    expect(onRenameTitle).not.toHaveBeenCalled();
    expect(reads()).toBe(1);
  });

  it("advertises the gesture with a glyph beside the title", () => {
    const { host } = mount();
    expect(titleRegion(host).querySelector("[data-outl-rename-hint]")).not.toBeNull();
  });

  /**
   * The title block is two 11-17px lines, so its own box is under the
   * 44pt floor. `py-1` grows the hit box and `-my-1` takes that padding
   * back out of the grid row, so the header does not get taller and the
   * title does not move. Deleting either half breaks one of those.
   */
  it("reaches 44pt without making the header taller", () => {
    const { host } = mount();
    const cls = titleRegion(host).className;

    expect(cls).toMatch(/(^|\s)py-1(\s|$)/);
    expect(cls).toMatch(/(^|\s)-my-1(\s|$)/);
  });
});

describe("JournalChrome: a journal title is not a rename door", () => {
  it("keeps its own tap and never renames", () => {
    const { host, onRenameTitle, onJumpToday } = mount({ view: pageView("journal") });
    const h1 = host.querySelector("h1") as HTMLElement;
    h1.click();

    expect(onJumpToday).toHaveBeenCalledTimes(1);
    expect(onRenameTitle).not.toHaveBeenCalled();
  });

  it("draws no rename glyph", () => {
    const { host } = mount({ view: pageView("journal") });
    expect(host.querySelector("[data-outl-rename-hint]")).toBeNull();
  });
});

/** Nothing has loaded yet, so there is no page to rename — and the boot
 *  placeholder must not advertise one. */
describe("JournalChrome: before the first page arrives", () => {
  it("neither renames nor hints", () => {
    const { host, onRenameTitle } = mount({ view: null });
    titleRegion(host).click();

    expect(onRenameTitle).not.toHaveBeenCalled();
    expect(host.querySelector("[data-outl-rename-hint]")).toBeNull();
  });
});
