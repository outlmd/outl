# Shared primitives — editing actions and client features

Everything a client calls to *change* a workspace or render a user-facing feature: block mutations, pages and journals, backlinks, code-block execution, undo / redo, templates, reminders, and the `@outl/shared` TS surface every GUI client wraps.
Every entry here routes its mutations through `Workspace::apply` — the op log stays the source of truth.

Part of the **Shared primitives catalog** — the index of every part lives in [`shared-primitives.md`](shared-primitives.md).

**Before writing any helper, scan these tables first.**
Most "I need a small string transform / id helper / md coercion / tree walk" needs already have an owner here —
the cost of finding the existing one is a `grep`;
the cost of missing it shows up later as drift between two parallel implementations (the user is the one who hits the divergence).

For the reuse-first rule (why this matters, past drift incidents, what to do when a primitive doesn't exist yet), see [Contributing → Reuse-first](contributing.md#reuse-first-no-parallel-implementations).

---

## 1. Block mutations (outl-actions::block + collapsed + todo + quote)

Every entry here routes through `Workspace::apply` — never build a `LogOp` from a client and apply it directly.

| Intent | Use this | File |
|---|---|---|
| Append a single block under a parent | `outl_actions::block::append_block` | `crates/outl-actions/src/block/create.rs` |
| Append a tree / forest (with children) under a parent | `outl_actions::block::append_tree` / `append_forest` (uses `BlockTreeSpec` → returns `BlockTreeOutcome`) | `crates/outl-actions/src/block/create.rs` |
| Create sibling before a block (vim `O`; floor-slot swap when the anchor is first child) | `outl_actions::block::create_before` | `crates/outl-actions/src/block/create.rs` |
| Create sibling after / child under a block | `outl_actions::block::create_after` / `create_under` | `crates/outl-actions/src/block/create.rs` |
| Create sibling after a block, appending at page end when the anchor is stale | `outl_actions::block::create_after_or_append` (the desktop/mobile `create_block` stale-anchor fallback — one owner, no per-client duplication) | `crates/outl-actions/src/block/create.rs` |
| Create sibling **before** a block, appending at page end when the anchor is stale | `outl_actions::block::create_before_or_append` (the `O` / new-block-above counterpart of `create_after_or_append` — same stale-anchor tolerance so a concurrent sync reload that re-mints the id never surfaces `block <id> is not in the tree`) | `crates/outl-actions/src/block/create.rs` |
| Edit a block's text | `outl_actions::block::edit_text` | `crates/outl-actions/src/block/edit.rs` |
| Split a block at a character offset (Enter mid-text): head stays in the block, tail becomes a new sibling right after it, children stay with the head | `outl_actions::block::split_block` | `crates/outl-actions/src/block/split.rs` |
| Move a block to sit **after an arbitrary target** (cut-and-paste-block; crosses pages; one `Op::Move`, preserving id + refs; rejects self-subtree cycles) | `outl_actions::block::move_after` | `crates/outl-actions/src/block/moves.rs` |
| Indent / outdent / move up / move down a block | `outl_actions::block::indent` / `outdent` / `move_up` / `move_down` | `crates/outl-actions/src/block/moves.rs` |
| Re-parent a block under an arbitrary page/block (cross-page move) | `outl_actions::block::move_under` | `crates/outl-actions/src/block/moves.rs` |
| Delete a block (`Move(node, TRASH_ROOT)`, **never** physical) | `outl_actions::block::delete` | `crates/outl-actions/src/block/moves.rs` |
| Toggle a block's collapsed flag (converges via `Op::SetCollapsed`) | `outl_actions::collapsed::toggle_block_collapsed` / `set_block_collapsed` | `crates/outl-actions/src/collapsed.rs` |
| Cycle / split / read task state — TODO → DOING → DONE, encoded as a text prefix. `TodoState::prefix` is the one owner of the marker spelling, and the widths differ (`DOING ` is 6 chars), so measure it instead of assuming | `outl_actions::todo::cycle_todo` / `split_todo` / `TodoState` / `TODO_PREFIX` / `DOING_PREFIX` / `DONE_PREFIX` | `crates/outl-actions/src/todo.rs` |
| Set TODO/DONE state outright (not "advance one step") | `outl_actions::todo::set_todo` | `crates/outl-actions/src/todo.rs` |
| Toggle TODO/DONE on a block in one call | `outl_actions::block::toggle_todo` | `crates/outl-actions/src/block/edit.rs` |
| Read / toggle blockquote state (encoded as `"> "` text prefix, CommonMark-compatible) | `outl_actions::quote::is_quote` / `split_quote` / `toggle_quote` / `QUOTE_PREFIX` | `crates/outl-actions/src/quote.rs` |
| Toggle blockquote on a block in one call | `outl_actions::block::toggle_quote` | `crates/outl-actions/src/block/edit.rs` |

---

## 2. Pages and journals (outl-actions::page + journal)

| Intent | Use this | File |
|---|---|---|
| Page-property keys (constants — don't hardcode the strings) | `outl_actions::page::SLUG_KEY` / `KIND_KEY` / `TYPE_KEY` / `TITLE_KEY` | `crates/outl-actions/src/page.rs` |
| Canonical `type::` value marking a page as a person (`@` mention autocomplete filter) | `outl_actions::page::PERSON_TYPE` | `crates/outl-actions/src/page.rs` |
| Page metadata (slug, kind, title, **`page_type`**) for a node id | `outl_actions::page::page_meta` / `PageMeta` / `PageKind` | `crates/outl-actions/src/page.rs` |
| Validate a slug for filesystem safety (`..`, `/`, `\`, control chars) | `outl_actions::page::is_valid_slug` | `crates/outl-actions/src/page.rs` |
| Derive a **deterministic page/journal-root id** from slug (so every creation path — in-app, `outl-md` reconcile, desync recovery — converges on ONE root; the single owner) | `outl_core::NodeId::from_slug` (thin wrapper `outl_actions::page::page_id_from_slug`) | `crates/outl-core/src/id.rs` |
| Find / list / create-if-missing pages (`find_by_slug` resolves a deterministic winner when a slug has >1 root, so a split-brain workspace stops flickering pre-merge) | `outl_actions::page::find_by_slug` / `list_all` / `open_or_create` | `crates/outl-actions/src/page.rs` |
| Repair a split-brain workspace where a slug has >1 page/journal root (re-parents every child under the canonical root, trashes the emptied duplicates, all via `Op`s so it converges on every device; idempotent) | `outl_actions::merge_duplicate_slug_roots` (impl `outl_actions::page_merge`) | `crates/outl-actions/src/page_merge.rs` |
| Repair journal titles doubled by concurrent offline creation (two devices minted the same deterministic root and each wrote the slug into the root's Yrs text, so the concurrent inserts concatenated into `"2026-06-252026-06-25"`; clears the text via `Op::Edit` so the title falls back to the slug; idempotent, journal-only) | `outl_actions::repair_doubled_journal_titles` (impl `outl_actions::page_repair_titles`) | `crates/outl-actions/src/page_repair_titles.rs` |
| Delete a page (move root to `NodeId::trash()` via one `Op::Move`; whole subtree travels with it; returns `PageMeta` so callers can drop projections + navigate away; `ActionError::PageNotFound` when the slug doesn't resolve) | `outl_actions::page::delete` (re-exported as `outl_actions::delete_page`) | `crates/outl-actions/src/page.rs` |
| Remove a page's `.md` + `.outl` from disk (the inverse of `apply_page_md_with_sidecar`; idempotent on missing files; pairs with `page::delete`) | `outl_actions::journal::remove_page_projection` (re-exported at crate root) | `crates/outl-actions/src/journal/paths.rs` |
| Open-or-create a page from a **human-typed name** (slugifies + keeps original as title, used when a `[[ref]]` / `#tag` / picker query may not be a valid slug) | `outl_actions::resolve::open_or_create_by_name` | `crates/outl-actions/src/resolve.rs` |
| Open-or-create whatever a **user-typed ref target** points at (date → journal, else literal/slugified/title match → existing page, else create) — handles `@`-prefixed mentions by stripping the `@` and marking new pages as `type:: person`; the one decision tree so frontend regex and backend parser cannot drift on `[[2026-13-01]]` or `[[@avelino]]` | `outl_actions::resolve::open_or_create_by_ref` | `crates/outl-actions/src/resolve.rs` |
| Search pages typed `type:: person`, fuzzy-ranked by query (powers the `@` mention autocomplete in every client) | `outl_actions::person::search_persons` | `crates/outl-actions/src/person.rs` |
| Read / write a property on a page (or any node) | `outl_actions::page::read_text_prop` / `set_property` | `crates/outl-actions/src/page.rs` |
| Migrate pre-page-model blocks under today's journal (run on boot) | `outl_actions::page::migrate_legacy_into_today` | `crates/outl-actions/src/page.rs` |
| Open / create the journal for a specific date or today | `outl_actions::page::open_journal` / `open_today` | `crates/outl-actions/src/page.rs` |
| Journal date labels & day arithmetic (slug ↔ date, title, `[[YYYY-MM-DD]]` ref, prev/next day) | `outl_actions::dates::journal_slug` / `journal_title` / `journal_ref` / `date_from_slug` / `previous_journal_date` / `next_journal_date` | `crates/outl-actions/src/dates.rs` |
| Today's journal date in the configured timezone (delegates to `clock`) | `outl_actions::page::today` | `crates/outl-actions/src/page.rs` |
| Week arithmetic — ISO-week tag (`#2026-W22`, `%G`-correct at year boundaries) and "days until next `<weekday>`" (same weekday → 7, never 0) | `outl_actions::dates::week_tag` / `days_until_next_weekday` | `crates/outl-actions/src/dates.rs` |
| Current date / time in the user's configured timezone (`[calendar] timezone`, DST-aware via chrono-tz; OS local when unset). Call `init` once per client at boot; `page::today` delegates here, so use `now_local` / `today` instead of `chrono::Local::now()` (issue #107) | `outl_actions::clock::init` / `now_local` / `today` | `crates/outl-actions/src/clock.rs` |
| Parse a **human-typed date** in any supported spelling (`2026-04-22`, `2026/04/22`, `22/04/2026`, Roam's `April 22nd, 2026`, `Sept 3rd, 2025`, `22 April 2026`) into a `NaiveDate`, or into the ISO label outl uses for journal slugs / `[[date]]` refs — the one owner of the ordinal-stripping logic that used to be copied in four places (paste normalization, `outl daily`, `outl import`, Obsidian frontmatter). `parse_date_arg` layers **relative offsets** (`+3d`, `-2w`, `+1m`, bare `5d`) on top for slash-command / CLI arguments | `outl_actions::dates::parse_flexible_date` / `parse_date_label` / `parse_date_arg` | `crates/outl-actions/src/dates.rs` |
| Parse an `outl://` deep link URL into a navigation target (one parser, every GUI client routes the result to its own `open_*` command — never reparse per client) | `outl_actions::parse_deep_link` / `DeepLinkTarget` / `DeepLinkError` / `DEEP_LINK_SCHEME` | `crates/outl-actions/src/deeplink.rs` |
| Filesystem paths for journals / pages / a specific page | `outl_actions::journal::journals_dir` / `pages_dir` / `page_md_path` | `crates/outl-actions/src/journal/paths.rs` |
| Render a page node out to `.md` | `outl_actions::journal::render_page_md` | `crates/outl-actions/src/journal/render.rs` |
| Apply an edited `.md` back into the workspace (with / without sidecar) | `outl_actions::journal::apply_page_md` / `apply_page_md_with_sidecar` | `crates/outl-actions/src/journal/apply.rs` |
| Apply an already-rendered `.md` string back into the workspace + sidecar, skipping a redundant re-render (the GUI commit path renders once for the undo diff and reuses it) | `outl_actions::journal::apply_page_md_with_sidecar_rendered` | `crates/outl-actions/src/journal/apply.rs` |
| Project a page after a **mutation** without deleting content the op log never saw — the post-mutation counterpart to `_if_stale`, which only guards read paths. Every GUI write path routes through it (`ProjectionWriter`, block move, template instantiate); refusing returns `PageMarkdownAheadOfLog` and the edit stays safe in the op log | `outl_actions::apply_page_md_with_sidecar_guarded` | `crates/outl-actions/src/journal/apply.rs` |
| Apply every page's `.md` to disk in one pass | `outl_actions::journal::apply_all_pages_md` | `crates/outl-actions/src/journal/apply.rs` |
| Run a closure that mutates a page's `.md` (read → modify → write atomically) | `outl_actions::journal::mutate_page_md` | `crates/outl-actions/src/journal/apply.rs` |
| Atomic `.md` write (crash-safe, wraps `outl_md::atomic::write_atomic`) | `outl_actions::journal::write_md_atomic` | `crates/outl-actions/src/journal/paths.rs` |
| Decide whether re-projecting a `.md` would delete content the op log never saw (multiset of content lines, whitespace-insensitive — **the** owner of that verdict, so the doctor's read-only listing and `--repair` cannot disagree). Owned by `outl-md` (it is also `reconcile_md`'s producer-side check); re-exported here so every existing `outl_actions::` path resolves | `outl_actions::content_lines_missing_from` (→ `outl_md::unlogged::content_lines_missing_from`) | `crates/outl-md/src/unlogged.rs` |
| Decide whether the sidecar you are about to pass to the verdict above can answer it at all — `false` for a pre-0.11 sidecar whose entries all carry `text: ""`. An empty verdict from a reference that cannot answer is not "nothing at risk", so a caller that skips this reads "I could not check" as permission to write. Same owner, same crate, re-exported for the same reason | `outl_actions::sidecar_can_answer` (→ `outl_md::unlogged::sidecar_can_answer`) | `crates/outl-md/src/unlogged.rs` |
| Scan the materialized tree for a block whose current text is a proper prefix of an earlier `Op::Edit` revision — i.e. a truncating edit and the dropped tail is still reconstructible from the append-only log | `outl_actions::scan_truncated_blocks` → `TruncatedBlock` | `crates/outl-actions/src/recover.rs` |
| Write a recovered revision back as a **new** `Op::Edit` (never a log rewrite); refuses when the block changed since the scan, so the write stays additive | `outl_actions::restore_truncated_block` | `crates/outl-actions/src/recover.rs` |

---

## 3. Backlinks (outl-actions::backlinks)

| Intent | Use this | File |
|---|---|---|
| Extract `[[ref]]` tokens out of a block's text (tolerates unbalanced openers) | `outl_actions::backlinks::extract_refs` | `crates/outl-actions/src/backlinks.rs` |
| Backlink DTO returned by the queries below (carries `ancestors: Vec<BacklinkCrumb>` — root-first ancestor chain of the citing block, excluding the page root, empty when the block sits at root level) | `outl_actions::backlinks::Backlink` | `crates/outl-actions/src/backlinks.rs` |
| One breadcrumb entry in `Backlink::ancestors` (plain text, TODO/DONE prefix stripped) | `outl_actions::BacklinkCrumb` | `crates/outl-actions/src/backlinks.rs` |
| Drop a `Backlink`'s source subtree (children) for the GUI wire — keeps the leaf (text, tokens, todo, properties) + `ancestors`; GUI rows only ever render `source_block.tokens`, the TUI keeps the full form since it reads the index in-process | `outl_actions::Backlink::into_shallow` | `crates/outl-actions/src/backlinks.rs` |
| Walk every backlink for a target / a `PageMeta` — one-shot convenience: builds a fresh `BacklinkIndex` and looks it up, fine for a single call (CLI, tests) but pays the `O(blocks)` build every time | `outl_actions::backlinks::backlinks_for_target` / `backlinks_for_page` | `crates/outl-actions/src/backlinks.rs` |
| Pre-computed inverted backlinks index (`target key -> referencing blocks`) — build once (`O(blocks)`, off the input path / a background thread) then look a page's backlinks up in `O(refs)`; `for_page` / `for_target` / `count_for_page` (no-clone count) / `len` / `is_empty`. A long-lived client (TUI, desktop, mobile) should hold one of these instead of calling `backlinks_for_page` per navigation | `outl_actions::BacklinkIndex` | `crates/outl-actions/src/backlinks_index.rs` |
| Build the backlinks index from the `.md` files **on disk** — the client-facing builder. Touches no `Workspace`, holds no lock, `Send`; use this from every client. Building it from the in-memory workspace instead (`Workspace::block_text` per block) forces a lazy-boot vault (#179) to materialize entirely and holds the workspace lock across the walk — the "opening the journal / pressing Esc freezes" regression | `outl_actions::build_backlink_index_from_disk` | `crates/outl-actions/src/backlinks_index.rs` |
| Build the backlinks index from an in-memory `Workspace` — **one-shot wrappers only** (`backlinks_for_page` / `backlinks_for_target`, CLI/tests with no `.md` on disk); do not call this from a client's index-rebuild path, use `build_backlink_index_from_disk` instead | `outl_actions::build_backlink_index` | `crates/outl-actions/src/backlinks_index.rs` |
| Order a backlinks list chronologically (group-stable by source page, newest- or oldest-first; drives the issue-#142 direction toggle on every client) | `outl_actions::sort_backlinks` | `crates/outl-actions/src/backlinks_sort.rs` |

---

## 4. Code-block execution (outl-actions::exec)

The **cross-client glue** every UI uses to wire a "run this fence" gesture (TUI `g x`, desktop `Cmd+Shift+X`, mobile long-press → "Run code") through to `outl-exec` and back.
`outl_actions::exec::run_code_block` is the **only** entry point a Tauri command / TUI action should call — never re-implement the flat-DFS walk, the `.md` path lookup, or the DTO shape per client.

| Intent | Use this | File |
|---|---|---|
| Resolve a `NodeId` to its flat DFS index inside an outline forest (the order `outl_exec::run_block_at_index` expects) | `outl_actions::flat_index_for_block` | `crates/outl-actions/src/outline.rs` |
| Orchestrate execution: walk DFS, resolve `.md` path, call `outl_exec::run_block_at_index`, build DTO | `outl_actions::exec::run_code_block` | `crates/outl-actions/src/exec.rs` |
| Serializable mirror of `outl_exec::ExecOutput` (stdout/stderr/duration_ms/exit) | `outl_actions::ExecOutputDto` | `crates/outl-actions/src/exec.rs` |
| Outcome shipped to the client (`language` + `result_ok` xor `error`; client adds the refreshed `view`) | `outl_actions::RunCodeBlockOutcome` | `crates/outl-actions/src/exec.rs` |

The runtime catalog (which languages are available) is selected by the **binary** that consumes this crate, via `outl-exec` features in its own `Cargo.toml`.
`outl-actions` itself depends on `outl-exec` with `default-features = false` so it doesn't drag `wasmtime` (Rust runtime) into the mobile IPA via the back door.

The `query` runtime (`outl_exec::runtimes::query`) is a special case: it returns `OutputFormat::Embeds` instead of `OutputFormat::Text`, so the orchestrator renders results as live `!((blk-XXXXXX))` embeds rather than a code-fence stdout dump.
It also overrides `Runtime::auto_run()` to return `true`, so query blocks always re-run on page load without needing the `auto-run::` property or manual `gx`.

The query engine also exposes a **structured API** for plugins and code that runs outside the ` ```query ` fence:

| Intent | Use this | File |
|---|---|---|
| Structured query from a `QueryParams` object (plugin-facing) | `outl_exec::run_query_structured` | `crates/outl-exec/src/runtimes/query.rs` |
| DSL query from a string (user-facing) | `outl_exec::run_query_dsl` | `crates/outl-exec/src/runtimes/query.rs` |
| Query parameters struct (status, tag, kind, since, text, sort, limit) | `outl_exec::QueryParams` | `crates/outl-exec/src/runtimes/query.rs` |
| Query result hit (handle, text, status, page) | `outl_exec::QueryHit` | `crates/outl-exec/src/runtimes/query.rs` |

In JS code blocks, the same API is available as `outl.query({ status: "todo", … })`.

---

## 5. Undo / redo history (outl-actions::history)

Bounded snapshot stacks with vim semantics (a new edit clears redo) shared by GUI clients — the desktop's `Cmd+Z` / `Cmd+Shift+Z` ride these.
Restores route through `outl_md::reconcile_md`, so an undo is **new ops in the log**, never a rewrite (invariant #1 holds).
This is *not* per-keystroke undo inside an uncommitted draft — that belongs to the client's editor widget.

| Intent | Use this | File |
|---|---|---|
| Bounded undo / redo stacks over any snapshot type (`record` / `undo` / `redo` / `can_undo` / `can_redo` / `clear`) | `outl_actions::history::HistoryStacks` | `crates/outl-actions/src/history.rs` |
| Default per-stack bound (matches the TUI's session cap) | `outl_actions::DEFAULT_HISTORY_CAP` | `crates/outl-actions/src/history.rs` |
| Restore a page to a previously-rendered `.md` snapshot (write + reconcile → min ops through `Workspace::apply`) | `outl_actions::restore_page_md` | `crates/outl-actions/src/history.rs` |

---

## 6. Templates

| Intent | Use this | File |
|---|---|---|
| List all template pages (any page with a non-empty `template::` property), sorted by name (each entry flags `duplicate` when another page shares its name) | `outl_actions::list_templates` → `TemplateEntry` | `crates/outl-actions/src/template/list.rs` |
| Resolve the page node for a `template:: <name>` (first in tree order; `tracing::warn!` on a name collision) | `outl_actions::template::list::find_template_by_name` | `crates/outl-actions/src/template/list.rs` |
| Instantiate (deep-copy) a structural template's subtree under a target block, with `{{token}}` substitution and `from-template::` traceability on each root clone | `outl_actions::instantiate_template` | `crates/outl-actions/src/template/instantiate.rs` |
| Resolve a callable template's code block (language, source, declared `params::`) | `outl_actions::resolve_call` → `CallResolution` | `crates/outl-actions/src/template/call.rs` |
| Parse a ` ```call:<name> ` block's `key: value` body into params | `outl_actions::parse_call_params` | `crates/outl-actions/src/template/call.rs` |
| The template name invoked by a ` ```call:<name> ` fence (inverse of the exec path's fence read; drives the backlinks traceability match) | `outl_actions::call_target_name` | `crates/outl-actions/src/template/call.rs` |
| Inject a `params` binding into a callable template's source (serde_json-escaped, language canonicalized via `outl_md::lang::canonical`, so quotes/newlines in a value can't break or inject into the generated program) | `outl_actions::inject_call_params` | `crates/outl-actions/src/template/call.rs` |
| Detect + parse a ` ```call:<name> ` block into `(name, params)` — the shared "is this a call invocation?" check every client uses before running normal exec | `outl_actions::parse_call_invocation` | `crates/outl-actions/src/template/run.rs` |
| Execute a callable template (resolve → inject params → run via a client `RuntimeRegistry` → write the `> **result:**` subtree). The single owner every client wraps for `call:` execution | `outl_actions::run_callable_block` | `crates/outl-actions/src/template/run.rs` |
| Template property key constant | `outl_actions::TEMPLATE_KEY` | `crates/outl-actions/src/template/mod.rs` |
| Traceability property key constant (set on structural-instance root blocks) | `outl_actions::FROM_TEMPLATE_KEY` | `crates/outl-actions/src/template/mod.rs` |
| Callable params key constant | `outl_actions::PARAMS_KEY` | `crates/outl-actions/src/template/mod.rs` |
| Reserved template name for the daily journal body — a page with `template:: journal` is auto-instantiated (untraced) into every fresh daily note | `outl_actions::JOURNAL_TEMPLATE_NAME` | `crates/outl-actions/src/template/mod.rs` |

---

## 7. Reminders (`remind::`)

Block-level notification rules.
The **schedule math has exactly one owner** — `outl_actions::reminders::next_fire_at`.
Every surface (TUI overlay, desktop panel, mobile sheet, each OS bridge) calls it; a second opinion in TS or Swift about when a reminder fires is drift that reaches the user before it reaches a test.
User-facing spec: [`reminders.md`](reminders.md).

| Intent | Use this | File |
|---|---|---|
| Property key that carries a reminder rule (don't hardcode `"remind"`) | `outl_md::REMIND_KEY` | `crates/outl-md/src/remind.rs` |
| Parse a `remind::` value into a rule + the warnings it triggered (never fails destructively — an unreadable rule just doesn't schedule) | `outl_md::parse_remind` → `RemindParse { rule: Option<RemindRule>, warnings }` | `crates/outl-md/src/remind.rs` |
| Pull the rule off a block's property list | `outl_md::rule_from_properties` | `crates/outl-md/src/remind.rs` |
| The parsed rule + its parts (`RemindAnchor::Now` / `At`, `RemindStop::Done` / `Time` / `Date`) | `outl_md::RemindRule` / `RemindAnchor` / `RemindStop` | `crates/outl-md/src/remind.rs` |
| Hard caps the parser enforces (1-minute interval floor, 10-fire ceiling) | `outl_md::MIN_INTERVAL_MINUTES` / `outl_md::MAX_FIRES_CAP` | `crates/outl-md/src/remind.rs` |
| **When does this rule next fire?** — pure, clock-free, takes `now` as a parameter. THE single owner | `outl_actions::next_fire_at` (+ `ReminderState`) | `crates/outl-actions/src/reminders/schedule.rs` |
| Every reminder in the workspace with its next fire resolved (reads pages from disk, consults the workspace only for the snooze table) | `outl_actions::scan_reminders` → `Vec<Reminder>` | `crates/outl-actions/src/reminders/scan.rs` |
| Device-local "already delivered" record the scan takes as input | `outl_actions::FiredLog` / `FiredRecord` | `crates/outl-actions/src/reminders/scan.rs` |
| Silence a block's reminder until an instant (writes `Op::SnoozeRemind`, so it converges to every device) | `outl_actions::snooze` / `snooze_until` | `crates/outl-actions/src/reminders/mod.rs` |
| Local wall clock ↔ epoch ms, resolved through the configured timezone (never `chrono::Local` directly) | `outl_actions::local_naive_to_epoch_ms` / `epoch_ms_to_local_naive` | `crates/outl-actions/src/reminders/mod.rs` |
| Read a block's converged snooze instant | `outl_core::tree::Tree::snoozed_until` / `snoozed_ids` | `crates/outl-core/src/tree/mod.rs` |
| Every node carrying a given property key, without a tree walk (the transpose of `properties_of`; `O(total properties)`, materializes no block text) | `outl_core::tree::Tree::nodes_with_property` | `crates/outl-core/src/tree/mod.rs` |
| Device-local delivery preferences (`enabled`, quiet hours as `(start, end)` minutes) | `outl_config::RemindersCfg` / `RemindersCfg::quiet_window` | `crates/outl-config/src/schema.rs` |
| Deliver what came due + update the device-local fired log (7-day TTL, `<root>/.outl/reminders-fired.json` — a dotfile so it never rides the sync surface). In `outl-actions` because **every** client delivers, the TUI included; behind the Tauri layer the TUI couldn't reach it | `outl_actions::take_due` (+ `load_fired_log` / `save_fired_log` / `fired_log_path` / `FIRED_TTL_DAYS`) | `crates/outl-actions/src/reminders/fired.rs` |
| Format "in 3h" / "tomorrow 09:00" and bucket a list Today / Tomorrow / This week / Later / Done (shared by both GUI clients) | `@outl/shared` `formatNextFire` / `groupReminders` | `crates/outl-frontend-shared/src/api/commands.ts` |

---

## 8. Frontend shared primitives (`@outl/shared`)

The TS + Solid catalog's canonical home is [`crates/outl-frontend-shared/CLAUDE.md`](../crates/outl-frontend-shared/CLAUDE.md) → "Today's surface".
The rows below are the embed / block-ref pieces the desktop wires for issue #147, mirrored here so a `grep` for the reuse index finds them.

| Intent | Use this | File |
|---|---|---|
| Render inline markdown tokens to JSX; the `blockref` token (`((blk))` / `!((blk))`) resolves to the source block's text when `embeds` carries the handle (orphan = raw chip) | `<MarkdownInline embeds= … />` (`@outl/shared/markdown`) | `crates/outl-frontend-shared/src/markdown/MarkdownInline.tsx` |
| Render an embed's subtree read-only — `↳`-nested, max depth 4 (mirrors the TUI's `emit_embedded_children`) | `<EmbeddedSubtree />` (`@outl/shared/markdown`) | `crates/outl-frontend-shared/src/markdown/EmbeddedSubtree.tsx` |
| The reply shape of `resolveEmbeds` (`{ handle, text, page_slug, status, children: BlockNode[] }`); `EmbedMap` is `Record<string, ResolvedBlock>` | `ResolvedBlock` (`@outl/shared/api/types`) | `crates/outl-frontend-shared/src/api/types.ts` |
| The handle **iff** a block is embed-only (a bare `!((blk))`), so a client knows to render `<EmbeddedSubtree />` below it | `embedOnlyHandle(tokens)` (`@outl/shared/outline`) | `crates/outl-frontend-shared/src/outline/index.ts` |
| Collect every blockref + embed handle in an outline (DFS) so a client resolves them in one `resolveEmbeds` round-trip | `collectBlockRefHandles(outline)` (`@outl/shared/outline`) | `crates/outl-frontend-shared/src/outline/index.ts` |
| Wire the Tauri webview's OS file drag-drop to a block-resolved handler; desktop and mobile both consume this so the drop geometry (physical→CSS pixels, `data-block-id` hit-test) can't drift | `installFileDrop(handlers)`, `physicalToCss`, `blockIdFromElement` / `blockIdAtPhysical`, `joinAssetMarkdowns`, `appendMarkdownToBlock` (`@outl/shared/drag-drop`) | `crates/outl-frontend-shared/src/drag-drop/index.ts` |
| Import a dropped file **without** creating a block, returning the ready-to-insert markdown link for the caller to splice at the drop target | `importAssetFile(sourcePath) → Promise<ImportedAsset>` (`@outl/shared/api/commands`) | `crates/outl-frontend-shared/src/api/commands.ts`; backend `import_asset_file` wraps `outl_actions::import_asset` ([Asset links](primitives-markdown.md#8-asset-links-outl-mdasset--outl-actionsasset)) |
