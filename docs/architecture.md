# Architecture

High-level structure and the major design decisions behind it.

## Overview

outl is split into six crates today:

```mermaid
flowchart TB
    bins["<b>Clients</b><br/>outl-cli · outl-tui · outl-mobile (Tauri 2 + Solid, iOS) · outl-desktop (Tauri 2)"]
    actions["<b>outl-actions</b><br/>UI-agnostic workspace operations + SyncEngine<br/>(edit, indent, toggle TODO, render md, reload workspace, scan orphans)"]
    md["<b>outl-md</b><br/>parse / render / sidecar / 3-level matching / diff / inline tokens / outline_ops"]
    core["<b>outl-core</b><br/>Tree CRDT, op log, storage trait, domain models<br/>(Workspace, Page, Journal, Block, Property, Tag)"]
    bins --> actions --> md --> core
```

`outl-core` knows nothing about files, markdown, or networks.
`outl-md` knows about markdown and sidecars but nothing about a workspace mutation pipeline.
`outl-actions` is the **only** crate where workspace-changing logic lives; every client routes through it so TUI and mobile cannot diverge on what "indent" or "toggle TODO" means.
`outl-cli`, `outl-tui`, and `outl-mobile` are I/O shells (key handling, Tauri commands, frontend wiring).

This split is what makes cross-device sync possible — both clients operate on the same op log, through the same operations, with the same matching algorithm.

---

## Major design decisions

These were locked in before code shipped.
Don't unilaterally pivot — ask first.

### 1. Markdown is source of truth

The user's words live in `.md` files.
The op log is **derived** from user-facing operations.
If we lose the op log, we can reconstruct the current state from `.md` + sidecar.

**Trade-off:** the file system is the canonical interface.
We accept the overhead of writing `.md` after every op.
The alternative (DB-only) is what Logseq moved to and what broke their community.

### 2. Op log on disk: JSONL per actor

Each device appends to a single `ops/ops-<actor>.jsonl` file inside the workspace. iCloud Drive (and any other file-level sync transport) syncs each actor's jsonl independently, so two devices never collide at the filesystem layer — the CRDT merges the per-actor streams after reading them.
This is the **only** persistent backend; 0.5.0 removed the older SQLite backend after it kept producing divergent op logs between clients on the same workspace.

**Trade-off:** JSONL is easy to inspect, easy to ship across any file-level transport, and trivial to reason about.
We give up SQL queries (we don't need them — replay + in-memory index is fast enough) and PRAGMA-level integrity guarantees (replaced by partial-write tolerance: a half-written tail line is just skipped).
`Storage` is still a trait — `ChronDbStorage` (issue #1) can land later without touching `outl-core` logic.

### 3. `Storage` is a trait, not a concrete struct

```rust
trait Storage: Send + Sync {
    fn append_op(&mut self, op: &LogOp) -> Result<()>;
    fn append_ops(&mut self, ops: &[LogOp]) -> Result<()>;
    fn ops_since(&self, ts: HLC) -> Result<Vec<LogOp>>;
    // ...
}
```

`outl-core` consumes `dyn Storage`.
`JsonlStorage` (in `storage/jsonl.rs`) is the only persistent impl; `MemoryStorage` (in `storage/memory.rs`) is the test double.

**Why:** swapping backends is a single-file change.
Test doubles are trivial.
The sync code can mock storage.
Future ChronDB integration is a PR adding `storage/chrondb.rs`.

`append_ops` amortizes durability over a batch instead of paying one fsync per op (default impl loops `append_op`; `JsonlStorage` overrides it with a single `sync_all`).
`Workspace::begin_batch()` is the apply-side half.
A composite action (append a forest, create a page, split a block) opens a `WorkspaceBatch` guard and runs its ops through the normal `apply` path one at a time.
The guard defers the persist until it commits: one `append_ops` call per storage destination for the whole action.
See [`docs/storage.md`](storage.md) for the durability contract and `outl-core/CLAUDE.md` for the guard's mechanics.

### 4. Sidecar JSON instead of inline IDs

Logseq writes `id:: 01HXY...` lines into the `.md`.
We refused that.

We write the IDs into a sidecar file `foo.outl` (JSON next to `foo.md`).
The `.md` stays clean.
VS Code shows what the user wrote.
GitHub renders it beautifully.
Obsidian doesn't get confused.
The sidecar lives in the same directory so the sync transport ships it alongside the `.md` — the dotfile form was abandoned because iCloud Documents (one `file`-transport option) skips dotted paths when syncing across devices.

**Trade-off:** external edits require **matching** to reconstruct IDs.
That's a real algorithm (`outl-md/src/matching.rs`) with three confidence levels and an orphan log.
It's more work than inline IDs, but the user experience is dramatically better.

### 5. Tree CRDT specifically, not a generic CRDT

We could use a generic op-based CRDT (Automerge).
We chose to implement Kleppmann 2022 directly because:

- The paper is short and the algorithm fits in ~300 lines of Rust.
- Domain-specific = better error messages, simpler API.
- We control the on-disk format (op log schema).
- No transitive deps on a heavyweight CRDT framework.

**Trade-off:** we're on the hook for correctness.
That's why the test battery is huge and the coverage target on the four critical functions is 100%.

### 6. Yrs for block text content

Tree CRDT moves blocks.
Yrs (Yjs in Rust) handles concurrent edits to the text **inside** a block.
Combining them gives us:

- Block-level structure: tree CRDT.
- Character-level text: Yrs.

Yrs is mature, battle-tested in Yjs-based apps.
Reusing it lets us focus on the part nobody else has solved (the tree).

### 7. ULID for IDs

128 bits, lexicographically sortable, monotonic per millisecond, no central authority.
Better than UUIDv4 (random, sorting nightmare) and better than UUIDv7 (good but ULID is established and the spec is finalized).

### 8. uhlc for timestamps

Hybrid Logical Clock = wall clock + logical counter + actor.
Comparing two HLCs gives a total order without coordination, and the wall-clock component keeps timestamps human-meaningful for debugging.

### 9. Journal is a first-class concept

Daily notes (`2026-05-24.md`) live in `<workspace>/journals/`, separate from `<workspace>/pages/`.
Navigation keys `[`, `]`, `t` are dedicated to journals.
When you open `outl-tui`, you land on today's journal.

This isn't an afterthought — it's the primary input path for the user's day-to-day notes.
Anything that makes journal access slow or hidden is wrong.

### 10. MIT license

One license, no dual-license boilerplate to maintain, no patent grant language to argue about.
Permissive enough for any downstream — including plugin authors who want to relicense their own crates differently.

### 11. iroh for P2P

QUIC, hole punching, no central servers, no STUN/TURN dependency in the common case, in Rust, BSD-licensed.
The alternatives are heavier (libp2p) or non-Rust.

### 11.5. Shared library crates

A handful of pure-data, dep-light crates exist solely to keep the clients (`outl-tui`, `outl-desktop`, future Tauri shells) honest about agreeing on shape:

- **`outl-actions`** — UI-agnostic workspace operations (block ops, journal render, sync engine). Anything two clients would otherwise re-implement lives here. See `crates/outl-actions/CLAUDE.md`.
- **`outl-theme`** — palette (named hex colors) for seven presets (`outl`, `dracula`, `nord`, …). TUI converts hex → `ratatui::Color`; desktop converts hex → CSS custom properties. One source for the look of every renderer.
- **`outl-shortcuts`** — `Action` enum + `Chord` + `Binding` catalog. TUI's key handler and desktop's `lib/shortcuts.ts` both consume the same `default_bindings()` so `j/k/i/o/dd/c/qq/⌘P/…` mean the same thing in both clients.
- **`outl-config`** — TOML at `~/.config/outl/config.toml` (XDG-style on every OS, including macOS). Reads global theme preset, vim-mode toggle, last-opened workspace, font size. The desktop's Settings modal writes here; the TUI reads it on startup. Per-workspace `.outl/config.toml` (workspace identity) overrides on top.
- **`outl-frontend-shared`** (`@outl/shared`) — TS/Solid library used by `outl-mobile` and `outl-desktop`. Pure helpers, DTO types, the `MarkdownInline` renderer, `invoke()` wrappers — anything the two frontends would duplicate.

The rule: a new helper or constant only lands in a client crate when it's *genuinely* client-specific. Otherwise it belongs in one of the shared crates above. The root `CLAUDE.md` "Reuse-first" section is the policy.

### 12. Tauri for desktop

> **Why it works this way:** [RFC 0002](rfcs/0002-tauri-for-every-gui-client.md) — one Rust surface under every GUI client, covering decision 13 below as well.

Rust core reuse, smaller binary than Electron, native webview.
Slightly worse UX consistency than fully-native, but acceptable for an outliner where the bulk of the UX is text and lists.

### 13. Tauri 2 for mobile (replaces the earlier uniffi plan)

Originally planned around `uniffi` with SwiftUI / Compose native UIs.
The plan changed when the mobile client landed: Tauri 2 ships a single Rust binary that hosts a `WKWebView` running a SolidJS + Tailwind frontend, with native bits (`NSMetadataQuery`, accessory toolbar, ref suggester) written in Objective-C alongside the Tauri shell in `gen/apple/Sources/outl-mobile/main.mm`.
Trade-offs:

- **Win:** the entire workspace operation surface is shared with the TUI via `outl-actions`.
  Zero duplicated business logic.
  Adding a feature on one client means adding it on the other for free.
- **Win:** Solid + Tailwind iterates fast and we control the rendering pipeline end-to-end.
- **Loss vs. uniffi:** the UI is webview-hosted, not native widgets.
  Acceptable for an outliner where the bulk of the UX is text and bullets; would be a worse trade for a graphics-heavy app.

Android is on that same Tauri 2 surface today, not "when it's prioritised" — a signed APK ships with every release.
The platform layer it needed turned out not to be an iCloud-watcher counterpart at all (mobile storage is a local folder synced by iroh, with no filesystem watcher in the Rust path).
It was `android_jni.rs`: iroh's relay TLS and system-DNS reads both call into the JVM, and nothing in Tauri/wry primes the process-wide JNI context they expect, so the first QUIC connection used to abort the process.

---

## Data flow

### User types in TUI (write path)

```mermaid
flowchart TB
    keystroke[User keystroke]
    input[TUI input handler]
    action["Workspace::apply_user_action() → Op"]
    applyop[outl-core::apply_op]
    storage["Storage::append_op<br/>(jsonl)"]
    tree[Update materialized tree]
    notify["Workspace notifies<br/>'page X changed'"]
    render["outl-md::render(page_ast)<br/>→ new .md text<br/>outl-md::sidecar_write"]
    fs["File system:<br/>pages/X.md + .X.outl updated"]
    p2p["broadcast Op<br/>to peers via iroh"]

    keystroke --> input --> action --> applyop
    applyop --> storage
    applyop --> tree
    storage --> notify
    tree --> notify
    notify --> render --> fs --> p2p
```

### User edits .md in VS Code (read-from-disk path)

```mermaid
flowchart TB
    save[File save in VS Code]
    notify["notify (filesystem watcher in outl serve)<br/><i>200ms debounce</i>"]
    parse["outl-md::parse(.md) → new AST (no IDs)<br/>outl-md::sidecar_read → old AST (with IDs)"]
    match["outl-md::matching::match_blocks(old, new)<br/><i>3-level confidence</i>"]
    diff["outl-md::diff::diff_to_ops(...) → Vec&lt;Op&gt;"]
    apply["For each Op: outl-core::apply_op<br/>(commits via transaction)"]
    write["outl-md::sidecar_write (refresh)"]
    p2p["broadcast Ops to peers"]

    save --> notify --> parse --> match --> diff --> apply --> write --> p2p
```

### Every client is async-on-write

The TUI write-path diagram above is exact for the `apply_op` / `Storage::append_op` / tree-update steps — those are synchronous, on the same call that handles the keystroke or the Tauri command.
The **render → `.md` + sidecar write** step is not: on every client it runs after that call returns, off the input path.
The TUI coalesces a commit and drains it the moment the event loop goes idle (bounded by `MAX_SAVE_DEFER`, forced on quit / `Ctrl+S` / navigation).
Desktop and mobile hand the page to a background `ProjectionWriter` thread and build the command's reply straight from the in-memory tree instead of waiting on the write.
See [`docs/clients.md` → Async projection writes](clients.md#async-projection-writes-performance) for the full picture across clients.

This is a project rule, not just today's implementation: the op log write is the one synchronous step, and nothing on the input path — a `.md` render, a backlink index rebuild, a plugin hook — gets to block the next keystroke again.

---

## Concurrency model

Two layers:

**Within one device.** `outl-tui` and `outl-mobile` each hold a single `Workspace` behind a `Mutex` (or its parking_lot equivalent).
All mutations route through `outl_actions::*` functions that take `&mut Workspace` and append to the actor's `ops-<actor>.jsonl` via `Workspace::apply`.
The TUI's optional file watcher and the mobile's Tauri command surface are the two writers; they serialise on the workspace lock.

**Across devices.** Each device only ever writes to its own `ops-<actor>.jsonl`.
The transport (iroh by default, file/iCloud opt-in) is responsible for shipping each actor's file to every other device.
`outl_actions::SyncEngine` is the shared piece that both the TUI poller and the mobile `NSMetadataQuery` watcher call when a peer file changes:

- `snapshot_peers()` lists every `ops-*.jsonl` *except this device's* so a client never reacts to its own writes (the destructive save-reload-race loop is closed at this filter).
- `reload_workspace()` reopens the workspace from disk, merging all per-actor jsonls by HLC and replaying through the move-op algorithm.
- `reproject_page(workspace, page_id)` re-emits the focused page's `.md` + sidecar from the new tree state.
- `scan_for_orphans()` finds `.md` files whose sidecar is missing or whose `last_synced_hash` no longer matches — fresh imports (Roam/Logseq dump, peer-shipped projection without sidecar) or external edits in vim.
  Both paths feed `outl_md::reconcile::reconcile_md`.

The TUI poller checks peer snapshots every ~2s on a worker thread.
Mobile registers `NSMetadataQuery` on the iCloud ubiquity container.
Both call into the same `SyncEngine`.
Insert mode in the TUI defers the reload via a `pending_reload` flag drained on commit — see [`crates/outl-tui/CLAUDE.md`](../crates/outl-tui/CLAUDE.md#peer-sync-coordination) for the policy.

---

## Error handling philosophy

- `thiserror` for typed errors in libs.
- `anyhow` only at the binary boundary (CLI prints errors with context).
- No `unwrap()` in non-test code.
- A corrupt sidecar is **recoverable**: `outl doctor --repair` regenerates it from the op log.
  A bare `outl doctor` only reports — repairs never happen without you asking.
  Don't crash, log + fall back.
- A corrupt op log is **catastrophic** but we surface it loudly via `outl doctor` so the user can intervene before further writes.

---

## Future considerations (documented, not built)

- **End-to-end encryption** of sync traffic — iroh supports it, we'll enable.
- **Per-workspace identity** — each device gets a stable ActorId, stored **outside** the workspace in the device store (`~/.config/outl/actors/<workspace-id>`, or `$OUTL_DEVICE_DIR`) so two devices syncing one directory can never read the same one.
  See [storage.md → Where the actor id lives](storage.md#where-the-actor-id-lives--outside-the-workspace).
  Global preferences — theme, vim mode, font size, last workspace — live separately in `~/.config/outl/config.toml` via the `outl-config` crate.
- **Read-only export** — Hugo, static HTML, PDF.
- **Plugin system** — a JavaScript runtime (Boa) ships today (consume op stream, op hooks, slash commands); deeper hooks (new query types, richer render hooks) are still planned.
