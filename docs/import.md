# Importing a graph

`outl import roam|logseq|logseq-db|obsidian|auto <src> <dst>` runs the adapter-based pipeline in the `outl-import` crate.

Split out of [`cli.md`](cli.md), which is the command reference: the dialect translations, the reconciliation contract and the resumability rules are their own subject and had grown to a third of that document.
`cli.md` carries the invocation; this page carries what the import actually does to your notes.

`outl import` runs the adapter-based pipeline in the `outl-import` crate for every source (`roam` = JSON backup file, `logseq` = graph directory, `logseq-db` = the EDN export of a Logseq DB graph, `obsidian` = vault directory; `auto` detects from the source's shape).
`((uid))` block refs and `{{embed}}`s resolve to real `((blk-XXXXXX))` handles, not page-link fallbacks.
Folded blocks (Roam `open: false`, Logseq `collapsed:: true`) land as `Op::SetCollapsed`.
A Roam block that is only `key:: value` lines becomes block properties; in a block that also has prose, those lines stay in the text, in the order written (Roam shows them as part of the block), escaped on disk as `\key:: value` so outl does not lift them out.
Each dialect is translated on the way in.
Roam: `__italic__` → `*italic*`, an org stamp (`DEADLINE: <2024-03-20 Wed>`, Logseq residue that Roam shows as plain text) kept exactly as written, flat `{{[[query]]}}` → ` ```query ` fences, `{{[[pdf]]: url}}` → a `[name](url)` file link (counted as `pdf embeds`; a local path is copied into `assets/` like any attachment).
Logseq: `DOING` and `NOW` → outl's `DOING` prefix (`NOW` also keeps a `state:: now` property, the nuance outl has no separate state for).
`LATER`/`WAITING` → `TODO` + `state::` property, `CANCELED` → `DONE` + `state::`, `[#A]` → `priority::`, `SCHEDULED:`/`DEADLINE:` → `[[date]]` links, `:LOGBOOK:` drawers dropped and counted.
A `DOING` block imported before outl had the state was flattened to `TODO ` + `state:: doing` and is indistinguishable from a real `TODO` in every query and count; re-importing that graph is what fixes it.
Two Roam pages whose titles land on one outl slug (`Cloudflare` and `cloudflare`, `@Ana` and `Ana`) are one page in outl: they merge, the page with the most blocks keeps the title, and every other spelling becomes an `alias::` name, a title with a comma written as `[[Plan, v2]]` so it stays one name.
A Roam `{{[[table]]}}` block becomes one [pipe-table](markdown-format.md#tables) block: its first row is the header, and a table with merged cells, multi-line or attribute cells, or a cell another block refers to keeps its subtree as it was.
Obsidian: frontmatter → `key:: value` properties, wiki-link variants collapse to `[[Note]]`.
Referenced local files are pulled into the workspace's `assets/` dir, content-addressed.
A local attachment (`![](../assets/pic.png)`) is copied and its link is rewritten to `[name](assets/<hash>.<ext>)`.
A remote image (`![alt](https://…)`, such as Roam's firebase uploads) keeps the link the source wrote, URL and alt text included, and outl renders it from that URL; it is counted in `remote images kept`.
`--download-remote-assets` downloads remote images into `assets/` too and points the link at the local copy, keeping the alt text as written; the URL is not kept, since a link holds one target.
An encrypted upload (`….enc`, from an encrypted Roam graph) is never downloaded: its bytes are ciphertext, so its link always stays as written.
A file that can't be pulled (missing, download failed, over `[assets] max_bytes`) keeps its original link and is counted in `assets missing`, never fatal.
Local files that can't be read are named in one `(graph)` warning (the first five paths, with `RUST_LOG=debug` logging every one), not one line per file.
`--no-assets` skips all of that, keeping every original relative/remote link verbatim.
A real (non-dry) import paints a live progress line on stderr — phase, page counter, percentage, current page, elapsed — TTY-only, so piped output stays clean.
`--dry-run` parses and reports without writing a byte — run it against a real backup to measure fidelity before migrating.
`--json` prints the full report (per-feature counts, warnings with location) as JSON.
`--preserve-timestamps` keeps source create/edit times as `created::`/`edited::` block properties (dropped and counted by default).

`--keep-source-ids` (Roam) lands every source block as exactly one outl block that carries its Roam uid as a `roam-uid::` property, and writes the page's uid the same way in its header (a page merged from several Roam pages lists every uid, comma-separated).
That is the mapping a sync back to Roam needs, and a property is the one place that travels with the block through every outl edit, move and peer sync.
Two shapes that otherwise fold change with it: the attribute blocks at the head of a page (`icon::`, `alias::`, `type::`) stay blocks, with their uid and timestamps, and their attributes are also copied to the page header so outl's page features still read them; and a `{{[[table]]}}` keeps its cell blocks instead of becoming one pipe-table block.
`roam-uid` is bookkeeping, so it is not offered as a key to add.

## Logseq DB graphs (`logseq-db`)

Logseq 2.x keeps a graph in SQLite (`<graph>/db.sqlite`), not in `pages/` + `journals/` markdown.
`outl import logseq` and `outl import auto` refuse such a directory with a message instead of reporting an empty import, and `outl import logseq-db` does not read `db.sqlite` directly either: it needs Logseq's own EDN export, the one export Logseq documents as fully capturing a graph.

1. In Logseq open the graph, then the three-dots menu → **Export graph** → **Export EDN file**.
2. `outl import logseq-db ~/Downloads/<graph>_<timestamp>.edn ~/notes` (`auto` picks it for an `.edn` file with the datoms format).

Only the whole-graph datoms export is accepted.
A page or block EDN snippet, the human-readable `:graph-human` shape and the "standard Markdown" zip fail with a message naming what was found.
The format is read as Logseq 2.0.2 writes it (schema `65.x`); an export from another schema major imports with a warning.

What lands:

| In the export | In outl |
| --- | --- |
| Pages and journals | named pages, and `journals/YYYY-MM-DD.md` from `:block/journal-day` |
| User tags (classes you created) | named pages; a block or page tagged with one gets `#tag` (page: a `tags::` property) |
| Blocks | the outline: `:block/parent`, siblings ordered by `:block/order` (byte-wise, Logseq's fractional index); `:block/collapsed?` → `Op::SetCollapsed` |
| `[[<uuid>]]` to a page, `#[[<uuid>]]` | `[[Page Title]]`, `#tag` (a multi-word tag becomes `[[Multi Word]]`, as for the other importers) |
| `[[<uuid>]]` / `((<uuid>))` to a block | a real `((blk-XXXXXX))` handle |
| Status | `TODO`/`DOING`/`DONE` prefixes; `Canceled` → `DONE` + `state:: canceled`; `Backlog` → `TODO` + `state:: backlog`; `In Review` → `DOING` + `state:: in-review` (outl has no state for those two) |
| Priority | `priority:: low\|medium\|high\|urgent` |
| Deadline, scheduled | `[[YYYY-MM-DD]]` link appended to the text, in the importing machine's time zone (the time of day is not kept) |
| Code blocks | a fenced block with its language |
| Quote blocks | `> ` lines |
| Math, query, cards, template, PDF annotation and asset nodes | the text, kept; counted by kind under `components verbatim` |
| User properties | `key:: value`, the key being the property's title lower-cased (`Reading status` → `reading-status`); several values are joined with `, `; a number, text, page link or date value is rendered, anything else is counted |
| `:block/created-at` / `:block/updated-at` | `created::` / `edited::` with `--preserve-timestamps` |

What is skipped, and where it is counted: built-in pages with no blocks (each one under `skipped:`), whiteboards (`skipped:`, with their blocks), built-in tags, property definitions, closed values and property value nodes, hidden blocks and blocks in the recycle bin (one aggregate `(graph)` warning with the counts), and built-in attributes outl has no equivalent for (`artifacts stripped`, named in a warning).
A reference to a uuid that is not part of the import keeps its `[[<uuid>]]` text, counts under `unresolved` and is named in a warning.

Not imported yet: reading `db.sqlite` directly, assets (the export carries no file bytes — the files sit in `assets/` next to `db.sqlite`), whiteboards, property history, what views and queries *do* (their text is kept), page-level and block-level EDN snippets, the `:graph-human` shape.

**Importing twice is destructive, so it's opt-in.**
An import overwrites every `.md` it emits and reconciles the result through the op log, so a second run against a workspace you've been using erases whatever you wrote there since the first import — there is no undo.
`outl import` therefore refuses a destination that already holds content, naming what it found and pointing at the escape hatch.
Pass `--force` when overwriting is exactly what you want; import into a fresh directory otherwise.
`--force` **replaces** every page the import writes: the blocks a page already held are moved to the trash (each recorded in `.outl/orphans.log` first) and the page ends up with exactly what the source has, including a page that sat in the trash, which comes back without its old blocks.
The report's `replaced_blocks` counts what was discarded.
Pages the source does not have are left as they are; there is no whole-workspace wipe.
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

Every legitimate reducer is subtracted by name — pages merged onto the same journal date, pages skipped (each listed under `skipped:` with the blocks that went down with it), blocks promoted into page properties (`blocks_lifted_to_props`), and Roam table cells folded into their table's block (`blocks_folded_into_tables`).
Whatever is left over is unexplained loss, and the human output says so in a block you can't miss (`UNACCOUNTED CONTENT — the import does not add up`), with the same numbers available under `reconciliation` in `--json`.

The `in the op log` line closes the other half of the contract.
Every other counter in the report is incremented in memory during rendering, before a byte reaches disk — they prove the parser and the renderer agree about your graph, not that your graph is in a workspace.
A page that fails to write, fails to reconcile, or loses blocks in the matcher is invisible to all of them.
So a real import also sums the block entries in each page's sidecar (written by `reconcile_md` straight off the materialized tree) and reports that as `landed_blocks`.
A gap prints as `CONTENT NEVER REACHED THE OP LOG` and makes `balanced` false.
`--dry-run` writes nothing, so it reports the landing as *not measured* rather than as zero-loss: there, `balanced: true` means only that parse and render agree.

Warnings are listed in full under `--json`.
The human output prints the first 20 and states how many it hid.

A Roam task marker right after a list or quote prefix at the head of a block (`- {{[[TODO]]}} x`, `1. {{[[TODO]]}} x`, `> {{[[DONE]]}} x`) is the block's task, as Roam draws it: the block becomes `TODO - x`, `TODO 1. x` or `DONE > x`, with the prefix kept as written.

One counted loss worth knowing about on Roam graphs: a `{{[[TODO]]}}` / `{{[[DONE]]}}` marker in the *middle* of a block keeps its literal `TODO`/`DONE` word but not its task state.
outl models one task per block, driven by the marker at the block's head, so such a block won't answer `outl query --kind=task`.
It's reported as `mid-block tasks` plus a single aggregate warning — one per import, not one per marker.
