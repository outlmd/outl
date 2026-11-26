# CI runners and cost

Where each job runs, how often, and what that costs.
Split out of [CI and release](ci-and-release.md), which keeps the workflow table and the release process.
Read this before changing a `runs-on:`, a trigger's cadence, or a cache step.

If anything here is wrong or out of date, that is a bug: fix it in the same PR that drifted the behavior.

---

## Runner sizing

Compile work runs on Blacksmith runners (`runs-on: blacksmith-<n>vcpu-<image>`, with `-arm` appended for ARM64 Linux).
These jobs run on GitHub-hosted runners instead, each on purpose:

- **`release.yml::publish_npm`** runs on `ubuntu-24.04` because npm trusted publishing (OIDC) does not accept self-hosted runners yet, and Blacksmith registers as self-hosted.
  Moving it back to Blacksmith breaks the npm publish.
  Its trusted publisher on npmjs.com (`@outl/plugin-sdk` → Settings → Trusted publishing) must stay configured as owner `outlmd`, repository `outl`, workflow `release.yml`, environment `release`, granting `npm publish`.
  See [npm trusted publishing](ci-and-release.md#npm-trusted-publishing) for the three fields that are easy to get wrong.
- **`nix-build.yml`** runs on `ubuntu-24.04` / `ubuntu-24.04-arm`.
  This repository is public, so GitHub-hosted Linux minutes are free, and the flake has no binary cache: every run is a full from-scratch compile of the workspace plus the Tauri desktop app, measured at 31 and 29 minutes for the two legs.
  Putting that on Blacksmith converts the single longest job in the repo into the single most expensive one, for a build whose only product is a signal.
  What it costs instead is wall clock a contributor waits on, which is why the trigger is narrow rather than the runner fast.
- **Under one billed minute and no compile goes hosted.**
  That is `ci.yml::fmt`, `ci.yml::advisories`, `ci.yml::frontend`, `hygiene.yml`, `cleanup-tags.yml` and `docker.yml::merge`, on `ubuntu-24.04`.
  Blacksmith bills a 16-second job as a full minute, and the repo is public, so a hosted runner does the same work for $0.
  The rounding matches GitHub's usage report within 1% but was never checked against `blacksmith usage`.
  `release.yml`'s orchestration jobs and the `gate` jobs fit the rule and have not moved yet.
- **`ci.yml::test (macos)`** runs on `macos-15`.
  It is post-merge and nobody waits on it, yet push-only it was still ~$44 of a ~$325 30-day bill (November 2026).
  Hosted M1 is 3 vCPU and 7 GB, so expect 2-3x the wall clock.
  If it OOMs at link time or runs past ~25 min, move it to a nightly `test (macos)` in `release.yml`, not back to the paid SKU.
- **`mobile.yml::swift-tests`** runs on `macos-15`: a ~24 s job that was billed as a full macOS minute ~270 times a month (~$16.5).
  `swift-tools-version:5.9` needs nothing newer than the Xcode that image ships.
- **`testflight.yml::upload`** runs on `macos-15`: it compiles nothing (download, `altool`, one API call, ~3 min of network).

A hosted job's caches live in GitHub's 10 GB budget, not Blacksmith's: see [Cache budget](#cache-budget).

Billing is per vCPU-minute, with a platform multiplier on top: ARM 0.625x, x64 Linux 1x, Windows 2x, macOS 6.67x.
That multiplier is why job placement matters more than job speed: a 29s macOS job costs more than a 5min Linux one.

Three rules follow from it, and all three are already applied in the workflows:

1. **Only platform-specific work runs on macOS or Windows.**
   Anything that produces the same result on Linux runs on Linux, once.
   `mobile.yml::rust-check` builds `outl-mobile` only for this reason — `outl-actions` / `outl-core` / `outl-md` are covered by `ci.yml::test` and `ci.yml::sync`, and running their suites again on macOS was 55% of a full run's bill.
2. **An expensive runner also has a cadence, and the cadence is a separate decision from the SKU.**
   Rule 1 asks whether a job belongs on macOS at all; this one asks how often it has to answer.
   Two jobs were paying every-event prices for a question that does not change that fast, and the evidence for both is in [Runner cadence](#runner-cadence) below.
3. **Size the SKU from measured CPU and memory, not from intuition.**
   `blacksmith jobs stats <job_id>` prints the per-job CPU timeseries, memory peak and OOM count; `blacksmith usage --breakdown-by workflow_job` prints what each job costs.

Sizes as of the Blacksmith migration, from a full run of every workflow, with the November 2026 moves to hosted:

| Job | SKU | Why |
|---|---|---|
| `ci.yml::fmt`, `ci.yml::advisories`, `ci.yml::frontend`, `hygiene.yml`, `cleanup-tags.yml`, `docker.yml::merge` | hosted `ubuntu-24.04` | nothing compiled, under a billed minute (16s for fmt, ~4s of vitest): see the hosted list above |
| `mobile.yml::frontend` | 2 vCPU | vitest + Vite build, 12s, never saturated 4 vCPU |
| `release.yml` orchestration (`prepare`, `tag`, `create_release`, `publish_*` except `publish_npm`, `update_tap`) | 2 vCPU | shell + `gh` calls, no compilation |
| `ci.yml::docs`, `ci.yml::sync`, `bench.yml` | 4 vCPU | doc/proptest jobs already finish in ~100s; bench stays fixed so numbers remain comparable run over run |
| `ci.yml::clippy`, `ci.yml::test (linux)`, `desktop.yml::check`, release builds | 8 vCPU | measured 71–76% average CPU on 4 vCPU with 2.6–4.7 GB of 16 GB used and zero OOM: CPU-bound, so more cores cut wall clock at roughly flat billing |
| macOS jobs that compile (`mobile.yml::rust-check`, `ios-build`, `ios-release`, `desktop.yml` and `release.yml` legs) | 6 vCPU | smallest macOS tier Blacksmith offers; the other macOS jobs are hosted |
| `release.yml` Linux legs (CLI `x86_64`/`aarch64`, desktop `x86_64`/`aarch64`), `docker.yml::build` | 8 vCPU | same CPU-bound release build on either architecture; the ARM legs use the `-arm` suffix of the same SKU, billed at 0.625x |

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

## Test scope

**`cargo test` runs `--lib --bins --tests`, never `--all-targets`, and that is worth 86% of the job.**

The difference between the two is `--benches`, and `cargo test --benches` does not merely compile a benchmark, it *runs* it in test mode.
This workspace has six criterion bench targets across `outl-core` and `outl-md`, which expand to 90 benchmarks including three tiers of `boot/replay_memory` and `boot/replay_disk` at 218k ops.

Measured on one machine with a fully warm target dir, so the number is execution only:

| Command | Wall clock | Suites | Tests passed |
|---|---:|---:|---:|
| `--all-targets` | 17m34s | 202 | 3818 |
| `--lib --bins --tests` | 2m52s | 200 | 3818 |

Of the 1054s the first one took, the test binaries' own `finished in` lines accounted for 173s.
The other 84% was benchmarks, running inside the job whose name says tests, on every pull request, with `sync` paying it a second time because `outl-core` is one of the two crates it names.
The two missing "suites" are the bench binaries, which reported `0 passed` either way.

Nothing is lost, and each piece has an owner that still runs on the same PR:

- **Running** the benchmarks belongs to `bench.yml`.
- **Compiling** them is covered by `clippy`, which does pass `--all-targets`, so a benchmark that stops compiling still fails this workflow.
- There are no `examples/` in the workspace, and all six bench targets are `harness = false`, so none of them holds a `#[test]` that would now go unrun.

`cargo-nextest` is the obvious next question, and the measurement argues against reaching for it yet.
The run above averaged 1.1 of 14 cores, because `cargo test` serialises *between* binaries, but the real work inside them is only 173s.
Fix the 881s first.

**One number above is now stale, on purpose rather than by neglect.**
[Runner sizing](#runner-sizing) puts `ci.yml::test (linux)` on 8 vCPU because it measured 71–76% average CPU on 4 vCPU, and that measurement was taken while the job was spending 84% of its time on benchmarks.
The right move is to re-measure with `blacksmith jobs stats <job_id>` once this has run a few times on `main` and resize from what it reports, which is the rule that section already states.
Do not resize it from this paragraph.

## Runner cadence

macOS is 6.67x Linux per vCPU-minute and 6 vCPU is the smallest tier offered, so a macOS job has no cheaper Blacksmith SKU to move to.
The levers left are how often it runs and, on a public repo, GitHub's free hosted runner.

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
That is the expected result, because the two legs ran the same crates.
`--exclude outl-mobile --exclude outl-desktop --exclude outl-tauri-shared` leaves exactly one piece of macOS-specific code under test, `platform_host_id` in `outl-core/src/device/host.rs`, which shells out to `ioreg`.
Every other `cfg` in those crates selects iOS, which this runner is not.
PRs were 2,672 of the 3,259 minutes, so the change is worth $213.76 a month at gross rates and leaves a macOS-only defect a window of exactly one merge.
The leg has since moved to a free hosted runner ([Runner sizing](#runner-sizing)), and the window is unchanged.

**`release.yml` cuts a beta nightly, not on every push to `main`.**
66 betas in October, 2.1 a day, each spending 15.3 macOS minutes across `build_desktop` and the two CLI legs.
The cron fires at 06:00 UTC and a `gate` job asks two questions, so an idle night and a broken night both cost nothing.
First, did anything other than the bot's `packaging: outl …` commits land since the commit the last beta was built from.
That commit is the head of the newest completed scheduled or dispatched run on `main` whose `create_release` succeeded, so a gate-skipped night or a dry run is never the anchor.
The run as a whole may be red: a night whose npm publish failed still built that commit, and requiring a green run would rebuild it every idle night until one went fully green.
It replaced a 24h window that erred both ways: it counted the packaging commit (beta.292 on Nov 5 was built off nothing else), and it missed commits whenever the cron fired late, which it did by 6 to 11 hours in November.
With no such run found, gate 1 falls back to the last 24h; with the anchor outside the 100 fetched commits, it proceeds.
Second, does CI already know the tree is red: `release.yml` runs no tests of its own, `ci.yml` owns them, and without the second question the nightly spends 9 to 34 minutes across ~20 build legs producing a beta from a tree whose tests fail.
A tag push and a manual dispatch both bypass both gates — **if you need a beta right now, Actions → Release → "Run workflow"**, which is unchanged.

**`mobile.yml::ios-release` builds nightly, not on every push to `main`.**
It archives, signs and hands an IPA to `testflight.yml`, so `on: push` meant one TestFlight build per merge.
That was 10 pushes in four days, each spending macOS-rate minutes on an archive nobody requested, and each one a build number a tester scrolls past to find the one they were told about.
The `gate` job is the same shape as `release.yml`'s and asks the same two questions, with one difference that matters: gate 1 counts commits **against this workflow's path list** rather than against `HEAD`.
A beta ships the whole repo, so any commit justifies one; an IPA is the mobile app, and a night whose only commits were in `docs/` or `outl-cli` has nothing new to put in front of a tester.
The fast guards (`rust-check`, `frontend`, `swift-tests`) still run per push and per PR, because those are what answer "did this change break mobile" and they are cheap.
A nightly and a manual dispatch each get a concurrency group keyed on the run id, so a merge landing at 05:10 UTC cannot cancel the night's build.
From Nov 21 the nightly built nothing: `gate` has no checkout, and `gh` there needs `GH_REPO` to know the repo, so every scheduled run failed with `failed to determine base repo`.
`testflight.yml` also accepted only `push` and `workflow_dispatch` Mobile runs, so a nightly IPA would not have shipped either.
Both are fixed, and `testflight.yml`'s `gate` now accepts `schedule`.
**If you need a TestFlight build now: Actions → Mobile → "Run workflow" with `release_build: true`**, which bypasses both gates.

**`bench.yml` has no `pull_request` trigger, and the reason is signal rather than cost.**
`bench-quick` compiled, measured and uploaded criterion's HTML, comparing against nothing: no baseline from the merge base in the run, no threshold, no step that could fail.
A PR paid ~3.4 minutes for a number that only becomes a signal if a human downloads two zips and diffs them by hand.
Adding a percentage comparison is not the fix, because wall clock on a shared 4 vCPU runner moves by double digits between identical runs, so the gate would fail on noise and be muted inside a sprint.
The gate that would work is deterministic instead: instruction counts via iai-callgrind plus absolute budgets over `bench-cli-xlarge`'s JSON, tracked as open work in [boot performance](boot-performance.md).
Until that lands, `main` carries the per-commit series and the cron carries the trend.

**What neither of these covers, and nothing covered before.**
APFS is case-insensitive and ext4 is not, so a slug collision that only reproduces on macOS is a real failure mode that no test in this workspace exercises.
The macOS CI leg never protected against it, on a PR or anywhere else, because no such test exists to run.
If that gap matters, the fix is a test for it on either platform, not restoring 12.8 minutes of macOS per pull request.

## Why `nix-build` does not watch `crates/**`

It did, and it was the largest consumer of runner time in the repo.
61 minutes per run (31 on `x86_64`, 29 on `aarch64`) across 27 runs in four days, because the flake has no binary cache and each leg compiles the whole Rust workspace plus the Tauri desktop app from nothing.

What that buys is the **packaging**, not the code.
Take `cargoHash`, `frontendNodeModulesHash`, `buildInputs`, `wrapProgram`, the `.desktop` entry and the home-manager module.
Every one of those is decided by `flake.nix`, `hm-module.nix`, a lockfile or `rust-toolchain.toml`, which is exactly the trigger list.
A change under `crates/**` can only break this workflow by failing to compile, and `ci.yml::clippy` reports that in about a minute on the same change, thirty times sooner.

The gap that leaves is narrow and real, so it is covered rather than ignored.
A change in `crates/**` that needs a **new system library** without touching any lockfile trips nothing in the list.
The nightly cron at 04:00 UTC catches it within a day, and `workflow_dispatch` catches it on demand, which is what to reach for before tagging a release if the flake matters to you.

## Cache budget

A repository gets **10 GB of Actions cache in total**, shared by every workflow, evicted least-recently-used, and entries expire after 7 days without a read.
That total is a resource with no owner, and it is possible for one workflow to consume all of it without failing.

That happened.
`docker.yml` exported `cache-to: type=gha,mode=max` under two scopes (`docker-amd64`, `docker-arm64`), which puts every intermediate layer of a multi-stage Rust build into the shared budget.
A census found 11.00 GB across 242 entries, **202 of them buildkit blobs**, with zero entries left for anything else.
The `Swatinem/rust-cache` entries in `ci.yml`, the bun caches, and the cargo caches in `bench.yml` and `release-screenshots.yml` (the latter since removed: it could never hit) had all been evicted, and the two docker legs were evicting each other.

The fix is `cache-to: type=registry`, pointing at `:buildcache-<arch>` next to the image in GHCR.
It is billed as package storage instead of cache, so it neither competes with the Rust caches nor expires at 7 days.
`cache-to` is gated on `github.event_name != 'pull_request'` because a fork's PR holds no write token; `cache-from` needs none against a public package.

Three rules follow:

1. **A workflow may not export `mode=max` to `type=gha`.**
   Intermediate layers of a compile belong in a registry cache.
   If you need `type=gha`, `mode=min` is the ceiling.
2. **Check the census before blaming a cold build.**
   `gh api --paginate "/repos/outlmd/outl/actions/caches?per_page=100"` lists every entry with its size, and `gh api /repos/outlmd/outl/actions/cache/usage` gives the total.
   A Rust job that suddenly takes 12 minutes instead of 2 is usually not a code change: it is a restore that found nothing.
3. **Only `main` saves a Rust cache.**
   Every `Swatinem/rust-cache` step sets `save-if: ${{ github.ref == 'refs/heads/main' }}` (`discussions-discord.yml` spells it with `default_branch`).
   A PR's save lands in its own ref scope and a tag's under `refs/tags/*`, and no later `main` or nightly run can read either.
   Each dependabot `Cargo.lock` PR wrote ~1.5 GB per job that way, and a GA tag run ~20 GB, all against the 25 GB Blacksmith cache main's entries live in.
   PRs still restore main's entries.

Blacksmith redirects `actions/cache` to its own colocated cache, so jobs on `blacksmith-*` runners are not what fills this budget.
The hosted jobs in [Runner sizing](#runner-sizing) do: `test (macos)`'s rust-cache (~1.35 GB) and `frontend`'s bun cache count against these 10 GB.
Buildx's `type=gha` talks to the GitHub Actions cache API directly and is not redirected, which is why one workflow could fill the whole thing while every other cache in the repo looked fine in its own YAML.
