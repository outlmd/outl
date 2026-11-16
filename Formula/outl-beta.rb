# This file is maintained by `.github/workflows/release.yml`.
# Every push to `main` runs the release workflow, which bumps the
# `version` line (computed from `Cargo.toml` + the workflow run
# number), the bottle `root_url`, and the eight `sha256` lines below in
# place. The `# anchor:` comments are how the workflow finds the right
# lines — do not remove them. One `# anchor:` pair (download + bottle)
# per platform the `build` matrix in that workflow ships; adding a
# platform there without adding its anchors here leaves `brew install`
# on the old binary with nothing going red.
#
# Values committed here are bootstrap placeholders: `version "0.0.0"`
# and zeroed SHAs make `brew install outl-beta` fail loudly until the
# first release fires. They become real on the next push to `main`.
class OutlBeta < Formula
  desc "Local-first outliner with CRDT sync (beta channel — every push to main)"
  homepage "https://outl.app"
  version "0.13.0-beta.310"
  license "MIT"

  # We ship pre-built binaries and compile nothing here, but a formula
  # without a bottle counts as "build from source" to Homebrew, which
  # then runs its fatal dev-tools checks and refuses to install on any
  # Mac whose Xcode is older than the running macOS wants:
  #
  #   Error: Your Xcode (26.6) at /Applications/Xcode.app is too outdated.
  #
  # The bottles below are the same binaries repacked into the Cellar
  # layout (`outl-beta/<version>/bin/outl`), which makes `pour_bottle?`
  # true and skips those checks entirely. macOS bottle tags fall back to
  # older releases, so one `ventura` tag per arch covers every macOS
  # from 13 upwards — no runner on the newest macOS required.
  bottle do
    root_url "https://github.com/outlmd/outl/releases/download/v0.13.0-beta.310" # anchor: bottle-root-url
    sha256 cellar: :any_skip_relocation, arm64_ventura: "ccdcd862383e009b01c276ac91ee2653d9d9f796e1c2d23c790c91ea1e69dd74" # anchor: bottle-macos-arm64
    sha256 cellar: :any_skip_relocation, ventura:       "f06568cae298f33585f4443aa359da66eb38cba4b6315cfb4144505e51ec0c29" # anchor: bottle-macos-x64
    sha256 cellar: :any_skip_relocation, arm64_linux:   "4c3794288a16d3c8edb26fbe41a8312593155a4348d43218c958fc36b33416d7" # anchor: bottle-linux-arm64
    sha256 cellar: :any_skip_relocation, x86_64_linux:  "f5450d8164b7119d42db6dbbbd983b4453344c49d3c644bcba74523527b5710b" # anchor: bottle-linux-x64
  end

  on_macos do
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-arm64.tar.gz"
      sha256 "3da07f625aab8eabc22f33fa2e1fb165185282d6206b4ed4ad5df16f90da001b" # anchor: macos-arm64
    end
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-x64.tar.gz"
      sha256 "0aba4fe600ec0e15804f0964e5b7d7e411a14da90bf92b0d87aec9fdab593453" # anchor: macos-x64
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-x64.tar.gz"
      sha256 "eae6e3134cdd83ad65424af7b65fdf259c168e80dcb823657e63d842c76246ea" # anchor: linux-x64
    end
    # ARM64 Linux is a Tier 1 Homebrew platform since 5.0.0, and the
    # binary is built natively (`blacksmith-8vcpu-ubuntu-2204-arm`), not
    # emulated. The zeroed digest is the bootstrap placeholder the first
    # release after this change replaces — same as every other anchor
    # here started out.
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-arm64.tar.gz"
      sha256 "fca7110b36912fb1366c4b788d17296fa71e906a3ab6a713d8be9e88a858b103" # anchor: linux-arm64
    end
  end

  # Beta and stable share the same `outl` binary name. Refuse to install
  # both side-by-side — `brew unlink outl` (or `outl-beta`) before
  # switching channels.
  conflicts_with "outl", because: "both install the `outl` binary"

  def install
    bin.install "outl"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/outl --version")
  end
end
