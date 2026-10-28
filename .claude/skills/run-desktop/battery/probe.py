"""Layout battery for outl-desktop, run inside the lab container.

One question, asked of every visible row: **does pointing at it move
anything?** A hover affordance that takes a line of its own pushes the
rest of the page down, and the page then jumps under the pointer — at a
row boundary it flickers, because the shift moves the next row out from
under the cursor, which collapses the row, which moves it back.

That shipped once (`PropertyEditor`'s `+ prop`, measured at 18px per
hover), which is why this is a battery and not a comment.

Mouse only, deliberately. Keys reach a focused field in this container
through the GTK input method without producing a JS `keydown`, so
document-level handlers never fire and a keyboard assertion here would
pass or fail for reasons that have nothing to do with the app.
"""
import subprocess
import sys
import time

import numpy as np
from PIL import Image

DISPLAY = ":99"
OUT = "/out"
PARK = (1450, 760)          # off the content column, over empty chrome
SETTLE = 0.45               # a hover transition is ~150ms; this clears it


def sh(*args: str) -> str:
    return subprocess.run(
        args, capture_output=True, text=True,
        env={"DISPLAY": DISPLAY, "PATH": "/usr/bin:/bin"},
    ).stdout.strip()


def window() -> str:
    w = sh("xdotool", "search", "--onlyvisible", "--name", "outl")
    if not w:
        sys.exit("no outl window on the display")
    return w.splitlines()[-1]


def shot(win: str, name: str) -> Image.Image:
    path = f"{OUT}/{name}.png"
    subprocess.run(["import", "-window", win, path], check=True,
                   env={"DISPLAY": DISPLAY, "PATH": "/usr/bin:/bin"})
    return Image.open(path).convert("RGB")


def hover(x: int, y: int) -> None:
    sh("xdotool", "mousemove", str(x), str(y))
    time.sleep(SETTLE)


def ink_profile(im: Image.Image) -> np.ndarray:
    """Contrast per screen row — where a band of content sits.

    Deviation from the row's own median, not darkness: outl ships a dark
    theme, so `255 - luminance` measures the *background* and reports
    every empty row as full of ink. Deviation works on either theme,
    which is the point — a battery that only runs in light mode is a
    battery that stops running the day the default flips.
    """
    a = np.asarray(im.convert("L"), dtype=float)
    return np.abs(a - np.median(a, axis=1, keepdims=True)).sum(axis=1)


def vertical_shift(before: np.ndarray, after: np.ndarray, lo: int,
                   span: int = 140, limit: int = 60) -> int:
    """How far the band starting at `lo` moved down between the two."""
    best, best_d = 0, None
    for s in range(limit):
        a, b = before[lo:lo + span], after[lo + s:lo + s + span]
        if len(b) < len(a):
            break
        d = float(np.abs(a - b).sum())
        if best_d is None or d < best_d:
            best_d, best = d, s
    return best


def row_centres(profile: np.ndarray, top: int, bottom: int) -> list[int]:
    """Rows of text, as the midpoint of each run of inked scanlines."""
    inked = profile > profile.max() * 0.04
    runs, start = [], None
    for y in range(top, bottom):
        if inked[y] and start is None:
            start = y
        elif not inked[y] and start is not None:
            if y - start >= 4:
                runs.append((start + y) // 2)
            start = None
    return runs


def wait_for_outline(win: str, timeout: float = 90.0):
    """Poll until the outline has painted, and settled.

    The window exists long before the workspace does: the first frames
    say "Opening your workspace...", and a battery that starts on the
    window measures the splash and reports no rows. Waiting on content
    is also the honest readiness check — a fixed sleep is a guess that
    gets slower to be safe and still races on a cold machine.

    Settled means two consecutive reads agree on the row count, so a
    half-rendered outline is not mistaken for a finished one.
    """
    deadline = time.time() + timeout
    previous = -1
    while time.time() < deadline:
        im = shot(win, "battery-base")
        profile = ink_profile(im)
        rows = row_centres(profile, int(im.size[1] * 0.12),
                           int(im.size[1] * 0.62))
        if len(rows) >= 4 and len(rows) == previous:
            return im, profile, rows
        previous = len(rows)
        time.sleep(1.5)
    return None, None, []


def main() -> int:
    win = window()
    hover(*PARK)
    base, base_p, rows = wait_for_outline(win)

    if base is None:
        print("FAIL: the outline never painted. Two things look identical")
        print("      here: a workspace still opening, and a debug build")
        print("      whose `devUrl` (vite on :1421) is not up — that one")
        print("      renders 'Could not connect to localhost'.")
        print("      /out/battery-base.png has the last frame.")
        return 1

    w, h = base.size

    print(f"{len(rows)} rows under test")
    failures = []
    for y in rows:
        hover(int(w * 0.35), y)
        after = ink_profile(shot(win, f"battery-hover-{y}"))
        hover(*PARK)
        # Measure a band starting well below the hovered row: that is what
        # a reflow moves, and it is unaffected by the hover styling itself.
        probe_at = min(y + 60, h - 200)
        moved = vertical_shift(base_p, after, probe_at)
        mark = "ok" if moved == 0 else f"MOVED {moved}px"
        print(f"  row y={y:4d}: {mark}")
        if moved:
            failures.append((y, moved))

    if failures:
        print()
        print(f"FAIL: {len(failures)} row(s) move the page when pointed at.")
        print("A control revealed on hover must not take a line of its own.")
        print("Anchor it instead — `BlockRow` is `relative` and already")
        print("positions controls that way.")
        return 1

    print("\nPASS: pointing at a row moves nothing.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
