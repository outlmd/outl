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
  version "0.13.0-beta.292"
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
    root_url "https://github.com/outlmd/outl/releases/download/v0.13.0-beta.292" # anchor: bottle-root-url
    sha256 cellar: :any_skip_relocation, arm64_ventura: "ffe3d1ee2f35d3c4acf062c326bc040bf8848ff28f732ce7bd45791ab3ff6439" # anchor: bottle-macos-arm64
    sha256 cellar: :any_skip_relocation, ventura:       "dadd06cbb9fec69c983fff4d3c0912f09d83697f646088d42cdbb75eeb6ff485" # anchor: bottle-macos-x64
    sha256 cellar: :any_skip_relocation, arm64_linux:   "b4ac8ff59f5c0cd589c9c030bad7808afccef9bc6b413f80377c778eda3844cd" # anchor: bottle-linux-arm64
    sha256 cellar: :any_skip_relocation, x86_64_linux:  "1f4d1a15289f60d9f6957be2d61fa91b9c16950ddf509f2ccf1f79af333f3f3e" # anchor: bottle-linux-x64
  end

  on_macos do
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-arm64.tar.gz"
      sha256 "74e84af46c7d32a21398e4633f9b9b8b9bc7fcc9dbecfd92ea6c5c9dbed946c0" # anchor: macos-arm64
    end
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-x64.tar.gz"
      sha256 "3ba7d8163bcbb2142b3ca975458f9130a7b80b60a8916a3ab24d6ec4c574bfb1" # anchor: macos-x64
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-x64.tar.gz"
      sha256 "60cbce2f4f15de336577d46ca3d60dcb2c8502a08c42c8d0c8d0a462b0407918" # anchor: linux-x64
    end
    # ARM64 Linux is a Tier 1 Homebrew platform since 5.0.0, and the
    # binary is built natively (`blacksmith-8vcpu-ubuntu-2204-arm`), not
    # emulated. The zeroed digest is the bootstrap placeholder the first
    # release after this change replaces — same as every other anchor
    # here started out.
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-arm64.tar.gz"
      sha256 "55057bb6edd50516174187f2107ab09bb95aa05c1e3ebbdb115381bf43a80c10" # anchor: linux-arm64
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
