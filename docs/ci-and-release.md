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
| [`ci.yml`](../.github/workflows/ci.yml) | Push / PR to `main` (skipped on docs-only, and on paths no test reads: `scripts/**`, the package manifests, the flake, sibling workflows; the header comment lists them and the rule for adding one) | `cargo fmt --check`, `cargo clippy -D warnings` (`--all-targets`), `cargo test` (`--lib --bins --tests`, deliberately not `--all-targets`: see [Test scope](ci-runners.md#test-scope)), `cargo doc -D warnings`, an **`advisories`** job (`cargo deny --locked check advisories` against `deny.toml`: any RustSec vulnerability fails, the whole graph is checked for unmaintained and unsound crates too, and every ignore carries a reason), plus a dedicated **`sync`** job (`outl-core` + `outl-sync-iroh` with `PROPTEST_CASES=1024`), and a **`frontend`** job (`bun run test` + `bun run typecheck` over every bun workspace package). Excludes `outl-mobile` + `outl-desktop` from the Rust jobs. Test matrix: `test (linux)` on every event, plus `test (macos)` on pushes to `main` only, on a GitHub-hosted runner (see [Runner sizing](ci-runners.md#runner-sizing)). `test (linux)` excludes `outl-core` and `outl-sync-iroh`, which `sync` already runs on Linux at 1024 cases; `test (macos)` still runs them. | **Yes** |
| [`hygiene.yml`](../.github/workflows/hygiene.yml) | Push / PR to `main`, **including** docs-only changes | `scripts/check-file-size.sh` (the file size ratchet), `scripts/check-docs-index.sh` (every RFC reachable from `docs/SUMMARY.md`), `scripts/tests/test_check_artifact_stripped.py` (the strip gate's own suite), and `scripts/check-release-assets.py --manifest` plus its suite (the three build matrices, `update_tap`'s loops, the Homebrew formula, the cask and the Scoop manifest all name the platforms `scripts/release_assets.py` names), and `scripts/check-ios-flavor.py` (every `$(OUTL_*)` in the iOS project has a definition in `Outl.xcconfig`, and no preview override reached a committed file). Kept out of `ci.yml` on purpose: that workflow skips `docs/**`, which is exactly the change set the index check has to see. | **Yes** |
| [`docs-reminder.yml`](../.github/workflows/docs-reminder.yml) | PR to `main` (opened, synchronize, reopened, ready for review, labeled, unlabeled), drafts skipped | Lists the PR's files and asks [`scripts/docs_reminder.py`](../scripts/docs_reminder.py) whether a user-visible path changed (CLI, MCP tools, TUI, desktop, mobile, shortcuts, config, themes, plugins, markdown, sync, import, exec) with no `docs/**`, `README.md` or `CHANGELOG.md` in the diff. If so it posts **one** sticky comment (edited in place, found by a hidden marker) with the four-question checklist from [Documentation standard](contributing.md#documentation-standard). Skips the `no-docs-needed` label, dependabot, and `chore(deps)` / `ci:` / `test:` / `refactor:` titles unless `needs-docs` is set. Soft by default, never fails the job; set the repo variable `DOCS_REMINDER_STRICT=true` to make a missing update red. The path map has its own suite, run by `hygiene.yml`. Fork PRs get the text in the job summary, since their token cannot comment. | No (**Yes** with `DOCS_REMINDER_STRICT=true`) |
| [`mobile.yml`](../.github/workflows/mobile.yml) | Push / PR touching mobile paths for the guards; **nightly cron at 05:00 UTC** (gated) or manual dispatch for the signed build | A `vite build`, Swift tests and the Rust mobile crate on every push / PR. The signed archive (`ios-release`, which feeds `testflight.yml`) is nightly and gated, never per push: see [Runner cadence](ci-runners.md#runner-cadence). The vitest suite is **not** here: `ci.yml::frontend` runs `bun run test` at the repo root, which is `bun --filter '*' test` and already covers this package, so running it here too meant two runners and two red checks for one cause. | Mobile changes only |
| [`desktop.yml`](../.github/workflows/desktop.yml) | Push / PR touching desktop paths | `check` on Linux: clippy, Rust tests and a debug `tauri build`. Then the same debug build on macOS arm64 and Windows x86_64. Vitest and `tsc` are not here: `ci.yml::frontend` runs both over every package. | Desktop changes only |
| [`bench.yml`](../.github/workflows/bench.yml) | **Push** to `main` touching `outl-core`, `outl-md`, `outl-actions` or `outl-tauri-shared`, plus weekly cron. No `pull_request` trigger: see [Runner cadence](ci-runners.md#runner-cadence) | Criterion (small/medium/large) + `outl-core`'s `boot` bench at 10k on every push; xlarge, boot at 50k/218k and CLI hyperfine on cron / manual dispatch. A trend line, not a gate: wall clock on a shared runner moves by double digits between identical runs ([boot budgets](boot-performance.md)). Artifacts retained 14–30 days. | No (informational) |
| [`release.yml`](../.github/workflows/release.yml) | Nightly cron at 06:00 UTC when a commit other than the bot's `packaging: outl …` landed since the last beta was cut **and** the newest conclusive `ci.yml` run on `main` is green (beta), `v*` tag (GA), manual. Runs no tests of its own: `ci.yml` owns them, and the gate is what keeps ~20 build legs off a red tree | Computes version from `Cargo.toml`, builds the CLI + TUI matrix (macOS `arm64`/`x86_64`, Linux `x86_64`/`aarch64`, Windows `x86_64`/`aarch64`), builds the desktop bundles (universal macOS dmg, Linux AppImage/deb/rpm and a Windows NSIS installer per architecture), the Android APK, drafts release, uploads assets, gates the publish on [`scripts/release_assets.py`](../scripts/release_assets.py), bumps the Homebrew formula + cask and the Scoop manifest (`bucket/outl-beta.json`), publishes `@outl/plugin-sdk` to npm and the embedder lib crates to crates.io. | n/a |
| [`testflight.yml`](../.github/workflows/testflight.yml) | `Mobile` workflow completing successfully, or manual dispatch | Skips the upload (green run, `gate` notice) when the Mobile run has no non-expired `outl-ios-release` artifact; manual dispatch bypasses that check. Otherwise downloads the signed `.ipa`, uploads to App Store Connect via `xcrun altool`, sets "What to Test" notes via App Store Connect API, on a GitHub-hosted `macos-15`. The IPA is not re-uploaded as an artifact: the Mobile run's `outl-ios-release` is the copy, kept 30 days. | n/a |
| [`docker.yml`](../.github/workflows/docker.yml) | Push to `main` touching what the image is built from (the crates `outl-cli` pulls in, manifests, lockfile, toolchain, `Dockerfile`, `docker/**`), any `v*` tag (`paths:` does not apply to tags), PR touching `Dockerfile` / `docker/**`, manual | Builds `ghcr.io/<owner>/outl-server` once per architecture on a runner of that architecture (`linux/amd64`, `linux/arm64`), pushes each by digest, merges them into one manifest list per tag, and attests that list through Sigstore (`gh attestation verify oci://…`). A PR builds and pushes nothing. `:latest` goes on the newest stable **release**, decided against `gh release list` and not `git tag --list`, because a stray `v1.0.12` left by the Raycast extension's publish outranked every outl version and held `:latest` back from every 0.13.x GA. | No |
| [`nix-build.yml`](../.github/workflows/nix-build.yml) | PR / push touching `flake.nix`, `flake.lock`, `hm-module.nix`, a lockfile, `rust-toolchain.toml` or the workflow, plus a nightly cron at 04:00 UTC | `nix flake check --no-build`, then builds `.#outl-desktop-node-modules` (the frontend fixed-output derivation, which is how a stale `frontendNodeModulesHash` reports itself), `.#outl`, `.#outl-desktop` and the `home-manager-module` check, on `x86_64-linux` and `aarch64-linux`. **`crates/**` is deliberately not a trigger**, see [Why `nix-build` does not watch `crates/**`](ci-runners.md#why-nix-build-does-not-watch-crates). | No |
| [`cleanup-tags.yml`](../.github/workflows/cleanup-tags.yml) | Called by `release.yml` from `needs: tag` (once per tag created), or manual | Keeps the 2 most recently generated `vX.Y.Z-beta.N` tags repo-wide and deletes every older one, release and tag together. GA tags are never candidates, and neither is a beta a shipped package manifest still points at. Each deletion is confirmed by re-reading the ref from origin, because a 46-tag drain once printed `- [deleted]` 46 times, left 21 refs in place and still exited 0. Not `on: push: tags:`: the tag is pushed with `GITHUB_TOKEN`, which triggers no workflow. A pruned name is never reusable, which is [its own section](#a-pruned-beta-tag-name-never-comes-back). | n/a |

[`discussions-discord.yml`](../.github/workflows/discussions-discord.yml) is not a CI check: it mirrors new Discussions and comments into a Discord forum, see [Discussions mirror to Discord](discussions-discord.md).
It builds and runs the `tools/discussions-discord` crate, whose tests run in `ci.yml` with the rest of the workspace.

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
  Both workflows now use `Swatinem/rust-cache` with a per-job `shared-key` (in `release.yml`: `release-<cli|desktop|android>-<triple>`), so they agree and nothing is left to hoist.
  `release.yml` used to cache `target` with `actions/cache`, and 54 of 60 legs restored nothing: a cold Windows leg takes 17-29 min against 6-7 warm.
  Its step sits above the version `sed`, so the nightly `-beta.N` does not enter the key.
- **The build.**
  `desktop.yml` runs a debug `--no-bundle` compile to answer "does it build".
  `release.yml` bundles, signs, strips, verifies and uploads.
  Those are different jobs that happen to share a prologue, which is why only the prologue moved.

### Runner sizing

Moved to [CI runners and cost](ci-runners.md#runner-sizing).

### Test scope

Moved to [CI runners and cost](ci-runners.md#test-scope).

### Runner cadence

Moved to [CI runners and cost](ci-runners.md#runner-cadence).

### Why `nix-build` does not watch `crates/**`

Moved to [CI runners and cost](ci-runners.md#why-nix-build-does-not-watch-crates).

### Cache budget

Moved to [CI runners and cost](ci-runners.md#cache-budget).

### What blocks merge

**Nothing, today.**
The `main` ruleset (Settings → Rules) carries `deletion` and `non_fast_forward`, and no `required_status_checks` at all.
So every row in the table above is advisory however red it goes, and the "Blocks merge?" column describes intent rather than enforcement.
Verify it rather than trusting this paragraph: `gh api /repos/outlmd/outl/rules/branches/main`.


**Turning them on is not a settings toggle, and that is why it has not happened.**
A required status check must report on *every* pull request, or the PR waits on a check that will never arrive and cannot be merged at all.
Every workflow in the table above is filtered:

- `desktop.yml`, `mobile.yml`, `bench.yml`, `nix-build.yml` and `docker.yml` carry a `paths:` allowlist, so they produce no check at all on a PR outside their paths.
- `ci.yml` carries `paths-ignore`, which is the same trap in the one case that matters most for it: a docs-only PR runs no `ci.yml` job, so `rustfmt` as a required check would block every documentation change forever.
- `hygiene.yml` is the only workflow with no filter of any kind, which is exactly why it was written that way, and the only one that could be required as it stands.

Three ways out, and the right one depends on how many people are merging:

1. **A merge queue.**
   The queue runs the full set against the merge result, so a path filter stops being a hole.
   It also removes the duplication where every merge re-runs on `main` what the PR just ran green, and it is the only one of the three that lets the `paths:` lists stop being written twice per workflow.
   It is also the most process for a repo with one main maintainer.
2. **Sentinel jobs.**
   A second, unfiltered job per workflow that reports success when the real one was skipped.
   It works, and it means every filtered workflow grows a job whose only purpose is to lie convincingly to branch protection.
3. **Leave it advisory and treat that as the design.**
   Which is the current state, now stated rather than assumed.

Option 3 is the one in force.
The consequence is a budget rule, not a shrug: **if the gate is a human reading a check, then a job that no human reads is not signal, it is spend.**
That is the test `bench.yml` failed on pull requests (a measurement compared against nothing), and the one `nix-build.yml` failed on `crates/**` (a 31-minute answer to a question `clippy` answers in one).
Apply it to the next job somebody wants to add.

Two precautions stay in the workflows regardless, because they cost nothing and they are what makes option 1 or 2 cheap later:

- Matrix legs carry a stable `label` instead of the runner SKU, because required checks match by name and a SKU bump would silently rename one.
- `ci.yml::sync` exists as its own job so a convergence regression shows up as its own red check rather than buried in a broad matrix job.
  It is also the only Linux run of `outl-core` and `outl-sync-iroh`, which `test (linux)` excludes, so filtering or deleting it means dropping that exclude.

The PR template and the policy in [`docs/contributing.md`](contributing.md) describe everything reviewers look at on top of CI.

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

A beta release is cut automatically **once a night**, at 06:00 UTC, and only when a commit other than the bot's packaging commit landed on `main` since the last beta was cut *and* the newest conclusive `ci.yml` run on `main` is green.
It used to be cut on every push; [Runner cadence](ci-runners.md#runner-cadence) has the measurement that changed it and `release.yml`'s `gate` job is where both checks live.
The CI check reads the newest run that actually concluded, so a `cancelled` run is skipped.
`ci.yml` cancels in progress per ref, which makes a cancelled run an answer about neither commit.
A gap in CI history reads as "unknown" and proceeds.
Refusing on unknown would turn any such gap into a silent release freeze, which is the worse of the two failures.
**To cut one immediately, run the workflow by hand** (Actions → Release → "Run workflow"), which bypasses the gate, as does a `v*` tag.

- Tag: `v<workspace.version>-beta.<run_number>` (e.g. `v0.6.0-beta.48`).
- Binary: reports the full beta version (the workflow `sed`s `Cargo.toml` in-place before `cargo build`; the change is local to the runner).
- GitHub: published as a prerelease with auto-generated release notes.
  `publish_release` appends one sponsor link at the end of the body ("Ask for sponsors in the release notes"), after any missing-assets note.
- Homebrew tap (`Formula/outl-beta.rb`, `Casks/outl-desktop-beta.rb`) bumped automatically with `[skip ci]` commit.

### A pruned beta tag name never comes back

Immutable releases are on, so every published release carries `immutable: true` and GitHub keeps its tag name reserved for good.
`cleanup-tags.yml` deletes the release and the tag together, and deleting both frees nothing.

So `git push --tags` fails on any machine still holding a tag the prune already took:

```
remote: error: GH013: Repository rule violations found for refs/tags/v0.13.0-beta.299.
remote: - Cannot create ref due to creations being restricted.
```

The message points at Settings → Rules, and the rules are not what refused it.
Neither repository ruleset carries a `creation` rule.
The reserved name is what refuses, and no ruleset edit makes it reusable.

Push the one ref you cut (`git push origin v0.13.6`), and drop the stale locals with `git fetch --tags --prune --prune-tags --force origin`.
`git push --tags` is the wrong command in this repo, and `-f` on top of it is worse.

CI never trips on this, because `N` in `-beta.N` is `github.run_number` and never repeats.
A GA tag is the one to watch, and the ruleset below is what watches it.
Deleting one is refused rather than permitted, because a spent name cannot be recovered and a refusal can be lifted.

### GA

Bump `workspace.package.version` (e.g. `0.6.0` → `0.7.0`), merge to `main`, then push a `v0.7.0` tag by hand.
The `tags: ["v*"]` trigger in `release.yml` picks it up.

### A GA tag cannot be deleted or re-pointed

`tag rules` (Settings → Rules, ruleset `27314778`) carries `deletion`, `non_fast_forward` and `update` over `refs/tags/v*`, and excludes `refs/tags/v*-alpha.*`, `refs/tags/v*-beta.*` and `refs/tags/v*-rc.*`.
A GA tag refuses to be deleted or moved.
A prerelease tag stays prunable, which is what keeps `cleanup-tags.yml` working with no bypass at all.

`update` is there because `non_fast_forward` does not cover every move.
It refuses a push only when the old commit is unreachable from the new one, so re-pointing `v0.13.5` at a descendant is an ordinary fast-forward ref update and goes straight through a ruleset carrying only `deletion` and `non_fast_forward`.
The client's `-f` has nothing to do with it, because the server sees a fast-forward either way.
`update` refuses the push whatever the reachability, and both rules stay, since the most restrictive one applies.

The exclude covers all three prerelease separators `release.yml` recognises (`-alpha`, `-beta`, `-rc`), even though the prune only globs `v*-beta.*` today.
A hand-cut `v0.14.0-rc.1` would otherwise be made permanent by a rule written for GA tags.
The dependency runs the other way too: the exclude and `cleanup_beta_tags.sh`'s `awk` both key on the literal `-beta.`, so changing the beta tag format stops the prune against a rule nobody can bypass.

Both halves of that condition are load-bearing.
Protecting every tag instead would mean a standing bypass for the Actions token, which is far more than one nightly prune needs.
`~ALL` would also cover `refs/tags/__raycast_latest_publish_ext/outl__`, which the Raycast publish flow re-points on every publish.

**The Raycast extension tags this repo too, and `refs/tags/v*` covers it.**
`extensions/raycast/` is published by hand with `bunx @raycast/api publish`, and that tool leaves a `v<extension-version>` tag behind.
`v1.0.12` is one of them, signed on 2026-09-24, sitting on a tap-bump commit, with no GitHub release behind it.
It is not an outl release, and nothing separates its name from a future outl `v1.0.12` by pattern, so it gets the same protection rather than an exception that would expire the day outl reaches 1.0.
Publishing the next extension version creates a new name and is unaffected.
Re-publishing a version that already has a tag is the one flow that now needs the two calls below.

`bypass_actors` is empty, so the rule applies to the org admin as well.
That is the point rather than an oversight.
The account that pushes GA tags by hand is the only account that can lose one, and the reserved name above makes the loss permanent.

**To re-point a GA tag pushed at the wrong commit**, turn the ruleset off and back on around the fix:

```sh
gh api -X PUT repos/outlmd/outl/rulesets/27314778 -f enforcement=disabled
# delete and re-push the tag
gh api -X PUT repos/outlmd/outl/rulesets/27314778 -f enforcement=active
```

A `PUT` carrying only `enforcement` leaves the conditions, the rules and the empty bypass list exactly as they were, so there is nothing to restore afterwards.
That was measured by reading the ruleset back between the two calls, not assumed from the verb.
Read the live state rather than this paragraph: `gh api /repos/outlmd/outl/rulesets/27314778`.

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

**The release body ends with one sponsor link.**
After the gap note, `publish_release` runs "Ask for sponsors in the release notes", which appends a one-line footer pointing at GitHub Sponsors.
It removes any existing copy before appending, so a retried job that adds another gap note below an earlier footer still leaves exactly one footer, at the end.

**Asset names use two architecture spellings, and both are deliberate.**
The CLI / TUI tarballs are `outl-linux-x64` / `outl-linux-arm64` (and `outl-macos-x64` / `outl-macos-arm64`), the desktop bundles are `outl-desktop-linux-x86_64` / `outl-desktop-linux-aarch64`.
The desktop set matches `uname -m` and Linux packaging convention, which is what a `.deb` / `.rpm` / AppImage user expects; the CLI set is older and is what the Homebrew anchors and every published release URL already say.
A release asset URL does not redirect, so renaming either family breaks links that are already in the wild — the spellings stay split, and the docs name both.

### TestFlight (iOS)

`mobile.yml` builds and signs the IPA (`ios-release`) nightly, from the 05:00 UTC cron when its `gate` proceeds (a mobile push run in the last 24h, CI on `main` not red), or on a `workflow_dispatch` with `release_build=true`.
`testflight.yml` runs after `mobile.yml` completes, downloads the IPA artifact, uploads to App Store Connect.
Its `gate` accepts Mobile runs from `schedule`, `push` and `workflow_dispatch`; it left out `schedule` until November 2026, so a nightly IPA never shipped.

A successful Mobile run does not imply an IPA, so `testflight.yml` starts with a Linux `gate` job.
It lists the triggering Mobile run's artifacts and lets `upload` run only when a non-expired `outl-ios-release` exists.
Otherwise `upload` is skipped: the TestFlight run ends green, never enters the `release` environment, and starts no macOS runner.
That is an expected skip, not a shipping failure.
To tell them apart, open the run's `Check for IPA artifact` job: a skip leaves a `::notice::` saying the Mobile run has no current `outl-ios-release` artifact, and the `Upload IPA to TestFlight` job shows as skipped.
A shipping failure is a red `Upload IPA to TestFlight` job.

A manual dispatch of `testflight.yml` bypasses the artifact check.
It always runs `upload` against `mobile_run_id` (or the latest successful Mobile run on `main`), so a run without an IPA fails loudly at the download step instead of skipping.

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

Two nightly crons, an hour apart, neither waiting on the other:

1. **`Release`** (`release.yml`, 06:00 UTC) — auto-bumps `Cargo.toml` locally to `<base>-beta.<run_number>`, cuts a `v<...>-beta.<N>` tag, builds desktop binaries, ships the Homebrew formula (never commits the bump back).
2. **`Mobile`** (`mobile.yml`, 05:00 UTC) — builds the signed IPA from the *unbumped* `Cargo.toml`, uploads it as `outl-ios-release`, triggers `TestFlight`.
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

### npm trusted publishing

`publish_npm` authenticates with OIDC, not a stored token, so there is no `NPM_TOKEN` to rotate and npm attaches provenance on its own.
The trust lives in an entry on npmjs.com under `@outl/plugin-sdk` → Settings → Trusted publishing, and it has to match the job exactly:

| Field | Value | Why |
|---|---|---|
| Publisher | GitHub Actions | |
| Organization or user | `outlmd` | |
| Repository | `outl` | |
| Workflow filename | `release.yml` | The **file name**, not the workflow's `name:` |
| Environment name | `release` | The job declares `environment: release`, so the OIDC token carries that claim; an empty field here does not match it |
| Allowed actions | `npm publish` only | The job runs `npm publish --access public --tag latest`. Nothing in the repo runs `npm dist-tag`, and the `--tag` above is an argument to `publish`, not that command |

Every mismatch fails the same way, as `npm error code ENEEDAUTH` in a job that otherwise looks healthy.
That is also what a missing entry looks like, which is how 13 of 20 nightly releases went red before the entry existed: a 17-second job at the end of a 21-job run, turning the whole release amber every morning until nobody read it any more.

**A new entry starts as `Pending validation` and expires.**
npm gives a deadline (48 hours, when this was last set up) and discards the entry if no publish has validated it by then.
The nightly cron is normally what validates it, which means `release.yml`'s `gate` has to pass first: a non-packaging commit must have landed since the last beta **and** `ci.yml` must be green on `main`.
So a red `main` does not merely skip a beta, it can burn the validation window.
To validate immediately instead, use Actions → Release → "Run workflow" with `dry_run: false`; a dry run does not validate, because `publish_npm` is gated on that input.

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

