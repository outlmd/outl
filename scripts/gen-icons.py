#!/usr/bin/env python3
"""Generate every app icon from the one master, per-platform.

    uv run --with pillow scripts/gen-icons.py

**Every client's icons come from here, which is the other half of the
point.** The desktop and the mobile crate each carried a full Tauri
icon set under the same sixteen filenames, generated at different times
from different runs: every one of the sixteen differed byte for byte.
Nothing could notice, because an icon has no test and no compiler. One
generator over one master is what stops that.

**The platforms do not want the same picture, and that is the whole
other point of this script.** macOS draws app icons on a 1024 canvas where
the tile occupies 824 and the rest is transparent margin, so every icon
in the Dock shares a silhouette and an optical size. An icon that fills
its canvas edge to edge is not "bigger" by mistake — it is drawn at the
size the others reserve for margin, and it stands out next to every
system app. outl shipped exactly that.

Windows and Linux apply no mask and reserve no margin: the same padding
there would make outl smaller than its neighbours instead. So the
margin is applied for the `.icns` alone, and the `.ico` and the loose
PNGs stay full bleed.

What is NOT generated here: the macOS tinted (monochrome) appearance.
It needs a grayscale variant inside an asset catalog or an Icon
Composer `.icon`, compiled with `actool`, which is Xcode's and not
Tauri's. `icon-mono-1024.png` is written for whoever wires that up;
see `docs/` and the note at the bottom of this file.
"""
import pathlib
import re
import shutil
import subprocess
import sys

from PIL import Image, ImageDraw

ROOT = pathlib.Path(__file__).resolve().parent.parent
MASTER = ROOT / "assets" / "source-icon.png"
# `assets/logo.png` is byte-identical to the master and is what the
# README embeds. Left alone deliberately: regenerating it would rewrite
# the bytes through this encoder for no change in the picture. Collapsing
# the two names is a call for whoever owns the README.
# iOS and the App Store take an opaque square: the OS applies its own
# mask, and an icon that arrives pre-rounded gets rounded twice.
SQUARE = ROOT / "assets" / "app-store-icon-1024.png"
DESKTOP = ROOT / "crates" / "outl-desktop" / "src-tauri" / "icons"
MOBILE = ROOT / "crates" / "outl-mobile" / "src-tauri" / "icons"
WEB = ROOT / "crates" / "outl-mobile" / "public"
APPICON = (
    ROOT / "crates" / "outl-mobile" / "src-tauri" / "gen" / "apple"
    / "Assets.xcassets" / "AppIcon.appiconset"
)

# Apple's macOS app icon grid: a 1024 canvas, the tile 824 across,
# centred, with a corner radius of ~22.5% of the tile.
CANVAS = 1024
TILE = 824
RADIUS = round(TILE * 0.225)


def squircle(size: int, radius: int) -> Image.Image:
    """A rounded-square mask at the macOS corner proportion."""
    m = Image.new("L", (size * 4, size * 4), 0)
    ImageDraw.Draw(m).rounded_rectangle(
        (0, 0, size * 4 - 1, size * 4 - 1), radius=radius * 4, fill=255
    )
    return m.resize((size, size), Image.LANCZOS)


def macos_master(src: Image.Image) -> Image.Image:
    """The master macOS wants: tile inset on a transparent canvas."""
    tile = src.resize((TILE, TILE), Image.LANCZOS)
    tile.putalpha(squircle(TILE, RADIUS))
    canvas = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    canvas.paste(tile, ((CANVAS - TILE) // 2, (CANVAS - TILE) // 2), tile)
    return canvas


def monochrome(src: Image.Image) -> Image.Image:
    """A grayscale read of the artwork, for the tinted appearance.

    Luminance only: macOS tints a monochrome image, so colour in it is
    either ignored or fights the tint.
    """
    grey = src.convert("LA").convert("RGBA")
    grey.putalpha(src.split()[3])
    return grey


def write(im: Image.Image, path: pathlib.Path, size: int | None = None) -> None:
    out = im.resize((size, size), Image.LANCZOS) if size else im
    path.parent.mkdir(parents=True, exist_ok=True)
    out.save(path)
    print(f"  {path.relative_to(ROOT)}  {out.size[0]}x{out.size[1]}")


def tauri_set(src: Image.Image, out: pathlib.Path) -> None:
    """The sixteen files a Tauri `bundle.icon` list names.

    Full bleed: Windows and Linux neither mask nor reserve margin, so
    the macOS inset would simply make outl smaller than its neighbours
    there. The `.icns` in this set is written separately, from the
    inset master.
    """
    write(src, out / "icon.png", 512)
    write(src, out / "32x32.png", 32)
    write(src, out / "128x128.png", 128)
    write(src, out / "128x128@2x.png", 256)
    src.resize((256, 256), Image.LANCZOS).save(
        out / "icon.ico",
        sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
    )
    print(f"  {(out / 'icon.ico').relative_to(ROOT)}  16..256")
    for size in (30, 44, 71, 89, 107, 142, 150, 284, 310):
        tile = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        inner = round(size * 0.66)
        art = src.resize((inner, inner), Image.LANCZOS)
        tile.paste(art, ((size - inner) // 2, (size - inner) // 2), art)
        write(tile, out / f"Square{size}x{size}Logo.png")
    write(src, out / "StoreLogo.png", 50)


def ios_appicon(square: Image.Image) -> None:
    """Every slot Xcode's asset catalog already declares.

    The sizes are read off the filenames the catalog ships with rather
    than invented: `AppIcon-29x29@3x.png` is 87px. Anything not already
    in the catalog is not a slot iOS asks for.
    """
    if not APPICON.is_dir():
        print("  (no appiconset — run `cargo tauri ios init` first)")
        return
    flat = square.convert("RGB")
    for f in sorted(APPICON.glob("AppIcon-*.png")):
        m = re.match(r"AppIcon-(\d+)x\d+@(\d)x", f.name)
        if not m:
            continue
        px = int(m.group(1)) * int(m.group(2))
        # No alpha: the App Store rejects an icon with one.
        flat.resize((px, px), Image.LANCZOS).save(f)
    print(f"  {APPICON.relative_to(ROOT)}  {len(list(APPICON.glob('AppIcon-*.png')))} slots")


def web(src: Image.Image) -> None:
    """The webview's own icons, named by `index.html`."""
    write(src, WEB / "logo.png", 512)
    write(src, WEB / "logo-192.png", 192)
    write(src, WEB / "apple-touch-icon.png", 180)
    src.resize((64, 64), Image.LANCZOS).save(
        WEB / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48), (64, 64)]
    )
    print(f"  {(WEB / 'favicon.ico').relative_to(ROOT)}  16..64")


def main() -> int:
    if not MASTER.exists():
        print(f"missing master: {MASTER}", file=sys.stderr)
        return 1
    src = Image.open(MASTER).convert("RGBA")
    if src.size != (CANVAS, CANVAS):
        src = src.resize((CANVAS, CANVAS), Image.LANCZOS)

    square = Image.open(SQUARE).convert("RGBA") if SQUARE.exists() else src

    print("macOS (inset on the 1024 grid, so the Dock sizes it like the rest)")
    mac = macos_master(src)
    write(mac, ROOT / "assets" / "macos-icon-1024.png")

    iconset = DESKTOP / "icon.iconset"
    if iconset.exists():
        shutil.rmtree(iconset)
    iconset.mkdir(parents=True)
    for size in (16, 32, 128, 256, 512):
        write(mac, iconset / f"icon_{size}x{size}.png", size)
        write(mac, iconset / f"icon_{size}x{size}@2x.png", size * 2)
    if shutil.which("iconutil"):
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(DESKTOP / "icon.icns")],
            check=True,
        )
        shutil.copyfile(DESKTOP / "icon.icns", MOBILE / "icon.icns")
        print(f"  {(DESKTOP / 'icon.icns').relative_to(ROOT)}  (+ the mobile copy)")
        shutil.rmtree(iconset)
    else:
        print("  iconutil not found (not macOS): .iconset left for a mac to compile")

    for label, out in (("desktop", DESKTOP), ("mobile", MOBILE)):
        print(f"\nTauri set, {label} (full bleed: Windows and Linux neither")
        print("  mask nor reserve margin, so the macOS inset would shrink it)")
        tauri_set(src, out)

    print("\niOS app icon (opaque square: the OS masks it itself)")
    ios_appicon(square)

    print("\nWeb icons in the mobile webview")
    web(src)

    print("\nmacOS tinted (monochrome source; needs actool to ship — see module doc)")
    write(monochrome(mac), ROOT / "assets" / "icon-mono-1024.png")
    return 0


if __name__ == "__main__":
    sys.exit(main())
