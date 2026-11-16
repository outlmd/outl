#!/usr/bin/env bash
#
# Run outl on a physical iPhone as a SECOND app, beside the real one.
#
#   scripts/ios-preview.sh "samba-phone"
#   scripts/ios-preview.sh "samba-phone" --debug
#
# Without this, `cargo tauri ios dev <device>` installs over the app from
# TestFlight: same bundle id, so it inherits that container, that paired
# iroh identity and that graph. Driving a dev build then writes synthetic
# edits into real notes, and the signed build is gone until the next
# TestFlight install.
#
# Two sides have to agree for a second app to work, and this script is
# what keeps them agreeing:
#
#   - Xcode decides the bundle id of what gets BUILT, from
#     `gen/apple/Outl.xcconfig` plus the optional `Preview.xcconfig` this
#     writes.
#   - The Tauri CLI decides what gets INSTALLED and LAUNCHED, from
#     `identifier` in `src-tauri/tauri.preview.conf.json`.
#
# That file is the owner of the identifier; every other preview name is
# derived from it here, so there is one place to change.
#
# `Preview.xcconfig` is removed on exit, including on Ctrl-C. Preview is a
# property of this command, not a mode the checkout sits in — a leftover
# file would otherwise make the next `cargo tauri ios build` produce an
# IPA that App Store Connect rejects for the wrong bundle id, long after
# anyone remembers running this.
set -euo pipefail

cd "$(dirname "$0")/.."

DEVICE="${1:-}"
if [[ -z "$DEVICE" ]]; then
  echo "usage: $0 <device-name> [--debug]" >&2
  echo >&2
  echo "connected devices:" >&2
  xcrun xctrace list devices 2>/dev/null \
    | sed -n '/^== Devices ==/,/^$/p' | sed '1d;/^$/d' >&2
  exit 2
fi
shift

# `--release` by default: this exists to look at the app, and a debug
# WKWebView build misrepresents scroll and typing latency badly enough to
# send you chasing a performance bug that does not ship. Pass --debug to
# opt out.
MODE=(--release)
for arg in "$@"; do
  [[ "$arg" == "--debug" ]] && MODE=()
done

connected_devices() {
  xcrun xctrace list devices 2>/dev/null \
    | sed -n '/^== Devices ==/,/^$/p' | sed '1d;/^$/d'
}

# Check the name here rather than letting the CLI find out. Handed a
# device it cannot see, `cargo tauri ios dev` OPENS XCODE and waits,
# which reads as a hang and is a strange thing to be given for a typo.
if ! connected_devices | grep -qF "$DEVICE"; then
  echo "error: no connected device named '$DEVICE'." >&2
  echo >&2
  echo "connected devices:" >&2
  connected_devices >&2
  echo >&2
  echo "A device has to be unlocked and trusted to appear here." >&2
  exit 1
fi

CONF="src-tauri/tauri.preview.conf.json"
APPLE="src-tauri/gen/apple"
BUNDLE_ID=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["identifier"])' "$CONF")
PROD_ID="app.outl.mobile-app"

if [[ "$BUNDLE_ID" == "$PROD_ID" ]]; then
  echo "error: $CONF carries the production identifier ($PROD_ID)." >&2
  echo "       A preview that shares the bundle id is not a preview." >&2
  exit 1
fi

# Everything that must differ, derived from the one identifier. The
# suffix is whatever the preview id adds to the production one.
SUFFIX="${BUNDLE_ID#"$PROD_ID"}"

# Everything below leaves something behind, so the trap is armed BEFORE
# the first write rather than after the last one. Armed after, a failure
# in between exits with the checkout still in preview.
#
# The app icon is swapped IN PLACE rather than selected by name, and
# that is not the first design. `ASSETCATALOG_COMPILER_APPICON_NAME` set
# to a variable, pointing at a second `AppIcon-Preview.appiconset`, is
# the clean way to do this and it does not survive: the Tauri CLI
# rewrites that setting back to the literal `AppIcon` on every run, the
# same way it rewrites the bundle id. The build then succeeds with the
# production icon and nothing says so.
#
# So the preview icons are written over `AppIcon.appiconset` and the
# original is restored on exit, exactly like the pbxproj below.
PREVIEW_ICONS="$APPLE/Assets.xcassets/AppIcon-Preview.appiconset"
APPICON="$APPLE/Assets.xcassets/AppIcon.appiconset"
APPICON_BACKUP="$(mktemp -d)/AppIcon.appiconset"
cp -R "$APPICON" "$APPICON_BACKUP"
# The Tauri CLI writes `identifier` into the APP target's build settings
# on every run, so a preview leaves `app.outl.mobile-app.preview` sitting
# in a committed file. Checking that in ships a TestFlight build under
# the preview id, which App Store Connect rejects, and the diff looks
# like an unrelated two-line change nobody reads twice.
#
# The extension targets are not touched by the CLI (it does not know they
# exist), which is why they keep deriving from the xcconfig while the app
# target keeps the literal.
PBX="$APPLE/outl-mobile.xcodeproj/project.pbxproj"
PBX_BACKUP="$(mktemp)"
cp "$PBX" "$PBX_BACKUP"

cleanup() {
  rm -f "$APPLE/Preview.xcconfig"
  rm -rf "$PREVIEW_ICONS"
  if [ -d "$APPICON_BACKUP" ]; then
    rm -rf "$APPICON"
    cp -R "$APPICON_BACKUP" "$APPICON"
    rm -rf "$(dirname "$APPICON_BACKUP")"
  fi
  if ! cmp -s "$PBX" "$PBX_BACKUP"; then
    cp "$PBX_BACKUP" "$PBX"
    echo "restored $PBX (the Tauri CLI had rewritten it)" >&2
  fi
  rm -f "$PBX_BACKUP"
}
trap cleanup EXIT

cat > "$APPLE/Preview.xcconfig" <<EOF
// Generated by scripts/ios-preview.sh — do not edit, do not commit.
// Removed when that script exits.
OUTL_BUNDLE_ID = $BUNDLE_ID
OUTL_DISPLAY_NAME = outl Preview
OUTL_URL_SCHEME = outl-preview
OUTL_CAPTURE_NOTIFICATION = app.outl.capture.arrived$SUFFIX
EOF

# The icon with the PREVIEW band, so the two installs are told apart on
# the Home Screen and not only in Settings. Generated rather than
# committed: an appiconset inside Assets.xcassets is compiled into EVERY
# build, so a committed one would ride along in the App Store copy, which
# can never show it. `gen-icons.py` owns the drawing (one generator for
# every icon in this repo); this only asks for it.
echo "drawing the preview app icon..."
if ! (cd ../.. && uv run --with pillow --with resvg-py \
        scripts/gen-icons.py --preview-appicon >/dev/null); then
  echo "error: could not draw the preview app icon." >&2
  echo "       Needs uv (https://docs.astral.sh/uv/)." >&2
  exit 1
fi
# Only the slots, never Contents.json: the catalog's own file is the
# same either way, and copying it would widen what has to be restored.
for f in "$PREVIEW_ICONS"/AppIcon-*.png; do
  cp "$f" "$APPICON/$(basename "$f")"
done

echo "preview flavor:  $BUNDLE_ID  (production stays $PROD_ID)"
echo "device:          $DEVICE"
echo

# --host is what makes the dev server reachable from the phone; without
# it the webview dials localhost on the phone itself and loads nothing.
#
# NOT `exec`: that replaces this shell, and the EXIT trap above dies with
# it, leaving Preview.xcconfig behind. The first version of this script
# did exactly that, and the leftover file is the failure the trap exists
# to prevent.
cargo tauri ios dev "$DEVICE" --host "${MODE[@]}" --config "$CONF"
