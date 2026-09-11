# Shared primitives catalog

**Before writing any helper, scan these tables first.**
Most "I need a small string transform / id helper / md coercion / tree walk" needs already have an owner here —
the cost of finding the existing one is a `grep`;
the cost of missing it shows up later as drift between two parallel implementations (the user is the one who hits the divergence).

This page is the **index**.
The tables live in three parts, split by responsibility.
Grep all four files together — they are one catalog:

```sh
grep -n 'the_symbol' docs/shared-primitives.md docs/primitives-*.md
```

> The catalog is mirrored (in condensed, review-checklist form) at [`.github/instructions/shared-primitives.instructions.md`](https://github.com/outlmd/outl/blob/main/.github/instructions/shared-primitives.instructions.md), a path-scoped Copilot instruction file (`applyTo: crates/**`).
> When you edit any part, sync that mirror — a `PostToolUse` hook flags drift, but the discipline starts before the hook fires.

For the reuse-first rule (why this matters, past drift incidents, what to do when a primitive doesn't exist yet), see [Contributing → Reuse-first](contributing.md#reuse-first-no-parallel-implementations).

---

## The three parts

| Part | Covers | Read it when |
|---|---|---|
| [Core state, sync, and durability](primitives-core.md) | op log, CRDT tree, HLC, ids, sync engine + transports, locks, `Storage` trait, local backups | you're about to mutate or read converged workspace state |
| [Markdown pipeline](primitives-markdown.md) | parse, render, external coercion + ingest, reconcile / matching / diff, sidecar, outline AST helpers, indices, inline tokenizers, assets | you're about to read or write `.md` / `.outl` |
| [Editing actions and client features](primitives-actions.md) | block mutations, pages + journals, backlinks, code execution, undo/redo, templates, reminders, `@outl/shared` | you're wiring a client gesture to a workspace change |

---

## Full section index

### [Core state, sync, and durability](primitives-core.md)

1. [Workspace lifecycle, op log, and HLC (outl-core)](primitives-core.md#1-workspace-lifecycle-op-log-and-hlc-outl-core)
2. [Tree reads (outl-core + outl-actions::tree)](primitives-core.md#2-tree-reads-outl-core--outl-actionstree)
3. [Sync engine, locks, storage trait](primitives-core.md#3-sync-engine-locks-storage-trait)
4. [Local backups (outl-actions::backup)](primitives-core.md#4-local-backups-outl-actionsbackup)

### [Markdown pipeline](primitives-markdown.md)

1. [Parse / render (outl-md::parse + render)](primitives-markdown.md#1-parse--render-outl-mdparse--render)
2. [External markdown coercion & ingest (outl-md::frontmatter + wikilink, outl-actions::paste + ingest)](primitives-markdown.md#2-external-markdown-coercion--ingest-outl-mdfrontmatter--wikilink-outl-actionspaste--ingest)
3. [Reconcile & matching (outl-md::reconcile + matching + diff)](primitives-markdown.md#3-reconcile--matching-outl-mdreconcile--matching--diff)
4. [Sidecar (outl-md::sidecar + atomic)](primitives-markdown.md#4-sidecar-outl-mdsidecar--atomic)
5. [In-flight outline AST helpers (outl-md::outline_ops)](primitives-markdown.md#5-in-flight-outline-ast-helpers-outl-mdoutline_ops)
6. [Indices and search (outl-md::index + block_index)](primitives-markdown.md#6-indices-and-search-outl-mdindex--block_index)
7. [View helpers for editors (outl-md::view + inline)](primitives-markdown.md#7-view-helpers-for-editors-outl-mdview--inline)
8. [Asset links (outl-md::asset + outl-actions::asset)](primitives-markdown.md#8-asset-links-outl-mdasset--outl-actionsasset)

### [Editing actions and client features](primitives-actions.md)

1. [Block mutations (outl-actions::block + collapsed + todo + quote)](primitives-actions.md#1-block-mutations-outl-actionsblock--collapsed--todo--quote)
2. [Pages and journals (outl-actions::page + journal)](primitives-actions.md#2-pages-and-journals-outl-actionspage--journal)
3. [Backlinks (outl-actions::backlinks)](primitives-actions.md#3-backlinks-outl-actionsbacklinks)
4. [Code-block execution (outl-actions::exec)](primitives-actions.md#4-code-block-execution-outl-actionsexec)
5. [Undo / redo history (outl-actions::history)](primitives-actions.md#5-undo--redo-history-outl-actionshistory)
6. [Templates](primitives-actions.md#6-templates)
7. [Reminders (`remind::`)](primitives-actions.md#7-reminders-remind)
8. [Frontend shared primitives (`@outl/shared`)](primitives-actions.md#8-frontend-shared-primitives-outlshared)

The module-by-module reference for `outl-actions` lives separately in [`outl-actions-surface.md`](outl-actions-surface.md) — this catalog is keyed by intent, that one by module.

---

## When your need isn't in this catalog

If you've grepped honestly and the primitive doesn't exist, that's a fair sign — add it in the upstream crate that owns the concept:

- **`outl-md`** for parse / render / sidecar / inline / tokenizers
- **`outl-actions`** for workspace mutations, ingest, page/journal helpers
- **`outl-core`** for op-log / tree / HLC / storage trait

Then add its row to the matching part **in the same commit**, and sync the mirror at `.github/instructions/shared-primitives.instructions.md`.
The `PostToolUse` hook will flag drift, but the discipline starts before the hook fires.

For the broader reuse-first rule and past drift incidents that justify this catalog, see [Contributing → Reuse-first](contributing.md#reuse-first-no-parallel-implementations).
