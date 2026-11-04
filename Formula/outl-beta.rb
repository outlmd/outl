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
  version "0.13.0-beta.291"
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
    root_url "https://github.com/outlmd/outl/releases/download/v0.13.0-beta.291" # anchor: bottle-root-url
    sha256 cellar: :any_skip_relocation, arm64_ventura: "fb22e288343ffdaa1fb75b6a0773a135892a71061d24f25d2c73ba050b610ed2" # anchor: bottle-macos-arm64
    sha256 cellar: :any_skip_relocation, ventura:       "e7ba0b436ca2f5833b25a38ffceb7b49885d1593bb0a3f5a4c607ef3f78a3807" # anchor: bottle-macos-x64
    sha256 cellar: :any_skip_relocation, arm64_linux:   "5cfa3a04434ab9d6a4e9d9f301523a3c8798e9a4e077ca9e2fabdb79e252d756" # anchor: bottle-linux-arm64
    sha256 cellar: :any_skip_relocation, x86_64_linux:  "8fc167eecbf3bd060ca7311151ec0cb0f94d30edf12d1428d52fe789f4a2536d" # anchor: bottle-linux-x64
  end

  on_macos do
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-arm64.tar.gz"
      sha256 "d6e81610fba5db318dc9bb3c0fc65acbb60deb38ed561e6b8157aa7ca07d646b" # anchor: macos-arm64
    end
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-x64.tar.gz"
      sha256 "b385d726f56ee7fb69dc6b3dae1d3bc8c36e9f5d5e9885af272c4752ab5a98da" # anchor: macos-x64
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-x64.tar.gz"
      sha256 "3ac05bc2f4e0a706a312f12f53ea9b58820c4a6d283df6a7051904ec93e0a7af" # anchor: linux-x64
    end
    # ARM64 Linux is a Tier 1 Homebrew platform since 5.0.0, and the
    # binary is built natively (`blacksmith-8vcpu-ubuntu-2204-arm`), not
    # emulated. The zeroed digest is the bootstrap placeholder the first
    # release after this change replaces — same as every other anchor
    # here started out.
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-arm64.tar.gz"
      sha256 "7103efba6f7200dfb0749e525c9964a192ef2645523c9e14bd49ca79de95f033" # anchor: linux-arm64
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
