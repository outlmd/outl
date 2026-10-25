/**
 * Keyboard reach for the window chrome: everything the mouse can hit,
 * the keyboard can reach without a chord to memorise.
 *
 * - Toolbars and lists use roving focus: Tab enters once, arrows move
 *   inside, Home/End jump to the ends, and it wraps.
 * - F6 / Shift+F6 cycle the regions marked `data-outl-region`
 *   (sidebar, outline, chrome), the convention native apps use.
 * - Esc from the sidebar or the chrome puts you back in the outline.
 *
 * Text fields are skipped on purpose: arrows there move the caret.
 */

import { createEffect, type Resource } from "solid-js";

const FOCUSABLE =
  'button:not([disabled]), a[href], [role="button"]:not([aria-disabled="true"]), [tabindex]:not([tabindex="-1"]), input, select, textarea';

export type Orientation = "horizontal" | "vertical";

/** Inputs whose arrows mean nothing, so the list may take them. */
const ARROWLESS_INPUTS = new Set(["checkbox", "button", "submit", "reset"]);

function isTextField(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  if (tag === "INPUT") return !ARROWLESS_INPUTS.has((el as HTMLInputElement).type);
  return tag === "TEXTAREA" || tag === "SELECT" || (el as HTMLElement).isContentEditable;
}

function visible(el: HTMLElement): boolean {
  return !el.closest("[hidden],[inert]");
}

function focusables(root: Element): HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(visible);
}

/**
 * Wire as `onKeyDown` on a toolbar or list container. With
 * `includeFields` (forms such as Settings) arrows can land on a text
 * field or select too; once there, arrows belong to it again.
 */
export function rovingKeydown(
  e: KeyboardEvent,
  orientation: Orientation,
  opts: { includeFields?: boolean } = {},
): void {
  const container = e.currentTarget as HTMLElement | null;
  if (!container || isTextField(e.target as Element)) return;
  const [prev, next] = orientation === "horizontal" ? ["ArrowLeft", "ArrowRight"] : ["ArrowUp", "ArrowDown"];
  if (![prev, next, "Home", "End"].includes(e.key)) return;
  const items = focusables(container).filter((el) => opts.includeFields || !isTextField(el));
  if (items.length === 0) return;
  const at = items.indexOf(document.activeElement as HTMLElement);
  let to: number;
  if (e.key === "Home") to = 0;
  else if (e.key === "End") to = items.length - 1;
  else to = (at + (e.key === next ? 1 : -1) + items.length) % items.length;
  e.preventDefault();
  items[to].focus();
}

/**
 * Land focus on a list's first row once per open, when the rows arrive.
 *
 * {@link rovingKeydown} only moves focus that is already inside the
 * list, so something has to put it there first. Two rules make that
 * subtle enough to own in one place, both learned from the panels that
 * call this:
 *
 * - **Once per open, not once per fetch.** Every mutation in a panel
 *   refetches, and a rule keyed on the data throws the user back to the
 *   top each time.
 * - **Not while loading.** On a reopen the resource still holds the
 *   previous open's rows until the fresh fetch lands, and focus landed
 *   on one of those is lost the moment they are replaced.
 *
 * Call it from the component body: it owns a `createEffect`, so it needs
 * a Solid owner. The row is resolved inside a microtask, after render.
 */
export function focusFirstRowOnOpen<T>(
  open: () => boolean,
  rows: Resource<T>,
  list: () => HTMLElement | undefined,
  rowSelector: string,
): void {
  let focused = false;
  createEffect(() => {
    if (!open()) focused = false;
    else if (!focused && !rows.loading && rows()) {
      focused = true;
      queueMicrotask(() => list()?.querySelector<HTMLElement>(rowSelector)?.focus());
    }
  });
}

/**
 * Keep the keyboard cursor on screen.
 *
 * `j` / `k` moved the selection and nothing moved the page with it, so
 * the cursor walked off the bottom and kept going: on an 80-row page,
 * 60 presses left the view still showing row 1 with the selection
 * ~55 rows below the fold. Pressing `j` then looks like it does
 * nothing, which reads as "navigation is skipping". vim scrolls its
 * buffer for exactly this reason. `zz` (`CenterViewport`) was the only
 * way back, and a key has to be known about before it can be pressed.
 *
 * `block: "nearest"` is deliberate: centring on every keypress makes
 * the page lurch under someone stepping one row at a time, and nearest
 * does nothing at all while the row is already visible — so this is
 * inert for the common case and only acts at the edges.
 *
 * Keyed on the id rather than wired into the two movement handlers, so
 * every path that moves the cursor inherits it: `g g`, `G`, a search
 * hit, a jump out of a panel.
 */
export function followSelection(selected: () => string | null): void {
  createEffect(() => {
    const id = selected();
    if (!id) return;
    // The row may be rendering in this same tick.
    queueMicrotask(() => {
      document
        .querySelector<HTMLElement>(`[data-block-id="${id}"]`)
        ?.scrollIntoView({ block: "nearest" });
    });
  });
}

function regions(): HTMLElement[] {
  return Array.from(document.querySelectorAll<HTMLElement>("[data-outl-region]")).filter(visible);
}

function enter(region: HTMLElement): void {
  if (region.getAttribute("data-outl-region") === "outline") {
    region.focus();
    return;
  }
  (focusables(region)[0] ?? region).focus();
}

/** Move focus to the next (`1`) or previous (`-1`) region. */
export function cycleRegion(dir: 1 | -1): void {
  const all = regions();
  if (all.length === 0) return;
  const at = all.findIndex((r) => r.contains(document.activeElement));
  const to = at < 0 ? (dir === 1 ? 0 : all.length - 1) : (at + dir + all.length) % all.length;
  enter(all[to]);
}

/** Global F6 and Esc handling. Returns the cleanup. */
export function installRegionKeys(): () => void {
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "F6") {
      e.preventDefault();
      cycleRegion(e.shiftKey ? -1 : 1);
      return;
    }
    if (e.key !== "Escape" || isTextField(document.activeElement)) return;
    const region = (document.activeElement as HTMLElement | null)?.closest("[data-outl-region]");
    const name = region?.getAttribute("data-outl-region");
    if (name === "sidebar" || name === "chrome") {
      const outline = document.querySelector<HTMLElement>('[data-outl-region="outline"]');
      if (outline) {
        e.preventDefault();
        outline.focus();
      }
    }
  };
  window.addEventListener("keydown", onKey);
  return () => window.removeEventListener("keydown", onKey);
}
