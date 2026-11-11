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
  version "0.13.0-beta.298"
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
    root_url "https://github.com/outlmd/outl/releases/download/v0.13.0-beta.298" # anchor: bottle-root-url
    sha256 cellar: :any_skip_relocation, arm64_ventura: "08868ec914d6496b3d5ea29fd4bd4cc8e47d82de0b7a0bd75e05c80200c53448" # anchor: bottle-macos-arm64
    sha256 cellar: :any_skip_relocation, ventura:       "69f2f47a0e235d170798842827fbef2ed356b5dd0be1efa3683a9b474a97ece6" # anchor: bottle-macos-x64
    sha256 cellar: :any_skip_relocation, arm64_linux:   "4a7eb5bcf4d48ca5c5ab56fada0d4df0b0a538c0da8919e0368b1be0f21fcb62" # anchor: bottle-linux-arm64
    sha256 cellar: :any_skip_relocation, x86_64_linux:  "87a9676fbd625f9002ee19b4798c34904fe91f7b307116b39cb82defb6a795e9" # anchor: bottle-linux-x64
  end

  on_macos do
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-arm64.tar.gz"
      sha256 "d6cd63e853f2724888af597d351b512744124093532641ddeff73d3d16adb1d9" # anchor: macos-arm64
    end
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-macos-x64.tar.gz"
      sha256 "8fc90263eb18667f0c9b2c668c9aac50af099bcfd0afeb233637d13962d7e5cd" # anchor: macos-x64
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-x64.tar.gz"
      sha256 "20436aec9df2552a272788121ba1e6961b6633441816918c55c8a03be218c5a1" # anchor: linux-x64
    end
    # ARM64 Linux is a Tier 1 Homebrew platform since 5.0.0, and the
    # binary is built natively (`blacksmith-8vcpu-ubuntu-2204-arm`), not
    # emulated. The zeroed digest is the bootstrap placeholder the first
    # release after this change replaces — same as every other anchor
    # here started out.
    on_arm do
      url "https://github.com/outlmd/outl/releases/download/v#{version}/outl-linux-arm64.tar.gz"
      sha256 "ce3a77f5f6a7a20922610cb8044741c88d3b5d1b569a6e634213f1f78a54f9d9" # anchor: linux-arm64
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
