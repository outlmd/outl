# CLAUDE.md — outl-exec

Context for Claude Code sessions working in this crate.
Read this before making any change.

## What this crate is

Code-block execution engine for outl: takes a fenced markdown block (` ```lisp `, ` ```python `, ...) and writes the result back into the page as a child subblock — idempotent on re-run.

The crate is intentionally tiny and modular:

- **`runtime::Runtime`** — the trait you implement to add a new language.
  Two required methods: `language()` (the fence info-string) and `execute()`.
  One optional override: `auto_run()` (default `false`).
- **`runtime::ExecContext`** — workspace root, stdin, timeout, mem limit.
- **`runtime::ExecOutput`** — stdout, stderr, duration, exit status, and `format: OutputFormat`.
- **`runtime::OutputFormat`** — `Text` (default: result subblock as `> **result:** …`) or `Embeds` (result subblock with one child bullet per stdout line, rendered as embeds).
- **`registry::RuntimeRegistry`** — resolves a fence info-string to the concrete `Runtime`.
- **`sandbox`** — cross-platform timeout helper.
- **`result_block`** — pure functions that find / create the result subblock under a code block.
  Includes `upsert_result_child` (text), `upsert_result_embeds` (embed children), and hash-stamped variants for auto-run cache.
- **`orchestrate::run_block_at_index`** — single entry point for every UI.
  Takes a workspace + page path + block flat-index, runs, persists, reconciles.
- **`orchestrate::run_block_at_index_if_source_changed`** — cache-aware variant used by auto-run loop.

## Adding a new language runtime

1. Add a `lang-<name>` feature in `Cargo.toml`.
2. Create `runtimes/<name>.rs` with one struct + one `impl Runtime`.
3. Register it in `RuntimeRegistry::with_builtins` behind the feature.
4. Add aliases to `KNOWN_ALIASES` in `crates/outl-md/src/lang.rs` **and** the TS mirror at `crates/outl-frontend-shared/src/highlight/aliases.ts`.
5. If the runtime needs workspace access, use `ctx.workspace_root` to build a `WorkspaceIndex`.
6. If the runtime should auto-run on page load, override `auto_run()` to return `true`.

See `runtimes/query.rs` for the most advanced example (workspace access, embed output, auto-run).

## The `query` runtime

The `query` runtime (`runtimes/query.rs`) is a special case:

- **Returns `OutputFormat::Embeds`**: each stdout line becomes an embed child (`!((blk-XXXXXX))`) under the result header.
  This makes query results **live references** to the original blocks, not copies.
- **Overrides `auto_run()` to `true`**: query blocks always re-run on page load, without needing `gx` or `auto-run::`.
- **Builds a `WorkspaceIndex`** from `ctx.workspace_root` on every execution.
- **DSL parser** (`runtimes/query::dsl`): line-by-line `key: value` directives, implicitly ANDed.
  Filters: `status`, `tag`, `kind`, `since`, `text`.
  Controls: `sort`, `limit`.
- **`engine::Status` mirrors `outl_actions::TodoState`** (`Todo` / `Doing` / `Done`) instead of importing it: `outl-actions` depends on **this** crate for `run_code_block`, so the arrow only points one way.
  A state added there has to be added here in the same change — nothing in the compiler enforces the pair.
  `status: open` means "is a task", DONE included; it kept that meaning when `doing` landed so existing queries don't change under the user.
- **The local `split_todo` deliberately reads *more* than its owner does.**
  It unwraps one optional `"> "` quote prefix before looking for the marker, so the legacy authoring order (`"> TODO foo"`) matches `status: todo`.
  `outl_actions::split_todo` does **not** do that, and the difference is forced by its return type, not by disagreement.
  It hands back a `&str` slice of the body, so stripping the quote there would drop the marker from `OutlineNode.text` and the GUI clients would stop drawing the `│` bar.
  Know the consequence before "fixing" either side.
  A block written `"> TODO foo"` is a task to the TUI render, the TUI progress chip and this query engine.
  It is **not** a task to the DTO the desktop / mobile clients receive, to `outl` CLI human output, or to plugins.
  The canonical order (`"TODO > foo"`) has no such split, and `cycle_todo` rewrites the legacy shape into it on the first toggle.

User-facing DSL docs live in `docs/query.md` — don't duplicate here.

## Query SDK API (`outl.query`)

The query engine exposes a **structured API** alongside the DSL, so plugins and JS code blocks can query the workspace without parsing a DSL string.

### Two entry points

- **`run_query_dsl(dsl, root)`** — user-facing DSL string → `Vec<QueryHit>`. Used internally by `QueryRuntime::execute`.
- **`run_query_structured(params, root)`** — plugin-facing struct → `Vec<QueryHit>`. Exposed to JS as `outl.query({ ... })`.

Both converge on the same engine pipeline.

### Public types (re-exported from `outl_exec`)

- `QueryParams` — `{ status, tag, kind, since, text, sort, limit }`, all optional.
- `QueryHit` — `{ handle, text, status, page }`, the result shape.

### JS binding

The JS runtime registers a global `outl` object with a `query` method.
It converts the JS argument to `QueryParams`, calls `run_query_structured`, and returns a JS array of `{ handle, text, status, page }` objects.

Full API docs: `docs/query.md` § Plugin SDK API.

## Output format contract

When `ExecOutput.format == OutputFormat::Embeds`, the orchestrator:

1. Splits stdout into non-empty lines.
2. Each line is expected to be an embed reference (`!((blk-XXXXXX))`).
3. Calls `upsert_result_embeds` to create child bullets under the result header.
4. The header reads `> **result:** (N blocks)`.

When `format == OutputFormat::Text` (default), the orchestrator calls `render_result_body` and `upsert_result_child` — the classic single-child `> **result:**` block.

## Auto-run mechanism

Two paths trigger execution:

1. **Manual `gx`** — calls `run_block_at_index`. Always re-runs, ignores cache.
2. **Auto-run loop** (TUI `actions/exec.rs:run_auto_run_blocks`) — calls `run_block_at_index_if_source_changed`.
   Normally gated by the `auto-run::` block property.
   **Runtimes with `auto_run() == true`** (only `query` today) are also collected as auto-run targets, regardless of the property.
   The TUI collector (`exec.rs`) and desktop (`run_auto_run_blocks` command) both honor this — query blocks auto-run on every page load and after every save.

## What this crate does NOT own

- The flat-DFS walk to find a block by cursor position — that's `outl_actions::flat_index_for_block`.
- The Tauri command surface — that's `outl_tauri_shared`.
- The TUI keybinding — that's `outl_tui`.
- Page rendering or sidecar management — that's `outl_md`.
- Workspace tree or op log — that's `outl_core`.

## Dependencies

- `outl-core` — for `Workspace`, `NodeId`, `HlcGenerator`.
- `outl-md` — for `parse`, `render`, `reconcile_md`, `WorkspaceIndex`, `KNOWN_ALIASES`.
- Language interpreters behind features: `steel-core` (lisp), `boa_engine` (js), `rustpython-vm` (python), `mlua` (lua), `wasmtime` (rust/wasm).
- The `query` runtime needs no external dependency — it runs against the in-process `WorkspaceIndex`.

## When you're done

Run `/check` (fmt + clippy + test on the workspace).
The crate has `#![warn(missing_docs)]` — every new `pub` item needs a doc comment.
