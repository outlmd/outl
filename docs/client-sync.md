# What reaches disk and the other devices, and when

Every client writes the op log synchronously and the `.md` after.
This is what that costs, when a re-projection is withheld, what the backup pass does, and how two devices sharing a folder keep up with each other.

Moved out of [`docs/clients.md`](clients.md), which had grown to the point where no edit to it was cheap.
That page keeps the cross-client architecture: the stack, who owns which fact, the locks an open takes, and the two conditions every client has to surface.

## Async projection writes (performance)

Every client is **async-on-write**: the op log is written synchronously (`Workspace::apply` → `Storage::append_op`), but the `.md` + sidecar projection — the file the user actually sees on disk — happens after, off the input path.
No client blocks a keystroke or a Tauri command reply on a `.md` render, an fsync, or a backlink rebuild.

| Client | Op log write | Projection write |
|---|---|---|
| TUI | Synchronous, on commit (`Esc`, `Enter`, structural ops) | Deferred. The commit marks the page dirty and repaints immediately; the render → `.md` write → `reconcile_md` drains the moment the event loop goes idle, or after 600ms mid-burst (`runtime::MAX_SAVE_DEFER`). Always flushed before quit, `Ctrl+S`, navigation, peer reload, or a `call:` re-run — see [`docs/tui.md` → Behavior worth knowing](tui.md#behavior-worth-knowing). |
| Desktop / Mobile | Synchronous, inside the Tauri command | Queued to a background `ProjectionWriter` thread (`outl_tauri_shared::ProjectionWriter::queue`), which coalesces bursts and writes `.md` + sidecar under the workspace lock. The command's reply is built straight from the in-memory tree (`build_page_view_from_tree`), not from a re-read of the file it just queued. |

A crash between the op-log write and a queued projection leaves the `.md` briefly behind the op log — that's not data loss.
The op log is authoritative: the next boot re-projects any stale page, and peers sync ops, never `.md` files.

### When re-projection is withheld

"The next boot re-projects any stale page" has three exceptions, and they are user-visible: the page keeps showing what is on disk instead of what the tree holds.
`outl_actions::apply_page_md_with_sidecar_if_stale` — the call every GUI open path makes — declines to write when:

- the `.md` no longer matches its sidecar (an external edit is pending; `outl reconcile` owns it);
- the `.md` holds content that exists in **no op**, which surfaces as an error naming the line count and one sample (`outl doctor` lists the pages, `outl reconcile --ahead-of-log` brings them into the log);
- the sidecar cannot vouch for what is on disk — it records no text for any block (`SidecarBlock::text` missing, i.e. written by a peer still on a pre-0.11 binary) **and** the `.md` holds lines that need vouching for.
  That page is already queued for the pipeline migration, whose reconcile rewrites the sidecar with text; the next open re-projects normally.

A page whose `.md` holds nothing but bare bullets is **not** one of these, and it used to be.
A fresh journal day, an unfilled `[[link]]` page and both pages `outl init` creates all carry a sidecar with no text, just like a pre-0.11 one, so the narrow check refused them.
Being queued for the pipeline migration did not clear it either: the page observed on a real workspace carried `pipeline_version` 4 against a current 5, so it *was* queued, but its blocks are genuinely empty, so each reconcile rewrote the same `text: ""` and the next open refused it again.
The page showed one empty bullet for good the moment a peer wrote into it, with no banner, no log line and no `doctor` entry, because `Ok(None)` is what `reproject_stale_md` reads as success ([issue #332](https://github.com/outlmd/outl/issues/332)).
`outl_md::unlogged::sidecar_can_vouch_for` is the single owner of that verdict now, and it asks about the bytes rather than the sidecar's shape.

The same rule governs the desync recovery (`recover_desynced_projection`): it recovers the lost ops but leaves the file alone when a block the log already knows carries different text on disk.
A stale view is recoverable; deleted bytes are not.
Reasoning and the incident behind it: [RFC 0210](rfcs/0210-md-content-outside-op-log.md), root `CLAUDE.md` invariant 8.

Plugin `onOp` hooks on desktop/mobile are fire-and-forget (no `await` on the reply path) for the same reason — a slow plugin can't stall the next keystroke or command.

## Automatic backups

`outl_actions::backup` is the shared engine: local, versioned git snapshots of a workspace, with `outl backup list|restore` on top.
`[backup] enabled` defaults to **on** ([config.md → `[backup]`](config.md#backup)), so the pass that makes that default true has to exist on the client, not just in the CLI.

**One call wires a client up:** `outl_actions::backup::spawn_auto_pass(root, enabled, interval_minutes)`, where `root` is a `BackupRoot`.
It spawns a detached background thread, waits `STARTUP_DELAY` (60 s — boot is already the busiest moment a client has), then snapshots whenever `interval_minutes` has passed since the newest commit.
The thread reads the current root at each pass instead of capturing a path, because the Tauri clients open their workspace asynchronously and can replace it at runtime; one thread per open would leave a snapshotter on every root the user has left.
The TUI sets its `BackupRoot` once.
The Tauri clients create one in `setup`, call `spawn_auto_pass` there, and hand it to `open_workspace_at`, which sets it only after the open succeeds.
With no workspace open the pass does nothing.
Two long-lived passes on one device (the TUI and the desktop on the same workspace) share one backup git dir and take no process lock, so the loser of an `index.lock` race logs a `warn!` and skips that interval.

| Client | Automatic snapshots | Where it's wired |
|---|---|---|
| TUI | **Yes** | `runtime::run_with_theme_override`, right after `clock::init`; sets its `BackupRoot` once |
| CLI subcommands | No — a one-shot process; `outl backup now` is the explicit path | — |
| Desktop | **Yes** | `setup` in `outl-desktop/src-tauri/src/lib.rs`; `open_workspace_at` retargets it on every workspace switch |
| Mobile | Wired, but only does something where a `git` binary exists — there is none on iOS, so the pass logs a `warn!` per interval and takes no snapshot | `setup` in `outl-mobile/src-tauri/src/lib.rs` |

Three rules any client that wires this up must keep:

- **Never on the edit path.**
  A snapshot walks the whole workspace and forks `git`; on a large graph that is seconds.
  It belongs on a background thread, not on a tick, a commit boundary, or an idle hook that the render loop waits on.
- **Never on the quit path.**
  Blocking an exit on a whole-workspace `git add` is a worse bug than a snapshot arriving one session late — and the interval floor is derived from the git history, so the next launch takes it.
- **Never fatal.**
  No `git` on `PATH`, no disk, a refused commit — all of it degrades to a `warn!` and `None`.

The repository is **device-local and outside the workspace** (`~/.config/outl/backups/<name>-<hash>.git`, workspace as `--work-tree`).
That is what makes it safe for the pass to create the repository on first run: it can't turn the user's notes folder into a git repo, it can't collide with a repo they already keep there, and no file transport replicates it between devices.

## iCloud sync (mobile + TUI, today)

The iOS app is on a public TestFlight beta — <https://testflight.apple.com/join/P2GdWAMd>.
Install it on the iPhone, then point the TUI at the same iCloud Drive container to share the workspace.

The mobile client persists the op log to the iCloud Ubiquity Container.
The TUI reaches the same workspace by pointing `--workspace` at the container's `Documents/` directory:

```
<container>/Documents/                ← TUI: outl --workspace "<container>/Documents"
├── journals/
│   └── YYYY-MM-DD.md                 ← daily journal projection
├── pages/
│   ├── <slug>.md                     ← regular page projection
│   └── <slug>.outl                   ← sidecar (block IDs + hashes)
└── ops/
    ├── ops-<this_device>.jsonl       ← only this device writes here
    ├── ops-<other_device>.jsonl
    └── ...
```

> The folder is **`ops/`**, not `.ops/`. iCloud Documents / Ubiquity Containers do not sync paths starting with `.` across devices, so a dotted name silently breaks multi-device sync.
> The same rule is why the sidecar moved from `.foo.outl` to `foo.outl` in v0.

Each device only writes to its own `ops-<actor>.jsonl`, so iCloud never has to merge file contents — the CRDT does that work after reading every actor's ops.
The `.md` projection is rewritten after every mutation — the TUI coalesces the write to its next idle/flush point, desktop/mobile queue it to a background writer (see "Async projection writes" above) — so it can briefly lag the op log by a beat.
Do **not** parse it back to reconstruct workspace state; the op log is authoritative.

### Shared sync engine

Both clients use `outl_actions::SyncEngine` for the reload-workspace + reproject-page flow.
Detection is client-specific (TUI runs a worker thread polling `snapshot_peers()` every ~2s; mobile registers `NSMetadataQuery` on the ubiquity container).
Once detection fires, the call site is identical:

```rust
let engine = SyncEngine::new(workspace_root, actor);
let fresh = engine.reload_workspace(&hlc)?;
engine.reproject_page(&fresh, focused_page_id)?;
```

`hlc` is the client's live generator, and passing it is not optional plumbing: the reload raises it past every op it just merged.
See [`docs/sync.md` → The shared sync engine](sync.md#the-shared-sync-engine) for why.

The TUI defers the reload while the user is in Insert mode (the in-flight `ParsedPage` would be clobbered) via a `pending_reload` flag drained on commit.
Mobile applies immediately because every mutation is one atomic Tauri command.
The policy diverges; the engine does not.

`engine.scan_for_orphans()` is the other shared piece: it walks `journals/` and `pages/` for `.md` files whose sidecar is missing or stale (fresh import from Roam/Logseq, peer-shipped projection without sidecar, external vim edit).
The TUI runs the scan every 10s on a worker thread; mobile runs it once at boot.
Both feed the same `outl_md::reconcile::reconcile_md`.

### Split-brain slug repair (boot)

`outl_actions::merge_duplicate_slug_roots(ws, hlc)` is the boot-time repair for the split-brain bug where two creators minted **different** ids for the same slug.
Two `2026-07-10` journal roots split a day's content across two roots, so the client flickers between them.
For each slug with more than one root it picks a canonical survivor: the `page_id_from_slug` id if one is present, else the root with the most descendants, tie-broken by smallest id.
It then re-parents every child of the other roots under the survivor preserving order (no data loss), and trashes the emptied duplicates.
Every step is an `Op` through `Workspace::apply`, so running it on **any** client converges on every device via the CRDT; it's idempotent (returns 0 on a clean workspace) and safe to call on every boot.
Clients call it once at startup alongside `migrate_legacy_into_today`.
Belt-and-suspenders: `find_by_slug` already resolves the same canonical winner deterministically, so the UI stops flickering even before the merge runs.

### Doubled journal title repair (background)

Page and journal roots get a **deterministic** id (`page_id_from_slug`), so two devices that create the same slug offline mint the same root and its `Op::Create` converges cleanly.
Before the fix, each device also wrote the title straight into the root's Yrs text.
Those two concurrent inserts at position 0 concatenated instead of converging — a `2026-06-25` journal opened offline on two devices ended up titled `2026-06-252026-06-25`.
`open_or_create` now writes the title into a `title::` property instead (`Op::SetProp`, last-write-wins by HLC), and only when the title differs from the slug.
A journal's title always equals its slug, so journals carry no `title::` property and no `title::` line lands in their `.md`.
Regular pages created in-app now render `title:: <title>` at the top of their `.md`.
`outl_actions::repair_doubled_journal_titles(ws, hlc)` cleans up journals corrupted before the fix.
Any journal root whose text is its slug repeated two or more times gets that text cleared — an `Op`, so the repair converges to every device — and the title falls back to the slug.
Idempotent, journal-only, and run on the **background** reconcile pass, not boot, since it scales with page count.
Desktop's `spawn_background_reconcile` and mobile's `spawn_workspace_opener` both call it.

### Peer reachability indicator (P2P / iroh transport)

When the iroh transport is running, the desktop / mobile "online / offline" dot reads `SyncTransport::peer_health()` — a reachability snapshot the transport fills from its **own** dials (boot connect, catch-up loop, gossip-triggered sync).
The GUI must **never** stand up a second iroh endpoint to probe peers: a second endpoint sharing the device identity hijacks the relay route from the live sync endpoint, and inbound sync gets refused.
The `outl_peer_status` command merges the snapshot onto the full `peers.json` list, so a peer the transport hasn't dialed yet shows offline.
The CLI's `outl peer status` is the lone exception (no running transport), so it keeps the transient-endpoint probe.
See `crates/outl-sync-iroh/CLAUDE.md` → "One endpoint per identity, elected not assigned".

### Sync progress feed (pairing screen)

Desktop's Sync panel and mobile's Devices sheet both show a live feed while a sync pass runs against a paired device.
It reads a `sync-progress` Tauri event the iroh transport emits as the pass advances — connecting, pulling a snapshot, ingesting or pushing ops, done or failed.
This is purely cosmetic: a dropped update never breaks anything, because the actual "reload the workspace" trigger is a separate, load-bearing signal the progress feed never touches.

What each phase shows:

| Phase | What the user sees |
|---|---|
| Connecting | "Connecting to `<peer>`…" |
| Snapshot | A real progress bar (%) — the total byte count comes from the frame's length prefix, known before the body arrives. |
| Asset | A real progress bar (%) for each binary asset (PDF / image) transferring, in bytes of the current file. Assets are content-addressed blobs replicated outside the op log; over iroh they travel on the `outl-asset/1` stream. |
| Received ops / Pushed ops | A live count ("Receiving 42 changes from `<peer>`"), never a bar — a batch's size is only known once it finishes landing, so it can't be turned into a percentage. |
| Synced / Failed | A one-line result for the pass. |
| Interrupted | Amber, `"<peer> went away — will retry"`. Not in the feed. |

`interrupted` changes **only the colour and the wording** — the pass is still an error internally and the peer is still re-pushed.
It stays out of the activity feed (pill only, like `connecting`): an event that fires on every screen lock would bury the lines that matter.
Why it is a separate phase, and the mirror-image bug where a *refused* dial showed nothing at all: [sync.md → Reading the Sync panel](sync.md#reading-the-sync-panel).

Below the pill, an activity feed lists "device → page" lines as ops land.
This is **best-effort**: the transport ships only the (capped) block ids a batch touched, and the client resolves them to page/journal slugs with a Tauri round-trip.
A block id that hasn't materialized on this device yet (the reload may still be in flight) resolves to nothing, and the line just keeps its raw op count instead of a page name.

**The per-page feed is empty during the initial pair.**
A freshly paired device pulls the whole log in one bulk pass, so naming every page it touched is meaningless — the feed only starts naming pages once ordinary incremental syncs (catch-up ticks, gossip-triggered pushes) run afterward.

Implementation: `outl_actions::SyncProgress` (`crates/outl-actions/src/sync.rs`) is the wire payload, threaded through `SyncTransport::set_progress_sink` and bridged to the `sync-progress` Tauri event by
`outl-tauri-shared`'s `iroh_sync::start_with_reload_bridge`.
`@outl/shared/peers` (`createSyncProgress` + `<SyncProgressView />`) is the one shared implementation both clients render, so the two panels can't drift.

See `crates/outl-mobile/CLAUDE.md` for the full bundle ID, signing team, container ID set required to build it, and the `NSFileCoordinator`-based peer-file materialisation step that has to run before any read of a peer `ops-*.jsonl`.
