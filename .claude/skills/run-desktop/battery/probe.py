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

import datetime
import pathlib
import socket
import subprocess
import sys
import time

import numpy as np
from PIL import Image

DISPLAY = ":99"
OUT = "/out"
PARK = (1450, 760)  # off the content column, over empty chrome
SETTLE = 0.45  # a hover transition is ~150ms; this clears it


def sh(*args: str) -> str:
    return subprocess.run(
        args,
        capture_output=True,
        text=True,
        env={"DISPLAY": DISPLAY, "PATH": "/usr/bin:/bin"},
    ).stdout.strip()


def window() -> str:
    w = sh("xdotool", "search", "--onlyvisible", "--name", "outl")
    if not w:
        sys.exit("no outl window on the display")
    return w.splitlines()[-1]


def shot(win: str, name: str) -> Image.Image:
    path = f"{OUT}/{name}.png"
    subprocess.run(
        ["import", "-window", win, path],
        check=True,
        env={"DISPLAY": DISPLAY, "PATH": "/usr/bin:/bin"},
    )
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


def vertical_shift(
    before: np.ndarray, after: np.ndarray, lo: int, span: int = 140, limit: int = 60
) -> int:
    """How far the band starting at `lo` moved down between the two."""
    best, best_d = 0, None
    for s in range(limit):
        a, b = before[lo : lo + span], after[lo + s : lo + s + span]
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
        rows = row_centres(profile, int(im.size[1] * 0.12), int(im.size[1] * 0.62))
        if len(rows) >= 4 and len(rows) == previous:
            return im, profile, rows
        previous = len(rows)
        time.sleep(1.5)
    return None, None, []


def _port_open(port: int) -> bool:
    """Is anything accepting on `port`, over either address family?

    `socket.socket()` defaults to AF_INET, so a bare 127.0.0.1 probe
    reports "nothing is listening" against a server bound to ::1 only
    — which is what Vite 8 does. That false negative is worse than no
    check: it names a cause with a confident fix and sends the reader
    away from the real one, which is the exact failure this whole
    function exists to stop.
    """
    try:
        addrs = socket.getaddrinfo("localhost", port, type=socket.SOCK_STREAM)
    except socket.gaierror:
        return False
    for family, kind, proto, _canon, sockaddr in addrs:
        with socket.socket(family, kind, proto) as s:
            s.settimeout(1.0)
            if s.connect_ex(sockaddr) == 0:
                return True
    return False


def why_blank() -> list[str]:
    """Name which of the three blank-page states this run is in.

    They look identical in a screenshot and have opposite fixes, so the
    battery used to report all three as "the outline never painted" and
    send the reader to the wrong one. The probe runs *inside* the lab,
    so it can ask the workspace and the dev server directly instead of
    inferring from pixels.

    Ordered by what has to be true first: a workspace that is not there
    makes the other two unanswerable.
    """
    journals = pathlib.Path("/ws/journals")

    if not journals.is_dir():
        return [
            "the workspace is not mounted. `/ws/journals` does not exist",
            "inside the lab, so the app opened nothing and painted its",
            "empty state. A bind mount follows the inode: deleting and",
            "recreating `ws` on the host while this container runs leaves",
            "it bound to the old one, and the host still shows a perfect",
            "fixture. `run.sh` recycles the container when it sees this —",
            "if you are reading it anyway, that check was bypassed.",
        ]

    seeded = sorted(p.stem for p in journals.glob("*.md"))
    if not seeded:
        return [
            "the workspace is mounted but empty: `/ws/journals` holds no",
            "`.md`. `fixture.sh` did not run, or wrote somewhere else.",
        ]

    # The app opens *its own* today, which is the container's clock —
    # UTC here. `fixture.sh` runs on the host. A developer behind UTC
    # therefore seeds one day and the app opens the next, empty, for as
    # many hours as the offset. The fixture seeds today and tomorrow to
    # cover it; this says so when something still lands outside both.
    today = datetime.datetime.now().strftime("%Y-%m-%d")
    if today not in seeded:
        return [
            f"the app opened {today} (this container's clock, UTC) and the",
            f"fixture seeded {', '.join(seeded)} — so it opened an empty",
            "day and painted it correctly. `fixture.sh` runs on the HOST,",
            "whose date can differ by the UTC offset. It seeds today and",
            "tomorrow for that reason; this fell outside both.",
        ]

    if not _port_open(1421):
        return [
            "nothing is listening on :1421. A debug build reads `devUrl`",
            "from tauri.conf.json rather than the bundled `dist`, so the",
            "window renders one line: 'Could not connect to localhost'.",
            "See /tmp/vite.log in the container.",
        ]

    return [
        f"the workspace is mounted, {today}.md is seeded and vite is up,",
        "so this is the app itself: still opening, or opening slower than",
        "the 90s budget. See /tmp/app.log in the container.",
    ]


def main() -> int:
    win = window()
    hover(*PARK)
    base, base_p, rows = wait_for_outline(win)

    if base is None:
        print("FAIL: the outline never painted.")
        for line in why_blank():
            print(f"      {line}")
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
