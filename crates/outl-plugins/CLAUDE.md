# CLAUDE.md — outl-plugins

The plugin system shared by **every** client (TUI, desktop, mobile, CLI).
Read this before editing the crate.

## What this crate is

A plugin is bundled JavaScript described by `plugin.json`.
It declares the **capabilities** it registers and the **permissions** it needs.
The user approves permissions on install; the loader intersects capabilities with what the current client implements.
A plugin written once runs on every client because it talks to *this* crate, never to anything client-specific.

> **Why an interpreter, and why the shipped runtime inverted the original proposal:** [RFC 0025](../../docs/rfcs/0025-plugin-system.md).

The runtime engine is **Boa** (already embedded in `outl-exec`, runs on iOS — pure-Rust, no JIT), behind the `PluginEngine` trait so it can move to QuickJS later **only if** gas/perf/async becomes a *measured* blocker.

## Non-negotiables (inherit the root invariants)

1. **Plugins never touch `outl-core` directly.**
   Every mutation goes through the host API → `outl-actions` → `Workspace::apply` → op log.
   No shortcut, no editing `.md`, no bypassing the CRDT.

2. **Every plugin op is stamped `actor = "plugin:<id>@<device>"`.**
   The op log is the audit trail — keep it that way.

3. **`storage:local` does not converge in d0.**
   It is per-device local KV.
   If a plugin needs cross-device state, model it as an `Op` (root invariant 7), never as a shared file.

4. **Permission is checked on every host call.**
   Deny by default.
   A capability the client lacks loads *partially* with a warning — never a silent crash, never a panic.

5. **The bundle hash is revalidated on every load.**
   A hash mismatch blocks the load.
   Plugins do not change silently.

6. **`network:<domain>` is enforced per hop, not per call.**
   Checking the URL the plugin typed and then handing the request to a client with reqwest's default `Policy::limited(10)` is not an allowlist — it is an allowlist for the first round trip and nothing after it.
   An approved host answering `302 Location: https://attacker.example/collect` delivered the plugin's headers there, and an open redirect on a legitimate API (`/logout?next=…`, `/r?url=…`) is common enough to have [its own CWE](https://cwe.mitre.org/data/definitions/601.html) — the host does not have to be hostile.
   `net.rs` re-checks **every** hop against the same `NetworkDomain` grants through the same `matches_host`, and refuses the redirect naming the host it refused.
   Still a re-check and not a blanket refusal: an `http → https` upgrade on an approved host, or an API redirecting inside its own approved domain, is legitimate and keeps working.

   **The chain is driven in `send_chain`, not by a `reqwest::redirect::Policy`, and that is load-bearing.**
   A policy may answer follow, stop or error — it cannot touch the headers of the request it is judging.
   reqwest strips `Authorization` / `Cookie` / `Proxy-Authorization` across a cross-host hop and forwards every **custom** header, so `X-Api-Key` is what actually travels, and a plugin holding `secrets` puts a keychain token in exactly such a header.
   The allowlist alone does not close that: under `*.s3.amazonaws.com`, `*.blob.core.windows.net` or `*.github.io`, every bucket the attacker owns is already "approved", so the hop is *allowed* and the token goes with it.
   So the rule is **treat every plugin-supplied header the way reqwest already treats `Authorization`**: `gate::check_hop` returns `Hop::CrossOrigin` on the same predicate reqwest uses (host, port **or** scheme differing) and `send_chain` clears the headers.
   Clearing, not refusing — refusing would break an API redirecting to its own CDN, which is the case the re-check exists to keep working.
   Going back to a `Policy` is the thing to refuse in review; it silently re-arms the leak with every test still green.

   **The scheme allowlist is ours, not reqwest's.**
   `ftp://localhost/x` and `ws://localhost/x` carry a host a `network:localhost` grant covers, and reqwest's own scheme check runs *after* the policy (`redirect.rs` calls `policy.check(..)`, then tests the scheme inside the `Follow` arm), so the gate was answering "follow" for a scheme it had no opinion about.
   `file:`/`data:` never reached it only because the URL spec normalises `file://localhost/x` to a hostless URL — an accident of the parser, not a rule.
   A backstop one dependency bump away from vanishing is not a backstop.

   **`timeoutMs` is clamped** (`gate::MAX_TIMEOUT_MS`, 60s, covering every hop together).
   The fetch blocks the one thread that owns the Boa `Context`, and every other plugin request waits on it with a bare `recv()` — so an unclamped `timeoutMs` against a host that accepts and never answers is not a slow fetch, it is every plugin feature in the app, permanently.

   **A refusal that only reaches the plugin did not happen.**
   The plugin can `catch (e) {}` the string it is handed, so an attempted exfiltration through an open redirect would leave no trace on the machine.
   Every refusal arm goes through `net::refuse`, which logs *and* renders, and the log names the plugin (`permission::NetGrant` carries the id next to the domains precisely so it can).
   Adding a refusal that returns without going through `refuse` puts the silence back.

   **The response body is capped too** (`MAX_RESPONSE_BYTES`): it is buffered whole and embedded in a JSON string for the JS side, so an approved host answering with a multi-gigabyte body is an OOM with no attacker creativity required, and a jetsam kill on mobile.
   [Issue #280](https://github.com/outlmd/outl/issues/280).

   **Still open, deliberately: the allowlist is about host *strings*, not destinations.**
   A plugin author can ask for `network:api.their-own.example`, have it approved, and point the A record at `127.0.0.1` or `169.254.169.254`.
   The plugin has no fs and no socket, so reaching loopback is a sandbox escape — outl's own MCP server, a local Ollama, cloud instance metadata — and no redirect gate helps, because the hop never leaves the approved host.
   Closing it means a `reqwest::dns::Resolve` that refuses non-global addresses; see the "What this gate does not cover" note at the top of `net.rs` before reaching for it, because the loopback test suite and `IpAddr::is_global` (still unstable on 1.95) both get in the way.

7. **`sync-transport` is trusted to move bytes, and for nothing else.**
   Not for the ops' content (they go through `Workspace::apply` and the CRDT, and a malformed line is skipped), not for the actors they name, and **not for their clocks**.
   `sync_pull` is the second path in the workspace that ingests ops written by someone else's clock, so it calls the same gate the first one does — `outl_core::hlc::skew_ahead_ms`, with the same `MAX_CLOCK_SKEW_MS` — **before** `hlc.observe`.
   After `observe` is too late: the generator is monotonic, so a timestamp from the year 584 million raises this device's clock and keeps it there, every op the device writes afterwards is past its peers' own gate, and the device keeps working locally while silently syncing nothing it writes.
   It needs no hostile plugin — a backend with a wrong clock, or a JSONL written by a device whose date was set forward once, is enough.
   Adding a second copy of "how far ahead is too far" here is the thing to refuse in review; a third ingestor is a third **caller** of that one function.
   [Issue #283](https://github.com/outlmd/outl/issues/283), and root `CLAUDE.md` invariant 10 — *who was standing on the thing you moved?*

## Layout

```
src/
├── lib.rs         # crate doc, re-exports, HOST_API_VERSION
├── error.rs       # PluginError
├── manifest.rs    # plugin.json parse + validation
├── capability.rs  # Capability enum + intersect()
├── permission.rs  # Permission enum + network domain rules + PermissionSet gating
├── lockfile.rs    # installed.json (InstalledPlugins / InstalledEntry) + bundle_hash
├── model.rs       # JS↔host data: ReadModel, BlockView, HostIntent, MoveTarget, TurnOutput
├── runtime.rs     # PluginEngine trait (engine seam: load / run_command / dispatch_op)
├── engine.rs      # BoaEngine (feature `js`): native bridge + JS prelude (describe→apply)
├── net.rs         # ctx.net.fetch: drives the redirect chain itself (non-negotiable 6)
├── net/
│   ├── gate.rs          # one hop's rules as pure fns: scheme, host, downgrade, origin change, timeout clamp
│   └── tests.rs         # loopback servers; the deny cases assert on what the unapproved server received
├── host.rs        # PluginHost: a plugin's lifetime — load, per-turn engine state, run_command, sync_hooks
├── host/          # the four jobs with their own rules, split out of host.rs
│   ├── project.rs       # &Workspace -> ReadModel / LogOpView; read-only, one producer of the JS shapes
│   ├── intents.rs       # the ONLY place the host mutates: permission-gated HostIntent -> outl-actions
│   ├── contributions.rs # manifest.contributes -> palette / chords / toolbar / transformers, capability-gated
│   └── sync.rs          # sync-transport push + pull, including the HLC skew gate (non-negotiable 7)
├── loader.rs      # disk loader + install_from_dir (.outl/plugins/<id>/ + installed.json + _dev)
└── registry.rs    # registry index (fetch/search) + marketplace API (feature `registry`)
```

The `host/` cut is by **responsibility, not size**, and two of the four exist so a rule has one address instead of being spread across an 855-line file.
`intents.rs` is the only module that takes `&mut Workspace`, so non-negotiables 1 and 4 are checked there, in that order, and nowhere else.
`sync.rs` is the whole `sync-transport` trust boundary (non-negotiable 7) — both directions in one file, because reading either half alone hides which checks are load-bearing.
`host.rs` keeps the plugin's lifetime and re-exports the four contribution types, so the crate's public surface is unchanged.

## Marketplace API (one owner, both GUI clients wrap)

`registry.rs` owns the shared marketplace surface so the desktop and mobile Tauri layers stay thin shims (the bug this prevents: the same `registry ∩ lockfile` mapping written once per client and drifting).
All four take a `&Path` storage root; the desktop resolves its `Arc<Mutex<Option<PathBuf>>>` to a `&Path` at the call site, mobile passes its owned `PathBuf` directly.

- `MarketplaceItem` — a registry entry + local `installed`/`enabled` state (the serialized row both clients render).
- `marketplace_list(&Path)` — fetch the index, cross-reference the lockfile.
- `marketplace_install(&Path, &ActorId, id)` — download + install an official plugin, returns its name.
- `set_enabled(&Path, id, enabled)` — flip the lockfile flag (no network).
- Uninstall reuses the existing `loader::uninstall`.

## Execution model: describe → apply

The JS engine never holds `&mut Workspace`.
Each turn the host hands the engine a read-only `ReadModel` (blocks + pages) and the plugin config.
The plugin **reads** from it and **emits** `HostIntent`s into a buffer.
The host then drains the buffer and applies each intent through `outl-actions` (permission-gated).
Plugin handlers live in JS-land (`globalThis.__OUTL`), so no `JsFunction` is ever stored in Rust.

**Anti-loop:** `PluginHost` tracks `last_seen` log length.
Ops a plugin produces advance `last_seen` too, so they never re-trigger `onOp` — no plugin→op→plugin cycle.
`sync_hooks` is the single post-mutation entry point a client calls after any action.

`PluginHost` is **not `Send`** (Boa `Context` is single-threaded).
Fine for TUI/CLI; GUI clients run it on a dedicated plugin thread.

## Status

- **Done:** manifest, capability, permission, lockfile, model, `BoaEngine`,
  `PluginHost` (commands + run_command + onOp via sync_hooks), disk loader + install,
  registry index, SDK + example plugin — all with tests, including an end-to-end run
  of the **real shipped bundle** (`real_example_bundle_archives_done_blocks`).
- **Wiring in progress:** CLI `outl plugin`, TUI slash/hooks.
- **Remaining:** desktop/mobile wiring (needs a dedicated plugin thread for `!Send`),
  `.outlpkg` packaging, `github:` install source.
  The `network` and `storage` host calls shipped (`net.rs`, `__outl_storage_*`); this line claimed otherwise long enough that #280 read as a review of unwritten code.

Full plan: the approved design doc the user signed off on (issue #25).

## Security tests are mandatory (root Rust rule)

Auth/ACL code needs deny ≥ allow.
Already covered: `network:*` rejected, leading-wildcard domain matching (apex and suffix-collision denied), permission growth via `covers()`, bundle-hash mismatch, keybinding-to-unknown-command.
Add to this set when you add a host call — happy path is **not** coverage.

**A deny test about the shape of the permission string is not a deny test about the connection.**
Every `network:` deny above was about parsing (`*`, a mid-string wildcard, an apex domain), and the one runtime test asserted that the **first** URL is refused — which is exactly the case that already worked.
Nothing covered what happens after the socket opens, which is how #280 lived in a function with a test named `net_fetch_refuses_unapproved_domain`.
So the network suite is against real loopback servers (`net.rs` tests, no new dev-dependency — a `TcpListener` and canned bytes), and the deny case asserts on **the unapproved server's own record of what it received**, not just on the error the caller got back:

- `a_redirect_off_the_allowlist_never_reaches_the_unapproved_host` — two loopback servers whose host *strings* differ (`127.0.0.1` vs `localhost`), the second one fails the test if it sees the request at all.
- `a_redirect_to_another_approved_host_is_still_followed` — the load-bearing allow.
  A gate that refuses everything passes every deny test ever written, and `Policy::none()` would pass every other test in this list.
- `a_response_declaring_a_body_over_the_cap_is_refused` / `a_close_delimited_body_over_the_cap_is_refused_mid_stream` — the declared-length and unknown-length halves of the cap; only the second one actually bounds memory.
- `a_custom_header_never_crosses_to_another_approved_host` — the hop the allowlist *approves*. It asserts the second server's own record lacks `X-Api-Key` **and** that the fetch still succeeded, so a future "just refuse cross-origin hops" cannot pass it.
- `a_blocked_redirect_is_logged_not_only_returned`, `a_body_over_the_cap_is_logged_not_only_returned`, `a_streamed_body_over_the_cap_is_logged_not_only_returned` — a refusal the host never recorded is a refusal nobody can act on. They read a captured `tracing` subscriber, and the first also pins that the line names the plugin.
- `a_non_http_scheme_is_refused_before_a_request_is_built` / `a_redirect_to_a_non_http_scheme_is_refused_even_when_the_host_is_approved` — both assert on **our** wording (`only http`), not on the word "scheme", because reqwest's own refusal says that too and a looser assertion passes with the gate deleted.
- `gate.rs`'s unit tests cover what a loopback server cannot: the `https → http` downgrade, the `http → https` upgrade that must still be followed, the port-change origin rule, and the `timeoutMs` clamp.

`tests/sync_skew_gate.rs` is the same discipline for non-negotiable 7: it pins that an op past the window is dropped, that the local clock did **not** move (the part that makes the bug permanent), and — the allow — that an op at the very edge of the window still applies.

## Reuse-first

The JS engine setup (Boa context + console shim) already exists in `outl-exec`.
When `BoaEngine` lands, extract the shared bits — do not write a second copy.
Host-API methods wrap `outl-actions` functions; never re-implement block/page ops here.
