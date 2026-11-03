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
  version "0.13.0-beta.289"
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
    root_url "https://github.com/outlmd/outl/releases/download/v0.13.0-beta.289" # anchor: bottle-root-url
    sha256 cellar: :any_skip_relocation, arm64_ventura: "6d1f49fc0510262cb80e911472612e82624596cde30cc9505dc22c062c70e89c" # anchor: bottle-macos-arm64
    sha256 cellar: :any_skip_relocation, ventura:       "3b58a81abf3260dac2d6f30d622b0d343076387cc59528e34bc91837ce2a6d81" # anchor: bottle-macos-x64
    sha256 cellar: :any_skip_relocation, arm64_linux:   "0bfa052ec382c81d14d1c25604af705e2cf065eb8526d237c7c8c3f8ce16acff" # anchor: bottle-linux-arm64
    sha256 cellar: :any_skip_relocation, x86_64_linux:  "a33db3135d62b33843a1f10b9e5f4eefca3697b0bbb3ab8a7cea30ce4f042901" # anchor: bottle-linux-x64
  end

  on_macos do
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-arm64.tar.gz"
      sha256 "fb5ba19aa27a589193465c91ec3dccdf6e46853cb9d2b75fc2397f7790bac928" # anchor: macos-arm64
    end
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-x64.tar.gz"
      sha256 "c34b1522746dfbe58eba5ad9faaa3efeb152cf21ef086099b8c03d49bf1ba55a" # anchor: macos-x64
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-x64.tar.gz"
      sha256 "a41e49ab84dcef9f606ab4100e1346145c483df58f3ed1a6c3195218df7a771b" # anchor: linux-x64
    end
    # ARM64 Linux is a Tier 1 Homebrew platform since 5.0.0, and the
    # binary is built natively (`blacksmith-8vcpu-ubuntu-2204-arm`), not
    # emulated. The zeroed digest is the bootstrap placeholder the first
    # release after this change replaces — same as every other anchor
    # here started out.
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-arm64.tar.gz"
      sha256 "386325c1cbb2e429ef982f67e331afeb6fc0a862db2f8063d7527ff868830a42" # anchor: linux-arm64
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
