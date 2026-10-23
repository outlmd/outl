#!/usr/bin/env python3
"""The one list of what a release publishes, and who consumes each asset.

Four places have to agree about the platforms outl ships, and none of them can
see the other three:

  * `.github/workflows/release.yml` — the matrices that build the artifacts and
    the `update_tap` loops that bump the package managers.
  * `Formula/outl-beta.rb` — a download `sha256` and a bottle digest per
    platform Homebrew serves.
  * `bucket/outl-beta.json` — a url + hash per architecture Scoop serves.
  * `Casks/outl-desktop-beta.rb` — the macOS desktop dmg.

Every way they can disagree is silent. A matrix leg added without its bump
entry uploads an asset nobody points at; a bump entry added without its anchor
runs a `sed` that matches no line and changes nothing, exits 0, and leaves
`update_tap` to finish on `git diff --quiet` — which only asks whether
*something* changed. The first thing to notice is a user's `brew install` or
`scoop install`, one release later, with a hash mismatch or the previous
build's binary under the new version number.

So the list lives here, once, and `check-release-assets.py` holds the four
files against it.

`required` is the other half of this file's job. A release used to be gated on
every build job succeeding, which meant one architecture's bundler could hold
back every other platform's assets — and the alternative that was in the repo
before it (best-effort, like the Android APK) published the gap in silence.
Neither is acceptable, so the gate is per asset: a missing `required` asset
fails the publish, a missing optional one publishes with the gap named in the
release notes and in the run summary. `optional` is for an architecture during
bring-up; promoting it is flipping one word here, and the flip is the record
that it has been green.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class Leg:
    """One build-matrix leg and the assets it uploads."""

    job: str
    """The `release.yml` job whose matrix carries this leg."""

    key: str
    """How the matrix identifies the leg: `target` under `build`, `arch` under
    the desktop jobs. The checker matches on this, so a renamed matrix key
    fails rather than silently describing a leg that no longer exists."""

    assets: tuple[str, ...]
    """Exact asset names. Every one also ships `<name>.sha256`, which the
    checker adds itself rather than listing twice."""

    required: bool
    """False means a failure here never holds back the rest of the release.
    The gap is still reported — see the module docstring."""

    anchor: str | None = None
    """The name the package-manager anchors use for this platform
    (`# anchor: linux-arm64`, `"// anchor: arm64"`). None for a platform no
    package manager serves."""

    bottle_tag: str | None = None
    """Homebrew bottle tag, when this leg stages a bottle."""

    taps: tuple[str, ...] = field(default_factory=tuple)
    """Which manifests point at these assets: `brew`, `cask`, `scoop`. A
    manifest only advances when every asset it points at is present, so these
    are the assets `update_tap` refuses to bump around."""


# CLI / TUI, from the `build` matrix. `artifact` in the matrix is the asset
# stem; the extension is `.tar.gz` everywhere but Windows.
CLI: tuple[Leg, ...] = (
    Leg(
        job="build",
        key="aarch64-apple-darwin",
        assets=("outl-macos-arm64.tar.gz",),
        required=True,
        anchor="macos-arm64",
        bottle_tag="arm64_ventura",
        taps=("brew",),
    ),
    Leg(
        job="build",
        key="x86_64-apple-darwin",
        assets=("outl-macos-x64.tar.gz",),
        required=True,
        anchor="macos-x64",
        bottle_tag="ventura",
        taps=("brew",),
    ),
    Leg(
        job="build",
        key="x86_64-unknown-linux-gnu",
        assets=("outl-linux-x64.tar.gz",),
        required=True,
        anchor="linux-x64",
        bottle_tag="x86_64_linux",
        taps=("brew",),
    ),
    Leg(
        job="build",
        key="aarch64-unknown-linux-gnu",
        assets=("outl-linux-arm64.tar.gz",),
        required=True,
        anchor="linux-arm64",
        bottle_tag="arm64_linux",
        taps=("brew",),
    ),
    Leg(
        job="build",
        key="x86_64-pc-windows-msvc",
        assets=("outl-windows-x64.zip",),
        required=True,
        anchor="windows-x64",
        taps=("scoop",),
    ),
    # Cross-compiled from the x64 Windows runner, which carries both
    # `VC.Tools.ARM64` and the VS clang-cl that `aws-lc-sys` needs for this
    # target. Optional until a release has shipped it: nothing else in the
    # tree has built this triple before.
    Leg(
        job="build",
        key="aarch64-pc-windows-msvc",
        assets=("outl-windows-arm64.zip",),
        required=False,
        anchor="windows-arm64",
        taps=("scoop",),
    ),
)

# Desktop bundles. One universal dmg on macOS, three bundles per architecture
# on Linux, one installer per architecture on Windows.
DESKTOP: tuple[Leg, ...] = (
    Leg(
        job="build_desktop",
        key="universal",
        assets=("outl-desktop-macos.dmg",),
        required=True,
        anchor="macos",
        taps=("cask",),
    ),
    Leg(
        job="build_desktop_linux",
        key="x86_64",
        assets=(
            "outl-desktop-linux-x86_64.AppImage",
            "outl-desktop-linux-x86_64.deb",
            "outl-desktop-linux-x86_64.rpm",
        ),
        required=True,
    ),
    Leg(
        job="build_desktop_linux",
        key="aarch64",
        assets=(
            "outl-desktop-linux-aarch64.AppImage",
            "outl-desktop-linux-aarch64.deb",
            "outl-desktop-linux-aarch64.rpm",
        ),
        required=False,
    ),
    Leg(
        job="build_desktop_windows",
        key="x86_64",
        assets=("outl-desktop-windows-x86_64-setup.exe",),
        required=True,
    ),
    Leg(
        job="build_desktop_windows",
        key="aarch64",
        assets=("outl-desktop-windows-aarch64-setup.exe",),
        required=False,
    ),
    # Signed with a throwaway debug key and sideload-only (issue #171), so a
    # failure here has never held a release and still does not.
    Leg(
        job="build_android",
        key="aarch64",
        assets=("outl-android-arm64.apk",),
        required=False,
    ),
)

LEGS: tuple[Leg, ...] = CLI + DESKTOP


def bottles(version: str) -> dict[str, str]:
    """`anchor -> bottle asset name` for a version, for every leg that stages
    one. The filename is not cosmetic: for a non-ghcr `root_url` Homebrew
    resolves `<name>-<version>.<tag>.bottle.tar.gz`, one dash between name and
    version, and a miss is a 404 instead of a fallback."""
    return {
        leg.anchor: f"outl-beta-{version}.{leg.bottle_tag}.bottle.tar.gz"
        for leg in LEGS
        if leg.bottle_tag and leg.anchor
    }


def expected(version: str) -> dict[str, Leg]:
    """Every asset a complete release carries, mapped to the leg that uploads
    it: the named assets, their `.sha256` sidecars, and the bottles."""
    out: dict[str, Leg] = {}
    for leg in LEGS:
        names = list(leg.assets)
        if leg.bottle_tag and leg.anchor:
            names.append(bottles(version)[leg.anchor])
        for name in names:
            out[name] = leg
            out[f"{name}.sha256"] = leg
    return out


def tap_assets(version: str, tap: str) -> dict[str, Leg]:
    """The assets one manifest points at. `update_tap` bumps a manifest only
    when all of them are present, because a manifest carrying a new version
    and a previous release's digest is a hash mismatch on install — strictly
    worse than a manifest that still points at a release that exists."""
    out: dict[str, Leg] = {}
    for leg in LEGS:
        if tap not in leg.taps:
            continue
        for name in leg.assets:
            out[f"{name}.sha256"] = leg
        if tap == "brew" and leg.bottle_tag and leg.anchor:
            out[f"{bottles(version)[leg.anchor]}.sha256"] = leg
    return out
