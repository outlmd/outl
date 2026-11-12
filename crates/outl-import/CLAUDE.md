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
        reconcile_md stamps sidecars → uid → NodeId → ref_handle
        rewrite goes through the op log (edit_text + re-render)
        collapsed flags land as Op::SetCollapsed
```

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
├── report.rs          # ImportReport (per-feature counts, warnings)
├── emit/
│   ├── mod.rs         # pipeline orchestration + dry-run simulation + landing measurement
│   ├── render.rs      # IR → markdown, placeholder emission, DFS bookkeeping
│   ├── assets.rs      # asset resolution: copy/download + placeholder → link (pre-write)
│   └── resolve.rs     # pass B: uid → handle, edit_text, SetCollapsed
└── adapters/
    ├── scan.rs        # shared low-level scanners (balanced, is_uid, alias_link, parse_prop_line)
    ├── asset_scan.rs  # shared asset-link scanner: CommonMark link → AssetRef + placeholder
    ├── roam/          # mod.rs (JSON → IR) + inline.rs (dialect scanner) + tests.rs
    ├── logseq/        # mod.rs (outline parser) + inline.rs + tests.rs
    ├── logseq_db/     # DB-version EDN export: edn/ (reader) + graph.rs (datoms → entities) + convert.rs / block.rs / props.rs / text.rs
    └── obsidian/      # mod.rs (frontmatter/wikilink policy) + stems.rs (collisions)

tests/
├── common/mod.rs      # shared harness: real Workspace over JsonlStorage in a tempdir
├── roam_e2e.rs
├── logseq_e2e.rs
├── logseq_db_e2e.rs
└── obsidian_e2e.rs    # ports every scenario the legacy pipeline covered
```

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

- ❌ Emitting `((blk-…))` directly from render — handles don't exist yet; that's resolve's job.
- ❌ Rewriting `.md` files in pass B instead of going through `edit_text`.
- ❌ Minting `NodeId`s for imported blocks — `reconcile_md` derives them; fresh ids break convergence.
- ❌ Dropping content silently when a construct doesn't parse — `BlockContent::Verbatim` + a report count is the floor.
- ❌ Writing dialect knowledge into `emit/` — if the emitter needs to know what `__` means, the adapter failed to translate.

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-import --all-targets -- -D warnings`
3. `cargo test -p outl-import`
4. Smoke a real backup with `--dry-run --json` and read the numbers.
