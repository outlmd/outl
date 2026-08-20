/**
 * The press-and-hold recogniser, shared by the block row and the page
 * title. It exists so those two cannot drift on how long a hold is or
 * how much a finger may wander.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createLongPress } from "./long-press";

function press(x = 0, y = 0): PointerEvent {
  return { clientX: x, clientY: y } as PointerEvent;
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("createLongPress", () => {
  it("fires once the hold completes", () => {
    const onLongPress = vi.fn();
    const lp = createLongPress({ onLongPress });
    lp.onPointerDown(press());
    expect(onLongPress).not.toHaveBeenCalled();
    vi.advanceTimersByTime(500);
    expect(onLongPress).toHaveBeenCalledTimes(1);
  });

  it("does not fire when the finger lifts early", () => {
    const onLongPress = vi.fn();
    const lp = createLongPress({ onLongPress });
    lp.onPointerDown(press());
    vi.advanceTimersByTime(200);
    lp.onPointerUp();
    vi.advanceTimersByTime(500);
    expect(onLongPress).not.toHaveBeenCalled();
  });

  it("does not fire when the finger drifts — that is a scroll", () => {
    const onLongPress = vi.fn();
    const lp = createLongPress({ onLongPress });
    lp.onPointerDown(press(0, 0));
    lp.onPointerMove(press(0, 40));
    vi.advanceTimersByTime(500);
    expect(onLongPress).not.toHaveBeenCalled();
  });

  it("tolerates a small wobble — fingers are not styluses", () => {
    const onLongPress = vi.fn();
    const lp = createLongPress({ onLongPress });
    lp.onPointerDown(press(0, 0));
    lp.onPointerMove(press(3, 3));
    vi.advanceTimersByTime(500);
    expect(onLongPress).toHaveBeenCalledTimes(1);
  });

  it("swallows exactly one click after a completed hold", () => {
    // Without this the title's hold would also step the journal a day,
    // and a block's hold would open its editor under the menu.
    const lp = createLongPress({ onLongPress: () => {} });
    lp.onPointerDown(press());
    vi.advanceTimersByTime(500);
    expect(lp.consumedClick()).toBe(true);
    expect(lp.consumedClick()).toBe(false);
  });

  it("reports no click to swallow when nothing fired", () => {
    const lp = createLongPress({ onLongPress: () => {} });
    lp.onPointerDown(press());
    lp.onPointerUp();
    expect(lp.consumedClick()).toBe(false);
  });

  it("honours `disabled` — an editing row must not arm the gesture", () => {
    const onLongPress = vi.fn();
    const lp = createLongPress({ onLongPress, disabled: () => true });
    lp.onPointerDown(press());
    vi.advanceTimersByTime(500);
    expect(onLongPress).not.toHaveBeenCalled();
  });

  it("cancel drops a hold in flight (the row unmounted)", () => {
    const onLongPress = vi.fn();
    const lp = createLongPress({ onLongPress });
    lp.onPointerDown(press());
    lp.cancel();
    vi.advanceTimersByTime(500);
    expect(onLongPress).not.toHaveBeenCalled();
  });
});
