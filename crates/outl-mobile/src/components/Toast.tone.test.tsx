/**
 * The toast's tone.
 *
 * One pill used to carry two meanings: a rename summary landed in the
 * destructive fill with a warning glyph, because the pill was
 * unconditionally red. That is root `CLAUDE.md` invariant 13 — a colour
 * token with one meaning per client — and it reaches the user hardest on
 * the rename path, where succeeding is the expected outcome.
 *
 * These pin that a success wears the accent fill pair, that a failure is
 * unchanged, that both pairs come from `Palette` tokens rather than a hex
 * literal, and that the multiline dwell a merge report needs survived.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Toast } from "./Toast";

let dispose: (() => void) | undefined;

function mount(props: Partial<Parameters<typeof Toast>[0]> = {}) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const onDismiss = vi.fn();
  dispose = render(
    () => <Toast message="done" onDismiss={onDismiss} {...props} />,
    host,
  );
  return { host, onDismiss };
}

/** The capsule itself: the element carrying the fill. */
const pill = (host: HTMLElement) =>
  host.querySelector('[role="status"] > div') as HTMLElement;

beforeEach(() => vi.useFakeTimers());

afterEach(() => {
  vi.useRealTimers();
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

describe("Toast tone", () => {
  it("defaults to the destructive pill, so nothing about failures moved", () => {
    const { host } = mount();
    const cls = pill(host).className;

    expect(cls).toContain("bg-(--color-outl-destructive)/95");
    expect(cls).toContain("text-(--color-outl-destructive-fg)");
    expect(cls).not.toContain("--color-outl-accent");
  });

  it("an explicit error tone is the same pill as the default", () => {
    const { host } = mount({ tone: "error" });

    expect(pill(host).className).toContain("bg-(--color-outl-destructive)/95");
  });

  it("a success wears the accent fill pair, not the destructive one", () => {
    const { host } = mount({ tone: "success", message: "Renamed. 2 links updated." });
    const cls = pill(host).className;

    expect(cls).toContain("bg-(--color-outl-accent)/95");
    expect(cls).toContain("text-(--color-outl-accent-fg)");
    expect(cls).not.toContain("--color-outl-destructive");
  });

  /** Colour is not the only carrier: the glyph says it too. */
  it("swaps the warning glyph for a check", () => {
    const { host: bad } = mount({ tone: "error" });
    const warning = bad.querySelector('[role="status"] circle');
    expect(warning).not.toBeNull();
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = "";

    const { host: good } = mount({ tone: "success" });
    expect(good.querySelector('[role="status"] circle')).toBeNull();
    expect(good.querySelector('[role="status"] svg path')?.getAttribute("d")).toBe(
      "M20 6L9 17l-5-5",
    );
  });

  /**
   * Every fill on this pill is a `Palette` field behind a
   * `--color-outl-*` token, so a theme cannot make it unreadable
   * (invariant 13). A hex literal here would be a second definition.
   */
  it("names no hex colour", () => {
    const { host } = mount({ tone: "success" });
    expect(pill(host).className).not.toMatch(/#[0-9a-f]{3,8}/i);
  });
});

describe("Toast dwell", () => {
  it("still holds a multiline report for 12s, not 5s", () => {
    const { onDismiss } = mount({
      tone: "success",
      message: "3 blocks moved.\nProperty conflict: kept \"open\".",
    });

    vi.advanceTimersByTime(5000);
    expect(onDismiss).not.toHaveBeenCalled();
    vi.advanceTimersByTime(7000);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("a one-line success dismisses on the usual 5s", () => {
    const { onDismiss } = mount({ tone: "success", message: "Renamed." });

    vi.advanceTimersByTime(5000);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
