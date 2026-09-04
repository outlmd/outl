# This file is maintained by `.github/workflows/release.yml`.
# Every push to `main` runs the release workflow, which bumps the
# `version` line (computed from `Cargo.toml` + the workflow run
# number) and the `sha256` line below in place. The `# anchor:`
# comment is how the workflow finds the right line — do not remove
# it.
#
# The dmg shipped here is **universal** (arm64 + x86_64 lipo'd into
# one binary, packaged on a single arm64 runner). Both Apple Silicon
# and Intel Macs use the same `outl-desktop-macos.dmg`.
#
# The dmg is **unsigned**. On first launch macOS Gatekeeper will
# refuse the app; users can right-click → "Open" (or run
# `xattr -dr com.apple.quarantine /Applications/outl.app`) to dismiss
# the warning. Once we wire an Apple Developer ID + notarisation
# (release.yml step pending), this caveat goes away.
cask "outl-desktop-beta" do
  version "0.12.0-beta.178"
  sha256 "599bbd6c44ad1e37cab7ae74c0ae3d206e39a673d533fc84f3aa656479a1a690" # anchor: macos

  url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-desktop-macos.dmg"
  name "outl Desktop"
  desc "Local-first outliner with CRDT sync (desktop beta — every push to main)"
  homepage "https://outl.app"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "outl.app"

  # We share the `outl` binary name with the CLI / TUI formula on
  # /usr/local/bin. The cask installs `outl.app` to /Applications,
  # which doesn't collide on PATH, but a future "open outl" CLI
  # alias would. Flag the relationship so users running both stay
  # aware.
  conflicts_with cask: "outl-desktop"

  caveats <<~EOS
    The dmg is **unsigned**. macOS Gatekeeper refuses the app on
    first launch with:

      "outl.app" could not be opened.
      Apple could not verify "outl.app" is free of malware...

    macOS Sequoia (15) tightened Gatekeeper: the old
    "right-click → Open" trick no longer dismisses the warning
    on its own. Two paths through:

    1. Terminal (fastest):
         xattr -dr com.apple.quarantine /Applications/outl.app

    2. System Settings → Privacy & Security:
         Try to open outl.app once. Then open System Settings,
         scroll to Privacy & Security, find the "outl.app was
         blocked..." entry, click "Open Anyway", and confirm
         with Touch ID or your password.

    Signing + notarisation lands together with the first GA release.
  EOS

  zap trash: [
    "~/Library/Application Support/app.outl.desktop",
    "~/Library/Preferences/app.outl.desktop.plist",
    "~/Library/Caches/app.outl.desktop",
    "~/Library/Logs/outl",
  ]
end
