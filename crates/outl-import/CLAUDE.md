# CLAUDE.md — outl-import

Adapter-based graph importers.
Today: **Roam** (JSON backup), **Logseq** (graph dir), **Logseq DB** (EDN graph export), **Obsidian** (vault) — every source the CLI accepts routes through this crate; the legacy string pipeline in `outl-cli/src/cmd/import/` is gone.
`outl import auto <src> <dst>` picks the adapter via each adapter's `detect()`.

## Architecture: three stages, one owner each

```text
source on disk (JSON backup / graph dir / vault)
   │
   ▼
[1] SourceAdapter::parse           ← ALL dialect knowledge lives here
   │    produces the typed IR (ir.rs) — no source syntax escapes
   ▼
[2] emit::render                   ← single owner of outl output syntax
   │    IR → .md; refs/embeds become inert placeholders
   │    ((outl-import:<uid>)) / !((outl-import-embed:<uid>))
   ▼
[3] emit::resolve                  ← 2-pass handle resolution
   │    reconcile_md stamps sidecars → uid → NodeId → ref_handle
   │    rewrite goes through the op log (edit_text / SetProp + re-render)
   │    collapsed flags land as Op::SetCollapsed
   ▼
[4] emit::canonicalize_projections ← every `.md` ends up byte-identical
        to what the op log projects, through
        apply_page_md_with_sidecar_if_stale (root invariant 8's guard)
```

Stage 4 exists because the file is a *projection*: one that is correct
but spelled differently is a **stale** projection, and `outl doctor`
says so about the entire fresh import.

## Why the placeholder + 2-pass dance

`((blk-XXXXXX))` handles derive from `NodeId`s, and `NodeId`s only exist after `reconcile_md` stamps the sidecar.
So pass A writes markdown with placeholders outl's tokenizer treats as plain text, reconciles everything, then pass B maps each source UID to its handle through the sidecar's depth-first block list (the same order the renderer counted).

**Pass B mutates through ops, never by rewriting `.md`.**
Editing the file and re-reconciling would push changed blocks through the 3-level matcher, which can re-identify a block (level 3 → fresh id) and dangle every handle already written elsewhere.
`edit_text` + `apply_page_md_with_sidecar` keeps ids stable by construction (root invariant #1: op log is source of truth, files are projections).

Two guards protect the index mapping:

- sidecar block count must equal the renderer's depth-first count, or the whole page degrades to `[[Title]]` links (counted + warned, never mis-wired);
- the target block's `content_hash` must match the rendered text before any edit lands.

## The IR is deliberately minimal

A construct only gets an `Inline` variant when the emitter must **transform or resolve** it (`BlockRef`, `Embed`, `Component`, `CodeSpan`).
Everything already valid in outl (`[[page]]`, `#tag`, `**bold**`, links) travels as `Inline::Text` verbatim — outl's own parser tokenizes it after emission.
Don't add variants for passthrough syntax.

Two page-body shapes (`PageBody`):

- `Outline(Vec<ImportBlock>)` — structured sources (Roam, Logseq); every block typed, UIDs DFS-mapped to sidecar entries.
- `Raw(String)` — free-markdown sources (Obsidian); text-level rewrites only, outl's permissive parser owns the structure downstream.
  Raw pages carry no UIDs, so the resolve pass never touches them.

Adapters with their own collision policy (Obsidian's path-derived suffixes in `adapters/obsidian/stems.rs`) pin the on-disk stem via `ImportPage::stem_override`; everyone else lets the emitter slugify the title.

## Assets: scan in parse, resolve in emit

Referenced files (local attachments, remote images) follow the same
"adapter detects, emit does the IO" split as everything else.

- **parse** (each adapter) runs
  [`adapters::asset_scan::scan_assets`](src/adapters/asset_scan.rs) over
  block text: it finds CommonMark links (`![alt](target)` /
  `[alt](target)`), records each as an `AssetRef` in `graph.assets`
  (`AssetSource::Local(abs)` for a relative path resolved against the
  source file's dir, `AssetSource::Remote(url)` for `http(s)`), and
  swaps the link for an inert `((outl-import-asset:<idx>))` placeholder.
  It does **no asset IO** — never opens, copies, downloads, or stat's a
  file.
  Block refs (`[a](((uid)))`), page links (`[a]([[Page]])`),
  anchors, `mailto:`, other schemes, and our own re-imported
  `assets/<64-hex>` links pass through untouched.
  Wiring points: Obsidian after `convert_image_links`+`rewrite_wikilinks`
  (`obsidian/mod.rs`, base = the note's dir); Logseq in `finalize` before
  inline tokenization (`logseq/mod.rs`, base = the `.md`'s dir); Roam in
  `convert_content` before tokenize (`roam/mod.rs`, base = the backup's
  dir — its images are remote).
- **emit** ([`emit::assets`](src/emit/assets.rs)) runs once, right after
  render and **before** any file is written: it copies each local file
  (`outl_actions::import_asset`) or downloads each remote URL
  (`reqwest::blocking`, rustls, 30 s timeout, body read bounded by
  `max_bytes`; extension from the URL path then `Content-Type`) and
  imports the bytes (`outl_actions::import_asset_bytes`), then rewrites
  every `((outl-import-asset:<idx>))` — in both the page text **and**
  each placeholder-block text — to the final content-addressed
  `[name](assets/<hash>.<ext>)` link.
  A file that can't be pulled keeps
  its original link verbatim (`AssetRef::original`) and lands in
  `assets_missing`; a pull is never fatal.
  `import_assets: false`
  (`outl import --no-assets`) keeps every original link and copies
  nothing.
  Because asset substitution happens before write/reconcile, no
  asset placeholder ever reaches the ref/embed resolve pass or the
  sidecar; `dry_run` does no IO and counts every asset as
  `assets_copied` (optimistic estimate, same asymmetry as
  `refs_page_fallback`).

## The report is the fidelity contract

Everything translated, degraded, or dropped is counted in `ImportReport` — per-feature, serializable, with located warnings.
`outl import roam <backup> <dst> --dry-run --json` answers "what would I lose" with numbers before any migration.
**Silent loss is a bug**: if you add a lossy path, count it.

Three rules that keep the contract honest:

- **Count what landed, not just what you rendered.**
  Every per-feature counter is bumped inside `emit/render.rs`, in memory, **before** a byte reaches disk.
  They prove the parser and the renderer agree; they say nothing about a page that failed to write, failed to reconcile, or lost blocks in `reconcile_md`'s 3-level matcher.
  So `emit::run` ends with `measure_landing`, which sums the block entries of each written page's sidecar — the one artefact `reconcile_md` writes *from* the materialized tree — into `landed_pages` / `landed_blocks` and sets `landing_measured`.
  A gap makes `Reconciliation::balanced` false and prints `CONTENT NEVER REACHED THE OP LOG`.
  `dry_run` leaves `landing_measured` false: it writes nothing, and claiming zero loss there would be the same lie the counters used to tell.
  **If you add a stage downstream of render, it has to be visible to this measurement or it is silent loss.**
- **Count the denominator, not just the numerator.**
  Per-feature counts only describe what the pipeline *knows* it emitted; a block lost in the parse hides from both sides.
  So an adapter also reports `source_pages` / `source_blocks`, counted straight off the parsed source structure, and `ImportReport::finalize` (called once by `run_import*` / `dry_run`) fills `reconciliation` with the input-vs-output balance.
  Every legitimate reducer is subtracted **by name**: `pages_merged` (two source pages onto one journal date), `skipped` (with `blocks_dropped` per entry), `blocks_lifted_to_props` (a leading pure-attribute block promoted to page props).
  What's left over is real, unexplained loss, and prints as an unmissable `UNACCOUNTED CONTENT` block.
  A new adapter that doesn't set the source counts simply gets no `reconciliation` (a fabricated "0/0, balanced" would be worse than silence); wire them up when you add the adapter.
  Go through `report.count_source(pages, blocks)` — **calling it is what marks the block denominator as measured**, which is how "an empty graph has 0 blocks" is told apart from "this source has no blocks to count".
  A free-markdown source (Obsidian: the page is its unit, its blocks are the ones outl's parser makes *after* the text is written) calls `count_source_pages` instead and reconciles pages only.
  Reporting `0` as its block denominator read as "every block vanished", and printing `blocks: 0` after importing someone's whole vault read worse.
- **A page the import *adds* comes off the emitted side, by name.**
  `pages_merged` is the reducer everyone remembers; `pages_added_as_templates` / `blocks_copied_into_templates` are its mirror image — a Logseq block template is copied into an outl template *page*, so the emitted count legitimately exceeds the source's.
  `finalize` subtracts them, and the human output names them, because `8/8 emitted` with two lifted templates otherwise reads as a coincidence rather than as six source pages arriving.
- **Never `continue` past a construct you're dropping.**
  Use `report.skip(path, reason, blocks)` — the block count matters, because dropping a page drops its whole subtree.
- **Aggregate high-frequency warnings, don't emit one per hit.**
  Roam's mid-block `{{[[TODO]]}}`/`{{[[DONE]]}}` keeps its literal word but loses the task state (outl models one task per block, at its head).
  A real graph carries tens of thousands of them, so the adapter bumps `tasks_midtext_literal` per occurrence and emits exactly **one** `(graph)`-scoped warning at the end of `parse`.

## Files

```
src/
├── lib.rs             # run_import / dry_run, ImportDest, ImportOptions
├── adapter.rs         # SourceAdapter trait + ImportError
├── ir.rs              # ImportGraph / ImportPage / PageBody / ImportBlock / Inline
├── report/            # mod.rs (counts + finalize's balance) + print.rs (the human
│                      #   summary) + tests.rs
├── emit/
│   ├── mod.rs         # pipeline orchestration + dry-run simulation + landing measurement
│   ├── render.rs      # IR → markdown, placeholder emission, DFS bookkeeping
│   ├── assets.rs      # asset resolution: copy/download + placeholder → link (pre-write)
│   └── resolve.rs     # pass B: uid → handle, edit_text, SetCollapsed
└── adapters/
    ├── scan.rs        # shared low-level scanners (balanced, is_uid, alias_link, parse_prop_line)
    ├── asset_scan.rs  # shared asset-link scanner: CommonMark link → AssetRef + placeholder
    ├── roam/          # mod.rs (JSON → IR) + inline.rs (dialect scanner) + tests.rs
    ├── logseq/        # mod.rs (adapter) + files.rs (what is read / refused) + names.rs
    │                  #   (filename → title) + outline/ (parser + org.rs) + inline.rs
    │                  #   + templates.rs (block `template::` → template page)
    ├── logseq_db/     # DB-version EDN export: edn/ (reader) + graph.rs (datoms → entities) + convert.rs / block.rs / props.rs / text.rs
    └── obsidian/      # mod.rs (frontmatter/wikilink policy) + stems.rs (collisions)

tests/
├── common/mod.rs      # shared harness: real Workspace over JsonlStorage in a tempdir
├── common/roam.rs     # Roam fixture shared by both Roam e2e binaries
├── roam_e2e.rs
├── roam_page_merge_e2e.rs  # pages sharing a slug / journal date merge
├── roam_reconciliation_e2e.rs  # report reconciliation + landing measurement
├── logseq_e2e.rs
├── logseq_refs_e2e.rs # every ref/embed spelling Logseq writes, end to end
├── logseq_db_e2e.rs
└── obsidian_e2e.rs    # ports every scenario the legacy pipeline covered
```

## The Logseq adapter (`logseq/`)

Five modules, one concern each — `mod.rs` is the adapter and page naming, and everything a reviewer is likely to get wrong lives in its own file:

- **`files.rs`** owns *which files are read and what is said about the rest*, and therefore the reconciliation's page denominator.
  The split that matters: a **document** (`.org`, a whiteboard, any other extension under `pages/`) counts in `source_pages` and goes in `skipped`; a file that was never a page (`logseq/config.edn`, `assets/`, `draws/`) is in neither and goes into **one** aggregate `(graph)` warning.
  Get that backwards and `finalize`'s `pages_accounted` stops matching `source_pages`, so `UNACCOUNTED CONTENT` cries wolf over a config file.
  `pages/` and `journals/` are walked **recursively**, because Logseq reads them that way.
- **`names.rs`** owns *filename → page title*, ported from Logseq's own `tri-lb-title-parsing` / `legacy-title-parsing` (`deps/graph-parser/src/logseq/graph_parser/extract.cljc`).
  **`___` is the namespace separator (`/`), not a space** — reading it as a space flattened every `[[parent/child]]` page in a modern graph into one unrelated title.
  A missing `:file/name-format` reads as `:triple-lowbar` here, where Logseq's own importer defaults to legacy; the module doc carries the reasoning (the two guesses are not symmetric — legacy turns every `.` into a separator, so `node.js` becomes `node/js`).
  A `;;`-commented key in `config.edn` is documentation, not a setting: the shipped config has most keys commented out, and reading one would flip the encoding for every page in the graph.
- **`outline/`** is the parser; `outline/org.rs` holds the two org-syntax constructs (`SCHEDULED:`/`DEADLINE:` stamps, `#+BEGIN_` blocks).
  A `((uuid))` in a **property value** becomes a ref placeholder here, because a property never reaches the inline tokenizer.
- **`templates.rs`** copies a block carrying `template::` into a `templates/<name>` page.
  It **copies**: the source block stays where the user wrote it, and every copied `uid` is stripped, or two blocks would claim one uid and the resolve pass could point an existing `((ref))` at the template.

## The Logseq DB adapter (`logseq_db/`)

Reads `export-type :graph` / `graph-format :datoms` (Logseq 2.0.2: "Export graph > Export EDN file"), **not** `db.sqlite`.
The module doc of `logseq_db/mod.rs` cites the Logseq sources the format was checked against; trust them over this summary.

- A hand-written EDN reader (`edn.rs`, `edn/atoms.rs`) — no dependency, errors carry line/column.
  The export's own types stay private to the module.
- `graph.rs` validates the envelope (any other export shape fails with its name) and groups the `[e a v]` triples per entity.
- `convert.rs` decides page / block / structural per entity and rebuilds each outline from `:block/parent` + `:block/order` (byte-wise); `block.rs` converts one block; `props.rs` owns status/priority/deadline and user properties; `text.rs` rewrites `[[<uuid>]]` / `#[[<uuid>]]` / `((<uuid>))` into the spellings the Logseq inline tokenizer (`logseq::inline::tokenize`, reused, not copied) already understands, so block refs reach the IR's `BlockRef`.
- **The reconciliation closes by construction.** Every skip entry is a source page (`skipped.len()` is `pages_skipped`), so only things counted in `source_pages` get a `report.skip`; structural nodes (property definitions, closed values, built-in tags, value nodes, recycled / hidden blocks) go in **one aggregate `(graph)` warning** instead.
- A DB graph *directory* is refused by `logseq`, `logseq-db` and `auto` through `logseq_db::db_graph_dir_message` — one owner for that wording.

Dialect boundaries worth remembering:

- Roam `__x__` is *italic* → translated to `*x*`; Logseq is CommonMark, so `__x__` stays bold — no formatting swaps there.
- Roam components are colon-shaped (`{{[[embed]]: ((uid))}}`); Logseq's are space-shaped (`{{embed ((uid))}}`).
- Roam's flat `{and:}` queries translate to ` ```query ` fences; Logseq's sexp queries stay verbatim components.

## Adding a new adapter (Bear, Notion, …)

1. `src/adapters/<source>/mod.rs` implementing `SourceAdapter` — parse only, never write.
2. All inline dialect translation in the adapter (see `roam/inline.rs` / `logseq/inline.rs` for the scanner shape); reuse `adapters/scan.rs` primitives.
3. Line-level source constructs map to IR fields, not text: task states → `TaskState`, `collapsed:: true` → `collapsed`, `key:: value` under a bullet → `props`, `SCHEDULED:`/`DEADLINE:` → `[[date]]` links in text (issue #63 model).
4. Outliner source → `PageBody::Outline`; free-markdown source → `PageBody::Raw` + text-level rewrites (see the Obsidian adapter).
5. Golden tests: one case per fidelity-matrix row, plus an e2e in `tests/` on the shared harness.
6. Route the CLI source in `outl-cli/src/cmd/import.rs::run` and wire `detect()` into `auto_detect` in the same PR.

## Anti-patterns

- ❌ Writing to stdout from this crate.
  `ImportReport::human_summary` returns the text and `outl-cli` prints it through its closed-pipe guard; a `println!` here is outside that guard and panics on `outl import | head`.
- ❌ Emitting `((blk-…))` directly from render — handles don't exist yet; that's resolve's job.
- ❌ Rewriting `.md` files in pass B instead of going through `edit_text`.
  A marker in a **property value** is covered by this too: resolve it with `Op::SetProp`, not through the file-fallback sweep.
  The sweep is a fallback — routing every page with a `related:: ((uuid))` through it cost one read, one `write_atomic` and one `reconcile_md` per page (measured: 25.4 s → 7.6 s of system time on a 2,028-page graph).
- ❌ Writing a page header in source order. The order is `outl_actions::tree::sort_in_presentation_order`'s, because the `.md` is a projection of the op log and the op log has no property order — a header in any other order is a **stale projection the moment it is written**, and `outl doctor` reports the whole import as needing `--repair`.
- ❌ Reading `___` in a Logseq filename as anything but `/`.
- ❌ Minting `NodeId`s for imported blocks — `reconcile_md` derives them; fresh ids break convergence.
- ❌ Dropping content silently when a construct doesn't parse — `BlockContent::Verbatim` + a report count is the floor.
- ❌ Writing dialect knowledge into `emit/` — if the emitter needs to know what `__` means, the adapter failed to translate.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-import --all-targets -- -D warnings`
3. `cargo test -p outl-import`
4. Smoke a real backup with `--dry-run --json` and read the numbers.
