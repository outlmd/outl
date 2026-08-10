# Embedding outl as a Rust library

outl is a binary first, but the core is a set of plain Rust crates.
Anything that wants a local-first outliner as its storage layer can open a workspace, mutate it through the op log, and stay a well-behaved peer next to a running TUI or desktop app.
A memory backend for an AI agent, a notes feature inside another app, a migration script — all the same contract.

This page is the contract for doing that safely.
Read [Architecture](architecture.md) first if you want the why behind the layering.

## The crates

**What embedding opens is the core and its ops** — the tree, the log, and the actions that mutate them.
Four crates carry that contract, published to [crates.io](https://crates.io) on every release at the same version the binaries report:

| crate | what it gives an embedder |
|---|---|
| `outl-ws` | workspace bootstrap: locks, actor resolution, per-page shards, slug repair — the multi-process contract in one call |
| `outl-actions` | every mutation as a high-level action (`append_block`, `edit_text`, `open_or_create`, …) plus the `.md` projection helpers |
| `outl-core` | the tree CRDT, op log, `Storage` trait — you rarely call it directly, but the types (`NodeId`, `Workspace`, `HlcGenerator`) come from here |
| `outl-md` | markdown parse/render, sidecar, block-ref handles |

A fifth crate, `outl-exec`, is also on crates.io — **not as embedding surface**, but because `outl-actions` references it and cargo won't publish a crate whose dependencies aren't in the registry.
It ships the code-block language runtimes, and the workspace pins it at `default-features = false` (`Cargo.toml`) precisely so those runtimes stay out of everything except the binaries that want them.
**Don't depend on it directly.**
Running fences is an app concern, not a storage-layer one; if you think you need it, that's a conversation for an issue.

Everything else in the workspace (`outl-cli`, `outl-tui`, the Tauri clients) is `publish = false` on purpose.

```toml
[dependencies]
outl-ws = "0.11"          # GA releases
outl-actions = "0.11"
```

A `"0.11.0-beta"`-style requirement rides the betas cut from `main`.
The whole workspace shares one version, so keep the crates pinned to the same requirement.

## Opening a workspace

```rust
use std::path::Path;

let mut ctx = outl_ws::open(Path::new("/path/to/workspace"))?;
// ctx.workspace : outl_core::workspace::Workspace (the materialized tree)
// ctx.hlc       : HlcGenerator bound to this process's actor
// ctx.paths     : on-disk layout (pages/, journals/, ops/, .outl/)
// ctx.root      : workspace root
```

`open` does the whole boot protocol for you: shared workspace lock, per-actor write lock, config seeding, op-log replay, per-page shard registration, and split-brain slug repair.
Hold the returned `WsCtx` for as long as you operate; dropping it releases the locks.

Two contracts hide behind that call, and they are the reason to use `outl-ws` instead of wiring `JsonlStorage` yourself:

**Actor resolution.**
Each process writes to its own `ops-<actor>.jsonl`.
If your embedder opens a workspace while the desktop app is running, `resolve_write_actor` hands you an ephemeral actor and a fresh file — nobody ever appends to somebody else's log.
`ctx.ephemeral_actor` tells you which case you got.

**Snapshot policy.**
`open()` defaults to the short-lived contract: snapshots are read at boot, never written, because a snapshot write from a transient process races with the long-lived app that owns the workspace.
A resident embedder (a daemon that stays open) should opt in, or its boot cost on a large workspace regresses to full log replay:

```rust
let opts = outl_ws::OpenOptions { write_snapshots: true };
let mut ctx = outl_ws::open_with(path, opts)?;
```

To create a workspace from scratch, `outl_ws::layout::init(&Paths::at(dir))` scaffolds the directory layout before the first `open`.

## Reading

```rust
use outl_actions::page;

for meta in page::list_all(&ctx.workspace) {
    println!("{} ({:?})", meta.slug, meta.kind);
}

let id = page::find_by_slug(&ctx.workspace, "ideas");
let outline = outl_actions::outline::read_page_outline(&ctx.root, &meta)?;
let links = outl_actions::backlinks_for_page(&ctx.workspace, &ctx.root, &meta);
```

`read_page_outline` reads the projected `.md` + sidecar, which is the same path every client renders from.

## Mutating

Every mutation follows one shape: it takes `&mut Workspace` and `&HlcGenerator`, computes op parameters, and routes through `Workspace::apply`.
You never construct ops by hand.

```rust
use outl_actions::page::{self, PageKind};

// A page (idempotent on the slug).
let page_id = page::open_or_create(
    &mut ctx.workspace, &ctx.hlc,
    "meeting-notes", "Meeting Notes", PageKind::Page,
)?;

// Blocks.
let block = outl_actions::append_block(
    &mut ctx.workspace, &ctx.hlc,
    Some(page_id), Some("TODO follow up with the team"),
)?;
outl_actions::edit_text(&mut ctx.workspace, &ctx.hlc, block, "DONE followed up")?;

// Whole subtrees in one call: append_tree / append_forest.
// Journals: page::open_journal / page::open_today.
```

Op-log appends are batched per action (one fsync per logical write), so composite mutations like a forest append cost single-digit milliseconds, not one disk sync per block.

**After mutating, project the page back to disk:**

```rust
outl_actions::journal::apply_page_md_with_sidecar(&ctx.workspace, &ctx.root, page_id)?;
```

This is the part embedders get wrong most often, so it gets its own paragraph.
The `.md` file is a projection of the op log, never the source of truth.
If you skip the projection, the file on disk goes stale and the next reader sees old content.
If you instead edit the `.md` directly and skip the ops, you have written state the CRDT knows nothing about.
A peer will reconcile that edit through the external-edit matching path, which works, but loses the block-identity guarantees a proper op has.
Mutate through actions, then project.

## What an embedder must never do

These are the [repo invariants](contributing.md) as seen from the outside:

- Never write `id::`, UUIDs, or any metadata into the `.md` — IDs live in the `.outl` sidecar only.
- Never edit a `.md` and its sidecar by hand to "fix" state; the op log is the source of truth.
- Never share an `ops-<actor>.jsonl` between two writers; `outl-ws` already guarantees this, don't work around it.
- Never bind an iroh endpoint from an embedder; a device binds one, taken by whichever long-lived outl process got there first ([one endpoint per identity](sync.md)).
  Your writes land on disk and reach peers through that process's transport or the next maintenance resync, exactly like the ephemeral CLI behaves.

## Coexisting with a running app

An embedder is a **passive writer**, same as the ephemeral CLI (see [CLI](cli.md)).
Practical consequences:

- Your process may get an ephemeral actor when the app holds this device's actor.
  That is normal, not an error.
  (The device actor itself comes from the device store outside the workspace, not from `.outl/config.toml` — see [storage.md](storage.md#where-the-actor-id-lives--outside-the-workspace).)
- Ops you write while a long-lived outl process runs (a GUI, the TUI, `outl mcp serve`) are picked up by its watcher and shipped to peers by its transport.
- Ops written while nothing else runs sit on disk until any long-lived surface opens, then converge.
- Reads see whatever the log held when you called `open`.
  A long-running embedder that needs fresh peer state reopens, or wires `outl_actions::sync::SyncEngine` the way the MCP server does.

## Versioning

The whole workspace shares `[workspace.package].version`, and the published crates follow semver with the project: GA versions from tags, betas from `main`.
The `Storage` trait, the `Op` enum, and the sidecar format carry compatibility guarantees documented in [Storage](storage.md); anything not documented there is internal surface that may move between minor versions.

## Dependency policy

An embedder with a `cargo deny` / `cargo audit` gate in CI is the case these crates are published for, so the dependency graph is treated as part of the public surface.
A crate that trips a policy gate blocks adoption just as hard as a missing feature.

**A gate will not come back clean today.**
Here is exactly what it finds, so you can decide before you add the dependency rather than after.

Across the whole four-crate graph, transitive included, the last sweep leaves **one flagged crate and no non-permissive licenses**:

| crate | advisory | why it is still here |
|---|---|---|
| `smallstr 0.3.1` | [RUSTSEC-2026-0215](https://rustsec.org/advisories/RUSTSEC-2026-0215), `unmaintained` | transitive through `yrs`, the block-text CRDT. `yrs` 0.27.3 still depends on it and there is no patched release, so this needs an upstream fix rather than a bump here. |

What changed is that `bincode` is gone.
It was a **direct** dependency of `outl-core` until schema 4 moved the snapshot encoder to [`postcard`](https://crates.io/crates/postcard).
Every bincode version is unmaintained under [RUSTSEC-2025-0141](https://rustsec.org/advisories/RUSTSEC-2025-0141).
See issue #207 and [Storage → Wire format](storage.md#wire-format-postcard-schema-4).
That one we owned and fixed.
`smallstr` we don't.

If your gate fails on `informational = "unmaintained"` and you need to ship before `yrs` moves, an advisory ignore scoped to RUSTSEC-2026-0215 is the honest workaround.

This is also why `outl-exec` isn't embedding surface.
Its language runtimes are where the rest of the noise lives: unmaintained crates plus **LGPL-3.0-only** (`malachite*`, via RustPython) and **MPL-2.0** (`im-rc`, `sized-chunks`, `bitmaps`).
The `default-features = false` pin keeps every one of them out of the graph you actually get.
Depend on it directly and you opt back into all of it.

If a policy gate rejects one of these crates, open an issue with the gate's output.
That's how #207 was found.

## A complete example

```rust
use std::path::Path;
use outl_actions::page::{self, PageKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut ctx = outl_ws::open(Path::new("./notes"))?;

    let page_id = page::open_or_create(
        &mut ctx.workspace, &ctx.hlc,
        "from-my-app", "From My App", PageKind::Page,
    )?;
    outl_actions::append_block(
        &mut ctx.workspace, &ctx.hlc,
        Some(page_id), Some("written by an embedder, synced like any block"),
    )?;
    outl_actions::journal::apply_page_md_with_sidecar(&ctx.workspace, &ctx.root, page_id)?;

    Ok(())
}
```

Open the same directory with `outl` afterwards and the block is there, ref-handled, sync-ready.
