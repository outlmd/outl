# Importing a graph

Bring your notes from Roam Research, Logseq or Obsidian into an outl workspace.
Your source is only read, never changed.
The import keeps pages, journals, nesting, tasks, properties, links, block references and attachments, and it tells you what it could not carry over.

## The five-minute path

```bash
# 1. Measure first. Writes nothing, always safe.
outl import logseq ~/Logseq/work ~/notes --dry-run

# 2. Read the report. Then take a checkpoint if ~/notes already exists.
outl -w ~/notes backup now -m "before import"

# 3. Import for real.
outl import logseq ~/Logseq/work ~/notes

# 4. Confirm.
outl -w ~/notes doctor
```

Pick the format and give the source as the second argument:

| You have | Command | `<src>` is |
|---|---|---|
| A Roam backup | `outl import roam <file> <dst>` | the `.json` from Roam's **Export all, JSON** |
| A Logseq file graph | `outl import logseq <dir> <dst>` | the graph folder (it holds `pages/` and `journals/`) |
| A Logseq DB graph (2.x) | `outl import logseq-db <file> <dst>` | the `.edn` from **Export graph, Export EDN file** ([steps](#logseq-db-graphs-logseq-db)) |
| An Obsidian vault | `outl import obsidian <dir> <dst>` | the vault folder |
| Not sure | `outl import auto <src> <dst>` | any of the above; outl detects it from the shape |

`<dst>` is the new workspace.
It is created if it does not exist, so `outl init` first is optional.
A real run ends with `Next: run outl --workspace <dst> to open the imported workspace.`

This is a real dry run on a three-page Logseq graph:

```console
$ outl import logseq lg imp --dry-run
Import summary (logseq):
  pages:               2
  journals:            1
  blocks:              5
  block refs:          0 resolved, 0 page-fallback, 0 unresolved
  embeds:              0 block, 0 page
  tasks:               LATER: 1, TODO: 1
  properties:          0 page-level, 2 block-level
  org dates:           1 became [[date]] links
  reconciliation:
    pages:             3/3 emitted (0 merged, 0 skipped)
    blocks:            5/5 emitted (0 lifted to page props, 0 in skipped pages)
    in the op log:     not measured — a dry-run writes nothing, so every count above stops at render
```

**How to read the report.**
`reconciliation` is the line that matters.
`3/3 emitted` means every page the source held is in the output, and `5/5` that every block is.
When the numbers do not add up the output says `UNACCOUNTED CONTENT`, and the same numbers are in `--json`.
On a real run, the last line (`in the op log`) confirms the blocks reached the op log, not just the files.

## Flags you will actually use

| Flag | Use it when |
|---|---|
| `--dry-run` | Always first. Parses and reports, writes nothing. |
| `--json` | You want the full report (every warning with its location) in a file. The human output shows the first 20 warnings. |
| `--preserve-timestamps` | You want source create and edit times kept as `created::` and `edited::` properties. Dropped and counted otherwise. |
| `--no-assets` | You want links left exactly as written and no files copied into `assets/`. |
| `--download-remote-assets` | You want remote images (Roam's uploads) stored locally. Off by default: the URL stays. |
| `--keep-source-ids` | Roam only. Each block keeps its Roam uid as a `roam-uid::` property, for a future sync back. |
| `--force` | The destination already holds notes and you want the import to replace the pages it writes. Read "Importing twice is destructive" below first. |

## When something goes wrong

| You see | It means | Do |
|---|---|---|
| `Error: <dst> already holds N page(s), starting with ...` | The destination has content, and importing again overwrites with no undo. | Import into a fresh folder. Or, if replacing is what you want, back up and add `--force`. |
| `the import stopped partway and <dst> is left half-populated` | A crash or a bad file mid-run. | Re-run the same command: it resumes without `--force`. To start over, delete `<dst>`. |
| `roam backup JSON: invalid type: map, expected a sequence` | The file is not a Roam "Export all, JSON" backup. | Export again from Roam and pick JSON. |
| `unknown import source: foo (expected: roam, logseq, logseq-db, obsidian, auto)` | Typo in the format. | Use one of the five names. |
| `UNACCOUNTED CONTENT` | Blocks or pages are missing from the output, and nothing explains it. | Keep the source, re-run with `--json`, and open an issue with the report. |
| `CONTENT NEVER REACHED THE OP LOG` | Files were written but some blocks are not in the log. | Same: do not delete the source. |
| A `(graph)` warning about files it could not read | Missing or unreadable attachments. They keep their original links. | Add `-v` to log every path: `outl -v import ...`. |
| Logseq says "DB graph" | Logseq 2.x keeps notes in SQLite and has no markdown to read. | Use `logseq-db` with the EDN export. |

Everything below is what the importers translate, refuse and count.

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
`LATER`/`WAITING` → `TODO` + `state::` property, `CANCELED` → `DONE` + `state::`, `[#A]` → `priority::`, `:LOGBOOK:` drawers dropped and counted.
Every marker Logseq accepts is a task, `WAIT`, `IN-PROGRESS` and `STARTED` included (the last two are its spellings of `DOING`).
`SCHEDULED:` / `DEADLINE:` become a `[[date]]` link in the text **and** a `scheduled::` / `deadline::` property, so which kind of date it was survives and is queryable; a time of day or repeater rides along in the property (`scheduled:: [[2026-11-20]] 09:00 .+1w`).
`#+BEGIN_SRC` / `#+BEGIN_QUOTE` org blocks become a fenced code block and `> ` lines — the shapes the `logseq-db` adapter already produced for the same constructs.
Free text before a page's first bullet (Logseq's "pre-block", common in a file written or edited outside Logseq) becomes a block instead of being dropped.
A `((uuid))` inside a **property value** (`related:: ((uuid))`) resolves to a real handle like one in prose, through the op log.
`{{embed ((uuid))}}` resolves, and so do the two spellings that used to arrive as plain text: the older triple-brace `{{{embed ((uuid))}}}` and a missing space (`{{embed((uuid))}}`).

**Page names come from the filename, in whichever of Logseq's two encodings the graph uses.**
`:file/name-format :triple-lowbar` (every graph Logseq has created since 0.6.7) writes `/` as `___` and percent-encodes the rest, so `pages/projeto___outl.md` is the page `projeto/outl` and `pages/com%3Ados.md` is `com:dos`.
A graph whose `logseq/config.edn` says `:legacy` is read the old way instead (`.` is the separator).
With no config, or no key in it, outl reads the modern format: Logseq's own file→DB importer defaults to legacy there, but it wrote that config and knows an old graph when it sees one, and guessing legacy on a modern graph renames `node.js` to `node/js`.
The title keeps the slash (`title:: projeto/outl`), the slug stays flat (`pages/projeto-outl.md`), and `[[projeto/outl]]` resolves through the title index.

**A Logseq block template becomes an outl template page.**
A Logseq template is a block carrying `template:: Name`; an outl template is a *page* with that property ([templates.md](templates.md)), so the property alone used to import as inert block metadata and `/template` showed nothing.
Each block template is now **copied** into `templates/<name>` — the original block stays where you wrote it — and `template-including-parent:: true` keeps the carrying block as part of the body.
A page that already has page-level `template::` is left alone, two templates with one name keep the first and warn, and a template with no body is reported rather than created as a name that stamps nothing.
The copies are counted (`templates lifted`) and subtracted by name in the reconciliation, since they are pages the import added rather than pages the source had.
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
Local files that can't be read are named in one `(graph)` warning (the first five paths, and `outl -v import ...` logs every one), not one line per file.
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
Pointing `outl import logseq` at a zip is the obvious thing to try after Logseq's "Export as standard Markdown", so that case says what to do with it: unzip it and point outl at the directory, or take the EDN export instead, which keeps more (refs, task state, properties).
An unzipped markdown export is a graph directory like any other — `detect()` accepts one that holds only `pages/` or only `journals/`, which is what such an export (or a graph whose owner never used journals) looks like.
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

**Every `.md` the import writes is already the projection of the op log.**
A page's file is a projection, so one that is correct but spelled differently from what the tree projects is a *stale* projection — and `outl doctor` says exactly that about it.
Reporting a fresh import as "N page(s) hold a stale `.md` projection … run `outl doctor --repair`" is a terrible first sentence to read about a graph you have just moved and still only half-trust, so the pipeline closes with a pass that leaves every file byte-identical to its projection.
It goes through the same `apply_page_md_with_sidecar_if_stale` every other writer uses, so [root invariant 8](../CLAUDE.md#critical-invariants-never-violate) still applies: a file holding a line the op log never saw is refused, not overwritten, and that refusal is warned about by name.
`outl doctor` on a freshly imported workspace reports `integrity OK`.

**Importing twice is destructive, so it's opt-in.**
An import overwrites every `.md` it emits and reconciles the result through the op log, so a second run against a workspace you've been using erases whatever you wrote there since the first import — there is no undo.
`outl import` therefore refuses a destination that already holds content, naming what it found and pointing at the escape hatch.
Pass `--force` when overwriting is exactly what you want; import into a fresh directory otherwise.
`--force` **replaces** every page the import writes: the blocks a page already held are moved to the trash (each recorded in `.outl/orphans.log` first) and the page ends up with exactly what the source has, including a page that sat in the trash, which comes back without its old blocks.
The page's own properties are replaced too: any page-level property the source no longer carries is cleared through the op log (the page's identity keys stay), and the ones it does carry are set fresh.
The report's `replaced_blocks` and `replaced_props` count what was discarded.
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
So the report also carries a `reconciliation` block — for **every** importer — whose denominators are counted straight off the parsed source:

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

**What each importer refuses, and where it says so.**
A file the importer will not read is either a *document* or an *attachment*, and the difference decides where it is reported.
A document counts in the page denominator and is named in `skipped:` with a reason; an attachment is not a page, so it stays out of the denominator and is reported once, in aggregate, in a `(graph)` warning.
Neither is ever silent: a file dropped with no trace is the one import bug you only discover months later, when the original graph is already stale.

| Source | Named in `skipped:` (a document) | One aggregate warning (not a page) |
| --- | --- | --- |
| **Logseq** | `.org` files, `whiteboards/*.edn`, any other extension under `pages/` or `journals/` | `assets/`, `logseq/` (config, custom CSS, backups), `draws/`, `version-files/`, loose files in the graph root |
| **Logseq DB** | built-in pages with no blocks, whiteboards | built-in tags, property definitions, closed values, value nodes, hidden and recycled blocks |
| **Obsidian** | `.canvas`, `.base` | every other extension (attachments); `.obsidian/` and dotfiles are vault config |
| **Roam** | a page the backup holds with no usable title | — |

`pages/` and `journals/` are read **recursively**, because Logseq reads them that way: a page filed into a subdirectory is a page, and the earlier depth-1 scan dropped it silently.

**A free-markdown source has no block denominator.**
An Obsidian vault is prose, not an outline: its blocks are the ones outl's own parser makes after the text is written, so there is no source count to compare them against.
That reconciliation reports pages only and says so, rather than printing `0` and implying every block vanished.
The block count it *can* report is the one measured in the op log after reconcile.

Warnings are listed in full under `--json`.
The human output prints the first 20 and states how many it hid.

A Roam task marker right after a list or quote prefix at the head of a block (`- {{[[TODO]]}} x`, `1. {{[[TODO]]}} x`, `> {{[[DONE]]}} x`) is the block's task, as Roam draws it: the block becomes `TODO - x`, `TODO 1. x` or `DONE > x`, with the prefix kept as written.

One counted loss worth knowing about on Roam graphs: a `{{[[TODO]]}}` / `{{[[DONE]]}}` marker in the *middle* of a block keeps its literal `TODO`/`DONE` word but not its task state.
outl models one task per block, driven by the marker at the block's head, so such a block is not a task.
It's reported as `mid-block tasks` plus a single aggregate warning — one per import, not one per marker.
