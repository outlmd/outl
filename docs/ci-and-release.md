# CI and release

What runs automatically, and what happens between a merge and a published artifact.

This page was split out of the [Development guide](development.md), which stayed the engineer onramp: clone, build, test, debug.
That one is what *you* run; this one is what the repo runs for you.
Read this before changing a workflow, a runner label, the release gate, or anything a package manager points at.

If anything here is wrong or out of date, that's a bug — open an issue or fix it in the same PR that drifted the behavior.

---

## 1. CI walkthrough

| Workflow | Triggers | What it runs | Blocks merge? |
|---|---|---|---|
| [`ci.yml`](../.github/workflows/ci.yml) | Push / PR to `main` (skipped on docs-only) | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo doc -D warnings`, an **`advisories`** job (`cargo deny --locked check advisories` against `deny.toml`: any RustSec vulnerability fails, the whole graph is checked for unmaintained and unsound crates too, and every ignore carries a reason), plus a dedicated **`sync`** job (`outl-core` + `outl-sync-iroh` with `PROPTEST_CASES=1024`), and a **`frontend`** job (`bun run test` + `bun run typecheck` over every bun workspace package). Excludes `outl-mobile` + `outl-desktop` from the Rust jobs. Test matrix: `test (linux)` on every event, plus `test (macos)` on pushes to `main` only (see [Runner sizing](#runner-sizing)). | **Yes** |
| [`hygiene.yml`](../.github/workflows/hygiene.yml) | Push / PR to `main`, **including** docs-only changes | `scripts/check-file-size.sh` (the file size ratchet), `scripts/check-docs-index.sh` (every RFC reachable from `docs/SUMMARY.md`), `scripts/tests/test_check_artifact_stripped.py` (the strip gate's own suite), and `scripts/check-release-assets.py --manifest` plus its suite (the three build matrices, `update_tap`'s loops, the Homebrew formula, the cask and the Scoop manifest all name the platforms `scripts/release_assets.py` names). Kept out of `ci.yml` on purpose: that workflow skips `docs/**`, which is exactly the change set the index check has to see. | **Yes** |
| [`mobile.yml`](../.github/workflows/mobile.yml) | Push / PR touching mobile paths | Frontend tests, Swift tests, Rust mobile crate, iOS archive + sign on `push` | Mobile changes only |
| [`desktop.yml`](../.github/workflows/desktop.yml) | Push / PR touching desktop paths | Tauri build matrix (macOS/Linux/Windows) | Desktop changes only |
| [`bench.yml`](../.github/workflows/bench.yml) | Push / PR touching `outl-md`, plus weekly cron | Criterion (small/medium/large) on every PR; xlarge + CLI hyperfine on cron / manual dispatch. Artifacts retained 14–30 days. | No (informational) |
| [`release.yml`](../.github/workflows/release.yml) | Nightly cron at 06:00 UTC when `main` moved in the last 24h **and** the newest conclusive `ci.yml` run on `main` is green (beta), `v*` tag (GA), manual. Runs no tests of its own: `ci.yml` owns them, and the gate is what keeps ~20 build legs off a red tree | Computes version from `Cargo.toml`, builds the CLI + TUI matrix (macOS `arm64`/`x86_64`, Linux `x86_64`/`aarch64`, Windows `x86_64`/`aarch64`), builds the desktop bundles (universal macOS dmg, Linux AppImage/deb/rpm and a Windows NSIS installer per architecture), the Android APK, drafts release, uploads assets, gates the publish on [`scripts/release_assets.py`](../scripts/release_assets.py), bumps the Homebrew formula + cask and the Scoop manifest (`bucket/outl-beta.json`), publishes `@outl/plugin-sdk` to npm and the embedder lib crates to crates.io. | n/a |
| [`testflight.yml`](../.github/workflows/testflight.yml) | `Mobile` workflow completing successfully | Downloads the signed `.ipa`, uploads to App Store Connect via `xcrun altool`, sets "What to Test" notes via App Store Connect API. | n/a |
| [`docker.yml`](../.github/workflows/docker.yml) | Push to `main` / `v*` tag, PR touching `Dockerfile` / `docker/**`, manual | Builds `ghcr.io/<owner>/outl-server` once per architecture on a runner of that architecture (`linux/amd64`, `linux/arm64`), pushes each by digest, merges them into one manifest list per tag, and attests that list through Sigstore (`gh attestation verify oci://…`). A PR builds and pushes nothing. | No |
| [`cleanup-tags.yml`](../.github/workflows/cleanup-tags.yml) | Called by `release.yml` from `needs: tag` (once per tag created), or manual | Keeps the 2 most recently generated `vX.Y.Z-beta.N` tags repo-wide and deletes every older one, release and tag together. GA tags are never candidates, and neither is a beta a shipped package manifest still points at. Not `on: push: tags:`: the tag is pushed with `GITHUB_TOKEN`, which triggers no workflow. | n/a |

### Action pinning

Every `uses:` points at a full commit SHA with the release tag in a comment (`actions/checkout@<sha> # v7.0.1`), so a moved or hijacked tag can't change what runs next to the release secrets.
Don't hand-edit a SHA back to a tag: `.github/dependabot.yml` bumps the GitHub Actions pins weekly in one grouped PR, along with Cargo and Bun.
To pin a new action, resolve the tag with `gh api repos/<owner>/<repo>/git/ref/tags/<tag>` and, when that returns a `tag` object, follow it to the commit.

The one exception is a **local** action, `uses: ./.github/actions/<name>`, which carries no SHA because there is nothing to pin to: it is read from the commit already checked out, so it is already exactly as pinned as the workflow calling it.
A local action still needs `actions/checkout` to have run first, since it is a file in the workspace rather than something the runner fetches.

### Shared build setup

`.github/actions/setup-tauri-build` holds the steps that are identical in every job compiling a Tauri client: the Linux system libraries Tauri links against, Bun, the Bun package cache, and `bun install` at the repo root.
Pass `linux-deps: "true"` on a Linux runner and leave it out everywhere else.

It exists because `desktop.yml` and `release.yml` each carried their own copy of the `apt-get install` list, byte for byte.
A Tauri upgrade needing one more `-dev` package gets added to whichever file the author had open, and the other one breaks.
`desktop.yml` breaks on the next PR, which is fine.
`release.yml` breaks in the nightly hours later, after the change already looked green.

It is a composite action and not a reusable workflow because these are **steps**.
A reusable workflow is a whole job with its own runner and checkout, so it cannot contribute a prologue to a job that then goes on to bundle and upload.

What it deliberately leaves alone, because none of it is duplication:

- **The Rust toolchain.**
  Three callers install it three different ways: `dtolnay/rust-toolchain` with `targets:`, plus two raw `rustup target add` scripts for the universal macOS and the Windows cross build.
  Flattening that is a per-platform behaviour change, not a dedup.
- **The cargo cache.**
  `release.yml` uses `actions/cache` with an explicit `path` and `key`, `desktop.yml` uses `Swatinem/rust-cache` with a `shared-key`.
  They should agree, and picking one costs a cold build per job plus a build-time measurement nobody has taken.
  Real finding, separate change.
- **The build.**
  `desktop.yml` runs a debug `--no-bundle` compile to answer "does it build".
  `release.yml` bundles, signs, strips, verifies and uploads.
  Those are different jobs that happen to share a prologue, which is why only the prologue moved.

### Runner sizing

Every workflow runs on Blacksmith runners (`runs-on: blacksmith-<n>vcpu-<image>`, with `-arm` appended for ARM64 Linux), never on GitHub-hosted ones.
The one exception is `release.yml::publish_npm`, which runs on `ubuntu-24.04`: npm trusted publishing (OIDC) does not accept self-hosted runners yet, and Blacksmith registers as self-hosted.
Moving it back to Blacksmith breaks the npm publish.
Its trusted publisher on npmjs.com (`@outl/plugin-sdk` → Settings → Trusted publishing) must stay configured as owner `outlmd`, repository `outl`, workflow `release.yml`, environment `release`, with publishing allowed.
Billing is per vCPU-minute, with a platform multiplier on top: ARM 0.625x, x64 Linux 1x, Windows 2x, macOS 6.67x.
That multiplier is why job placement matters more than job speed: a 29s macOS job costs more than a 5min Linux one.

Three rules follow from it, and all three are already applied in the workflows:

1. **Only platform-specific work runs on macOS or Windows.**
   Anything that produces the same result on Linux runs on Linux, once.
   `mobile.yml::rust-check` builds `outl-mobile` only for this reason — `outl-actions` / `outl-core` / `outl-md` are covered by `ci.yml::test`, and running their suites again on macOS was 55% of a full run's bill.
2. **An expensive runner also has a cadence, and the cadence is a separate decision from the SKU.**
   Rule 1 asks whether a job belongs on macOS at all; this one asks how often it has to answer.
   Two jobs were paying every-event prices for a question that does not change that fast, and the evidence for both is in [Runner cadence](#runner-cadence) below.
3. **Size the SKU from measured CPU and memory, not from intuition.**
   `blacksmith jobs stats <job_id>` prints the per-job CPU timeseries, memory peak and OOM count; `blacksmith usage --breakdown-by workflow_job` prints what each job costs.

Sizes as of the Blacksmith migration, from a full run of every workflow:

| Job | SKU | Why |
|---|---|---|
| `ci.yml::fmt`, `ci.yml::advisories` | 2 vCPU | rustfmt and cargo-deny compile nothing (16s for fmt) |
| `mobile.yml::frontend`, `ci.yml::frontend`, `hygiene.yml::hygiene` | 2 vCPU | vitest + Vite build, 12s, never saturated 4 vCPU; the other two are ~4s of vitest and a line-count scan, with no compilation at all |
| `release.yml` orchestration (`prepare`, `tag`, `create_release`, `publish_*` except `publish_npm`, `update_tap`), `cleanup-tags.yml` | 2 vCPU | shell + `gh` calls, no compilation |
| `ci.yml::docs`, `ci.yml::sync`, `bench.yml` | 4 vCPU | doc/proptest jobs already finish in ~100s; bench stays fixed so numbers remain comparable run over run |
| `ci.yml::clippy`, `ci.yml::test (linux)`, `desktop.yml::check`, release builds | 8 vCPU | measured 71–76% average CPU on 4 vCPU with 2.6–4.7 GB of 16 GB used and zero OOM: CPU-bound, so more cores cut wall clock at roughly flat billing |
| macOS jobs | 6 vCPU | smallest macOS tier Blacksmith offers |
| `release.yml` Linux legs (CLI `x86_64`/`aarch64`, desktop `x86_64`/`aarch64`), `docker.yml` | 8 vCPU | same CPU-bound release build on either architecture; the ARM legs use the `-arm` suffix of the same SKU, billed at 0.625x |

**The Linux release legs are pinned to `ubuntu-2204`, not `2404`, and that is not a cost decision.**
A glibc-linked binary runs on the glibc it was built against or newer, never older, so the image the artifact is built on *is* its compatibility floor: 22.04 is glibc 2.35 (Debian 12, every supported Ubuntu LTS), 24.04 is 2.39 (and nothing older than itself).
`build_desktop_linux` was pinned for this from the start and said so; the CLI leg was built on 24.04 until [#411](https://github.com/outlmd/outl/issues/411), which made `outl-linux-x64.tar.gz` die with `version GLIBC_2.38 not found` on exactly the distros where someone downloads a tarball instead of using a package manager.
Bumping either to 24.04 raises that floor for everyone; do it on purpose, or not at all.

**The Windows ARM64 desktop leg is a cross-compile, and the target is installed with `rustup target add`.**
`build_desktop_windows` runs on the x64 `windows-2025` image, which already has stable for `x86_64-pc-windows-msvc` plus the MSVC ARM64 linker.
`dtolnay/rust-toolchain`'s `targets:` input runs `rustup toolchain install stable --target <triple>`, and on a toolchain the image already has, that does not install the extra target.
The job then died in `tauri build` with the target not installed, so `outl-desktop-windows-aarch64-setup.exe` never landed on a release.
`rustup target add` installs it on the toolchain `rust-toolchain.toml` selects.
The CLI matrix on the same image already did, which is why `outl-windows-arm64.zip` shipped and the installer did not.

**Check names must not contain the runner SKU.**
Branch protection matches required checks by name, so `test (ubuntu-latest)` becoming `test (blacksmith-8vcpu-ubuntu-2404)` leaves `main` blocked on a check that will never report again.
Matrix jobs therefore carry a stable `label` (`linux`, `macos`, `macOS arm64`, …) and the SKU lives only in `matrix.os`.

### Runner cadence

macOS is 6.67x Linux per vCPU-minute and 6 vCPU is the smallest tier offered, so a macOS job has no cheaper SKU to move to.
The only lever left is how often it runs.

The October 2026 bill is what sized both changes below: 5,657 macOS minutes, $452.56 of a $699.92 gross month, 65% of the bill on one platform.
Attributing those minutes to jobs put 63% of them in a single place.

| Job | min/month | $ gross | min/run |
|---|---:|---:|---:|
| `ci.yml::test (macos)` | 3,259 | $260.75 | 12.8 |
| `release.yml::build_desktop` (universal) | 525 | $41.96 | 7.9 |
| `mobile.yml::rust-check` | 352 | $28.18 | 1.7 |
| `desktop.yml::build` (macOS arm64) | 240 | $19.23 | 1.6 |
| `release.yml::build` (both Apple targets) | 425 | $34.07 | 3.7 |
| everything else on macOS | 400 | $32.02 | — |

**`ci.yml::test (macos)` runs on pushes to `main`, not on pull requests.**
The matrix is built from `github.event_name` rather than written out, so the leg is simply absent on a PR and the check keeps the name `test (macos)` on the runs that have one.
What justifies it: across July–October 2026, 403 runs where both legs reached a conclusion, the macOS leg caught **zero** failures the Linux leg did not.
It failed alone twice, both in September, and both were test calibration rather than platform defects — recalibrated in `085303b6` and `8f64fa4b`, after which October ran 254 of these clean.
Once, the reverse happened and Linux caught what macOS passed.
That is the expected result, because the two legs run the same crates: `--exclude outl-mobile --exclude outl-desktop --exclude outl-tauri-shared` leaves exactly one piece of macOS-specific code under test, `platform_host_id` in `outl-core/src/device/host.rs`, which shells out to `ioreg`.
Every other `cfg` in those crates selects iOS, which this runner is not.
PRs were 2,672 of the 3,259 minutes, so the change is worth $213.76 a month at gross rates and leaves a macOS-only defect a window of exactly one merge.

**`release.yml` cuts a beta nightly, not on every push to `main`.**
66 betas in October, 2.1 a day, each spending 15.3 macOS minutes across `build_desktop` and the two CLI legs.
The cron fires at 06:00 UTC and a `gate` job asks two questions, so an idle night and a broken night both cost nothing.
First, did anything land in the last 24h.
Second, does CI already know the tree is red: `release.yml` runs no tests of its own, `ci.yml` owns them, and without the second question the nightly spends 9 to 34 minutes across ~20 build legs producing a beta from a tree whose tests fail.
A tag push and a manual dispatch both bypass both gates — **if you need a beta right now, Actions → Release → "Run workflow"**, which is unchanged.

**What neither of these covers, and nothing covered before.**
APFS is case-insensitive and ext4 is not, so a slug collision that only reproduces on macOS is a real failure mode that no test in this workspace exercises.
The macOS CI leg never protected against it, on a PR or anywhere else, because no such test exists to run.
If that gap matters, the fix is a test for it on either platform, not restoring 12.8 minutes of macOS per pull request.

### What blocks merge

The `ci.yml` jobs are the merge gate.
The PR template and the policy in [`docs/contributing.md`](contributing.md) describe everything else reviewers look at on top of CI.

### Debugging a red CI

1. **Read the failing job's "Annotations"** at the top of the PR check page — it usually points at the file + line.
2. **Reproduce locally** with the exact command the workflow ran (the YAML is the source of truth).
3. **If `cargo doc` fails** with `rustdoc::private_intra_doc_links`, see the [doc gotcha](development.md#cargo-doc-gotcha) in the dev guide.
4. **If clippy fails on a target you don't have** (e.g. you're on Linux and it's the macOS leg), check that the failure isn't environmental before suspecting your patch.
   `outl-mobile` and `outl-desktop` are explicitly excluded from `ci.yml`'s clippy job; if you somehow re-included them, that's the bug.

### Flakes

Treat a flaky test as a real bug.
The CRDT and parser have no inherent flakiness — if a test fails twice and passes the third time, it's hiding a race or a non-deterministic order in something we control.
Don't add retries; find the cause.

---

## 2. Release process

### Version source of truth

`[workspace.package].version` in the root `Cargo.toml`.
Crate manifests inherit via `version.workspace = true`.
**Bumping the workspace bumps everything.**

`crates/outl-mobile/src-tauri/tauri.conf.json` deliberately omits `version`; CI reads `Cargo.toml` and injects the value into `cargo tauri ios build` via `--config`.
This is non-negotiable — see `crates/outl-mobile/CLAUDE.md` § "Versioning + TestFlight release".

### Beta cadence

A beta release is cut automatically **once a night**, at 06:00 UTC, and only when something landed on `main` in the preceding 24h *and* the newest conclusive `ci.yml` run on `main` is green.
It used to be cut on every push; [Runner cadence](#runner-cadence) has the measurement that changed it and `release.yml`'s `gate` job is where both checks live.
The CI check reads the newest run that actually concluded, so a `cancelled` run is skipped.
`ci.yml` cancels in progress per ref, which makes a cancelled run an answer about neither commit.
A gap in CI history reads as "unknown" and proceeds.
Refusing on unknown would turn any such gap into a silent release freeze, which is the worse of the two failures.
**To cut one immediately, run the workflow by hand** (Actions → Release → "Run workflow"), which bypasses the gate, as does a `v*` tag.

- Tag: `v<workspace.version>-beta.<run_number>` (e.g. `v0.6.0-beta.48`).
- Binary: reports the full beta version (the workflow `sed`s `Cargo.toml` in-place before `cargo build`; the change is local to the runner).
- GitHub: published as a prerelease with auto-generated release notes.
- Homebrew tap (`Formula/outl-beta.rb`, `Casks/outl-desktop-beta.rb`) bumped automatically with `[skip ci]` commit.

### GA

Bump `workspace.package.version` (e.g. `0.6.0` → `0.7.0`), merge to `main`, then push a `v0.7.0` tag by hand.
The `tags: ["v*"]` trigger in `release.yml` picks it up.

### What a release publishes

**Adding a platform starts in [`scripts/release_assets.py`](../scripts/release_assets.py).**
That file is the list of what a release publishes: one entry per build-matrix leg, naming its assets, whether it is `required`, and which package manager points at it.
Then the matrix leg in `release.yml`, its entries in `update_tap`'s loops, and the `url` / `sha256` / bottle anchors in the manifest that serves it.
Every way those can disagree is silent at release time — a `sed` that matches no line exits 0, `update_tap` finishes on `git diff --quiet`, which only asks whether *something* changed — so somebody's `brew install` or `scoop install` is the first thing to notice, one release later.
`scripts/check-release-assets.py --manifest` runs in `hygiene.yml` and fails on the mismatch instead; it parses rather than builds, so it takes a second and needs nothing installed.

**A release publishes on what is on the draft, not on whether every job went green.**
The gate used to be `needs.<job>.result == 'success'` per build job, which made each job a single point of failure for every other job's assets: one architecture's bundler, or one flaky runner, and a release with 25 finished artifacts published none of them.
The repo's own workaround for `build_android` was the opposite extreme — absent from the conditions, so a missing APK published in silence, and the same shape produced a v0.12.0 draft with zero assets that nothing complained about.

So `required` lives in that table and the pipeline reads it in three places:

- a leg marked optional carries `continue-on-error: ${{ matrix.optional || false }}`, which keeps the workflow run from going red over a platform still in bring-up;
- `publish_release` does not read job results at all, which is the part that actually keeps the release moving — a half-failed matrix reports whatever it reports to `needs`, and the gate never asks;
- what it reads instead is the draft's asset list, against the table: a missing **required** asset fails the publish, and a missing optional one publishes with the gap named in that release's own notes and in the run summary;
- `update_tap` moves a manifest only when every asset *that manifest* points at is present, because a manifest carrying a new version and the previous release's digest is a hash mismatch on install — worse than one still pointing at a release that exists.

Promoting a platform out of bring-up is flipping `required` in the table, and that flip is the record that it has been green.
Nothing about this is a reason to leave a platform optional: the gap is loud in three places, and a leg that keeps failing is a bug with a name.

**Asset names use two architecture spellings, and both are deliberate.**
The CLI / TUI tarballs are `outl-linux-x64` / `outl-linux-arm64` (and `outl-macos-x64` / `outl-macos-arm64`), the desktop bundles are `outl-desktop-linux-x86_64` / `outl-desktop-linux-aarch64`.
The desktop set matches `uname -m` and Linux packaging convention, which is what a `.deb` / `.rpm` / AppImage user expects; the CLI set is older and is what the Homebrew anchors and every published release URL already say.
A release asset URL does not redirect, so renaming either family breaks links that are already in the wild — the spellings stay split, and the docs name both.

### TestFlight (iOS)

`mobile.yml` builds + signs the IPA on every push.
`testflight.yml` runs after `mobile.yml` completes, downloads the IPA artifact, uploads to App Store Connect.

**Release notes** ("What to Test") come from `conventional-changelog-cli` (preset `conventionalcommits`) reading the commit log since the last tag.
**Use Conventional Commits.**
A commit without a `feat:` / `fix:` / `chore:` prefix lands under a generic "Other changes" bucket — the user loses context.

### iOS version propagation and TestFlight

**Single source of truth: `Cargo.toml` workspace `version`.**
To bump the app version, edit `[workspace.package].version` at the repo root — everywhere else inherits:

| Field | Where it lives | How it's resolved |
|-------|----------------|-------------------|
| Rust crate version | `crates/outl-mobile/src-tauri/Cargo.toml` | `version.workspace = true` |
| Tauri config version | `crates/outl-mobile/src-tauri/tauri.conf.json` | Field intentionally **omitted** in the source; CI injects it via `cargo tauri ios build --config '{"version": "<short>"}'` |
| `CFBundleShortVersionString` | iOS `Info.plist` | Tauri propagates from `--config` during `cargo tauri ios build` |
| `MARKETING_VERSION` / `CURRENT_PROJECT_VERSION` | `gen/apple/.../project.pbxproj` | Same — Tauri regenerates from the merged config every build |

**Why `--config` and not Tauri's `Cargo.toml` fallback?**
The iOS code path doesn't honor that fallback — it uses `1.0.0` instead.
So CI reads the workspace version (`awk` against `Cargo.toml` in `Compute build metadata`) and passes it via `--config`; the `Patch archive CFBundleVersion` step aborts if the propagated short version mismatches.
**Never** put `"version": "x.y.z"` back in `tauri.conf.json` — Tauri would then use the static value over the `--config` override and the two drift on the next bump.

#### CI release flow

A push to `main` triggers in parallel:

1. **`Release`** (`release.yml`) — auto-bumps `Cargo.toml` locally to `<base>-beta.<run_number>`, cuts a `v<...>-beta.<N>` tag, builds desktop binaries, ships the Homebrew formula (never commits the bump back).
2. **`Mobile`** (`mobile.yml`) — builds the signed IPA from the *unbumped* `Cargo.toml`, uploads it as `outl-ios-release`, triggers `TestFlight`.
3. **`TestFlight`** (`testflight.yml`) — downloads the artifact, uploads to App Store Connect (`xcrun altool`).

#### CFBundleVersion (build number) scheme

Apple needs `CFBundleVersion` strictly monotonic across every IPA, but the marketing version (`0.5.1`) repeats across beta builds.
Scheme: `CFBundleShortVersionString = <SHORT_VERSION>` (e.g. `0.5.1`); `CFBundleVersion = <SHORT_VERSION><BETA_PAD>` (e.g. `0.5.1027`, beta number zero-padded to 3 digits).
`BETA` comes from the latest `v<SHORT_VERSION>-beta.<N>` git tag (set by `Release`), falling back to Mobile's `github.run_number`; re-runs append `.<run_attempt>` as a 4th component to dodge Apple's duplicate guard.
The build number is patched into the `.xcarchive`'s embedded `Info.plist` after `cargo tauri ios build` but **before** `xcodebuild -exportArchive` — the only injection point that survives, since Tauri exposes a single `version` field.

#### What goes wrong if you forget this

- Patching `gen/apple/.../Info.plist` pre-build is a no-op (Tauri regenerates it); `xcrun altool` exits 0 even on 409, so `testflight.yml` greps for `ERROR:`, don't simplify it.

### Homebrew

The tap is at the root of this same repo (`Formula/`, `Casks/`).
The `update_tap` job in `release.yml` patches version + sha256 anchors for both the CLI formula and the desktop cask after every beta release.

The desktop dmg is **unsigned** today (Apple Developer account pending).
The cask carries a `caveats` block with the `xattr -dr com.apple.quarantine` workaround.

### crates.io (lib crates for embedders)

The `publish_crates` job in `release.yml` publishes `outl-core`, `outl-md`, `outl-actions`, `outl-ws` and `outl-exec` to crates.io on every release, beta and GA, at the same version the binaries report.
The first four are the [embedding contract](embedding.md); `outl-exec` ships only because `outl-actions` references it and cargo refuses to publish a crate whose dependencies aren't in the registry.
Every other crate in the workspace carries `publish = false`; flipping one later is a one-line change (plus adding it to the job's publish loop).

SemVer keeps consumers safe: a plain `outl-core = "0.11"` requirement never resolves a `-beta.N` prerelease, so crates.io consumers only see GA versions unless they opt in with a `"0.11.0-beta"`-style requirement.

The crates depend on each other with an exact `=` requirement, written by [`scripts/pin_workspace_deps.py`](../scripts/pin_workspace_deps.py) after the version rewrite.
A caret on `0.13.0-beta.N` also matches the stable `0.13.0`, and cargo picks the stable because a release sorts above its prereleases.
`cargo publish` verifies against that resolution, so a beta would build against the stable already on crates.io and miss any API added since the tag.
The publish loop stays in dependency order (`outl-core`, `outl-md`, `outl-exec`, `outl-actions`, `outl-ws`).
The exact pin is what makes the next crate's verify select the version this job just published.
cargo waits for that version to show up in the sparse index before starting the next crate.

The job is idempotent (it checks crates.io before each publish, so re-runs resume instead of failing) and self-gating (`cargo publish` verifies each crate by building it).
It authenticates with the `CARGO_REGISTRY_TOKEN` secret in the `release` environment.

---

