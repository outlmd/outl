# RFC 0377 — Crash reporting that is on by default and cannot see your notes

| | |
|---|---|
| **Status** | Draft |
| **Issue** | — (to be filed) |
| **PR** | — |
| **Date** | 2026-10-18 |
| **Reference doc** | [privacy.md](../privacy.md), [UX.md](../../UX.md), [development.md](../development.md), [client-parity.md](../client-parity.md) |
| **Invariant** | root `CLAUDE.md` invariants 9 and 12 |
| **Guarded by** | see **How it cannot regress** |

## Why

outl ships to the App Store, Play, Homebrew, npm and crates.io.
A client that breaks on a user's machine breaks silently: there is no server, no log aggregator, and no session the author can replay.
The only channel today is a user who cares enough to open an issue.

The most recent commit on `main` is the shape of the problem.
`cdac784b` fixes an Android app that closed on open, on devices paging at 16 KB, before running a line of its own code.
Nothing on the device was wrong and nothing in the app was reachable to say so.
That class of failure is invisible by construction, and it is exactly what a crash reporter answers on the day of the release rather than whenever somebody files.

It is also the class this RFC does **not** fully fix, which is stated up front rather than discovered later — see **Declared gaps**.

### What makes this expensive

outl currently promises the opposite, in four places, and three of them are declarations to a store:

| Where | What it says today |
|---|---|
| [`privacy.md`](../privacy.md) | "We do not include crash reporting SDKs (no Sentry, Crashlytics, Bugsnag)" |
| [`app-store-description.md`](../app-store-description.md) (lines 54, 131) | "No analytics, no tracking, no telemetry" / App Privacy = "No, we do not collect data from this app" |
| [`app-store-review-notes.md`](../app-store-review-notes.md) (line 61) | "No analytics, no crash reporting, no telemetry, no advertising" |
| `PrivacyInfo.xcprivacy` | `NSPrivacyCollectedDataTypes = []` |

`privacy.md` already anticipated this change and already chose a word for it:

> If we ever introduce a feature that would change what is described here (for example, **optional** crash reporting), this document will be updated to reflect it before the feature ships.

This RFC departs from that word.
Reporting is **on by default**, with an opt-out, because a reporter nobody enables answers nothing — and the whole point is the user who hits a crash and never files an issue.
That decision is the author's, made knowingly, and the four declarations above move in the same PR.
What it costs is in **Risks accepted**.

## What we chose

### 1. One crate owns what may leave the machine

A new crate, `outl-telemetry`, with a `sentry` feature that is **off by default**.

Not a module in `outl-tauri-shared` (which would drag `tauri` into a concern that has nothing to do with it, and is unreachable from the CLI), and never a copy per client.

The reason is not tidiness.
The dangerous half of this work is the predicate "may this string leave the user's machine", and that predicate needs **one owner and one test suite**, for the same reason `outl_md::unlogged::sidecar_can_vouch_for` and `Filter::Not` do.
Two copies of a safety predicate drift, and the direction a scrubber drifts is always toward sending more.
An over-cautious scrubber drops a stack frame the author can notice missing.
An over-permissive one publishes a page title, and nobody can notice a leak that already happened.

With the feature off the crate compiles to a no-op and no byte of the SDK is linked.

### 2. Four conditions, all necessary, for one event to leave

| # | Condition | What it stops |
|---|---|---|
| 1 | `--features sentry`, off by default | Somebody who clones the repo and runs `cargo build --release` does not link the SDK at all |
| 2 | `OUTL_SENTRY_DSN` read at compile time through `option_env!` | Without the CI secret the constant is `None` and init returns a no-op, even if condition 1 is accidentally satisfied |
| 3 | `[telemetry] enabled`, default `true`, **and** `OUTL_TELEMETRY` not `0` | The user's opt-out, persistent and per-invocation |
| 4 | `CI` / `GITHUB_ACTIONS` unset | An `outl` invocation inside somebody else's pipeline reporting into our project |

Condition 1 is what answers the actual requirement, and it is worth saying why the obvious mechanism does not.
`cfg(debug_assertions)` is **false in any release build**, including a release build made by somebody who cloned the repo, so gating on it would ship the reporter to exactly the person this RFC promises to exclude.
"Release" in the request means *our* release, which is a property of the build pipeline, not of the optimization level.

Condition 2 is compile-time rather than a runtime environment variable on purpose.
A runtime DSN is repointable by anyone running the binary and configurable by anyone reading it.
A DSN is write-only by design and is not a credential, so this is hygiene rather than secrecy — but a constant baked at link time is also the only form that cannot be set by accident in a shell profile.

Conditions 1 and 2 are independent, which is the point: either alone fails closed.
Conditions 3 and 4 are the user's and the environment's; 1 and 2 are the build's.

**Condition 3 has two sources, and both live behind one condition** because they are the same answer arriving by different routes: the persistent flag a client writes, and `OUTL_TELEMETRY=0` for one run.
The persistent half is read by the *caller* and passed to `init`; the per-invocation half is read by `outl_telemetry::silenced_by_env` itself.
That split is deliberate. The config flag needs a loaded `Config`, which only the client has; the variable does not, and putting it in each client would be four chances to forget, breaking in the direction where the user exported it and the app reported anyway.
Only `0` silences, unlike condition 4 where any non-empty value is enough — there a lost report is the cheap way to be wrong, while here `OUTL_TELEMETRY=1` has to keep meaning *on*, or the variable contradicts its own name.

**Condition 4 applies to every client, not only the CLI.**
An earlier draft of this table said "CLI only", on the reasoning that a CLI is the thing that runs in other people's automation. That was the right reason and the wrong scope: the guard lives inside `init`, every client calls `init`, and so every client inherits it. Nothing had to be built for that, and nothing should be built to undo it — a desktop build running inside a pipeline is unusual, not impossible, and it has no more business reporting from there.
Reporting out of somebody else's CI spends our quota on failures we cannot act on and carries paths out of an environment that is not ours to read.

### 3. Allowlist, not scrubbing, for everything but the panic message

Only panics and crashes are sent.
No captured `Result::Err`, no performance transactions, no tracing — those are a separate decision with a much larger surface, and this RFC does not take it.

An event carries exactly: client name, version, OS, architecture, panic location (a file name and line, `mod.rs:42`) and the panic message.
It does not carry, and `outl-telemetry` has no code path that could add: workspace path, page slug, block text, hostname, IP, username, environment variables, or I/O breadcrumbs.
`send_default_pii = false` and `server_name` disabled.

IP address deserves its own line because the client cannot fix it: Sentry attaches the IP at **ingest**, so it is disabled in the Sentry project settings ("Prevent Storing of IP Addresses"), which is configuration living outside this repo and therefore in the checklist below rather than in code.

The panic message is the one free-form field, and it is the one real hazard.
`expect("explicit reason")` is authored by us and safe; `expect(&format!("page {slug}"))` is authored by us and is a page title.
So the message passes a path-and-identifier scrubber before it is sent, and that scrubber is the most heavily tested thing in the crate.

### 4. Where the opt-out lives, and the four questions about its new home

`[telemetry] enabled` in `~/.config/outl/config.toml` (`outl_config`), not in the workspace and not in the op log.

Invariant 7's default position is "model it as an Op", so the departure needs a reason rather than an omission.
The invariant is about state two devices can disagree about and have to reconcile; a crash is not a thing two devices have to agree about, and consent is not a value one device can hold a newer version of.
Every other section of `outl_config` is a rendering or delivery preference, which is why this one is device-local for a different reason than the rest: it is the person's consent, a property of *them* and not of the workspace.

Per invariant 9, the destination's rules:

- **Who writes it** — the GUI Settings surfaces through the existing flat `Settings` shape (`settings.rs`), and a new `outl telemetry` subcommand for the terminal clients. There is **no** `outl config` today (only `outl theme` touches config), so the terminal opt-out has to be built, not assumed — see the table in section 7.
- **Who reads it** — `outl-telemetry` at process start, before a workspace exists. This is what decides the location: a crash during boot happens before there is a workspace to read a flag from, so the workspace cannot hold it.
- **How a test gets its own copy** — `$XDG_CONFIG_HOME`, which `config_dir()` already honours ahead of every other branch.
- **What cleans it up** — nothing, correctly. It is one boolean, not a per-workspace record like the `actors/` bindings that had to grow a GC.

One property of this home is worth stating because it reads like the bug from [RFC 0211](0211-state-that-leaves-a-boundary.md) and is not.
`~/.config/` is replicated by chezmoi, Migration Assistant, Time Machine and NFS, as [`device-store.md`](../device-store.md) records.
For `actor_id` that replication *was* the defect.
For a consent flag it works in our favour: consent is a property of the person, not of the hardware, so a user who turns reporting off on one machine should find it off on the next.

`outl_config` is also shared by the TUI and the desktop, so every client reads one flag rather than each client inventing its own.

### 5. Symbols, or the reporter returns unreadable traces

`Cargo.toml:170` sets `strip = "symbols"` on `[profile.release]`, so a release binary carries no symbol names.
Today an event would arrive as bare addresses.
On iOS it is worse: the `.xcarchive` holds the dSYMs and is never packaged — the release bundle carries only `outl.ipa`, `release-notes.md` and `metadata.env`, and the generated `ExportOptions.plist` sets `uploadSymbols: true` with `destination: export`, which sends them nowhere.

So:

- `[profile.release]` gains `debug = "line-tables-only"` and `split-debuginfo = "packed"`, and **loses** `strip = "symbols"`.
- The strip moves into the release pipeline, after `sentry-cli debug-files upload`.
- The iOS job packages `$ARCHIVE/dSYMs` alongside the IPA so the upload step can reach them.
- The published artifact is **verified stripped** by reading the finished file back, following the precedent of `crates/outl-mobile/scripts/android-apk-page-alignment.py`, which already inspects the built APK and fails the job.

That last point is load-bearing. Moving the strip out of the profile and into the pipeline means a pipeline change can silently start shipping a binary with full debug info. A check that reads the artifact is the only thing that notices.

### 6. What the user is told, where, and who owns the words

Two requirements land here, and one of them is not a UX preference.

Reporting is on by default, so **prior disclosure is what the legitimate-interest footing in Risks accepted rests on**.
A user who first learns of this by reading the source has not been disclosed to.

The second is the framing: a crash report is a contribution, and the sentence should say so without begging.
The user is helping fix a device we do not own and cannot reproduce on.

#### The constraint that shapes it

[`UX.md`](../../UX.md) principle 6 is explicit: *"no onboarding that must be dismissed before the first bullet."*
Principle 2 adds that nothing blocks a keystroke.
So the obvious move — a first-run modal explaining crash reporting — is barred by the product's own rules, and rightly: a notes app that greets you with a consent dialog has already failed the person who opened it to write something down.

The disclosure is therefore **three surfaces, none of them blocking**:

1. **A dismissible strip on first run.** Not modal, does not take focus, does not swallow a keystroke, does not reappear once dismissed. The first bullet is typeable with it on screen.
2. **A permanent Settings section**, with the toggle and the full account of what is and is not sent.
3. **The published privacy policy**, which the stores require to be reachable before download — so disclosure precedes the install, not just the first crash.

#### Reporting is live from the first boot, and that is a choice

An alternative was to hold every event until the strip had been shown once (`telemetry.notice_seen`).
Rejected: it blinds the reporter to first-boot crashes, which is the single most valuable class and the one that motivated this RFC — the Android library-load failure killed the app in `onCreate`, before any strip could render.
A gate that excuses itself exactly where the bug lives is not a safeguard.
Prior disclosure is carried by surface 3, which the store guarantees is readable before the binary runs at all.

#### One owner for the words

Per `UX.md` → Voice ("one condition, one sentence, one owner") the copy has one owner per medium, not one owner total — because the repo already decided this and putting GUI copy in Rust would fight the decision.
The GUI strings live in `@outl/shared/onboarding` beside `STORAGE_STEP` and `SYNC_STEP`, which is where shared onboarding copy already lives and is already consumed by both clients.
The terminal strings (`outl telemetry status`) live in Rust, because that is the only place that can reach them.
What must not diverge is the *promise*, not the wording, so a test pins that both mediums still say notes are never sent — mirroring `onboarding.test.ts`, which already asserts on the promises its copy makes rather than on its exact phrasing.
A client does not phrase this itself. There are three bad copies of this fact waiting to happen, and the store declarations make one of them a false statement to Apple.

The strip:

> When outl crashes, it sends the crash to us, which is how it gets fixed.
> Your notes are never part of it. You can turn this off in Settings.

Settings:

> **Crash reports**
> When outl crashes, it sends what broke: the error, where in the code it happened, your OS and the app version.
> That is how a crash on a device we do not have gets fixed.
> Your notes are never included — no page titles, no block text, no file paths, and no account, because there isn't one.

Both pass the existing nudge tests: over 20 characters, no `DEV_WORDS`, and they name what the user can do rather than apologising.

### 7. Where the toggle lives on each client

Reporting is on by default, so a client that cannot turn it off is a client whose user has no opt-out.
That is not a missing convenience, it is what the legitimate-interest footing in Risks accepted depends on.
So every surface gets a verdict here, and `outl_shortcuts::capability_support` holds it as an exhaustive `match` — a new client does not compile until it declares one.

Verified against the tree as it stands today:

`Missing`, not `Native`, for the TUI — and the distinction is the point of the `Support` enum.
`Native` means *reachable with no handler and no nudge, because the platform does it* (the canonical case is `Backspace` on an empty textarea).
A CLI command is not that: the surface genuinely is absent from the TUI, and the user needs the sentence to learn where the switch lives.
Calling it `Native` would hide a real gap behind a state that means "nothing to see here".

| Surface | Settings UI today | Where the toggle goes | Work |
|---|---|---|---|
| **desktop** | `SettingsModal.tsx` — a Privacy section now holds the toggle and renders `CRASH_REPORTS_SETTING` | Built | Done |
| **mobile** | `SettingsSheet.tsx` — same section, same shared copy, same `Support` row | Built | Done |
| **TUI** | None. It has no settings screen at all | `outl telemetry off`, plus the config file | No UI; declares `Support::Missing` with a nudge naming the command |
| **CLI** | N/A | `outl telemetry` (`status` / `on` / `off`) and `OUTL_TELEMETRY=0` for one invocation | New subcommand |
| **MCP** | N/A | Nothing. It is a program's surface, not a person's, and it reports nothing of its own | Declared gap |

The same table for the *other* half, disclosure: desktop and mobile mount the shared strip (`Full`); the TUI says nothing unprompted (`Missing`, with a nudge naming `outl telemetry status` and the published policy); the CLI and MCP have no column of their own in the catalog, and the CLI inherits the terminal's gap.

Two consequences worth naming before implementation:

**A new subcommand, not a generic `config set`.**
`outl telemetry` is three verbs over one boolean.
A generic `outl config set <key> <value>` would need a schema story for every key in `Config`, which is a much larger change arriving as a side effect of this one — the mistake invariant 11 is about.

**`OUTL_TELEMETRY=0` is separate from the CI guard in condition 4.**
The guard is automatic and covers somebody else's pipeline; the env var is a person deliberately silencing one run.
They overlap, and neither replaces the other: a user on their own machine should not have to pretend to be CI to opt out of a single invocation.

### 8. Store declarations move with the change

**Two of these four are not in the repo.**
`docs/app-store-*` is matched by `.gitignore:78` and has never been tracked, so the description and the review notes are local working documents, not PR artifacts.
The declaration that actually binds lives in App Store Connect, on the web, and no test or review in this repo can hold it to the truth.
They are edited on disk as a staging area for the submission, and the checklist at the end of this RFC is the only mechanism that carries them.

The two that *are* versioned, and therefore reviewable, are `PrivacyInfo.xcprivacy` and `privacy.md`.

Not follow-ups. An app whose binary reports crashes and whose App Privacy answer says "no data collected" is a false declaration to Apple.

- `privacy.md` — rewrite the "does NOT" section: that crashes are sent to Sentry, the exact field list, that notes are never part of it, how to turn it off, and retention. The page currently names Sentry as something we do not use; it becomes the page that says we do, and under what limits.
- `app-store-description.md` — drop "no telemetry" (line 54); App Privacy becomes **Crash Data** and **Other Diagnostic Data**, both *not linked to the user*, purpose *App Functionality* (line 131).
- `app-store-review-notes.md` — rewrite line 61.
- `PrivacyInfo.xcprivacy` — which data types, and whether any API reason joins the four already there, is settled below in **Settled: two data types, and no new API reason**. `NSPrivacyTracking` stays `false` either way: this is not cross-app tracking.
- Play Data Safety — a form, not a file. It goes in the release checklist in [`android-platform.md`](../android-platform.md).

### 9. Declared gaps (invariant 12)

The `sentry` Rust crate catches **Rust panics**.
It does not catch an Objective-C exception, a Java or Kotlin exception, or a `SIGSEGV` raised outside Rust.
Full mobile coverage would need `sentry-cocoa` and `sentry-android` on top of this work, and this RFC does not include them.

Most of outl's risk is in Rust (the CRDT, markdown parsing, storage), so this covers the bulk.
But `MainActivity.kt` and the ObjC bridge are outside it — which means the Android library-load failure that motivated this RFC is *itself* in the uncovered set. That is not a reason to skip the work; it is a reason to not claim the work covers it.

The opt-out toggle is a per-client UI surface, so it enters `outl_shortcuts::capability_support` as its own `Capability`.
That forces each client to declare whether it can turn reporting off, rather than leaving it to be discovered.
This matters more than a usual parity row: reporting is on by default, so a client with no toggle is a client with **no opt-out**, which is a legal problem and not a missing convenience. The exhaustive `match` is what makes that impossible to ship unnoticed.

**Two rows, not one, and the second is a real gap.**
`CrashReportingOptOut` asks whether the user *can* say no. `CrashReportingDisclosure` asks whether they were ever told there was something to say no to. Section 6 makes disclosure load-bearing, not decorative, so a client can satisfy one row and fail the other and the catalog has to be able to say which.

Both GUI clients are `Full` on both: the shared `CRASH_REPORTS_NOTICE` strip is mounted at their shell, dismissible, once.
**The terminal is `Missing` on disclosure**, and that is a declared gap rather than an oversight. `outl telemetry status` answers truthfully and answers only when asked, and a pull is not a disclosure; `docs/privacy.md` carries the full field list and is not in the product.

It is left open because the obvious fix is not obviously right. A terminal's first-run output is a contract with whatever is reading the pipe, and printing a notice into somebody's script is its own defect — the kind of fix that trades a disclosure gap for a corrupted `jq` pipeline. Closing it properly means deciding where a CLI may speak unasked (stderr only? once, keyed on a state file? suppressed when stdout is not a tty?), and that is a product decision with its own blast radius, not a line to add on the way past.
Recorded here so it is a decision somebody made, not a thing nobody noticed.

## How it cannot regress

Verified against the tree: every name below is a test that exists and runs.
An earlier draft of this table named six tests that were never written, which is worse than no table — it promises a gate and delivers a sentence.
What still has no test is listed separately, under its own heading, where it cannot be mistaken for coverage.

**The gate.** Deleting or relaxing any of these is how a build of outl starts reporting from a machine whose owner did not agree to it.

| Test | Where | What it pins |
|---|---|---|
| `the_sentry_feature_is_off_by_default` | `outl-telemetry/src/lib.rs` | Condition 1, for **this crate's** manifest. Read from the file rather than through `cfg`, because a `cfg`-based test passes exactly when the mistake has been made |
| `no_client_enables_crash_reporting_by_default` | `outl-telemetry/tests/client_manifests.rs` | Condition 1 where it can actually fail: the four client manifests. This crate being clean is not enough — a client that puts `sentry` in its own `default` links the reporter into every build on every machine |
| `no_client_pins_the_sentry_feature_on_the_dependency` | same | The quieter way to defeat condition 1: leave `default` empty and pin `features = ["sentry"]` on the dependency, so the client's own feature looks right while the SDK is linked unconditionally |
| `init_without_a_dsn_is_a_no_op` | `outl-telemetry/src/lib.rs` | Condition 2. Fails if a missing DSN starts meaning "use some fallback" — which is what `sentry::init` does on its own, since `apply_defaults` reads `SENTRY_DSN` from the environment |
| `init_respects_the_config_opt_out` | same | Condition 3, persistent half |
| `each_condition_closes_the_gate_alone` | same | That no single condition can be relaxed without the gate opening |
| `the_environment_closes_the_gate` | `outl-telemetry/tests/ci_env.rs` | Conditions 3 and 4 against the real process environment, including the two baseline phases with no variable set — without those, the rest would pass against a function that always returns `true` |

**The payload.**

| Test | Where | What it pins |
|---|---|---|
| `no_absolute_path_survives_the_scrubber`, `no_windows_path_survives_the_scrubber`, `no_page_slug_survives_the_scrubber` | `outl-telemetry/src/redact.rs` | The scrubber, as proptests over generated input rather than the strings we thought of. They assert an **exact** output, not `!contains(segment)`: a one-character segment is a substring of `<file>` |
| `a_relative_source_path_survives_the_scrubber` | same | The other direction. A scrubber that replaced everything would pass every test above |
| `an_event_carries_only_allowlisted_fields` | `outl-telemetry/src/scrub.rs` | The allowlist, by asserting the serialized event's key set against `ALLOWED_EVENT_KEYS` |
| `no_populated_field_leaves_a_path_behind` | same | The key set alone would pass if a *surviving* field still held the leak, so the payload is searched for it too |
| `the_scrubber_writes_the_os_and_the_arch` | same | Section 3's OS and architecture, which have no other producer: `sentry-contexts` is not linked |
| `an_incoming_os_context_is_replaced_not_merged` | same | What makes that write safe. The fixture arrives with a path in `os.name`, shaped like what `sentry-contexts` would produce, and none of it may survive |
| `the_panic_itself_survives_the_scrubber`, `a_debug_image_keeps_its_id_and_loses_its_path` | same | The over-scrubbing direction. An empty event is not a safe event, it is a useless one |
| `the_fixture_really_populates_everything` | same | Guards the guard: if the fixture stops populating a field, the key-set test starts passing for the wrong reason |

**The surfaces.**

| Test | Where | What it pins |
|---|---|---|
| `"crash-reporting settings copy"` (3 tests) | `outl-frontend-shared/src/onboarding/onboarding.test.ts` | Section 6. The *promises* — what is sent, that it helps, that notes never are — plus the nudge rules, in the one place the copy lives. Pins the claim, not the sentence, so a copy edit is free and dropping a promise is not |
| `"crash-reporting first-run notice"` (5 tests) | same file | The strip's copy: that a crash is sent and that sending it is how it gets fixed, that notes are never part of it, that it names where the opt-out lives (the strip is not one), the nudge rules, and that it stays **shorter** than the Settings section it points at |
| `"the crash-reporting strip"` (7 tests) | `outl-frontend-shared/src/polish/CrashReportsNotice.test.tsx` | `UX.md` principles 2 and 6, which is the half most likely to be broken by a well-meaning change: it does not take focus, does not stop the first bullet taking a keystroke, announces with `role="status"` rather than `alert` or `dialog`, stays dismissed across launches, and still shows for someone who already dismissed the 30-second tour (separate keys, because dismissing the tour is not dismissing this) |
| `"crash-reporting disclosure"` (3 tests) | `outl-frontend-shared/src/polish/parity.test.ts` | That both GUI clients mount the shared strip at their shell and read the shared Settings copy, and — the half that actually bites — that **no client source spells out what a report carries**. A client writing its own sentence is a second copy of a claim that also goes to Apple and Google |
| `telemetry_off_survives_a_restart` | `outl-config/tests/telemetry_persistence.rs` | That the GUI's `load → mutate → save` path reaches the file a fresh process reads. Goes through `save()` / `load_result()` with no path argument, because path resolution is half of what can break and a test with an explicit path would pass even if `config_path()` pointed where no client looks |
| `off_persists_to_the_config_file_a_second_process_reads` | `outl-cli/tests/cli_telemetry.rs` | The same property through the CLI, with two real processes. The pair is deliberate: this one covers the command, the one above covers the command bodies the GUIs call, and neither exercises the other's path |
| 6 tests from `"shows the shared copy…"` to `"is a different row from the toolbar lock"` | `outl-mobile/src/components/SettingsSheet.test.tsx` | Section 7 on mobile: the shared copy, the default read as on rather than assumed, the switch moving only after the write lands, and not moving when it fails |
| `mobile_capability_column_is_backed_by_real_commands` | `outl-mobile/src-tauri/src/capability_parity.rs` | That mobile's `Full` verdict is backed by a registered command |
| `the_parity_doc_matches_the_code` | `outl-shortcuts` + `shortcuts.support.test.ts` | The new `Capability` row, via the exhaustive `match` |
| 5 tests | `outl-cli/tests/cli_telemetry.rs` | `outl telemetry status` / `on` / `off`, including that `status` exits non-zero rather than reporting `on` over a config it could not read |

### What has no test yet

Named here rather than in the table above, because a row in that table is a claim that something cannot regress.

- **The shipped artifact is checked by a script, not by a test named in this table.** `scripts/check-artifact-stripped.py` reads the built ELF / Mach-O / PE (including fat binaries, and inside `.tar.gz` / `.zip` / `.apk` / `.ipa`) and fails the job on leftover DWARF, on a local symbol table, or on the name of a CI runner variable found in the binary (`GITHUB_RUN_ID`, `RUNNER_TEMP`, `ACTIONS_RUNTIME`, `BLACKSMITH_`) — what a dependency's build script leaves behind when it snapshots `std::env::vars()` into the crate it compiles. It runs in the release workflow, after the symbol upload. There is no `cargo test` that covers it, so a change to the workflow's step order is caught by the job going red, not by the suite. The script's own verdicts are pinned by `scripts/tests/test_check_artifact_stripped.py`, which the Hygiene workflow runs on every PR.
- **The macOS archive's strip and the Android AGP strip are the two unverified links.** Whether `STRIP_INSTALLED_PRODUCT` and AGP's strip leave local symbols behind could not be confirmed outside CI, so the first release run is where that is learned. Escape hatches are in place (`--allow-symbols` for the strip rules, `--allow-build-env` for the runner-variable rule, neither passed by any step today); Android is best-effort and does not block the release, and the IPA check runs after the artifact upload so a failure still leaves the IPA and dSYMs downloadable.
- **The mobile strip's safe-area offset was calibrated by reading geometry, not by looking at a device.** `--outl-notice-offset` is `calc(max(env(safe-area-inset-bottom), 12px) + 60px)`, derived from the `BottomBar`'s own values. Worth one visual pass before release, particularly with the iOS keyboard open, where the webview resizes.
- **The desktop's Privacy section has no UI test.** There is no `SettingsModal.test.tsx` at all, and rendering one needs mocks for five modules, which would pin the mocks. Mobile's six tests cover the shared behaviour; the desktop's wiring is covered only by `tsc` and by review.
- **No cross-client test asserts that every GUI client can turn reporting off.** The `Capability` row records the verdict and the generated doc pins the table, but a client could ship `Full` with a section nobody wired. Mobile's `capability_parity.rs` closes that for mobile only.

## Risks accepted

**Opt-out is weaker ground than opt-in in the EU.**
For non-identifying diagnostic data, opt-out rests on legitimate interest rather than consent.
That is defensible when the data identifies nobody, the opt-out is easy and visible, and disclosure comes first — and it is widely practiced — but it is more fragile than opt-in, and a regulator could disagree.
Section 6 is what discharges the disclosure half of that test, which is why the first-run strip is a requirement of this RFC and not a nicety.
The mitigation is to send no persistent identifier at all: no device id, no actor id, no IP, nothing that joins two events into one person.

**The privacy promise stops being verifiable by reading one sentence.**
Today a user reads "nothing is sent" and is done.
After this, the claim is conditional ("this is sent, unless"), and a conditional claim is harder to audit and easier to break without anyone noticing.
That is the real cost of this RFC, and the test table above is the entire answer to it.

**Four build jobs gain reach they do not have today.**
`build_desktop*` and `build_android` consume no `secrets.*` at all right now, only `github.token`.
Reaching the DSN means either adding `environment: release` to them (which also widens access to the Apple signing secrets living there) or using a repository secret.
This RFC chooses the **repository secret**: a DSN is write-only and not a signing credential, so it does not justify broadening who can read certificates.

**A new CLI subcommand is scope this RFC adds to itself.**
`outl telemetry` exists only because the CLI is in scope, and the CLI is in scope by the author's decision against the recommendation here.
It is small (three verbs, one boolean) but it is surface that has to be documented in `cli.md`, carried in the parity table, and supported forever.

**Taking `strip` out of `[profile.release]` reaches every build, not just the pipeline's.**
Section 5 removes `strip = "symbols"` from the release profile, correctly: a stripped binary reports bare addresses, which is the same as not reporting. The strip moves into the pipeline, after `sentry-cli` has read the symbols.
That is invariant 9 in miniature. The strip used to be a property of *the profile*, so everyone who built with it got it without asking; now it is a property of *a job*, and everyone who is not that job lost it. Measured on this machine: 71.1 MB unstripped against 58.9 MB stripped, so 12.2 MB, 17%.
Three consumers were standing on the old rule.
`Dockerfile` is the one that mattered and is fixed here: it builds `-p outl-cli` without `--features sentry`, so it has no reporter and no use for symbols, and `docker.yml` pushes that image on every non-PR push. It now strips after `install`, with the reason written next to the line.
`cargo install outl-cli` is the one left paying: a user installing from crates.io gets a binary 12 MB larger with no reporter in it to justify the symbols. Accepted rather than fixed, because the alternatives are worse in ways that are not worth it here — a separate `[profile.release-reporting]` changes `target/<profile>/` and every artifact path in two workflows, and `RUSTFLAGS="-C strip=none"` cannot be used on the Android job, where it would replace the `[target.*] rustflags` carrying `max-page-size=16384` (the beta.226 crash).
A local `cargo build --release` pays the same 12 MB, which costs a developer nothing.

**`outl-tui` can report and nothing ships it.**
The standalone `outl-tui` binary calls `init` and carries the `sentry` feature, and no release job builds it, so it gets no DSN and no uploaded symbols. The code is inert in distribution rather than wrong: a TUI session reached the ordinary way is the `outl` binary, which does report. It is listed here so nobody reads `outl-tui/src/main.rs` and concludes the standalone binary reports today.

**One promise in `privacy.md` is not enforced by any code in this repo.**
"Your IP address is never sent" is true of the SDK — `send_default_pii = false`, and the scrubber clears `event.user` and `event.request` regardless — but an IP does not have to be *sent* to be *recorded*: Sentry's ingest sees the source address of the request itself, and whether it stores it is a project setting ("Prevent Storing of IP Addresses"), not a payload field.
So that line is the one claim on the page whose truth lives in a dashboard, and no test here can fail if somebody turns it off.
It is in the checklist below, and it is the reason the checklist says `outl.app/privacy` is republished **after** those settings are in place and not before — publishing the page first is making a promise we have not yet arranged to keep.
Every other claim on that page is pinned by a test.

**Two mechanisms with no precedent here.**
There is no `option_env!` anywhere in the workspace and no `--features` in any workflow.
This adds both. The existing build-time injection pattern (`sed` rewriting `Cargo.toml`, `--config` JSON for Tauri) reaches the Tauri config and the version, not Rust constants, so neither existing mechanism can carry a DSN.

## Rejected alternatives

**`cfg(debug_assertions)` as the gate.** False in any release build, including a third party's. It ships the reporter to the person the requirement excludes.

**Opt-in (what `privacy.md` promised).** Keeps every declaration as it stands and needs no store re-submission. Rejected because the enable rate on a tool like this is small, and the user who crashes and never files an issue is the exact person the feature exists for.

**Ask after a crash, on next boot.** Per-event consent, nothing leaves without a click, and it keeps App Privacy at "no data collected" until a user says yes. Rejected for this round as materially more work (local queue, review UI) for a narrower answer, and worth revisiting if the legitimate-interest footing proves uncomfortable.

**A module in `outl-tauri-shared`.** Drags `tauri` into the crate and is unreachable from the CLI, which this RFC covers.

**Scrubbing free-form error strings instead of an allowlist.** A denylist over text we did not author is a guess, and the failure direction is a published note.

## Settled: two data types, and no new API reason

`PrivacyInfo.xcprivacy` declares `NSPrivacyCollectedDataTypeCrashData` and `NSPrivacyCollectedDataTypeOtherDiagnosticData`, both *not linked* and *not tracking*, purpose *App Functionality*.
`NSPrivacyAccessedAPITypes` does not change.
`NSPrivacyTracking` stays `false`.

Sentry's own privacy-manifest page asks for a third data type, `NSPrivacyCollectedDataTypePerformanceData`, and for `NSPrivacyAccessedAPICategoryFileTimestamp` with reason `C617.1`.
That page is written for `sentry-cocoa`, gives no rationale for any single entry, and does not describe the crate this RFC links:

```toml
sentry = { version = "0.49.3", default-features = false, features = [
    "panic", "backtrace", "reqwest", "rustls", "debug-images",
] }
```

`contexts`, `logs`, `metrics`, `release-health` and `tracing` are all off, and each is `cfg`-gated rather than disabled at runtime, so the code that would feed those declarations is never compiled.
That is what makes the Cocoa document inapplicable rather than merely cautious: the surfaces it declares for are absent from this binary.

### What one event carries

Read off `Client::prepare_event` and the three linked integrations, after the `before_send` in section 3 zeroes `server_name`, `user`, `request`, `extra` and `breadcrumbs`:

| Field | Source | Note |
|---|---|---|
| `event_id` | `random_uuid()`, per event | Fresh every time, so it joins nothing to anything |
| `sdk` | client | The reporting SDK's own name and version |
| `release`, `environment`, `platform` | `ClientOptions` | App version; `platform` is the literal `"native"` |
| `level`, `timestamp` | `sentry-panic` | `Fatal` |
| `exception[0]` | `sentry-panic` | `ty: "panic"`, `mechanism`, `value` = the scrubbed message |
| `exception[0].stacktrace` | `sentry-backtrace` | Frames, each with a **basename** and a line (`mod.rs:42`). Not a repo-relative path: `sentry-backtrace`'s `process.rs` takes the basename for `filename` and puts the build path in `abs_path`, which the scrubber redacts. The readable location is reconstructed server-side from the uploaded debug files |
| `debug_meta.images` | `sentry-debug-images` | Every loaded shared library: path, debug UUID, code id, load address, size |

Nothing populates `OsContext`, `DeviceContext` or `RuntimeContext`.
Those are written only by `sentry-contexts`, which is not linked, and `hostname::get()` lives in that same crate — so `server_name` is empty at the source rather than scrubbed after the fact.
The OS and architecture named in section 3 are therefore fields `outl-telemetry` sets itself, from `std::env::consts`, and belong to the allowlist rather than to the SDK's output.

### `CrashData` — declared

Apple's category is crash logs from the app, and a `Fatal` event carrying an exception and a backtrace is one.

### `PerformanceData` — refused

Apple defines it as launch time, hang rate or energy use.
Every mechanism that could produce one of those is a feature this RFC leaves off: `tracing` and `metrics` for transactions and measurements, `release-health` for the sessions a crash-free or hang rate is computed from.
The Rust crate ships no app-start instrumentation and no ANR watchdog in the first place.

So the row would describe collection that does not happen, and a privacy card is read by users and not only by reviewers.
This is where Sentry's page and this RFC actually diverge: `sentry-cocoa` does ship app-start tracking, slow and frozen frame detection and an ANR watchdog, which genuinely are launch time and hang rate.
Declaring on its behalf would import a Cocoa SDK's behaviour into the manifest of a binary that does not contain it.

### `OtherDiagnosticData` — declared

The catch-all is any other data collected to measure technical diagnostics related to the app, and `debug_meta.images` is what lands in it.

The case against nearly won, so it is recorded rather than dropped.
Apple's own `.ips` crash report carries a **Binary Images** section, for exactly the reason `debug-images` is enabled here, and `DebugImagesIntegration::process_event` attaches the list only to an event — which, in this configuration, only a panic produces.
On that reading the images are part of the crash log and `CrashData` already covers them.

What decided it is that the list does not describe the crash.
It is an inventory of every shared library resident in the process, system frameworks included, each with a UUID identifying its exact build.
That describes the device's software, which "crash logs from your app" does not plainly reach, and section 5 makes it deliberate rather than incidental: `debug-images` is on precisely so the frames resolve.

The refusal above and this inclusion are not in tension, and the asymmetry between them is the whole argument:

> `PerformanceData` would be a **false** row, because nothing in that category is collected at all.
> `OtherDiagnosticData` is a **bucket choice** over collection that demonstrably happens.

Under-declaring and over-declaring are both unacceptable, but only the first is a question about *whether* data leaves the device.
The second is a question about which of two categories describes data that leaves either way, and there the broader bucket is the honest answer: the user is told something true under both readings instead of something true under only one.

The precedent agrees without settling it.
Firebase Crashlytics' shipping manifest declares `CrashData` and `OtherDiagnosticData` and no `PerformanceData`, which is where this RFC lands.
It is not proof, because Crashlytics also sends free RAM, disk, orientation and custom keys, any of which earns the second row on its own.

### No API reason is added

The four existing entries stay as they are, because the crate reaches none of those APIs:

| Category | Sentry's page | This crate |
|---|---|---|
| `FileTimestamp` | asks for `C617.1` | `debug_images()` reads the dynamic loader's in-memory image list through `findshlibs` and never stats a file; `env::current_exe()` is a name fallback, not metadata. The only `std::fs::read` in the dependency tree is `Envelope::from_path`, a public constructor that a `minidump`-free, cache-free configuration never calls |
| `UserDefaults` | asks for `CA92.1` | No Foundation anywhere in the crate. Already declared here for unrelated reasons |
| `SystemBootTime` | asks for `35F9.1` | `DeviceContext::boot_time` is a `sentry-types` struct field with no writer outside `sentry-contexts` |
| `DiskSpace` | — | Same shape: `free_storage` is `sentry-contexts`-only |

Declaring an access that does not occur is the same class of error as omitting one that does, so `C617.1` is not added.

**One adjacent finding, deliberately left alone.**
The existing `FileTimestamp` entry declares `0A2A.1`, the reason reserved for a third-party SDK that wraps the timestamp API on a host app's behalf.
outl-mobile is an app, and the reason for an app reading metadata of files in its own container — which outl does, for every `.md` and `.outl` sidecar — is `C617.1`.
That reads like a pre-existing mis-declaration. It predates this RFC, and per invariant 11 a cost found while investigating a change is not evidence about that change, so it needs its own issue.
It is named here so the next reader does not arrive at `C617.1` by way of Sentry's page and conclude this RFC was wrong to leave it out.

## Checklist outside this repo

- Two Sentry projects (desktop, mobile) plus one for the CLI, under the `outl` org
- "Prevent Storing of IP Addresses" enabled on each
- Data retention set to the shortest the plan allows
- `SENTRY_DSN_*` and `SENTRY_AUTH_TOKEN` added as repository secrets
- App Store Connect → App Privacy updated to declare Crash Data **and** Other Diagnostic Data before the next submission
- Play Console → Data Safety updated likewise
- `outl.app/privacy` republished from `docs/privacy.md` **after** the two settings above are in place, and **before** the release that ships this — the page claims the IP is not stored, which only the project setting makes true
