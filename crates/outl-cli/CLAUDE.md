# CLAUDE.md — outl-cli

The `outl` binary.
Thin shell over `outl-core` + `outl-md` + `outl-actions` + `outl-tui`.
**No business logic lives here** — only argument parsing, file orchestration, watcher setup, human-readable output, and the JSON envelope used by every machine-shaped subcommand.

## UX rule: no subcommand → open the TUI

`outl` with no subcommand opens `outl-tui` in the current directory.
This is the primary mode — Roam/Logseq users expect to launch the app and see their notes, not a help screen.

The TUI library is reused via `use outl_tui;` (the crate exposes both a library and a binary).
Don't fork the TUI logic into the CLI.

### Workspace path resolution (`resolve_path` in `main.rs`)

Every subcommand that operates on a workspace runs through one helper.
Precedence — first hit wins:

1. **Subcommand-positional** path (e.g. `outl page get … <PATH>`).
2. **Global `--workspace <DIR>`** flag.
3. **`[workspace] last`** in `~/.config/outl/config.toml`, read via `outl_config::load()`.
   Same file the desktop's Settings modal writes — opening a workspace in the GUI makes `outl` (no args) land on it from the terminal.
4. **Current working directory** — final fallback (`cd ~/notes && outl`).

A path stored in `config.toml` that no longer exists on disk is **skipped silently** (`tracing::warn!` only) so a deleted/unmounted workspace doesn't crash the launch — the chain falls through to cwd.

**Opening a workspace created by a GUI client or P2P sync.**
The desktop, mobile, and the iroh transport seed a workspace with `.outl/workspace-id` + `ops/` + the page/journal dirs, but **never** the per-workspace `.outl/config.toml`.
They keep the device actor in `<app-config-dir>/actor`, not in the workspace.
The CLI/TUI/MCP used to read the device actor from `config.toml`, so pointing them at a GUI-made workspace failed with "no outl workspace — run `outl init`".
`workspace_layout::read_or_init_config` fixes that: when the `.outl/` dir exists but `config.toml` doesn't, it seeds a fresh one and proceeds, so `outl --workspace <gui-folder>` just works.
`ws::open` (CLI + MCP) and `outl_tui`'s `open_workspace` both go through this lazy-seed path; a genuinely-missing `.outl/` still errors.

**Never read the write actor out of `config.toml`.**
Every opener here resolves it through `outl_ws::actor::resolve_device_actor`, which reads `outl_core::DeviceStore` — a directory outside the workspace.
`.outl/config.toml` rides the file-sync surface on every transport except iCloud, so an actor read from it is the *same* on two devices.
The per-actor `flock` cannot arbitrate either (advisory, machine-local): both devices append to one `ops-<actor>.jsonl` and lose ops silently.
That applies to `init`, `serve`, `doctor` and `migrate-to-per-page-ops` too — `doctor` reports the **resolved device actor**, not `cfg.workspace.actor_id`.
See `outl-core/CLAUDE.md` → "Actor id is device-local, and the workspace cannot hold it".

> Full schema + per-OS path of `config.toml` is documented in [`docs/config.md`](../../docs/config.md).
> The `outl-config` crate is the only reader; never re-parse the TOML by hand here.

## Commands

> Full subcommand surface (every flag, JSON envelope shape, MCP mapping) lives in [`docs/cli.md`](../../docs/cli.md).
> The lists below are a navigable index for contributors — one line each, by intent.
> Don't add full flag tables here; they belong in `docs/cli.md` (root `CLAUDE.md` → "One owner per fact").

### Lifecycle / one-shot

- `outl` — open TUI in current directory (also `outl tui [<path>]`).
- `outl init <path>` — scaffold a workspace (pages/, journals/, .outl/).
  Seeds `templates/journal` as a **page** (`template:: journal`), not a `templates/journal.md` file (issue #146).
  A legacy file, if present, migrates into the page body best-effort.
  Opening today's journal then auto-instantiates it.
- `outl serve [<path>] [--once]` — run file watcher; `--once` reconciles every `.md` and exits (smoke tests, scripting).
- `outl doctor [<path>] [--json] [--repair]` — integrity check.
  **Read-only by default**; `--repair` is the only writing mode.
  Full user-facing check list lives in [`docs/doctor.md`](../../docs/doctor.md).
  Parser warnings are appended to `.outl/orphans.log` tagged `parse-warning <iso> <path>:<line> <kind> <raw>` so the trail persists across runs.

  `cmd/doctor/` is a module dir, one file per class of check.
  `oplog.rs` — raw `.jsonl` line sweep, snapshot decode, offset-index coherence.
  `files.rs` — `.md` ↔ sidecar, parse warnings, orphan block refs, sync-conflict copies.
  `tree.rs` — trash contents, unmaterialized ops, projection drift (needs a booted `Workspace`).
  Its drift check asks `outl_actions::content_lines_missing_from` before offering a page for re-projection, because the sidecar hash gate proves the sidecar agrees with the bytes on disk and **not** that those bytes came from the log.
  A page holding unlogged content is reported and withheld from the plan, so the read-only listing never promises a repair `--repair` then refuses (invariant 4 below).
  `repair.rs` — the `--repair` pass.
  `mod.rs` — report types + orchestration.

  Two invariants for anyone touching this:

  1. **The raw `.jsonl` sweep in `oplog.rs` must stay independent of `JsonlStorage`.**
     `JsonlStorage::open` skips malformed records on purpose — one torn tail line must never lock a user out of their workspace — and reports them only via `tracing::warn!`.
     Doctor is the only surface that names *which line of which file* was lost, so it frames records itself rather than asking the storage layer.
  2. **`--repair` writes projections and caches, never the op log.**
     Allow-list, inside the workspace: re-project a `.md` + sidecar from the log, rebuild a missing sidecar over byte-identical content, delete a corrupt snapshot, prune old repair backups.
     Allow-list, in the device store: drop an actor binding whose workspace is provably gone, delete a scratch file a killed writer abandoned.
     It never deletes a `.md`, never touches `ops/`, never moves a block to the trash, and never picks a winner between sync-conflict copies.
     Every file it writes is copied to `.outl/repair-backup/<timestamp>/` first; that directory is pruned by `BACKUP_KEEP` / `BACKUP_MIN_AGE_DAYS` (an entry must fail **both** to be removed).
     **All six are `Plan` entries**, so `describe()` announces each prune like the rest and a run whose only work is a prune still happens — an otherwise-healthy workspace is exactly the one that would keep every generation forever.

     The last two are the entries whose subject lives **outside** the workspace (`<device_dir>/actors/`), and they carry two consequences worth stating.
     `outl-core`'s `device/gc.rs` owns the "is this safe to drop" verdict entirely, and nothing here re-derives it.
     `DeviceStore::prune_binding` re-asks it immediately before deleting, because the workspace can come back between the listing and the write.
     And because the store is machine-global, `collect_internal` takes the `DeviceStore` as a **parameter**.
     Resolving it inside the pass would make every test in the battery judge and delete from one shared `.dev-device-store` — issue #211 reintroduced by its own fix (root `CLAUDE.md` invariant 9, third question).
     `collect_with_scope` in `cmd/doctor/tests/mod.rs` is what hands each run its own.
     The page write itself delegates to `outl_actions::apply_page_md_with_sidecar{,_if_stale}` so the doctor never develops its own opinion about when a projection is safe to overwrite.
  3. **A damaged op log has no authority to overwrite its own projection.**
     `JsonlStorage` skips malformed records by design, so a torn `ops/` file replays into a **truncated tree** — while the `.md` on disk still matches its sidecar hash and therefore still looks "faithful".
     Re-projecting there overwrites a good `.md` with an incomplete render and rewrites the sidecar to declare the truncation faithful, which is a silent, unrecoverable delete dressed as a repair.
     So `OpLogHealth` gates `reproject` **and** `rebuild_sidecar`: both are cleared before `describe()`, so a damaged log doesn't even *offer* them in the read-only listing.
     Three things mark the log damaged — a line carrying no usable op, an unreadable file / mid-scan I/O error, and a sync-conflict copy inside `ops/` (a forked log whose ops never reached the tree).
     Snapshot deletion is deliberately **not** gated: it is pure cache, and dropping it only forces the full replay a damaged log wants anyway.
  4. **Read-only means read-only, including the index sidecars.**
     `JsonlStorage::open` runs `create_dir_all` + `reload`, and `reload` persists `.ops-*.idx` / `.ops-*.nodes.idx` — so merely *opening* the storage writes into `ops/`, in both modes.
     `ops_guard.rs` snapshots `ops/` before the open and restores it afterwards, in **both** modes, so "never touches `ops/`" holds for the whole command rather than just the repair pass.
     The test asserts over the whole directory (names + bytes); comparing a single file is what let this hide.

     Two things the restore pass must never get wrong, because both destroy user data while reporting success:

     - **A file that appeared mid-run is not automatically the doctor's to delete.**
       `ops/` is a sync target and this guard holds no lock, so iCloud, Syncthing or a co-resident client can land a brand-new `ops-<peer>.jsonl` during a replay that takes minutes on a large graph.
       Only an index sidecar is ours to un-create; a `.jsonl` is reported and left exactly where it is.
       `remove_file` returning `Ok` means the alternative would erase a peer's whole history without a single line in the report.

     - **An op log that only grew is someone else appending, not a defect.**
       On a synced workspace that is the common case, and calling it "a bug in the doctor" (an error, exit 1) trains the user to ignore the loudest line in the report.
       Growth is a `warn` saying the findings are a snapshot rather than a live view; only a log that shrank or was rewritten is an error.
  5. **A destructive operation must state its scale before it runs, and stop when the scale is large.**
     Re-projection removes content legitimately — a peer deleted a block, the log is right, the `.md` is behind.
     What it must not do is remove *thousands* of lines because something systemic is wrong, print a page count, and leave nothing to compare against afterwards.
     That is not hypothetical: `--repair` printed `708 fixed` while removing 1,426 lines from 233 pages, and no line in the output mentioned a line ([RFC 0210](../../docs/rfcs/0210-md-content-outside-op-log.md)).
     So `tree.rs` measures, per page, how many content lines the new projection would **not** reproduce, and carries the number in `repair::PageWrite`.
     `Plan::volume()` totals it into `RepairVolume`; anything past `CONFIRM_ABOVE_LINES` (100) or `CONFIRM_ABOVE_PAGES` (20) needs `--repair --force` (`RepairScope::Forced`).
     Three things this gets right that a page count cannot:
     a page the write only *adds* to counts as zero, so a device that just paired and has the whole graph unprojected never asks for a flag it does not need;
     the volume is announced in **both** modes, because read-only is where the user decides whether to authorise the write at all;
     and the suppression is all-or-nothing over the page writes, since removing half of a bulk deletion is the failure the guard exists to prevent.
     The measurement routes through `outl_md::content_lines_missing_from`, the same owner invariant 8 uses, with the render's blocks as the reference instead of the sidecar's.
     "Will this line survive the write" and "does the log know this line" are different questions, and both get asked here.
     Guarded by four tests in `cmd/doctor/tests/safety.rs`:
     `a_repair_that_would_delete_a_lot_of_content_stops_and_asks`,
     `the_same_repair_runs_once_it_is_explicitly_forced`,
     `an_ordinary_amount_of_deletion_repairs_without_a_flag`,
     `the_volume_is_announced_by_a_read_only_run_too`.
- `outl reconcile [<path>] [--ahead-of-log] [--allow-bulk-delete]` — no flags, list orphans pending manual resolution.
  `--ahead-of-log` reconciles the pages whose `.md` holds content that exists in no op, **bypassing the sidecar hash gate** (it clears `last_synced_hash` so `reconcile_md` stops short-circuiting).
  It has to exist because such a page is hash-faithful, so the ordinary reconcile reads it as in-sync and never looks at it.
  Opt-in on purpose: it emits ops for content the log has never seen, which is a deliberate write, not a repair.
  Detection is `outl_actions::content_lines_missing_from` against the **sidecar's blocks** — the same owner `doctor` and the write-side guard use, so the three cannot disagree about which pages qualify.
  Run it only on a build whose parser preserves the content: reconciling with a parser that still drops prose after a block property writes the truncated text into the log, which is the one place the loss currently is not.
  `--allow-bulk-delete` is the **only** reachable `OrphanGuard::Disabled` in the binary.
  `reconcile_md` refuses a pass that would trash more than 500 blocks of a page or more than 75% of one, and that refusal is only defensible while the user can say the deletion was meant — a guard with no escape hatch is a wall (root `CLAUDE.md` invariant 9).
  It was unreachable from any user-facing surface for one commit, so `the_bulk_delete_escape_hatch_is_reachable_from_the_command_line` in `main.rs` pins the wiring rather than the policy.
- `outl recover [<path>] [--apply] [--min-lines N]` — the **op-log-side** counterpart to `--ahead-of-log`, for a page whose `.md` was already overwritten before that guard existed.
  Reads `Workspace::block_text_history` for a block whose current text is a proper prefix of an earlier `Op::Edit` — a truncating edit whose predecessor is still in the append-only log.
  Issue #210's producer emitted the truncation as a real op; it never erased anything.
  Read-only by default; `--apply` writes each recovered revision back as a **new** `Op::Edit` (never a log rewrite), refusing per-block when the text changed since the scan.
  `--min-lines` (default 1) raises the report threshold.
  Full behaviour: [`docs/cli.md`](../../docs/cli.md#outl-recover).
- `outl migrate-to-shared [<path>]` — copy local sqlite log into shared `ops/` JSONL for cross-device sync.
- `outl import roam|logseq|obsidian|auto <src> <dst>` — graph import.
  Every source routes through the adapter-based `outl-import` crate (`--dry-run`, `--json`, `--preserve-timestamps`; real `((blk-XXXXXX))` ref/embed resolution, `Op::SetCollapsed`).
  `auto` picks the adapter from the source's shape.
  A non-dry run **refuses a destination that already holds content** (`cmd/import/guard.rs`): re-importing overwrites those pages and reconciles the result, erasing anything written in outl since the last import.
  `--force` is the explicit opt-in; `--dry-run` never reaches the guard.
  Three things that guard gets right and a file-counting one cannot.
  It reads occupancy off the **materialized tree**, not `pages/*.md` — a device paired over iroh holds the whole graph with nothing projected, and importing into it fuses the new blocks under the existing (slug-derived, identical) page roots.
  It treats `outl init`'s output (template page + empty journal) as vacant, so the documented `init` + `import` flow never asks for `--force` — a guard that fires on the happy path only teaches users to type the destructive flag.
  And it recognises `.outl/import-in-progress.json`, the marker a real import holds for its duration, so an import that died at page 40k of 66k is resumable with a plain re-run instead of `--force`.
  See `crates/outl-import/CLAUDE.md`; `cmd/import/mod.rs` here is pure glue (args, workspace bootstrap, report printing).
- `outl theme list|show <preset>` — TUI theme inspection.
- `outl plugin init|list|install|run|enable|disable|remove` — manage the workspace's JS plugins (under `<workspace>/.outl/plugins/`), wrapping `outl-plugins`.
  `init <NAME> [--id <ID>] [--dir <PATH>]` scaffolds a buildable plugin project (manifest + `package.json` + `tsconfig` + `src/index.ts` + README); it touches no workspace.
  Templates live in `cmd/plugin_init.rs`.
  `list` loads every installed plugin and prints version + enabled state + contributed slash commands.
  `install <SOURCE>` takes a local directory **or** a `github:owner/repo[/subdir][#tag]` source and shows the requested permissions.
  GitHub sources are cloned at an immutable semver tag (newest when not pinned, never a mutable branch) — the clone + tag resolution live in `cmd/plugin_source.rs` (shells out to `git`).
  It asks for approval (`--yes` to skip, required when stdin isn't a TTY) before copying the plugin in and freezing the approved permissions in the lockfile.
  `run <ID> <CMD>` runs a contributed command and re-renders every `.md` (op log is source of truth; files are a projection).
  `enable|disable <ID>` flip the `enabled` flag in `installed.json`.
  `remove <ID>` (aliases `uninstall`, `rm`) deletes the plugin's directory and its lockfile entry (the id is validated against path traversal before any deletion).
  Unlike the machine-shaped commands, `plugin` uses `anyhow` at the boundary (operator-facing, interactive), like `peer`.
- `outl peer pair|list|remove|status` — manage paired devices for P2P sync.
  Reads the per-**device** `~/.outl/identity.key` + the per-**workspace** `<workspace>/.outl/peers.json` via `outl-sync-iroh` (`IrohIdentity`, `PeersStore`).
  All four resolve the workspace (`--workspace` / `resolve_path`) so the pair belongs to the graph, not the OS; a one-time migration copies any legacy global `~/.outl/peers.json` into the workspace on first touch.
  `pair` runs the real iroh handshake.
  The host prints a ticket + ASCII QR and waits for one inbound connection.
  `--ticket <str>` connects, exchanges `PeerEntry`s, and writes the peer to `peers.json`.
  It **also adopts the host's `WorkspaceId`** (written to `<workspace>/.outl/workspace-id`) so later sync isn't refused as `workspace-mismatch` (issue #197), printing the `WorkspaceAdoption` outcome.
  `--name <str>` is the alias THIS device advertises (it lands under our node id in the peer's `peers.json`).
  It defaults to the machine hostname via `default_device_name` (best-effort `hostname` shell-out, `.local` trimmed) so the peer list reads a real name instead of a node-id stub.
  A small `tokio` runtime drives the async `host_pairing` / `join_pairing` helpers from this sync binary.
  `status` is still a static listing; live reachability lands with the running transport.

### Machine-shaped (JSON envelope, `--json` everywhere)

These are the surface called by scripts, agents, and the MCP shim.
Each handler returns a `serde_json::Value` so the same code path serves both the CLI and `outl mcp serve`.

- `outl page get|create|update|delete|list|rename|render|history` (`create` takes `--content=<JSON|->` to seed the outline in one call)
  `page history` / `block history` are the **read-only** window on the op log's past (issue #241): what changed on a page, when, by whom, and the text on either side.
  `cmd/history.rs` renders both — glue only, `outl_actions::timeline` owns what an event is.
  Two things the renderer must not lose: `--limit` caps the listing and never the count (a capped list that reports its own length as the total reads as the whole history), and a `deleted` row always prints the text the deletion took, since that is what someone opens a history to find.
  **No MCP tool, and not one call away from having one.** Both are `run_page(path, …) -> i32` / `run_block(path, …) -> i32`: they open the workspace themselves and print, where MCP dispatch wants `fn(ctx: &WsCtx, …) -> Result<Value, ApiError>` like every other handler in this crate. Wiring them up means extracting the `Value` half first, then registering in `mcp/tools::list` + `run_tool`.
- `outl block get|append|append-tree|insert|update|move|delete|toggle-todo|tree|history` (`append-tree` takes `--tree=<JSON|->`)
- `outl daily today|get|append|range`
- `outl asset add <file> [--page=<slug>] [--daily]` — import a file into `<workspace>/assets/` (content-addressed) and append its markdown link as a new block (daily by default, or a page).
  Glue only: copy + hash + link live in `outl_actions::import_asset`; the block append routes through `outl-actions` like every other mutation.
  CLI + MCP (`outl_asset_add`) share the `cmd::asset::add_asset` handler so they can't drift.
- `outl search "<query>" [--in=blocks|pages|all] [--limit=N]`
- `outl query [--tag=…] [--priority=…] [--since=…d] [--kind=…] [--prop key=value …]`
- `outl backlinks page|block|embed`
- `outl tag list|pages`
- `outl prop set|get|list`
- `outl template list|apply|resolve|run` — template pages.
  `list` finds every page with a non-empty `template::` property.
  `apply` instantiates a structural template under a target block.
  `resolve` returns a callable template's code block + declared params.
  `run` executes a callable template: inject params, run through the shared `run_callable_block` path, write the `> **result:**` subtree under `--block`.
  `apply`/`run` reject a `--block` that belongs to a page other than `--page` (`INVALID_ARG`).
- `outl export hugo|md|json`
- `outl batch [--ops=<JSON|->]` — runs a list of write ops in one workspace session (stop-on-first-error, returns `failed_at` / `applied` on the partial outcome)
- `outl workspace info`

The full mapping (CLI ↔ MCP tool) is documented in [`docs/cli.md`](../../docs/cli.md).

### MCP

- `outl mcp serve [--workspace=…]` — JSON-RPC 2.0 over stdio implementing the MCP protocol surface Claude Desktop expects (`initialize`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, `prompts/list`, `prompts/get`).
  Every tool is a thin router that delegates to the same handler the CLI subcommand calls — there is no second business-logic path.

## P2P sync: the MCP takes the endpoint when nobody else has it

iroh's relay routes only ONE endpoint per `node_id` at a time.
A second endpoint on that id breaks the holder's sync **in both directions** for any relay-reachable peer.
The demoted endpoint receives nothing, and its outbound catch-up stalls too, because the peer's return traffic follows the node id to whoever is ACTIVE.
That is real, and it is why `outl mcp serve` + the desktop GUI used to break each other.

The fix for *that* was to hard-code the MCP as a passive writer.
It removed the collision and introduced a worse one.
On a machine with **no GUI** — an agent driving `outl mcp serve`, which is a normal way to run outl — nothing bound an endpoint at all, so the device's ops never left and no peer's ops ever arrived.
The other device just showed "offline" (issue #220).

The constraint was never "only the GUI"; it is "one live endpoint per device identity".
So the answer is a lease, not a policy: `outl_sync_iroh::build_transport` hands the endpoint to whichever process asked first, and everyone else degrades.
Full mechanism: [`outl-sync-iroh/CLAUDE.md`](../outl-sync-iroh/CLAUDE.md) → "One endpoint per identity, elected not assigned".

What that means for the surfaces in this crate:

- **The MCP server contends, and runs the file poller either way.**
  On first workspace open (`mcp/mod.rs::ensure_transport`) it always starts `outl_actions::FileSyncTransport` — a disk poller that flips `peer_dirty` when a co-resident process changes `ops/`, so the next tool call reopens and its reads stay fresh.
  It *also* asks `build_transport` for the endpoint, and takes it when granted: then it announces after every mutating tool (`tools/dispatch.rs` → `ServerCtx::announce_local_ops`) and serves inbound, shutting down on stdin close.
  Refused — a GUI got there first — it is exactly the passive writer it was before, and that GUI pushes its ops out.
  It also declines the endpoint when the workspace has **no paired peers**: nothing to sync, and dropping the lease leaves it free for a GUI that wants it for pairing.
  The ask is repeated on every workspace reopen while it has no endpoint, because every reason to decline is something the user changes from outside this process (pairing a first device, closing the GUI).
  Answering once would strand a session that started before its device was paired — an MCP server lives for hours.
- **The ephemeral CLI never contends.**
  A `page`/`block`/`daily`/`batch`/`import` command runs in ~200ms — far too short to establish a QUIC connection (which takes seconds), so binding a transport just to drop it would steal the relay route for nothing.
  These commands write `ops-<actor>.jsonl` and rely on whichever process holds the endpoint plus every device's catch-up re-sync (`MAINTENANCE_RESYNC`) to converge.
- **`outl sync` contends and stands down when it loses.**
  It is the explicit flush for scripts: bring a transport up, force a push/pull pass against every peer, wait, exit.
  When another local process already holds the endpoint it says so and exits, because that process is already pushing these ops out and a 25s route steal would break the sync `outl sync` was asked to help.
- **`outl peer pair`/`status`** use a transient endpoint they close before returning (CLI-only, no long-lived client should be mid-pair at the same time).

## JSON envelope (CLI + MCP)

```json
{ "ok": true,  "data": { … }, "error": null }
{ "ok": false, "data": null,  "error": { "code": "X", "message": "…" } }
```

Stable error codes live in `output::codes` (`NO_WORKSPACE`, `PAGE_NOT_FOUND`, `BLOCK_NOT_FOUND`, `INVALID_BLOCK_ID`, `INVALID_DATE`, `CONFIRM_REQUIRED`, `CYCLE_REJECTED`, `SLUG_CONFLICT`, `PROP_NOT_FOUND`, `INTERNAL`, `INVALID_ARG`).
Add new codes by appending — never renumber existing ones (LLMs cache them).

Exit codes follow:

- `0` success
- `1` user error (`ApiError` with non-`INTERNAL` code)
- `2` internal error (`ApiError::INTERNAL`)
- `3` nothing was done, and that is not a failure.
  `outl sync` returns it when it stood down instead of flushing (another local process holds the endpoint, P2P is off, no device paired), so a script can tell "I pushed" from "someone else will" without treating either as broken.

## Layout

```
src/
├── main.rs                # clap entry, dispatches to commands
├── output.rs              # JSON envelope, ApiError, exit codes
├── ws.rs                  # WsCtx — open Workspace + HlcGenerator + lock
├── workspace_layout.rs    # filesystem layout (.outl, pages/, journals/)
├── sync_engine.rs         # shared reconcile path (serve/doctor reuse)
├── cmd/
│   ├── mod.rs
│   ├── init.rs            # outl init
│   ├── serve.rs           # outl serve
│   ├── doctor/            # outl doctor — one file per class of check
│   │   ├── mod.rs         #   report types + orchestration
│   │   ├── oplog.rs       #   raw .jsonl sweep, snapshots, offset indexes
│   │   ├── files.rs       #   .md ↔ sidecar, parse warnings, conflicts
│   │   ├── tree.rs        #   trash, unmaterialized ops, projection drift
│   │   ├── ops_guard.rs   #   restores ops/ byte-for-byte after the run
│   │   └── repair.rs      #   the --repair pass
│   ├── reconcile.rs       # outl reconcile
│   ├── recover.rs         # outl recover — op-log-side text recovery
│   ├── theme.rs           # outl theme
│   ├── import/            # outl import — glue over the outl-import crate
│   │   ├── mod.rs         #   adapters, --dry-run, auto-detect, progress line, report printing
│   │   └── guard.rs       #   re-import safety: op-log occupancy + the in-progress marker
│   ├── migrate_to_shared.rs
│   ├── export.rs          # legacy `outl export --to fmt` placeholder
│   ├── export_v2.rs       # outl export {hugo,md,json}
│   ├── asset.rs          # outl asset add … (+ shared add_asset for MCP)
│   ├── page.rs            # outl page …
│   ├── plugin.rs          # outl plugin …
│   ├── block.rs           # outl block …
│   ├── daily.rs           # outl daily …
│   ├── search.rs          # outl search
│   ├── query.rs           # outl query
│   ├── backlinks.rs       # outl backlinks …
│   ├── tag.rs             # outl tag …
│   ├── prop.rs            # outl prop …
│   ├── template.rs        # outl template …
│   ├── batch.rs           # outl batch
│   └── workspace_info.rs  # outl workspace info
└── mcp/
    ├── mod.rs             # stdio loop, dispatch
    ├── protocol.rs        # JSON-RPC 2.0 shapes + error codes
    ├── tools.rs           # tool registry + handler dispatch
    ├── resources.rs       # outl:// URI handlers + templates
    └── prompts.rs         # /outl-* prompts
```

Every `commands/*.rs` handler is `pub fn` so `mcp/tools.rs` reuses it directly.
New tools land by:

1. Adding a function in the relevant `cmd/*.rs` returning `Result<Value, ApiError>`.
2. Threading it through the local `Subcommand` and `run()` switch.
3. Registering the tool in `mcp/tools::list` (schema) and `mcp/tools::run_tool` (dispatch).

## Conventions

- `clap` derive for parsing.
- Every `--json` flag forces JSON envelope output; otherwise the human formatter inside each `cmd/*.rs` runs.
- Machine-shaped handlers always return `Result<Value, ApiError>`.
- Mutating commands take the workspace lock through `ws::open`.
  Two `outl` processes can't race against `outl serve` or each other.
- `anyhow::Result` on lifecycle commands (`init`, `serve`, `doctor`) is kept — those produce human errors and never JSON.

## What this crate does NOT do

- ❌ Implement the CRDT (use `outl-core`)
- ❌ Parse markdown (use `outl-md`)
- ❌ Hold workspace mutation logic (use `outl-actions`)
- ❌ Render TUI directly (use `outl-tui` as a library or sub-binary)
- ❌ Network anything (P2P sync lives in `outl-sync`)
- ❌ Duplicate logic between CLI and MCP shim (always route through the same `cmd/*::pub fn`)
- ❌ Add a helper here that re-implements something already in `outl-core` / `outl-md` / `outl-actions`.
  `cmd/*` handlers are glue — they parse args, call the upstream API, and JSON-envelope the result.
  If you need a new operation, add it upstream first (`outl-actions` is the usual home), then call it.
  See root [`CLAUDE.md`](../../CLAUDE.md#reuse-first) for the policy.
