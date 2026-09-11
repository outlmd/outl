# Importing a graph

`outl import roam|logseq|obsidian|auto <src> <dst>` runs the adapter-based pipeline in the `outl-import` crate.

Split out of [`cli.md`](cli.md), which is the command reference: the dialect translations, the reconciliation contract and the resumability rules are their own subject and had grown to a third of that document.
`cli.md` carries the invocation; this page carries what the import actually does to your notes.

`outl import` runs the adapter-based pipeline in the `outl-import` crate for every source (`roam` = JSON backup file, `logseq` = graph directory, `obsidian` = vault directory; `auto` detects from the source's shape).
`((uid))` block refs and `{{embed}}`s resolve to real `((blk-XXXXXX))` handles, not page-link fallbacks.
Folded blocks (Roam `open: false`, Logseq `collapsed:: true`) land as `Op::SetCollapsed`.
Each dialect is translated on the way in.
Roam: `__italic__` → `*italic*`, flat `{{[[query]]}}` → ` ```query ` fences.
Logseq: `DOING` and `NOW` → outl's `DOING` prefix (`NOW` also keeps a `state:: now` property, the nuance outl has no separate state for).
`LATER`/`WAITING` → `TODO` + `state::` property, `CANCELED` → `DONE` + `state::`, `[#A]` → `priority::`, `SCHEDULED:`/`DEADLINE:` → `[[date]]` links, `:LOGBOOK:` drawers dropped and counted.
A `DOING` block imported before outl had the state was flattened to `TODO ` + `state:: doing` and is indistinguishable from a real `TODO` in every query and count; re-importing that graph is what fixes it.
Obsidian: frontmatter → `key:: value` properties, wiki-link variants collapse to `[[Note]]`.
Referenced files are pulled into the workspace's `assets/` dir, content-addressed.
A local attachment (`![](../assets/pic.png)`) is copied and a remote image (Roam's firebase URLs) is downloaded; either way the link is rewritten to `[name](assets/<hash>.<ext>)`.
A file that can't be pulled (missing, download failed, over `[assets] max_bytes`) keeps its original link and is counted in `assets missing` — never fatal.
`--no-assets` skips all of that, keeping every original relative/remote link verbatim.
A real (non-dry) import paints a live progress line on stderr — phase, page counter, percentage, current page, elapsed — TTY-only, so piped output stays clean.
`--dry-run` parses and reports without writing a byte — run it against a real backup to measure fidelity before migrating.
`--json` prints the full report (per-feature counts, warnings with location) as JSON.
`--preserve-timestamps` keeps source create/edit times as `created::`/`edited::` block properties (dropped and counted by default).

**Importing twice is destructive, so it's opt-in.**
An import overwrites every `.md` it emits and reconciles the result through the op log, so a second run against a workspace you've been using erases whatever you wrote there since the first import — there is no undo.
`outl import` therefore refuses a destination that already holds content, naming what it found and pointing at the escape hatch.
Pass `--force` when overwriting is exactly what you want; import into a fresh directory otherwise.
`--dry-run` writes nothing and is never blocked.

What counts as "already holds content" is read from the **op log's materialized tree**, not from the `.md` files on disk.
A device paired over iroh receives every op through sync, but only projects a page's `.md` when that page is opened.
A freshly-paired laptop therefore holds your whole graph with an empty `pages/` directory, and a file-counting guard would wave the import straight into it.
Two extra signals round it out: markdown dropped into `pages/` by hand (no sidecar beside it, so the tree cannot see it) also blocks, and so does any `.md` in a destination that isn't an outl workspace at all.

The output of `outl init` is **not** content.
`init` seeds a journal-template page and today's (empty) journal, so `outl init ./notes && outl import roam backup.json ./notes` — the documented migration flow — runs with no flags.
A page only counts once it holds a block with real text.
That distinction matters: `--force` is the flag that destroys, and a guard that fires on the normal flow just teaches you to type it by reflex.

**`outl init --bare` writes no ops at all** — no journal-template page, no journal for today.
It exists for the one case where seeding is wrong: a workspace created only to become a replica of an existing graph, which then joins it with `outl peer pair --ticket`.
Pairing adopts the *host's* workspace id but keeps the *joiner's* ops, so a seeded replica pushes its own `templates/journal` page into the host's graph — two page nodes with one slug, both projecting to `pages/templates/journal.md`.
It is what the [self-hosted server image](self-hosting.md) runs.
Don't use it for a workspace you'll write notes in directly; the journal template won't be there.

**A failed import is resumable, without `--force`.**
The pipeline writes page by page, so a failure at page 40k of 66k leaves the destination half-populated.
For the duration of a real import, `outl import` keeps a marker at `<workspace>/.outl/import-in-progress.json` (adapter, source path, start time) and deletes it on success.
If a run dies, the marker survives and the error message says exactly how to recover.
Re-running the same command then imports again **without** `--force`: everything in that destination came from the run that never finished, so there is nothing of yours to protect.
Delete the destination instead if you'd rather start clean.
A marker that is missing or unparseable is treated as "no unfinished import", so a corrupt file is never a free pass.

**Reconciliation: what the source held vs. what landed.**
The per-feature counts only describe what the pipeline knows it produced — a block lost in the parse would show up in neither the numerator nor the denominator.
So the report also carries a `reconciliation` block (Roam today; other adapters as they start reporting source counts) whose denominators are counted straight off the parsed source:

```text
  reconciliation:
    pages:             3/4 emitted (1 merged, 0 skipped)
    blocks:            12/15 emitted (2 lifted to page props, 1 in skipped pages)
    in the op log:     12/12 emitted blocks confirmed on disk after reconcile
```

Every legitimate reducer is subtracted by name — pages merged onto the same journal date, pages skipped (each listed under `skipped:` with the blocks that went down with it), and blocks promoted into page properties (`blocks_lifted_to_props`).
Whatever is left over is unexplained loss, and the human output says so in a block you can't miss (`UNACCOUNTED CONTENT — the import does not add up`), with the same numbers available under `reconciliation` in `--json`.

The `in the op log` line closes the other half of the contract.
Every other counter in the report is incremented in memory during rendering, before a byte reaches disk — they prove the parser and the renderer agree about your graph, not that your graph is in a workspace.
A page that fails to write, fails to reconcile, or loses blocks in the matcher is invisible to all of them.
So a real import also sums the block entries in each page's sidecar (written by `reconcile_md` straight off the materialized tree) and reports that as `landed_blocks`.
A gap prints as `CONTENT NEVER REACHED THE OP LOG` and makes `balanced` false.
`--dry-run` writes nothing, so it reports the landing as *not measured* rather than as zero-loss: there, `balanced: true` means only that parse and render agree.

Warnings are listed in full under `--json`.
The human output prints the first 20 and states how many it hid.

One counted loss worth knowing about on Roam graphs: a `{{[[TODO]]}}` / `{{[[DONE]]}}` marker in the *middle* of a block keeps its literal `TODO`/`DONE` word but not its task state.
outl models one task per block, driven by the marker at the block's head, so such a block won't answer `outl query --kind=task`.
It's reported as `mid-block tasks` plus a single aggregate warning — one per import, not one per marker.
