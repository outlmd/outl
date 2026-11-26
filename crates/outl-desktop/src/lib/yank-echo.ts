/**
 * The second `y` of a Visual `yy`.
 *
 * In Visual, `y` alone yanks the range and drops back to Normal. A vim
 * hand types `yy` anyway, and that second `y` lands in Normal as the
 * start of `y y` / `y r`. The `p` that follows then completes `[y, p]`,
 * which matches nothing, and the dispatcher throws the `p` away: the
 * user yanked, pressed paste, and nothing happened.
 *
 * So after a `YankRange` fired by a plain `y`, exactly one more plain
 * `y` within the chord window is swallowed, and so is every key repeat
 * of a held `y`. Any other key ends it. Nothing is re-dispatched: a
 * broken sequence replayed as something else (`d t` jumping to today)
 * is worse than a key that did nothing.
 */

import type { Chord } from "./api";

/** Same window the dispatcher gives a chord prefix. */
const ECHO_MS = 1000;

export interface YankEcho {
  /** A plain `y` just yanked a Visual range. */
  arm(): void;
  /** Whether this keydown is the echo to swallow. Any other key disarms. */
  swallows(e: KeyboardEvent, chord: Chord): boolean;
  reset(): void;
}

export function createYankEcho(): YankEcho {
  let deadline: number | null = null;
  let spent = false;

  return {
    arm() {
      deadline = Date.now() + ECHO_MS;
      spent = false;
    },
    swallows(e, chord) {
      if (deadline === null) return false;
      const plainY =
        chord.mods === 0 && chord.key.kind === "Char" && chord.key.value === "y";
      if (plainY && e.repeat) return true;
      if (plainY && !spent && Date.now() <= deadline) {
        spent = true;
        return true;
      }
      deadline = null;
      return false;
    },
    reset() {
      deadline = null;
    },
  };
}
