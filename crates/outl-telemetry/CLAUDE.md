# CLAUDE.md — `outl-telemetry`

Crash reporting.
Spec: [RFC 0377](../../docs/rfcs/0377-crash-reporting.md).

## What this crate owns

Two decisions, and nothing else:

1. **Whether anything is sent at all** — `init`, `suppressed_by_ci`, and the private `report_target` gate.
2. **What a sent event may contain** — `redact` (one string) and `scrub_event` (one event).

It owns those two so they have **one owner and one test suite**, for the same reason `outl_md::unlogged::sidecar_can_vouch_for` and `Filter::Not` do.
Two copies of a safety predicate drift, and the direction a scrubber drifts is always toward sending more.

It is deliberately **not** a module in `outl-tauri-shared`: that would drag `tauri` into a concern unrelated to it, and would be unreachable from the CLI.

## What this crate does NOT own

- **The persistent opt-out value.**
  `[telemetry] enabled` lives in `outl_config`, and the *caller* reads it and passes it as `init`'s `enabled` argument.
  This crate never reads a config file, because a crash during boot happens before there is a workspace or a loaded config to consult.
  The caller asks **`outl_config::telemetry_enabled()`** (or `Loaded::telemetry_enabled()` when it already loaded), never `load().telemetry.enabled`.
  The difference is the whole finding: `load` hands back `Config::default()` for a file it could not parse, `TelemetryCfg::default()` is `enabled = true`, and a user who had run `outl telemetry off` would start reporting again the moment that file picked up a stray bracket or arrived half-written from another device.
  Four binaries and the GUI command all go through the one function so they cannot disagree about an unreadable file.
  The *per-invocation* override is the exception and does live here (`silenced_by_env`, reading `OUTL_TELEMETRY=0`): `docs/privacy.md` does not restrict that variable to the CLI, and four clients applying it themselves is four chances to forget.
- **Which clients can turn reporting off.**
  That is a `Capability` row in `outl_shortcuts::capability_support` (invariant 12).
- **The user-facing copy.**
  GUI strings live in `@outl/shared/onboarding`; terminal strings live in the CLI.
  A client never phrases "what we send" itself.
- **Uploading symbols, or stripping the artifact.**
  That is release CI.
  See the `[profile.release]` comment in the root `Cargo.toml`.

## The four conditions (RFC §2)

| # | Condition | Enforced by |
|---|---|---|
| 1 | `--features sentry`, **off by default** | the compiler: with the feature off, `start` is a `fn` returning `None` and no SDK is linked |
| 2 | `OUTL_SENTRY_DSN` through `option_env!` | the private `DSN` constant |
| 3 | the user has not opted out | `init`'s `enabled` argument (the caller asks `outl_config::telemetry_enabled`, which fails closed on a config it could not read) **and** `silenced_by_env` (`OUTL_TELEMETRY=0`) |
| 4 | `CI` / `GITHUB_ACTIONS` unset | `suppressed_by_ci` |

Condition 3 has two sources because the user has two ways to say no: the persistent config flag a client writes, and the variable that silences one run.
Both sit behind one condition because they are the same answer arriving by different routes.
Only `0` silences, unlike condition 4 where any non-empty value is enough — there a lost report is the cheap way to be wrong, here `OUTL_TELEMETRY=1` has to keep meaning *on*.

Conditions 1 and 2 are independent on purpose: either alone fails closed.

### Never pass `dsn: None` to `sentry::init` and call it a gate

`sentry::init` runs `apply_defaults` internally, which reads `SENTRY_DSN`, `SENTRY_RELEASE`, `SENTRY_ENVIRONMENT`, `HTTP_PROXY`, `HTTPS_PROXY` and `SSL_VERIFY` from the environment **unconditionally**.
With `dsn: None` it adopts `SENTRY_DSN` from the env, so anybody could repoint the binary at their own project with a shell variable.

Only the early `return None` closes that.
`init` therefore resolves the gate **before** the SDK is touched, and `start` is never reached without a DSN.

## Reading the scrubber

`scrub_event` runs as `before_send`, which `sentry-core` invokes **last**.
It runs after every integration's `process_event`, and after `release` / `environment` / `server_name` have been filled in from the client options (`sentry-core/src/client/mod.rs`, `prepare_event`).
So it is the final word on the payload.
Nothing downstream can add a field back.

It is an **allowlist**: a field is cleared unless RFC §3 authorizes it, and `ALLOWED_EVENT_KEYS` names the survivors.

Three things that look redundant and are not:

- **`event.server_name = None`.**
  The `contexts` integration is what fills it from the machine's hostname, and that feature is off.
  Clearing it anyway is what keeps the promise if somebody turns the feature on later.
- **`event.tags` / `extra` / `modules` cleared.**
  outl writes none of them today.
  Same reason.
- **`debug_meta` image paths redacted.**
  `SymbolicDebugImage::name` is the absolute path to the loaded binary, so on a `cargo install` or Homebrew machine it reads `/Users/<username>/.cargo/bin/outl` — a username, from a feature we deliberately enable.
  The debug **id** is what Sentry matches the uploaded symbols against, and that is kept, so redacting the path costs nothing.

The `match` in `scrub_debug_meta` is exhaustive on purpose: a new `DebugImage` variant should fail to compile until somebody decides whether it carries a path.

### The one field the scrubber writes

`contexts` is the one field the scrubber **writes** rather than clears, and the order is load-bearing.
RFC §3 promises the OS and CPU architecture, and three user-facing texts repeat that promise (`CHANGELOG.md`, the RFC, and the GUI copy in `@outl/shared/onboarding`), so the event needs a producer for them — the `contexts` integration that would normally fill them is deliberately not linked.
They come from `std::env::consts::OS` / `ARCH`, which are **compile-time constants**: reading them touches nothing on the running machine, so none of the hostname, kernel version or device model that integration would have collected alongside them can arrive.
The insert happens *after* `contexts.clear()`, which is what keeps this an allowlist by construction: enable that integration tomorrow and everything it gathered still dies in the clear, while those two fields are still only what the scrubber put there.
An allowlist of keys *inside* `contexts` would have the opposite property — it would trust what arrived.


## Why `redact` keeps relative paths

`crates/outl-core/src/tree/mod.rs:42` is the panic location.
It is fixed at compile time and describes **our** source tree, not the user's disk, and it is the entire reason a report is actionable.
An absolute path describes the user's disk, and goes.

That distinction is the one thing `redact`'s pattern must get right, and it is why the POSIX rule carries a boundary capture group.
`regex` has no lookbehind, so without it the pattern would match the `/outl-core/src/tree/mod.rs` inside a relative path and eat the location.

Six rules, and three of them exist because a review broke the first three.
What they close, with the input that proved each one:

- **Rule 0, `std` is also an author.** `core`'s slicing panics quote the string they failed on — measured on the pinned toolchain, `end byte index 16 is not a char boundary; it is inside 'ã' (bytes 15..17) of \`<the note>\``.
  `outl-md` slices block text by byte offset in `cursor.rs` and `wikilink.rs`, so an off-by-one on an accented line published the note.
  The rule cuts at the quote and keeps the diagnosis.
  This file used to assume we were the only author of a panic message.
- **Rule 3, a path does not end at a space.** Rules 1 and 2 stop at whitespace, and what follows is the rest of a home directory: `C:\Users\Joao Silva\AppData\…` left `Silva\AppData\Local\…`, and `~/Library/Application Support/outl/actors/<ULID>.json` left the actor id.
  The old note here called this acceptable "because the leaf carries the page title" — true, and beside the point, since the *middle* carries the username.
- **Rule 5, the actor id.** The one identifier in outl that is stable per device, which is what the promise means by "nothing that ties two reports together".
- **Rule 1 covers UNC** (`\\server\share\…`). Neither other path rule can match it, and `transport = "file"` over a shared filesystem is documented, so a crash from that setup published the file server's hostname.
- **Rule 4 takes the whole relative path**, not just the leaf: an outl slug keeps `/` for hierarchy, so `pages/buser/tech/x.md` → `pages/buser/tech/<file>` left the page's namespace standing.

Still open, stated so it is not rediscovered from a report:

- A path component with a space, followed by no separator, survives as an isolated word (`/Users/x/My Secret Notes` leaves `Secret`).
  Closing it needs to know where a space-containing path ends, which no pattern can decide.
- A bare slug with no path and no extension (`page "terapia" not found`) cannot be caught.
  That is the hazard RFC §3 names: the panic message is the one free-form field.
- A URL loses its path (`https://host/a` → `https:<path>`).
  Over-cautious on purpose — an absolute path is not less sensitive for having a scheme.

## What the scrubber cannot see

`before_send` is the last word on the **event**, and that is not the same as the last word on the envelope.
`Client::capture_event` appends `scope.attachments` after `prepare_event` has run, so an attachment would never pass through `scrub_event`.
Nothing in outl adds one today, and this is the note for whoever first wants to.

## Testing

`redact` is feature-independent, so its proptests run on **every** `cargo test`, not only on the build that ships.
The scrubber tests need the feature.

```
cargo test  -p outl-telemetry
cargo test  -p outl-telemetry --features sentry
cargo clippy -p outl-telemetry --all-targets -- -D warnings
cargo clippy -p outl-telemetry --features sentry --all-targets -- -D warnings
```

Both clippy runs matter: the crate has to compile clean **with and without** the feature, and the no-feature build is the one every contributor gets.

### The regression net

Do not delete or relax these.
They are the gate.

- `the_sentry_feature_is_off_by_default` — reads this crate's own manifest.
  Asserted from the file rather than through `cfg`, because a `cfg`-based test passes exactly when the mistake has been made.
- `no_client_enables_crash_reporting_by_default` and `no_client_pins_the_sentry_feature_on_the_dependency` (`tests/client_manifests.rs`) — condition 1 where it can actually fail.
  This crate's own manifest being clean is not enough: each client declares its own `sentry` feature, and a client that puts it in `default`, or pins `features = ["sentry"]` on the dependency, links the reporter into every build on every machine.
  That is the one requirement the whole feature was conditioned on — clone the repo, run it, no Sentry — and nothing else in the suite fails if a client breaks it, because a client that reports is a *working* client.
  The manifests are parsed as `toml::Table`, not grepped: a textual check for `default = []` passes against `default = [ "sentry" ]` written with other spacing.
  A new client belongs in `CLIENTS` on the day it is created, which is the day it would otherwise start reporting unguarded.
- `init_without_a_dsn_is_a_no_op`, `init_respects_the_config_opt_out`, `a_suppressed_environment_closes_the_gate`, `each_condition_closes_the_gate_alone` — conditions 2, 3 and 4.
  They assert on `report_target` as well as on `init`, because `DSN` is `None` in every test build: a test that can only observe `None` cannot tell a closed gate from a missing one.
- `the_environment_closes_the_gate` (`tests/ci_env.rs`) — conditions 3 and 4 for real, including the two baseline phases where *no* variable is set.
  Without those phases the rest would pass against a function that always returns `true`.
  One file, one `#[test]`: an env var is process-wide, by the precedent of `outl-sync-iroh/tests/endpoint_lease.rs`.
  `one_does_not_silence` and `an_unparseable_opt_out_value_does_not_silence` pin the asymmetry with condition 4 — widening the match to `off` / `no` / `false` is a decision with a test to change, not an accident.
- `no_absolute_path_survives_the_scrubber`, `no_windows_path_survives_the_scrubber`, `no_page_slug_survives_the_scrubber` — proptests over generated input rather than the strings we thought of.
  They assert an **exact** output string, not `!contains(segment)`: a one-character segment like `f` is a substring of `<file>`, so a containment check would fail for a reason that is not a leak.
- `a_relative_source_path_survives_the_scrubber` — the other direction.
  A scrubber that replaced everything would pass every test above.
- `an_event_carries_only_allowlisted_fields` — builds an `Event` with every field populated with sensitive data, scrubs it, serializes it, and asserts the key set against `ALLOWED_EVENT_KEYS`.
- `no_populated_field_leaves_a_path_behind` — the key set alone would pass if a *surviving* field still held the leak, so the serialized payload is searched for it too.
- `a_quoted_string_does_not_survive_a_slicing_panic`, `a_unc_path_does_not_survive`, `a_path_with_a_space_does_not_leave_a_tail`, `a_relative_page_path_does_not_survive`, `an_actor_id_does_not_survive` — one per rule the review added, each asserting the exact string the old implementation returned.
  `an_index_panic_is_not_mistaken_for_a_quoted_string` and `a_path_tail_stops_at_prose` are their other halves: rule 0 must not truncate the most common panic in the codebase, and rule 3 must not eat the half of the message that says where we were.
- `an_unreadable_config_does_not_consent` (`outl-config/tests/telemetry_persistence.rs`) — condition 3's fail-closed half.
  It asserts the trap as well as the fix: that `Config::default()` is still ON, so if that ever changes the test fails instead of passing for a reason that stopped holding.
- `save_restores_the_sections_the_desktop_never_models` (`outl-desktop/src-tauri/src/settings.rs`) now covers `[telemetry]`.
  Mutation-checked: deleting the restore line fails it, and before that assertion existed the whole suite stayed green.
- `the_scrubber_writes_the_os_and_the_arch` and `an_incoming_os_context_is_replaced_not_merged` — the two halves of the one field we write.
  The second is what makes the write safe: the fixture arrives with a path sitting in `os.name`, shaped like what the `contexts` integration would produce, and none of it may survive.
  Without it, "the scrubber writes the OS" would also pass against an implementation that wrote the OS and left the rest of the map alone.
- `the_panic_itself_survives_the_scrubber` and `a_debug_image_keeps_its_id_and_loses_its_path` — the over-scrubbing direction.
  An empty event is not a safe event, it is a useless one.
- `the_fixture_really_populates_everything` — guards the guard: if the fixture stops populating a field, the key-set test starts passing for the wrong reason.

## Who calls `init`, and with what

Four binaries, each passing the surface that crashed rather than the file that ran:

| Binary | `client` | Where |
|---|---|---|
| `outl` | `"cli"`, or `"tui"` with no subcommand | `outl-cli/src/main.rs` |
| `outl-tui` | `"tui"` | `outl-tui/src/main.rs` |
| desktop | `"desktop"` | `outl-desktop/src-tauri/src/lib.rs` → `run()` |
| mobile | `"mobile"` | `outl-mobile/src-tauri/src/lib.rs` → `run()` |

`outl` with no subcommand **is** the TUI, so it reports as `tui`: a report labelled `cli@…` for a TUI session sends triage to the wrong half of the codebase.

Two things a caller must not change:

- **The guard is bound to a named variable**, never `let _ =`.
  It flushes pending events on drop, and `let _ =` drops it on that line — which compiles, runs, and reports nothing.
- **`init` is called from the binary, not from `outl_tui::run`.**
  `run` is also called by the `outl` binary, which has already started its own reporter by then; initialising inside the library means two `sentry::init` calls in one process on that path.

In both GUI clients it is the **first** statement of `run()`, before the `tracing` subscriber and before the rustls provider. The crashes worth the most are the boot ones — the Android app that closed on open is what motivated this RFC, and a reporter installed after the failing call reports nothing.

## Open work (not in this crate)

Release CI owns injecting `OUTL_SENTRY_DSN`, uploading symbols, and checking the shipped artifact is stripped.
The store declarations (App Store Connect App Privacy, Play Data Safety) are outside any test in this repo; RFC 0377's final section is their checklist.
