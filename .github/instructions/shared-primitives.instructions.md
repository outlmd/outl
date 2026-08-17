---
applyTo: "crates/**"
---

<!-- Mirror of docs/shared-primitives.md + docs/primitives-*.md, for Copilot review.
     Both sides must change together — .claude/hooks/catalog-sync-guard.sh enforces it. -->

# Shared primitives catalog

The catalog is indexed at [`docs/shared-primitives.md`](../../docs/shared-primitives.md).
Its rows live in [`primitives-core.md`](../../docs/primitives-core.md), [`primitives-markdown.md`](../../docs/primitives-markdown.md) and [`primitives-actions.md`](../../docs/primitives-actions.md); the index says which part owns what.
Before approving a helper, grep all four (`docs/shared-primitives.md docs/primitives-*.md`) and scan the relevant sub-table.
If the diff adds a primitive that overlaps with a catalog entry, it is a duplicate — block the PR and point at the existing function with `file:line`.

**Review checklist on every PR that adds a helper:**

- Does the new function name / signature describe something already in the catalog?
  If yes → blocker, point at the existing one.
- Does the PR add a `normalize`, `coerce`, `strip`, `slugify`, `hash`, `derive`, or `extract` helper without grepping the catalog first?
  Ask: "did you check `<catalog entry>` before writing this?"
- Does the new code create a page / write `.md` / mint a `NodeId` / build a `LogOp` outside the catalog primitives?
  Block — that's how invariants drift.
- Does the PR add a new `pub fn|struct|enum|const` in `crates/outl-{core,md,actions}/src/`?
  The new symbol **must** appear in the Shared primitives catalog (the local `doc-sync-guard.sh` + `catalog-sync-guard.sh` hooks enforce this pre-merge; the same rule applies in review).

Recently added — durability primitives.
**Block any PR that reads a file it is about to write back with `read_to_string(..).unwrap_or_default()`.**
That is the silent-page-wipe bug: a failed read parses as an empty AST, gets rendered, and atomically replaces a full page with nothing.

| Intent | Use this | File |
|---|---|---|
| Read a `.md` you are about to mutate and write back — missing file → empty, **every other I/O error propagates** | `outl_md::atomic::read_for_rewrite` | `crates/outl-md/src/atomic.rs` |
| Build one sidecar block entry — **the only way to build one** unless you're preserving an expanded `ref_handle`; keeps `content_hash`, `ref_handle` and the level-2 `text` derived from the same revision. A hand-rolled literal is how they end up describing different revisions | `outl_md::sidecar::SidecarBlock::from_text(id, line, indent, text)` | `crates/outl-md/src/sidecar.rs` |
| Sidecar format version — currently `2`. **Block any PR that bumps it for an additive field.** Feature detection is by field *presence*, never by version number; an already-shipped binary rejects a higher version, rebuilds the sidecar from scratch, and duplicates every block. Bump only when an existing field changes meaning or encoding. Never hardcode the integer in a `Sidecar` literal | `outl_md::sidecar::SIDECAR_VERSION` / `MIN_READABLE_SIDECAR_VERSION` | `crates/outl-md/src/sidecar.rs` |
| Low-level crash-safe write (prefer the `journal::write_md_atomic` wrapper) | `outl_md::atomic::write_atomic` | `crates/outl-md/src/atomic.rs` |
| Path of a workspace's orphan log — **every `reconcile_md` call must pass this**, never `None`. One owner: `outl_ws`'s `Paths::at` derives from it | `outl_actions::sync::orphans_log_path` / `SyncEngine::orphans_log` | `crates/outl-actions/src/sync.rs` |

Recently added — device identity.
**Block any PR that reads the write actor from `<workspace>/.outl/config.toml`.**
That file rides the file-sync surface (Syncthing / Dropbox / NFS / git all replicate `.outl/`), `flock(2)` is machine-local, so two devices reading one `actor_id` both append to one `ops-<actor>.jsonl` and lose ops with no error.

| Intent | Use this | File |
|---|---|---|
| Resolve which actor this **device** writes as for a workspace (migration-safe; the only supported entry point for CLI / TUI / MCP / embedders) | `outl_ws::actor::resolve_device_actor` | `crates/outl-ws/src/actor.rs` |
| Device-local actor store — per **workspace instance** (`actor_for_instance`, keyed by `WorkspaceId` **plus the canonical root path**, so a copied directory forks instead of sharing an op log, while a moved one keeps its actor) and device-wide (`device_actor`, the Tauri clients' `<dir>/actor`) | `outl_core::DeviceStore` (errors: `outl_core::DeviceError`) | `crates/outl-core/src/device/mod.rs` |
| Stable fingerprint of one physical device — the claim marker that lets exactly one device adopt a legacy `config.toml` actor | `outl_core::MachineId` (via `DeviceStore::machine_id`) | `crates/outl-core/src/device/mod.rs` |
| Directory holding this device's identity files (`$OUTL_DEVICE_DIR`, else `$XDG_CONFIG_HOME/outl`, else `~/.config/outl`) — **never** inside a workspace | `outl_core::device_dir` | `crates/outl-core/src/device/mod.rs` |
| Whether an actor binding may be dropped — the **single owner** of that verdict, so a listing and a prune cannot disagree (root gone **and** its parent present **and** past the TTL; anything unreadable keeps the binding) | `outl_core::BindingVerdict`, `outl_core::ActorBinding`, `outl_core::STALE_BINDING_TTL` (via `DeviceStore::actor_bindings` / `stale_actor_bindings` / `prune_binding`) | `crates/outl-core/src/device/gc.rs` |
| Device-store scratch files a killed writer left half-published (never bindings, never backed up) | `outl_core::STALE_SCRATCH_TTL` (via `DeviceStore::stale_scratch` / `prune_scratch`) | `crates/outl-core/src/device/gc.rs` |

| Snapshot / restore a workspace locally (git-backed; the git dir lives **outside** the workspace, so no file transport replicates it and the user's own repo in that folder is never touched). Use `git_available`, never a raw `git` spawn | `outl_actions::backup::{init, snapshot, snapshot_best_effort, list, restore, repo_dir, git_available, is_initialized, BackupRepo, BackupEntry, BackupError}` + `outl_config::BackupCfg` | `crates/outl-actions/src/backup/` |
| Wire a client up to periodic backups — one call, detached thread, interval floor read back out of git, never on the edit or quit path | `outl_actions::backup::{spawn_auto_pass, maybe_snapshot, STARTUP_DELAY}` | `crates/outl-actions/src/backup/auto.rs` |

Also block any `reconcile_md(.., None)` outside a test.
Matching level 3 trashes the blocks it can't place, and `outl-md`'s hard rule is that they are recorded in `orphans.log` first — passing `None` is how the desktop and mobile boot paths deleted silently.

Recently added — bulk-delete volume guards (catalog: `docs/primitives-markdown.md` → "Reconcile & matching").
**Block any new caller that turns matching orphans into `Move(node, TRASH_ROOT)` through the raw `match_blocks`.**
Level 3 treats 1 orphan and 5,000 identically, so a `.md` that arrived truncated (iCloud placeholder, half-flushed write) empties a page as quietly as deleting one bullet.

| Intent | Use this | File |
|---|---|---|
| Match blocks with the volume of the resulting deletion checked first — `Err` over the **whole** pass, never a shortened orphan list | `outl_md::matching::guard::match_blocks_guarded` → `MatchGuardError` | `crates/outl-md/src/matching/guard.rs` |
| The thresholds (500 orphans absolute, 0.75 of the page, ratio off under 20 known blocks) and the explicit opt-out a caller wires to a user-facing `--force` | `outl_md::matching::guard::{OrphanGuard, OrphanVolume}` (`OrphanGuard::Disabled` is the escape hatch, reached by `outl reconcile --allow-bulk-delete`) | `crates/outl-md/src/matching/guard.rs` |
| `reconcile_md` with an explicit orphan-volume policy — `reconcile_md` itself delegates here with `OrphanGuard::Enforced` | `outl_md::reconcile::reconcile_md_with_guard` | `crates/outl-md/src/reconcile.rs` |
| Whether a sidecar's blocks can answer "does the log know this line" at all — `false` only for a pre-0.11 sidecar whose entries all carry `text: ""`; an empty verdict from a reference that cannot answer is not permission to write | `outl_md::unlogged::sidecar_can_answer` (re-exported `outl_actions::sidecar_can_answer`) | `crates/outl-md/src/unlogged.rs` |
| How much content a `outl doctor --repair` pass would remove, measured **before** the write, plus the ceilings past which it needs `--force` | `crate::cmd::doctor::{RepairVolume, RepairScope}` | `crates/outl-cli/src/cmd/doctor/{repair,mod}.rs` |

Recently added — recovering text an `Op::Edit` truncated (catalog: `docs/primitives-actions.md` §2, `docs/primitives-core.md` §1).
**Block a second "diff the op history for a truncation" implementation** — `outl-cli`'s `recover` subcommand must call these, not re-walk `Workspace::block_text_history` itself.

| Intent | Use this | File |
|---|---|---|
| Scan the tree for a block whose current text is a proper prefix of an earlier `Op::Edit` revision (truncated, and the dropped tail is still in the log) | `outl_actions::scan_truncated_blocks` → `TruncatedBlock` | `crates/outl-actions/src/recover.rs` |
| Write a recovered revision back as a **new** `Op::Edit`; refuses when the block changed since the scan | `outl_actions::restore_truncated_block` | `crates/outl-actions/src/recover.rs` |
| Every intermediate text a block held, replayed from **storage** (never the resident log / text cache, so a snapshot boot can't silently shorten it) | `outl_core::Workspace::block_text_history` | `crates/outl-core/src/workspace/text_history.rs` |

Recently added — check these before writing a parallel reminder helper (catalog: `docs/primitives-actions.md` → "Reminders"):

| Intent | Use this | File |
|---|---|---|
| **When does a `remind::` rule next fire?** — pure, clock-free, THE single owner. Never re-derive it in TS / Swift / an OS bridge | `outl_actions::next_fire_at` | `crates/outl-actions/src/reminders/schedule.rs` |
| Every reminder in the workspace with its next fire resolved (tree-driven — the `.md` is projected asynchronously, so disk is stale right after authoring) | `outl_actions::scan_reminders` | `crates/outl-actions/src/reminders/scan.rs` |
| Every node carrying a property key, without walking the tree | `outl_core::tree::Tree::nodes_with_property` | `crates/outl-core/src/tree/mod.rs` |
| Parse a `remind::` value (permissive — a bad rule warns, never drops the block) | `outl_md::parse_remind` | `crates/outl-md/src/remind.rs` |
| Silence a reminder across every device | `outl_actions::snooze` / `snooze_until` (`Op::SnoozeRemind`) | `crates/outl-actions/src/reminders/mod.rs` |
| Deliver what came due + the device-local fired log. In `outl-actions` because every client delivers, the TUI included | `outl_actions::take_due` | `crates/outl-actions/src/reminders/fired.rs` |
| Format "in 3h" / bucket a reminder list for a GUI | `@outl/shared` `formatNextFire` / `groupReminders` | `crates/outl-frontend-shared/src/api/commands.ts` |
| **Mark a block DONE outright** (cancels its rule). Not `cycle_todo` — on a block with no marker, one cycle lands on `TODO` and arms the nag | `outl_actions::todo::set_todo` | `crates/outl-actions/src/todo.rs` |

Recently added — check these before writing a parallel template helper (catalog: `docs/primitives-actions.md` → "Templates"):

| Intent | Use this | File |
|---|---|---|
| Inject a `params` binding into a callable template's source (serde_json-escaped, language-canonicalized) | `outl_actions::inject_call_params` | `crates/outl-actions/src/template/call.rs` |
| The template name invoked by a ` ```call:<name> ` fence | `outl_actions::call_target_name` | `crates/outl-actions/src/template/call.rs` |
| Reserved template name for the daily journal auto-stamp | `outl_actions::JOURNAL_TEMPLATE_NAME` | `crates/outl-actions/src/template/mod.rs` |
| Detect + parse a ` ```call:<name> ` block into `(name, params)` | `outl_actions::parse_call_invocation` | `crates/outl-actions/src/template/run.rs` |
| Execute a callable template (shared by TUI `gx` + desktop exec) | `outl_actions::run_callable_block` | `crates/outl-actions/src/template/run.rs` |
| Resolve the page node for a `template:: <name>` (first in tree order; `tracing::warn!` on a name collision, and `list_templates` flags `TemplateEntry.duplicate`) | `outl_actions::template::list::find_template_by_name` | `crates/outl-actions/src/template/list.rs` |
| Derive a page/journal-root id from a slug (single owner — every creation path routes here so two paths converge on one root) | `outl_core::NodeId::from_slug` (wrapper `outl_actions::page::page_id_from_slug`) | `crates/outl-core/src/id.rs` |
| Read / write the raw snapshot boot cache on disk (`<root>/.outl/snapshots/snap-<actor>.bin`, workspace-owned — NOT a `Storage` method; boot reads via `read_best_from_disk` which prefers this device's own snapshot but adopts a peer's when absent — Phase 2, local ops preserved by the per-actor delta replay; `save_snapshot` + background writer go via `write_to_disk`) | `outl_core::snapshot::read_from_disk` / `read_best_from_disk` / `write_to_disk` (`SnapshotBody`) | `crates/outl-core/src/snapshot.rs` |
| Snapshot wire-format version — bump it in lockstep with any `SnapshotBody` or encoder change; `decode` rejects every version but this one, older and newer alike, and the caller falls back to full op-log replay (postcard since `4`, bincode through `3` — #207) | `outl_core::snapshot::SCHEMA_VERSION` | `crates/outl-core/src/snapshot.rs` |
| Repair a split-brain workspace where a slug has >1 root (re-parents children under the canonical root, trashes duplicates; all `Op`s; idempotent) | `outl_actions::merge_duplicate_slug_roots` (impl `outl_actions::page_merge`) | `crates/outl-actions/src/page_merge.rs` |
| Create sibling before a block, appending at page end when the anchor is stale (`O` / new-block-above; the stale-anchor counterpart of `create_after_or_append`) | `outl_actions::block::create_before_or_append` | `crates/outl-actions/src/block/create.rs` |
| Repair journal titles doubled by concurrent offline creation (two devices minted the same deterministic root and each wrote the slug into the root's Yrs text, concatenating into `"2026-06-252026-06-25"`; clears the text via `Op::Edit`; idempotent, journal-only) | `outl_actions::repair_doubled_journal_titles` (impl `outl_actions::page_repair_titles`) | `crates/outl-actions/src/page_repair_titles.rs` |
| Order a backlinks list chronologically (group-stable by source page, newest-/oldest-first; drives the issue-#142 direction toggle on every client — never re-sort backlinks by hand per client) | `outl_actions::sort_backlinks` | `crates/outl-actions/src/backlinks_sort.rs` |
| Resolve the page/journal slug a node sits under (walks up to a registered page root; `None` if unregistered or not yet materialized) | `outl_core::Workspace::slug_for_node` | `crates/outl-core/src/workspace.rs` |
| Batch a composite action so its ops persist in one `append_ops` per destination instead of one fsync per `apply` (RAII guard, derefs to `Workspace`; commit or drop flushes; nests via a depth counter) | `outl_core::Workspace::begin_batch` → `outl_core::WorkspaceBatch` | `crates/outl-core/src/workspace/batch.rs` |
| Live sync-progress update pushed while a sync pass runs (connecting / snapshot bytes / ops received-pushed / synced / failed) — cosmetic only, distinct from the load-bearing reload trigger | `outl_actions::SyncProgress` + `SyncTransport::set_progress_sink` (default no-op) | `crates/outl-actions/src/sync.rs` |
| Backlink DTO's ancestor breadcrumb — `Backlink::ancestors: Vec<BacklinkCrumb>` (root-first, excludes the page root, empty when the citing block is at root level) | `outl_actions::Backlink` / `outl_actions::BacklinkCrumb` | `crates/outl-actions/src/backlinks.rs` |
| Pre-computed inverted backlinks index — build once (`O(blocks)`, off the input path) then look a page's backlinks up in `O(refs)` instead of re-scanning the workspace on every navigation (`for_page` / `for_target` / `count_for_page` / `len` / `is_empty`); `backlinks_for_page` / `backlinks_for_target` are now one-shot wrappers over this | `outl_actions::BacklinkIndex` | `crates/outl-actions/src/backlinks_index.rs` |
| Build the backlinks index from the `.md` files on disk (client-facing builder — no `Workspace` touched, no lock held, `Send`); `build_backlink_index` (from an in-memory `Workspace`) is for the one-shot wrappers only — building a client's index from the workspace forces a lazy-boot vault (#179) to materialize and holds the workspace lock across the walk | `outl_actions::build_backlink_index_from_disk` | `crates/outl-actions/src/backlinks_index.rs` |
| Apply an already-rendered `.md` string back into the workspace + sidecar, skipping a redundant re-render (the GUI commit path renders once for the undo diff and reuses it) | `outl_actions::journal::apply_page_md_with_sidecar_rendered` | `crates/outl-actions/src/journal/apply.rs` |
| Project a page after a **mutation** without deleting content the op log never saw — the post-mutation counterpart to `_if_stale`, which only guards read paths. Every GUI write path routes through it (`ProjectionWriter`, block move, template instantiate); refusing returns `PageMarkdownAheadOfLog` and the edit stays safe in the op log | `outl_actions::apply_page_md_with_sidecar_guarded` | `crates/outl-actions/src/journal/apply.rs` |
| Decide whether re-projecting a `.md` would delete content the op log never saw — multiset of content lines, whitespace-insensitive. **The** owner of that verdict, so the doctor's read-only listing and `--repair` cannot disagree. Owned by `outl-md` (also `reconcile_md`'s own producer-side check, invariant 8); `outl_actions::content_lines_missing_from` is a re-export. See [RFC 0210](../../docs/rfcs/0210-md-content-outside-op-log.md) | `outl_md::unlogged::content_lines_missing_from` | `crates/outl-md/src/unlogged.rs` |
| Split a block at a character offset (Enter mid-text): head stays in the block, tail becomes a new sibling right after it, children stay with the head | `outl_actions::block::split_block` | `crates/outl-actions/src/block/split.rs` |
| Insert a sibling after a path, seeded with text (the TUI's in-flight block-split: tail of the split goes into the new sibling) | `outline_ops::insert_sibling_after_with_text` | `crates/outl-md/src/outline_ops.rs` |
| Resolve the markdown link `[text](url)` under a caret position (anchor OR url) — the URL a client opens externally (TUI `gx` opens it in the browser when the block isn't code) | `outl_md::inline::link_at_cursor` → `Option<&str>` | `crates/outl-md/src/cursor.rs` |
| Project a **parsed** subtree (`.md` AST, no sidecar) into wire `OutlineNode`s with `tokens` attached — ids are **transient** (fresh per call), for read-only surfaces that re-resolve on navigation (the `!((blk))` embed subtree expansion) | `outl_actions::outline::project_parsed_subtree` (re-exported `outl_actions::project_parsed_subtree`) | `crates/outl-actions/src/outline.rs` |
| Content-hash an uploaded file's bytes / build its workspace-relative link target / test whether a link points at a workspace asset / test whether a name is a safe asset basename (anti-traversal, the one owner the P2P transport validates peer-sent names through) — all pure, no filesystem | `outl_md::hash_bytes` / `outl_md::asset_rel_path` / `outl_md::is_asset_link` / `outl_md::is_safe_asset_name` (+ `outl_md::ASSETS_DIR`) | `crates/outl-md/src/asset.rs` |
| Copy an uploaded file into `<root>/assets/<hash>.<ext>` (content-addressed, atomic, size-capped by `[assets] max_bytes`; `import_asset_bytes` takes already-in-memory bytes for a remote image downloaded during a Roam import) and resolve a `[name](assets/…)` link back to an on-disk path (traversal-safe) — the bytes never enter the op log, only the link does | `outl_actions::import_asset` / `outl_actions::import_asset_bytes` / `outl_actions::resolve_asset_path` | `crates/outl-actions/src/asset.rs` |

Frontend shared primitives (`@outl/shared`) — canonical home is [`crates/outl-frontend-shared/CLAUDE.md`](../../crates/outl-frontend-shared/CLAUDE.md) → "Today's surface"; the embed / block-ref pieces the desktop wires for issue #147:

| Intent | Use this | File |
|---|---|---|
| Render inline markdown tokens to JSX; the `blockref` token (`((blk))` / `!((blk))`) resolves to the source block's text when `embeds` carries the handle (orphan = raw chip) | `<MarkdownInline embeds= … />` (`@outl/shared/markdown`) | `crates/outl-frontend-shared/src/markdown/MarkdownInline.tsx` |
| Render an embed's subtree read-only — `↳`-nested, max depth 4 (mirrors the TUI's `emit_embedded_children`) | `<EmbeddedSubtree />` (`@outl/shared/markdown`) | `crates/outl-frontend-shared/src/markdown/EmbeddedSubtree.tsx` |
| The reply shape of `resolveEmbeds` (`{ handle, text, page_slug, status, children: BlockNode[] }`); `EmbedMap` is `Record<string, ResolvedBlock>` | `ResolvedBlock` (`@outl/shared/api/types`) | `crates/outl-frontend-shared/src/api/types.ts` |
| The handle **iff** a block is embed-only (a bare `!((blk))`), so a client knows to render `<EmbeddedSubtree />` below it | `embedOnlyHandle(tokens)` (`@outl/shared/outline`) | `crates/outl-frontend-shared/src/outline/index.ts` |
| Collect every blockref + embed handle in an outline (DFS) so a client resolves them in one `resolveEmbeds` round-trip | `collectBlockRefHandles(outline)` (`@outl/shared/outline`) | `crates/outl-frontend-shared/src/outline/index.ts` |
| Wire the Tauri webview's OS file drag-drop to a block-resolved handler; desktop and mobile both consume this so the drop geometry (physical→CSS pixels, `data-block-id` hit-test) can't drift | `installFileDrop(handlers)`, `physicalToCss`, `blockIdFromElement` / `blockIdAtPhysical`, `joinAssetMarkdowns`, `appendMarkdownToBlock` (`@outl/shared/drag-drop`) | `crates/outl-frontend-shared/src/drag-drop/index.ts` |
| Import a dropped file **without** creating a block, returning the ready-to-insert markdown link for the caller to splice at the drop target | `importAssetFile(sourcePath) → Promise<ImportedAsset>` (`@outl/shared/api/commands`) | `crates/outl-frontend-shared/src/api/commands.ts`; backend `import_asset_file` wraps `outl_actions::import_asset` |
