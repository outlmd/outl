# Scoop bucket (Windows)

outl ships a [Scoop](https://scoop.sh) manifest from the same `outlmd/outl` repo as the Homebrew tap, so there is no separate bucket repository to add.

```powershell
scoop bucket add outl https://github.com/outlmd/outl
scoop install outl/outl-beta
```

That puts `outl.exe` on your PATH, unprivileged, under `~\scoop\apps\outl-beta\`.
`outl --help` lists the commands; `outl init <dir>` scaffolds a workspace, and `outl` with no arguments opens the TUI on today's journal.

| Path | What it ships |
|---|---|
| [`/bucket/outl-beta.json`](../bucket/outl-beta.json) | Beta CLI / TUI / MCP server, rebuilt on every push to `main`. |

Both architectures are served from one manifest: `64bit` is the x64 build, `arm64` is a native ARM64 binary rather than an x64 one under emulation.
Scoop picks the one matching the machine, so the install line is the same either way.

## Updating

```powershell
scoop update          # refresh the bucket
scoop update outl-beta
```

`scoop status` shows when a newer beta has landed.
The manifest carries a `checkver` block pointing at the repo's releases feed, which is also what Scoop's own update tooling reads.

The beta channel tracks **every push to `main`**, so `scoop update` is a frequent event.
`scoop hold outl-beta` pins the installed version until you `scoop unhold` it.

## Why `outl-beta` and not `outl`

Same channel split as [the Homebrew tap](homebrew.md): `outl-beta` is cut from every push to `main`, and the GA manifest lands with the first non-prerelease tag.
They install the same `outl.exe`, so run one or the other, not both.

## The desktop app is not a Scoop package

The GUI ships as an NSIS installer (`outl-desktop-windows-x86_64-setup.exe`, `outl-desktop-windows-aarch64-setup.exe`) on each [GitHub release](https://github.com/outlmd/outl/releases), and Scoop's model is a portable app directory rather than an installer that writes to Program Files and the registry.
Wrapping the installer in a manifest is possible and is not done here: the install would not be portable, which is the property people add a bucket for.
See [Getting started → Desktop app on Windows](getting-started.md#desktop-app-on-windows).

## winget and Chocolatey

Neither is published today.
Both are central repositories rather than user-added buckets, so each needs a pull request into somebody else's repo per release, with a review queue in between — the opposite of the "one commit on `main` and every channel moves" shape the release workflow has.
`winget` is the one worth doing next, because it ships with Windows; it is tracked with the rest of the packaging work in [issue #413](https://github.com/outlmd/outl/issues/413).

## How updates happen

`release.yml`'s `update_tap` job bumps this manifest in the same pass as the Homebrew formula and the desktop cask, after a release publishes.
It rewrites `version` and both `architecture.*.hash` values with `jq` — the manifest is JSON, so there is a parser for it and no anchor comments are needed.
The urls are not touched: they carry `$version`, which Scoop substitutes itself.

Two rules that the job enforces rather than hopes for:

- **A manifest moves only when every asset it points at is on the release.**
  Both Windows zips, in this case.
  A manifest carrying a new `version` and a previous release's digest is a hash mismatch at install time, which is strictly worse than one still pointing at a release that exists — so a release missing one of them leaves this file alone and says so in the run summary.
  The next release picks it up.
- **The architectures here and the build matrix cannot drift.**
  `scripts/check-release-assets.py --manifest` (run by `hygiene.yml`) fails when an `architecture` block has no build leg behind it, or a Windows build leg has no block here.

## Troubleshooting

**`scoop install` fails with a hash mismatch** — the manifest on `main` drifted from the release asset, usually because an asset was re-uploaded.
Re-run the release via `workflow_dispatch`; the bump re-renders from the current digests.

**`scoop install outl/outl-beta` says the bucket is unknown** — `scoop bucket add outl https://github.com/outlmd/outl` is the custom-URL form; without the URL, Scoop looks for a bucket named `outl` in its directory, where this one is not listed.

**ARM64 machine installed the x64 build** — check `scoop cat outl-beta` against `$env:PROCESSOR_ARCHITECTURE`.
Scoop resolves `arm64` on Windows 11 ARM; on Windows 10 ARM it falls back to `64bit`, which runs under emulation.
