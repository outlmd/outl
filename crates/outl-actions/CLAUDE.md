# CLAUDE.md — outl-actions

The **UI-agnostic** workspace operations layer.
Every outl client (`outl-tui`, `outl-mobile`, future Tauri desktop) consumes this crate so we never duplicate edit / indent / toggle / journal-render logic.

If you add a workspace operation that two or more clients need, **it belongs here**, not in the binary that asked for it first.

## Layering

```text
outl-core           (CRDT, op log, storage trait)
   ↑
outl-md             (.md parse/render, sidecar, matching)
   ↑
outl-actions        ← you are here
   ↑
outl-cli / outl-tui / outl-mobile / future clients
```

## Public surface

The full catalogue — every function, its signature and when to reach for it — lives in
[`docs/primitives-actions.md`](../../docs/primitives-actions.md) → "The `outl-actions` public surface".

It is reference material: you look up an entry, you do not read it top to bottom.
Keeping it here cost every task in this crate the context to load it, and pushed this file past the size ceiling (issue #216).

**Before adding a helper here, search that catalogue first** — see [Reuse-first](#reuse-first) below.

## Contract

Every mutating function:

1. Takes `&mut Workspace` (caller-owned) and `&HlcGenerator` (caller-owned).
2. Reads tree state, computes op parameters, generates a `LogOp` with a fresh HLC.
3. Routes the op through `Workspace::apply` so the op log stays the single source of truth (invariant #1 of `outl-core`).
4. Returns `Result<T, ActionError>` — never panics on user error.

Functions **never**:

- Touch storage directly.
  Storage is `Workspace::apply`'s responsibility.
- Touch the filesystem outside of `journal::write_md_atomic`.
  **Deliberate exception: `asset::import_asset` / `asset::resolve_asset_path`.**
  An uploaded file's *bytes* are not workspace state.
  A multi-MB PDF replayed through the CRDT would bloat every device's op log irreversibly, so the bytes never enter the op log and can't route through `write_md_atomic`, which is a `.md`-projection concern.
  `import_asset` copies the file into `<root>/assets/<hash>.<ext>` directly (still atomic: tmp file + rename) and hands back the markdown link.
  Only that **link** is workspace state, and it enters the op log the ordinary way, as an `Op::Edit` when the caller inserts it into a block.
  Every other function in this crate stays filesystem-free outside the journal path — don't use this as precedent for a second exception without the same "bytes vs. state" argument.
- Hold per-client state (selections, modes, toasts, keymaps).
- Round-trip through `.md` to reconstruct workspace state.
  The op log is the source of truth; `.md` is a projection.

A composite action calls `apply` more than once for a single user-visible mutation (`append_forest`/`append_tree`, `page::open_or_create`, `paste::paste_markdown`, `template::instantiate_template`, `block::split_block`).
It wraps its whole body in one `Workspace::begin_batch()` so the N ops flush as one `Storage::append_ops` per destination instead of fsyncing per op.
Each op still goes through `apply` individually (dedup, Yrs merge, and the CRDT stay untouched); only the persist is deferred to the guard's `commit()`.
See `outl-core/CLAUDE.md` → "Batch append" and [`docs/storage.md`](../../docs/storage.md) for the mechanics and durability contract — this crate only needs to know: open a batch around a multi-`apply` action, commit it before returning.

## Page model

Pages are **regular nodes** directly under [`NodeId::root`] tagged with a `page-slug` property.
A `page-kind` property says whether the page is a regular `page` or a date-keyed `journal`.
The node's text is the page's title; its children are the page's blocks.
Keeping pages as ordinary nodes lets the tree CRDT handle move / delete / re-parent for free.

Disk layout when projected to `.md`:

```text
<root>/
├── journals/YYYY-MM-DD.md     ← page-kind = "journal"
├── pages/<slug>.md            ← page-kind = "page"
├── pages/<slug>.outl          ← sidecar (block IDs + hashes)
└── ops/ops-<actor>.jsonl      ← op log, one file per actor
```

`migrate_legacy_into_today` reshuffles any pre-page-model blocks (direct children of root that lack `page-slug`) under today's journal.
Clients call it once on startup; it's idempotent.

## Task state convention

Task state lives **in the block's text** as a prefix:

```
"foo"             ← plain block
"TODO foo"        ← open task
"DOING foo"       ← task somebody has started
"DONE foo"        ← completed task
```

This matches the TUI's existing wire format.
`cycle_todo` walks `None → TODO → DOING → DONE → None`.

**`split_todo` reads a second spelling it never writes**: the CommonMark checkbox (`"[ ] foo"`, `"[/] foo"`, `"[x] foo"`, `"[X] foo"`), issue #230.
A block typed that way is a task everywhere a `TODO ` block is one, and the first `cycle_todo` / `set_todo` rewrites it to the word form.
The bytes on disk are untouched until then — recognising a spelling must never turn opening a page into a write.
The trailing space is load-bearing: `[x](url)` is a markdown link and stays one.
Three consumers keep their own copy of this alphabet: `outl-exec`'s query engine and `@outl/shared`'s `cycleTodo` (the dependency arrow forbids importing it) plus `outl_md::outline_ops::count_todos`.
All three learn a new spelling in the same change, or a block reads as a task on one surface and as prose on another.

**One deliberate asymmetry:** those copies also unwrap a leading `"> "`, so the legacy order `"> TODO foo"` reads as a task there.
`split_todo` cannot: it returns a `&str` slice of the body, so dropping the quote would strip the marker from `OutlineNode.text` and the GUI would stop drawing the `│` bar.
That shape is therefore a task to the TUI render, the TUI chip and `status:` queries, and prose to the DTO, the CLI and plugins.
`cycle_todo` rewrites it to canonical order on the first toggle.

**`TodoState::prefix` is the single owner of the marker spelling**, and the prefixes are **not** the same width — `"DOING "` is six characters, the other two are five.
Any caller doing cursor math (the TUI's inline cycle, a GUI draft splice) must measure the prefix it is adding or removing instead of assuming five.
A local `match` re-spelling the markers is the second owner that goes stale the next time a state lands: `quote.rs` had one, and it stopped compiling the moment `DOING` existed — which is the good outcome, but only because the enum is exhaustive.
`edit_text` writes the caller's text **verbatim** — including the prefix — so the user can drop a TODO just by erasing `TODO `/`DONE ` in the editor.
UIs that surface state separately (mobile checkbox) must reattach the prefix before calling `edit_text`; helper `rawTextWithTodo` on the mobile side does this.
The historical "auto-preserve prefix" behaviour was removed because it made `TODO`/`DONE` impossible to delete from the editor.

## What this crate does NOT own

- **UI state.**
  Selections, modes, keymaps, and the undo stack for **in-flight text editing** (per-keystroke history inside an uncommitted draft) live in the clients.
  Committed-mutation undo is different: the bounded snapshot stacks + `.md` restore live here in `history` so every GUI shares one engine.
- **In-flight outline AST.**
  When the user is typing into a buffer that hasn't been parsed yet, the manipulation happens on `Vec<OutlineNode>` via `outl_md::outline_ops` (re-exported through the `outl-tui/src/outline_ops.rs` shim).
  We don't pull that up because it's not workspace-grounded — it's a stage *before* ops exist.
  It lives in `outl-md` because the mobile client also needs it, but no `Workspace` is touched, so it stays out of `outl-actions`.
- **Storage backends.** `JsonlStorage`, future `ChronDbStorage` implement `outl_core::Storage` and live in the binary that needs them.

## Reuse-first

This is the **shared layer**.
Every client (TUI, mobile, future desktop) consumes it — and they all consume the same struct, the same constants, the same policy.
Two parallel implementations of the same concept across clients is the bug we paid to delete,
see the `outl_md::index::Backlink` → `outl_actions::Backlink` consolidation,
where policy drifted on self-references and the user was the one who caught it.

When adding a new operation here:

1. **Search first.** `rg` for the symbol across `outl-core`, `outl-md`, and this crate before writing it.
2. **Promote, don't fork.**
   If a client crate already has a helper for the same concept, lift it here (and delete the client copy) — even if it's a small refactor.
   The `flatten_backlink_subtree` → `flatten_subtree_paths` move from `outl-md` is the canonical pattern: one owner, every client wraps.
3. **Generalize the parameter set** when migrating.
   The Backlink rewrite added `source_block: OutlineNode` + `source_path` so *both* the mobile linear renderer and the TUI subtree renderer could share the same struct.
   Capping features at "what mobile needs today" would force the TUI to keep its own copy.

The root [`CLAUDE.md`](../../CLAUDE.md#reuse-first) "Reuse-first" section documents the policy at the workspace level, in full at [`docs/contributing.md`](../../docs/contributing.md#reuse-first-no-parallel-implementations).

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-actions -- -D warnings`
3. `cargo test -p outl-actions`
4. If you changed the public API surface, update the table in "Public surface" above and the matching entry in the root `CLAUDE.md`.
