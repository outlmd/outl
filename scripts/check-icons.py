#!/usr/bin/env python3
"""Check the committed app icons against what `gen-icons.py` promises.

An icon has no test and no compiler, which is how the desktop and the
mobile crate ended up carrying a full Tauri set under the same sixteen
filenames with every one of the sixteen differing byte for byte. Nothing
noticed until a user said outl looked bigger than its neighbours in the
Dock. `scripts/gen-icons.py` made one master the only source; this is
what stops the drift coming back through a hand-edited file or a
half-finished run.

Three things it asks, all from the PNG header, so it needs no Pillow and
nothing but the standard library:

1. **The two Tauri sets are the same set.** Same filenames, byte for
   byte identical. That is the drift above.
2. **Each PNG is the size its name claims.** `128x128@2x.png` at 128
   square, or a `Square107x107Logo.png` at 106, is a generator run that
   went wrong quietly.
3. **No iOS app icon carries an alpha channel.** Apple rejects an
   upload whose icon has one ("can't be transparent nor contain an
   alpha channel"), and the rejection arrives from App Store Connect
   after the build, not from the build. The colour-type byte in the
   PNG header answers it outright.

What it deliberately does not check: the macOS margin, the thing that
was actually reported. Verifying it means decoding pixels, and a PNG
decoder inside a hygiene script is a worse trade than the generator
being the only writer of that file. `gen-icons.py` holds the grid and
the reason.

    python3 scripts/check-icons.py
"""
import hashlib
import pathlib
import re
import struct
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
DESKTOP = ROOT / "crates" / "outl-desktop" / "src-tauri" / "icons"
MOBILE = ROOT / "crates" / "outl-mobile" / "src-tauri" / "icons"
APPICON = (
    ROOT / "crates" / "outl-mobile" / "src-tauri" / "gen" / "apple"
    / "Assets.xcassets" / "AppIcon.appiconset"
)
MASTERS = [
    ROOT / "assets" / "source-icon.png",
    ROOT / "assets" / "app-store-icon-1024.png",
    ROOT / "assets" / "macos-icon-1024.png",
    ROOT / "assets" / "icon-mono-1024.png",
]

# PNG colour types that carry an alpha channel outright. Type 3
# (palette) can still be transparent through a tRNS chunk, which is
# checked separately.
ALPHA_TYPES = {4, 6}


def header(path: pathlib.Path) -> tuple[int, int, int]:
    """`(width, height, colour_type)` from the PNG's IHDR."""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path.relative_to(ROOT)}: not a PNG")
    width, height = struct.unpack(">II", data[16:24])
    return width, height, data[25]


def has_trns(path: pathlib.Path) -> bool:
    return b"tRNS" in path.read_bytes()


def expected_size(name: str) -> int | None:
    """The square side the filename claims, or None if it claims none.

    Three spellings, because three toolchains name their own slots:
    Tauri's loose PNGs, the Windows `SquareNxNLogo` set, and Xcode's
    `AppIcon-<pt>@<scale>x`, where the points can be fractional
    (`83.5x83.5@2x` is 167px) and a `-1` suffix is a second slot asking
    for the same picture under another name.
    """
    if m := re.fullmatch(r"(\d+)x\1(?:@(\d)x)?\.png", name):
        return int(m[1]) * int(m[2] or 1)
    if m := re.fullmatch(r"Square(\d+)x\1Logo\.png", name):
        return int(m[1])
    if m := re.fullmatch(r"AppIcon-([\d.]+)(?:x\1)?@(\d)x(?:-\d)?\.png", name):
        return round(float(m[1]) * int(m[2]))
    return {"StoreLogo.png": 50, "icon.png": 512}.get(name)


def main() -> int:
    bad: list[str] = []

    for master in MASTERS:
        if not master.is_file():
            bad.append(f"missing master: {master.relative_to(ROOT)}")

    # 1. One set, two places.
    left = {p.name for p in DESKTOP.iterdir() if p.is_file()}
    right = {p.name for p in MOBILE.iterdir() if p.is_file()}
    for name in sorted(left ^ right):
        where = "mobile" if name in left else "desktop"
        bad.append(f"{name}: missing from the {where} icon set")
    for name in sorted(left & right):
        a = hashlib.sha256((DESKTOP / name).read_bytes()).hexdigest()
        b = hashlib.sha256((MOBILE / name).read_bytes()).hexdigest()
        if a != b:
            bad.append(
                f"{name}: desktop and mobile differ "
                f"({a[:12]} vs {b[:12]}) — regenerate with scripts/gen-icons.py"
            )

    # 2. The size the name claims.
    for d in (DESKTOP, MOBILE, APPICON):
        if not d.is_dir():
            bad.append(f"missing icon directory: {d.relative_to(ROOT)}")
            continue
        for png in sorted(d.glob("*.png")):
            want = expected_size(png.name)
            if want is None:
                continue
            w, h, _ = header(png)
            if (w, h) != (want, want):
                bad.append(
                    f"{png.relative_to(ROOT)}: {w}x{h}, name claims {want}x{want}"
                )

    # 3. Apple takes no alpha.
    if APPICON.is_dir():
        for png in sorted(APPICON.glob("*.png")):
            _, _, colour = header(png)
            if colour in ALPHA_TYPES or has_trns(png):
                bad.append(
                    f"{png.relative_to(ROOT)}: has an alpha channel; "
                    "App Store Connect rejects the upload"
                )

    if bad:
        print("icon check failed:", file=sys.stderr)
        for line in bad:
            print(f"  {line}", file=sys.stderr)
        return 1
    print("icons ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
