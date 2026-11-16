#!/usr/bin/env python3
"""Check that the committed iOS project builds the shipping app.

The app's identity (bundle id, App Group, URL scheme, capture
notification, display name) is declared once in `Outl.xcconfig` and read
through `$(OUTL_*)` from the pbxproj, both entitlements files and all
three `Info.plist` files. `crates/outl-mobile/scripts/ios-preview.sh`
overrides those values for a preview build installed beside the real
app, and undoes it on exit.

Every failure that costs a release lives in the gap between those two
sentences, and none of them is loud on its own:

1. **A `$(OUTL_*)` with nothing behind it expands to the empty string**,
   so a missing or unstaged `Outl.xcconfig` gives the widget the bundle
   id `.widget`. `xcodebuild -showBuildSettings` prints it without a
   word of complaint; the build fails later, in signing, saying nothing
   about an xcconfig.
2. **A preview override left behind ships under the preview id.** The
   release is then rejected by App Store Connect for a bundle id
   mismatch, after the upload, and the diff that caused it is two lines
   in a file nobody reads.

Standard library only, so it runs anywhere in CI.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
APPLE = ROOT / "crates" / "outl-mobile" / "src-tauri" / "gen" / "apple"
XCCONFIG = APPLE / "Outl.xcconfig"
# Written only by ios-preview.sh, removed when it exits, gitignored.
PREVIEW_ARTIFACTS = (
    APPLE / "Preview.xcconfig",
    APPLE / "Assets.xcassets" / "AppIcon-Preview.appiconset",
)
# Everything that reads the flavor.
CONSUMERS = (
    APPLE / "outl-mobile.xcodeproj" / "project.pbxproj",
    APPLE / "outl-mobile_iOS" / "Info.plist",
    APPLE / "outl-mobile_iOS" / "outl-mobile_iOS.entitlements",
    APPLE / "OutlShare" / "Info.plist",
    APPLE / "OutlShare" / "OutlShare.entitlements",
    APPLE / "OutlWidget" / "Info.plist",
)
PRODUCTION_ID = "app.outl.mobile-app"
VAR = re.compile(r"\$\((OUTL_[A-Z_]+)\)")


def defined() -> set[str]:
    out = set()
    for line in XCCONFIG.read_text().splitlines():
        line = line.split("//")[0].strip()
        if "=" in line and line.startswith("OUTL_"):
            out.add(line.split("=", 1)[0].strip())
    return out


def main() -> int:
    bad: list[str] = []

    if not XCCONFIG.is_file():
        print(f"{XCCONFIG.relative_to(ROOT)} is missing.")
        print("Every $(OUTL_*) would expand to nothing and the widget would")
        print("build as '.widget'. If you just added it, `git add` it.")
        return 1

    known = defined()
    for f in CONSUMERS:
        if not f.is_file():
            bad.append(f"{f.relative_to(ROOT)}: missing")
            continue
        text = f.read_text()
        for name in sorted(set(VAR.findall(text))):
            if name not in known:
                bad.append(
                    f"{f.relative_to(ROOT)}: $({name}) has no definition in "
                    f"{XCCONFIG.name}, so it expands to nothing"
                )
        # A preview id reaching a committed file is a preview run whose
        # cleanup did not happen. It builds, signs, and is refused by
        # App Store Connect after the upload.
        if f"{PRODUCTION_ID}.preview" in text:
            bad.append(
                f"{f.relative_to(ROOT)}: carries a preview bundle id. "
                "Run `git checkout` on it, or re-run ios-preview.sh and let "
                "it exit cleanly."
            )

    for p in PREVIEW_ARTIFACTS:
        if p.exists():
            bad.append(
                f"{p.relative_to(ROOT)}: a preview override is present. "
                "It is written per-run and gitignored; it must never be "
                "committed, and a release built with it is rejected."
            )

    if "preview" in XCCONFIG.read_text().lower().split("//")[0]:
        bad.append(f"{XCCONFIG.name}: holds a preview value; it is the production file")

    if bad:
        print("iOS flavor check failed:\n")
        for b in bad:
            print(f"  - {b}")
        return 1

    print(f"ios flavor ok ({len(known)} settings, {len(CONSUMERS)} consumers)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
