#!/usr/bin/env python3
"""Generate every app icon from the outl mark, per platform.

    uv run --with pillow --with resvg-py scripts/gen-icons.py

**The mark is geometry, not a bitmap.** `MARK_BODY` and `MARK_TIP`
below are the outl mark (the octopus, logo v2) on its 91x128 grid. Every
master is drawn from them as SVG and rasterised with resvg, so there is
one source for the picture and no hand-exported PNG to drift from it.
`assets/brand/` carries the same geometry as plain SVG files for
anything outside this repo (the site, docs, press kit).

**Every client's icons come from here.** The desktop and the mobile
crate each carry a full Tauri icon set under the same sixteen filenames;
before one generator owned them, every one of the sixteen differed byte
for byte and nothing noticed, because an icon has no test and no
compiler. `scripts/check-icons.py` holds the line in CI.

**The platforms do not want the same picture.**

- macOS draws app icons on a 1024 canvas where the tile occupies 824 and
  the rest is transparent margin, so every icon in the Dock shares a
  silhouette and an optical size. The margin is applied for the `.icns`
  alone.
- Windows and Linux apply no mask and reserve no margin: the same padding
  there would make outl smaller than its neighbours. The `.ico` and the
  loose PNGs are the rounded tile, full bleed.
- iOS and the App Store take an opaque square: the OS applies its own
  mask, and an icon that arrives pre-rounded gets rounded twice.
- Android 8+ composes an adaptive icon from a background layer (the
  gradient) and a foreground layer (the white mark inside the 66dp safe
  zone of a 108dp canvas); Android 13+ also tints a monochrome layer.
  Older launchers read the legacy `ic_launcher` and `ic_launcher_round`
  bitmaps.

What is NOT generated here: the macOS tinted (monochrome) appearance.
It needs a grayscale variant inside an asset catalog or an Icon Composer
`.icon`, compiled with `actool`, which is Xcode's and not Tauri's.
`icon-mono-1024.png` is written for whoever wires that up.
"""
import io
import pathlib
import re
import shutil
import subprocess
import sys

import resvg_py
from PIL import Image

ROOT = pathlib.Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
BRAND = ASSETS / "brand"
# The rounded app tile, full canvas. Also what the README embeds
# (`assets/logo.png` is written from the same picture).
MASTER = ASSETS / "source-icon.png"
# iOS and the App Store: opaque, square, unrounded.
SQUARE = ASSETS / "app-store-icon-1024.png"
DESKTOP = ROOT / "crates" / "outl-desktop" / "src-tauri" / "icons"
DESKTOP_WEB = ROOT / "crates" / "outl-desktop" / "public"
DMG = ROOT / "crates" / "outl-desktop" / "src-tauri" / "dmg"
MOBILE = ROOT / "crates" / "outl-mobile" / "src-tauri" / "icons"
WEB = ROOT / "crates" / "outl-mobile" / "public"
APPICON = (
    ROOT / "crates" / "outl-mobile" / "src-tauri" / "gen" / "apple"
    / "Assets.xcassets" / "AppIcon.appiconset"
)
ANDROID_RES = (
    ROOT / "crates" / "outl-mobile" / "src-tauri" / "gen" / "android"
    / "app" / "src" / "main" / "res"
)
RAYCAST = ROOT / "extensions" / "raycast" / "assets" / "command-icon.png"

# The outl mark on its 91x128 grid: the body (head plus tentacles) and
# the tip of the lowest tentacle, which only the dark mark paints lime.
MARK_W, MARK_H = 91, 128
MARK_BODY = (
    "M0 21 A21 21 0 0 1 21 0 L70 0 A21 21 0 0 1 91 21 L91 90.2 L78 98 "
    "L78 66 L72 66 L72 104.2 L59 112 L59 66 L52 66 L52 117.2 L34 128 "
    "L34 66 L25 66 L25 91 Q25 94 22 94 L3 94 Q0 94 0 91 Z"
)
MARK_TIP = "M34 115 L52 104.2 L52 117.2 L34 128 Z"

# Palette.
INK = "#0c0814"
VIOLET = "#7c3aed"
LAVENDER = "#a78bfa"
LIME = "#d6ff47"
PAPER = "#f4f1fa"

# The app tile: a diagonal gradient, darker violet bottom-left to
# lavender top-right, with the mark in paper white, its height 56% of
# the tile, centred on its bounding box.
GRADIENT = (VIOLET, LAVENDER)
MARK_SCALE = 0.56

# Apple's macOS app icon grid: a 1024 canvas, the tile 824 across,
# centred, with a corner radius of ~22.5% of the tile.
CANVAS = 1024
TILE = 824
RADIUS_RATIO = 0.225

# Android adaptive icon: a 108dp canvas, of which a 72dp disc is ever
# visible and the inner 66dp is guaranteed. The mark is sized so its
# whole bounding box stays inside that guaranteed circle.
ADAPTIVE_DP = 108
ADAPTIVE_MARK_DP = 40
DENSITIES = {"mdpi": 1, "hdpi": 1.5, "xhdpi": 2, "xxhdpi": 3, "xxxhdpi": 4}


def mark_group(cx: float, cy: float, height: float, body: str, tip: str | None = None) -> str:
    """The mark as an SVG `<g>`, its bounding box centred on (cx, cy)."""
    s = height / MARK_H
    x = cx - MARK_W * s / 2
    y = cy - height / 2
    t = f'<path d="{MARK_TIP}" fill="{tip}"/>' if tip else ""
    return (
        f'<g transform="translate({x:.3f} {y:.3f}) scale({s:.5f})">'
        f'<path d="{MARK_BODY}" fill="{body}"/>{t}</g>'
    )


def gradient_def(gid: str = "g") -> str:
    return (
        f'<defs><linearGradient id="{gid}" x1="0" y1="1" x2="1" y2="0">'
        f'<stop offset="0" stop-color="{GRADIENT[0]}"/>'
        f'<stop offset="1" stop-color="{GRADIENT[1]}"/>'
        "</linearGradient></defs>"
    )


def tile_svg(shape: str, size: int = CANVAS, scale: float = MARK_SCALE, inset: float = 0) -> str:
    """The app tile as SVG.

    `shape` is "square" (opaque, full bleed), "rounded" (the macOS corner
    proportion) or "circle". `inset` leaves a transparent margin, as a
    fraction of the canvas, on every side.
    """
    pad = size * inset
    side = size - 2 * pad
    if shape == "square":
        bg = f'<rect width="{size}" height="{size}" fill="url(#g)"/>'
    elif shape == "rounded":
        r = side * RADIUS_RATIO
        bg = (
            f'<rect x="{pad:.3f}" y="{pad:.3f}" width="{side:.3f}" height="{side:.3f}" '
            f'rx="{r:.3f}" fill="url(#g)"/>'
        )
    elif shape == "circle":
        bg = f'<circle cx="{size / 2}" cy="{size / 2}" r="{side / 2:.3f}" fill="url(#g)"/>'
    else:
        raise ValueError(shape)
    mark = mark_group(size / 2, size / 2, side * scale, PAPER)
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
        f'viewBox="0 0 {size} {size}">{gradient_def()}{bg}{mark}</svg>'
    )


def foreground_svg(size: int) -> str:
    """Android's adaptive foreground: the white mark on a transparent 108dp canvas."""
    h = size * ADAPTIVE_MARK_DP / ADAPTIVE_DP
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
        f'viewBox="0 0 {size} {size}">{mark_group(size / 2, size / 2, h, "#ffffff")}</svg>'
    )


def render(svg: str, size: int, **kw) -> Image.Image:
    png = resvg_py.svg_to_bytes(svg_string=svg, width=size, height=size, **kw)
    return Image.open(io.BytesIO(bytes(png))).convert("RGBA")


def write(im: Image.Image, path: pathlib.Path, size: int | None = None) -> None:
    out = im.resize((size, size), Image.LANCZOS) if size and im.size != (size, size) else im
    path.parent.mkdir(parents=True, exist_ok=True)
    out.save(path)
    print(f"  {path.relative_to(ROOT)}  {out.size[0]}x{out.size[1]}")


def write_text(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    print(f"  {path.relative_to(ROOT)}")


def macos_master(src: Image.Image) -> Image.Image:
    """The master macOS wants: the tile inset on a transparent canvas."""
    return render(tile_svg("rounded", inset=(CANVAS - TILE) / 2 / CANVAS), CANVAS)


def monochrome(src: Image.Image) -> Image.Image:
    """A grayscale read of the artwork, for the tinted appearance.

    Luminance only: macOS tints a monochrome image, so colour in it is
    either ignored or fights the tint.
    """
    grey = src.convert("LA").convert("RGBA")
    grey.putalpha(src.split()[3])
    return grey


def brand_svgs() -> None:
    """Plain SVGs of the mark and the tile, for use outside this repo."""
    def mark(body: str, tip: str | None = None) -> str:
        t = f'<path d="{MARK_TIP}" fill="{tip}"/>' if tip else ""
        return (
            f'<svg xmlns="http://www.w3.org/2000/svg" width="{MARK_W * 4}" height="{MARK_H * 4}" '
            f'viewBox="0 0 {MARK_W} {MARK_H}"><path d="{MARK_BODY}" fill="{body}"/>{t}</svg>\n'
        )

    write_text(BRAND / "mark-dark.svg", mark(LAVENDER, LIME))
    write_text(BRAND / "mark-light.svg", mark(VIOLET))
    write_text(BRAND / "mark-mono.svg", mark(PAPER))
    write_text(BRAND / "app-icon.svg", tile_svg("rounded") + "\n")
    write_text(BRAND / "app-icon-square.svg", tile_svg("square") + "\n")


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
        print("  (no appiconset, run `cargo tauri ios init` first)")
        return
    flat = square.convert("RGB")
    for f in sorted(APPICON.glob("AppIcon-*.png")):
        # `AppIcon-512@2x.png` (the 1024 App Store slot) names no height.
        m = re.match(r"AppIcon-([\d.]+)(?:x[\d.]+)?@(\d)x", f.name)
        if not m:
            continue
        px = round(float(m.group(1)) * int(m.group(2)))
        # No alpha: the App Store rejects an icon with one.
        flat.resize((px, px), Image.LANCZOS).save(f)
    print(f"  {APPICON.relative_to(ROOT)}  {len(list(APPICON.glob('AppIcon-*.png')))} slots")


def vector_mark(fill: str) -> str:
    """The mark as an Android vector drawable on the 108dp adaptive canvas."""
    s = ADAPTIVE_MARK_DP / MARK_H
    x = (ADAPTIVE_DP - MARK_W * s) / 2
    y = (ADAPTIVE_DP - ADAPTIVE_MARK_DP) / 2
    return f"""<?xml version="1.0" encoding="utf-8"?>
<!-- Generated by scripts/gen-icons.py from the outl mark. Do not edit. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <group
        android:translateX="{x:.4f}"
        android:translateY="{y:.4f}"
        android:scaleX="{s:.6f}"
        android:scaleY="{s:.6f}">
        <path
            android:fillColor="{fill}"
            android:pathData="{MARK_BODY}" />
    </group>
</vector>
"""


def android() -> None:
    """Adaptive layers (vector) plus the legacy bitmaps, per density."""
    if not ANDROID_RES.is_dir():
        print("  (no android project, run `cargo tauri android init` first)")
        return
    write_text(ANDROID_RES / "drawable" / "ic_launcher_background.xml", f"""<?xml version="1.0" encoding="utf-8"?>
<!-- Generated by scripts/gen-icons.py: the app tile gradient. Do not edit. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:aapt="http://schemas.android.com/aapt"
    android:width="108dp"
    android:height="108dp"
    android:viewportWidth="108"
    android:viewportHeight="108">
    <path android:pathData="M0,0h108v108h-108z">
        <aapt:attr name="android:fillColor">
            <gradient
                android:type="linear"
                android:startX="0"
                android:startY="108"
                android:endX="108"
                android:endY="0"
                android:startColor="{GRADIENT[0].upper()}"
                android:endColor="{GRADIENT[1].upper()}" />
        </aapt:attr>
    </path>
</vector>
""")
    write_text(ANDROID_RES / "drawable-v24" / "ic_launcher_foreground.xml", vector_mark("#FFFFFF"))
    write_text(ANDROID_RES / "drawable" / "ic_launcher_monochrome.xml", vector_mark("#FFFFFF"))
    adaptive = """<?xml version="1.0" encoding="utf-8"?>
<!-- Generated by scripts/gen-icons.py. Do not edit. -->
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@drawable/ic_launcher_background" />
    <foreground android:drawable="@drawable/ic_launcher_foreground" />
    <monochrome android:drawable="@drawable/ic_launcher_monochrome" />
</adaptive-icon>
"""
    for name in ("ic_launcher", "ic_launcher_round"):
        write_text(ANDROID_RES / "mipmap-anydpi-v26" / f"{name}.xml", adaptive)

    # Legacy launchers (before Android 8) and the foreground bitmaps the
    # Tauri template ships. The legacy square keeps the material margin
    # of 2dp on a 48dp canvas.
    legacy = render(tile_svg("rounded", inset=2 / 48, scale=0.5), CANVAS)
    legacy_round = render(tile_svg("circle", inset=2 / 48, scale=0.5), CANVAS)
    fg = render(foreground_svg(CANVAS), CANVAS)
    for bucket, k in DENSITIES.items():
        d = ANDROID_RES / f"mipmap-{bucket}"
        write(legacy, d / "ic_launcher.png", round(48 * k))
        write(legacy_round, d / "ic_launcher_round.png", round(48 * k))
        write(fg, d / "ic_launcher_foreground.png", round(108 * k))


def web(src: Image.Image) -> None:
    """The webviews' own icons, named by each `index.html`."""
    write(src, WEB / "logo.png", 512)
    write(src, WEB / "logo-192.png", 192)
    # apple-touch-icon is masked by iOS like an app icon: opaque square.
    write(render(tile_svg("square"), CANVAS).convert("RGB"), WEB / "apple-touch-icon.png", 180)
    for out in (WEB / "favicon.ico", DESKTOP_WEB / "favicon.ico"):
        src.resize((64, 64), Image.LANCZOS).save(
            out, sizes=[(16, 16), (32, 32), (48, 48), (64, 64)]
        )
        print(f"  {out.relative_to(ROOT)}  16..64")


def dmg_background() -> None:
    """Re-render the DMG backdrop, which embeds `assets/logo.png`."""
    svg = DMG / "background.svg"
    if not svg.is_file():
        return
    im = Image.open(DMG / "background.png")
    png = resvg_py.svg_to_bytes(
        svg_path=str(svg), width=im.size[0], height=im.size[1], resources_dir=str(DMG)
    )
    (DMG / "background.png").write_bytes(bytes(png))
    print(f"  {(DMG / 'background.png').relative_to(ROOT)}  {im.size[0]}x{im.size[1]}")


def main() -> int:
    print("Brand SVGs")
    brand_svgs()

    print("\nMasters")
    src = render(tile_svg("rounded"), CANVAS)
    write(src, MASTER)
    write(src, ASSETS / "logo.png")
    square = render(tile_svg("square"), CANVAS).convert("RGB")
    write(square, SQUARE)

    print("\nmacOS (inset on the 1024 grid, so the Dock sizes it like the rest)")
    mac = macos_master(src)
    write(mac, ASSETS / "macos-icon-1024.png")

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
    else:
        # Pillow writes the same ic07..ic10 entries iconutil would.
        mac.save(DESKTOP / "icon.icns", format="ICNS")
    shutil.copyfile(DESKTOP / "icon.icns", MOBILE / "icon.icns")
    print(f"  {(DESKTOP / 'icon.icns').relative_to(ROOT)}  (+ the mobile copy)")
    shutil.rmtree(iconset)

    for label, out in (("desktop", DESKTOP), ("mobile", MOBILE)):
        print(f"\nTauri set, {label} (full bleed: Windows and Linux neither")
        print("  mask nor reserve margin, so the macOS inset would shrink it)")
        tauri_set(src, out)

    print("\niOS app icon (opaque square: the OS masks it itself)")
    ios_appicon(square)

    print("\nAndroid (adaptive layers + legacy bitmaps)")
    android()

    print("\nWeb icons in the webviews")
    web(src)

    print("\nRaycast command icon")
    write(src, RAYCAST, 512)

    print("\nDMG background (embeds assets/logo.png)")
    dmg_background()

    print("\nmacOS tinted (monochrome source; needs actool to ship, see module doc)")
    write(monochrome(mac), ASSETS / "icon-mono-1024.png")
    return 0


if __name__ == "__main__":
    sys.exit(main())
