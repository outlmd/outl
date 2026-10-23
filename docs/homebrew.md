# Homebrew tap

outl ships pre-built binaries through the same `outlmd/outl` repo — no separate `homebrew-outl` repo.
Two surfaces live on `main`:

| Path | What it ships |
|---|---|
| [`/Formula/outl.rb`](../Formula/outl.rb) | GA CLI / TUI, semver tag `vX.Y.Z`. |
| [`/Formula/outl-beta.rb`](../Formula/outl-beta.rb) | Beta CLI / TUI, rebuilt on every push to `main`. |
| [`/Casks/outl-desktop-beta.rb`](../Casks/outl-desktop-beta.rb) | Beta **desktop app** (.dmg), rebuilt on every push to `main`. |

## Install the CLI / TUI

```bash
brew tap outlmd/outl https://github.com/outlmd/outl
brew trust outlmd/outl    # one-time: required for third-party taps
brew install outl-beta     # beta channel — every push to main
# brew install outl        # GA channel — semver tags only
```

The custom-URL form (`brew tap <name> <url>`) tells Homebrew to use this repo as the tap directly, without the usual `homebrew-<name>` naming convention.

Four platforms are covered, one pre-built binary each: macOS `arm64` and `x86_64`, Linux `x86_64` and `arm64`.
ARM64 Linux has been a Tier 1 Homebrew platform since 5.0.0, and the binary is built on a native ARM runner rather than emulated.
The Linux binaries are built on Ubuntu 22.04, so they need glibc 2.35 or newer — the same floor Homebrew's own Linux bottles carry.

> **Tapped `avelino/outl` before?** The repo moved to the `outlmd` org, and a tap name is not a redirect — Homebrew keeps pulling from the old clone under the old name. Re-tap once:
>
> ```bash
> brew untap avelino/outl
> brew tap outlmd/outl https://github.com/outlmd/outl
> brew trust outlmd/outl
> ```
>
> Installed formulae keep working until the next `brew upgrade`; `brew untap` does not uninstall anything.

> `brew trust` is required since Homebrew 4.x for any tap outside `homebrew-core` / `homebrew-cask`. Without it, `brew install` fails with `Refusing to load formula outlmd/outl/outl-beta from untrusted tap`. The trust is per-tap (not per-formula) and one-time; subsequent installs / upgrades don't ask again.
>
> If `brew trust` itself errors with `Refusing to write insecure trust store: target directory ... is not owned by the current user`, your `~/.config/homebrew` is a symlink to a read-only path (common on Nix-managed dotfiles). Replace the symlink with a writable directory before re-running.

> Why `outl-beta` and not `outl@beta`? Homebrew's `Formulary.class_s` only converts `@<digits>` into a valid Ruby class name (`python@3.12` → `PythonAT312`). A non-numeric channel like `outl@beta` would generate the invalid class `Outl@beta`, so the dash form is the only one that works for an alphabetic channel.

`outl` and `outl-beta` install the same `outl` binary and conflict by design.
Switching channels:

```bash
brew unlink outl-beta && brew install outl       # to GA
brew unlink outl      && brew install outl-beta  # to beta
```

## Install the desktop app (macOS)

```bash
brew tap outlmd/outl https://github.com/outlmd/outl   # same tap as the CLI
brew trust outlmd/outl                                 # skip if already trusted for the CLI
brew install --cask outl-desktop-beta
```

That drops `outl.app` into `/Applications` via the **universal** dmg the release workflow builds on a single arm64 runner (`--target universal-apple-darwin` → `lipo` merge of arm64 + x86_64 into one binary).
Both Apple Silicon and Intel Macs install the same dmg.

> **Linux desktop:** Homebrew has no GUI cask on Linux, so the app ships as AppImage / `.deb` / `.rpm` assets on each [GitHub release](https://github.com/outlmd/outl/releases).
> See [Getting started → Desktop app on Linux](getting-started.md#desktop-app-on-linux).
The cask sits in [`/Casks/outl-desktop-beta.rb`](../Casks/outl-desktop-beta.rb) and is bumped automatically alongside the CLI formula on every push to `main`.

The CLI formula (`outl-beta`) and the desktop cask coexist without conflicts — the formula installs `/usr/local/bin/outl`, the cask installs `/Applications/outl.app`.
Both point at the same workspace on disk if you configure the same path; the op log (`ops/ops-<actor>.jsonl`) reconciles edits made from either surface.

### First-launch Gatekeeper warning

The desktop dmg is **unsigned** today.
On the first launch macOS Gatekeeper refuses the app with:

> "outl.app" Not Opened
> Apple could not verify "outl.app" is free of malware that may harm your Mac or compromise your privacy.

macOS Sequoia (15) tightened Gatekeeper — the old "right-click → Open" shortcut no longer dismisses the warning by itself.
Two ways through that actually work on current macOS:

**Terminal (fastest):**

```bash
xattr -dr com.apple.quarantine /Applications/outl.app
```

This drops the quarantine attribute that Gatekeeper checks against; subsequent launches work normally.

**System Settings → Privacy & Security:**

1. Try to open `outl.app` once and dismiss the warning.
2. Open **System Settings → Privacy & Security**.
3. Scroll to the bottom — there's an entry "*outl.app was blocked from use because it is not from an identified developer*".
4. Click **Open Anyway**, then confirm with Touch ID or your password.
5. Try opening `outl.app` again — Gatekeeper now prompts for a final confirmation; click **Open**.

Signing + notarisation will land with the first GA release (the cask's `caveats` block reminds the user of this on every install until then).

### Updating

```bash
brew update
brew upgrade --cask outl-desktop-beta
```

The `livecheck` block in the cask points Homebrew at the GitHub releases feed, so `brew outdated` knows when a new beta dmg lands.

## How updates happen

The release workflow ([`/.github/workflows/release.yml`](../.github/workflows/release.yml)) has an `update_tap` job that runs after `publish_release` succeeds on a prerelease.
It does five things:

1. Checks out `main`.
2. Downloads every `.sha256` sidecar from the GitHub release (CLI tarballs, the desktop dmg, the Windows zips).
3. Asks, per manifest, whether it may advance: **only when every asset that manifest points at is on the release.**
   A manifest carrying a new `version` and a previous release's digest is a SHA256 mismatch at install time, which is worse than one still pointing at a release that exists.
   So a release missing the Linux `aarch64` tarball leaves the formula alone (loudly, in the run summary) while the cask still moves, and the next release picks it up.
   `scripts/release_assets.py` owns which assets each manifest points at.
4. Uses `sed` to bump the version + sha lines in each file it may touch:
   - **`Formula/outl-beta.rb`** — `version` plus four download `sha256` lines tagged `# anchor: macos-arm64`, `# anchor: macos-x64`, `# anchor: linux-x64`, `# anchor: linux-arm64`, and the four matching `# anchor: bottle-*` lines.
     The two loops in the job are keyed by those names, so a `build` matrix leg added without its anchors bumps nothing — and `git diff --quiet` only sees that *something* changed, so nothing goes red.
   - **`Casks/outl-desktop-beta.rb`** — `version` plus one `sha256` line tagged `# anchor: macos` (universal dmg, both architectures in the same file).
   - **`bucket/outl-beta.json`** — the [Scoop manifest](scoop.md), rewritten with `jq` rather than `sed`: it is JSON, so there is a parser for it and no anchors are needed.
5. Commits whatever moved back to `main` with `[skip ci]` in the message — that prevents the commit from re-triggering the release workflow itself.

The version comes from the `prepare` job, which reads `workspace.package.version` out of `Cargo.toml` and appends `-beta.<run_number>`.
So the only source of truth for the base version is `Cargo.toml`; nothing is hardcoded in the formula.

`Formula/outl-beta.rb` and `Casks/outl-desktop-beta.rb` are both **committed with bootstrap placeholders** (`version "0.0.0"`, zeroed SHAs).
The first beta release after each lands bumps it to real values.
Until then, `brew install outl-beta` and `brew install --cask outl-desktop-beta` both 404 on the download URL.

`Formula/outl.rb` and `Casks/outl-desktop.rb` (the GA channels) don't exist yet.
When the first non-prerelease tag (`vX.Y.Z` with no `-beta`) ships, bump them by hand the first time; after that we can add a GA-flavored `update_tap` job if it's worth automating.

## Authentication

The job uses the default `GITHUB_TOKEN` (with `contents: write` permission scoped to the job).
No PAT, no extra secret to manage.

## Troubleshooting

**A release published but the formula never moved** — check whether `update_tap` ran at all, not whether it failed.
A job's implicit `success()` reads the whole ancestor tree rather than its direct `needs`, so any failed job upstream of `publish_release` skipped this one, even with `publish_release` green.
`build_android` is best-effort and did exactly that three times; v0.12.0-beta.207 published while the formula stayed on `.206`, and no job went red to say so.
`update_tap` carries `!cancelled()` now, so what keeps it out is its own condition — a release that never published, or a GA tag — and no longer a failure somewhere else in the run.
Any new job added upstream needs the same guard, or it inherits the same silence.

**Tap update commits but `brew install` still pulls the old version** — run `brew update` first; Homebrew caches tap state.

**`brew install outl-beta` fails with `SHA256 mismatch`** — the formula on `main` drifted from the release asset.
Usually means someone re-uploaded an asset on the GitHub release.
Re-trigger the failed release via `workflow_dispatch` and `update_tap` will re-render with the current SHAs.

**A manual edit on `Formula/*.rb` got bumped over** — partly by design.
The job rewrites the `version`, the bottle `root_url` and every `# anchor:`ed digest in place; it does **not** template the file, so structure (platform blocks, `test do`, `conflicts_with`) is edited here and survives.
Adding a platform therefore takes four edits in one PR: the entry in [`scripts/release_assets.py`](../scripts/release_assets.py), the matrix leg in `release.yml`, its two entries in that workflow's `update_tap` loops, and the `url` / `sha256` / bottle anchors here.
`scripts/check-release-assets.py --manifest` (run by `hygiene.yml`) fails when those disagree, which is the one failure mode none of the files can see on its own.

**Renaming the binary or adding a second binary** — `bin.install "outl"` ships the `outl` executable only.
If/when outl grows additional binaries (e.g.
`outl-tui` as a separate binary), add them to that line and to the `Stage artifact` step in `release.yml`, which is what puts files in the tarball.
