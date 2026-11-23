/**
 * The question the desktop asks before something it cannot undo.
 *
 * `window.confirm` answered the same question and could take no theme
 * token, no destructive signal and no focus rule — so what is under test
 * here is the three things it could not do: the sentence is the caller's,
 * the acting button carries the `destructive` role, and the caret goes
 * back where the question interrupted it.
 */

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ConfirmDialog } from "./ConfirmDialog";
import { askConfirm, pendingConfirm } from "../lib/confirm";

let dispose: (() => void) | undefined;

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <ConfirmDialog />, host);
}

const buttons = () => [...document.body.querySelectorAll("button")] as HTMLButtonElement[];
const byText = (text: string) => buttons().find((b) => b.textContent?.trim() === text);
const card = () => document.body.querySelector("[role='alertdialog']");

const press = (key: string, init: KeyboardEventInit = {}) =>
  (document.activeElement ?? document.body).dispatchEvent(
    new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }),
  );

const destructive = {
  message: "Merge this page? This cannot be undone here.",
  confirmLabel: "Merge",
  label: "Merge pages",
  destructive: true,
};

describe("ConfirmDialog", () => {
  it("asks nothing until someone does", () => {
    mount();
    expect(card()).toBeNull();
    expect(pendingConfirm()).toBeNull();
  });

  it("shows the caller's sentence and resolves true on the acting button", async () => {
    mount();
    const answer = askConfirm(destructive);
    await vi.waitFor(() => expect(card()).not.toBeNull());
    expect(document.body.textContent).toContain(destructive.message);
    byText("Merge")!.click();
    expect(await answer).toBe(true);
    expect(card()).toBeNull();
  });

  it("resolves false on cancel, on Escape and on the backdrop", async () => {
    mount();

    const cancelled = askConfirm(destructive);
    await vi.waitFor(() => expect(card()).not.toBeNull());
    byText("Cancel")!.click();
    expect(await cancelled).toBe(false);

    const escaped = askConfirm(destructive);
    await vi.waitFor(() => expect(card()).not.toBeNull());
    press("Escape");
    expect(await escaped).toBe(false);

    const backdropped = askConfirm(destructive);
    await vi.waitFor(() => expect(card()).not.toBeNull());
    const backdrop = card()!.parentElement as HTMLElement;
    backdrop.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(await backdropped).toBe(false);
  });

  /** A merge deletes a page: the acting button is a solid `destructive`
   *  fill with its paired foreground, never `warn` and never a hex. */
  it("paints a destructive action with the destructive role", async () => {
    mount();
    void askConfirm(destructive);
    await vi.waitFor(() => expect(card()).not.toBeNull());
    const act = byText("Merge")!;
    expect(act.className).toContain("bg-(--color-outl-destructive)");
    expect(act.className).toContain("text-(--color-outl-destructive-fg)");
    expect(act.className).not.toContain("warn");
    expect(document.body.innerHTML).not.toMatch(/#[0-9a-fA-F]{6}\b/);
  });

  it("opens on the safe button and keeps Tab inside the card", async () => {
    mount();
    void askConfirm(destructive);
    await vi.waitFor(() => expect(document.activeElement).toBe(byText("Cancel")));
    press("Tab");
    expect(document.activeElement).toBe(byText("Merge"));
    press("Tab");
    expect(document.activeElement).toBe(byText("Cancel"));
    press("Tab", { shiftKey: true });
    expect(document.activeElement).toBe(byText("Merge"));
  });

  it("puts the caret back where the question interrupted it", async () => {
    mount();
    const opener = document.createElement("button");
    document.body.appendChild(opener);
    opener.focus();

    const answer = askConfirm(destructive);
    await vi.waitFor(() => expect(card()).not.toBeNull());
    byText("Cancel")!.click();
    await answer;

    expect(document.activeElement).toBe(opener);
  });

  /** The dialog owns the keyboard while it is up: the Normal-mode
   *  dispatcher must not read these as chords. */
  it("keeps its keystrokes away from the Normal-mode dispatcher", async () => {
    const seen = vi.fn();
    window.addEventListener("keydown", seen);
    try {
      mount();
      void askConfirm(destructive);
      await vi.waitFor(() => expect(card()).not.toBeNull());
      press("d");
      expect(seen).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("keydown", seen);
    }
  });

  /**
   * With nothing mounted, a caller's `await` would never settle and the
   * flow behind it would hang silently — the quietest failure there is.
   * The unanswerable case resolves to the side that writes nothing.
   */
  it("refuses at once when no dialog is mounted to answer", async () => {
    const logged = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      expect(await askConfirm(destructive)).toBe(false);
      expect(logged).toHaveBeenCalled();
    } finally {
      logged.mockRestore();
    }
  });

  /** Two questions cannot be on screen at once, and a promise nobody
   *  can answer is a flow that never finishes: the older one is a
   *  refusal, which writes nothing. */
  it("refuses the older question when a second one arrives", async () => {
    mount();
    const first = askConfirm(destructive);
    const second = askConfirm({ ...destructive, message: "second" });
    expect(await first).toBe(false);
    await vi.waitFor(() => expect(document.body.textContent).toContain("second"));
    byText("Merge")!.click();
    expect(await second).toBe(true);
  });
});
