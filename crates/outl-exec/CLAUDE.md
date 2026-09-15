# CLAUDE.md — outl-exec

Context for Claude Code sessions working in this crate.
Read this before making any change.

## What this crate is

Code-block execution engine for outl: takes a fenced markdown block (` ```lisp `, ` ```python `, ...) and writes the result back into the page as a child subblock — idempotent on re-run.

The crate is intentionally tiny and modular:

- **`runtime::Runtime`** — the trait you implement to add a new language.
  Two required methods: `language()` (the fence info-string) and `execute()`.
  Two optional overrides: `auto_run()` and `needs_workspace_index()` (both default `false`).
  `needs_workspace_index()` is how a caller knows whether to derive a `WorkspaceIndex` at all — a `python` fence must not pay for a facility only `query` uses.
  If your runtime reads `ctx.index`, say so here, and still handle `None`.
- **`runtime::ExecContext<'a>`** — workspace root, stdin, timeout, mem limit, **`index: Option<&'a WorkspaceIndex>`**.
  Borrowed, not owned: a caller that keeps an index (the TUI rebuilds one on a background thread) would otherwise clone a 64k-block structure per fence to serve a field that exists to avoid exactly that work.
  The lifetime is why every `Runtime::execute` signature reads `&ExecContext<'_>`.
  The `query` runtime cannot answer without an index; left `None` it builds one from `workspace_root` on **every** fence execution (walkdir + comrak + every sidecar).
  A caller holding a `Workspace` should derive it once (`outl_actions::index::derive`) and inject it.
  This crate cannot derive one itself — `outl-actions` depends on it, so injection is what keeps the dependency acyclic while the work still happens once.
- **`runtime::ExecOutput`** — stdout, stderr, duration, exit status, and `format: OutputFormat`.
- **`runtime::OutputFormat`** — `Text` (default: result subblock as `> **result:** …`) or `Embeds` (result subblock with one child bullet per stdout line, rendered as embeds).
- **`registry::RuntimeRegistry`** — resolves a fence info-string to the concrete `Runtime`.
- **`sandbox`** — cross-platform termination helpers.
  `Deadline` hands the interpreter a flag it polls, so it stops itself; `with_timeout` runs the work on a thread and releases the caller without stopping it.
  They are not interchangeable — see [What a block may reach, and for how long](#what-a-block-may-reach-and-for-how-long).
- **`result_block`** — pure functions that find / create the result subblock under a code block.
  Includes `upsert_result_child` (text), `upsert_result_embeds` (embed children), and hash-stamped variants for auto-run cache.
- **`orchestrate::run_block_at_index`** — single entry point for every UI.
  Takes a workspace + page path + block flat-index, runs, persists, reconciles.
  Takes a `Option<&WorkspaceIndex>` — pass one whenever you have one, and a client running many blocks (an auto-run sweep) must pass **one** index for the whole loop, never derive per block.
- **`orchestrate::run_block_at_index_if_source_changed`** — cache-aware variant used by auto-run loop.


## What a block may reach, and for how long

Two questions every runtime has to answer, pinned together in `tests/sandbox.rs` rather than per runtime file — adding a language must not get to answer them differently by omission.

**A fence body is not always written by the person running it.**
It arrives over iroh from a paired device, through `outl import` from someone else's graph, or under an LLM agent driving `outl_template_run` over MCP.
So the host surface an interpreter exposes is a security boundary, and it is an **allowlist**: a denylist fails open, because the next release of an interpreter crate adds a library nobody decided to grant.

**`lisp` had the same hole and it was found reviewing the fix for `lua`.**
`Engine::new()` registers `steel/filesystem`, `steel/process`, `steel/tcp` and `steel/http`, so a fence had `command` (a shell), `open-output-file` and `tcp-connect`.
It now builds with `Engine::new_sandboxed()` **and** shadows the names that survive it (`HOST_BINDINGS` in `runtimes/lisp.rs`) — that second half is a denylist and fails open, which is a known weakness held shut by `lisp_cannot_reach_the_host` rather than by design.

`lua` is the cautionary example (issue #278).
It used `Lua::new()`, which loads `StdLib::ALL_SAFE` — and "safe" in mlua's vocabulary means *memory-safe*, not sandboxed.
It excludes `debug` and `ffi` and **includes** `os` (which carries `execute`), `io` and `package`.
`runtimes::lua::stdlib` now names the allowed set, and `LOADERS` strips the base-library loaders separately, because Lua's base library is not covered by any `StdLib` flag.

**Execution is in-process and synchronous**, so a block that does not terminate does not hang "the block" — it hangs the TUI event loop, the desktop while it holds the workspace mutex, and `outl mcp serve` (issue #279).
`Runtime::execute` must honour `ctx.timeout`, and there are two grades of doing so:

| runtime | mechanism | stops the work? |
|---|---|---|
| `lua` | mlua `set_global_hook`, every 10k VM instructions | yes — the VM unwinds itself |
| `rust` | wasmtime epoch interruption (`wasm::module`) | yes |
| `python` | `sandbox::with_timeout` | no — caller released, thread leaks |
| `lisp` | `sandbox::with_timeout` | no — caller released, thread leaks |
| `js` | `sandbox::with_timeout` | no — caller released, thread leaks |
| `query` | none — bounded by the workspace, not by a clock | n/a |
| `echo` | none — returns its input | n/a |

`set_global_hook`, not `set_hook`: the per-thread variant is not inherited by a coroutine the *script* creates, and mlua responds to a missing per-thread callback by disabling the hook — so `coroutine.wrap(function() while true do end end)()` ran unbounded.

**A deadline does not bound memory.**
It counts VM instructions, so it cannot fire inside one long C call — `string.rep('x', 2e9)` is a single instruction and two gigabytes.
`ctx.mem_limit` is the other half, and `lua` is the only runtime that can honour it (`Lua::set_memory_limit`); it does, when a caller sets one.
No caller does today — `orchestrate` passes `None` — so the field is inert rather than false.

The weak form is a deliberate trade, not an oversight: the alternative to a leaked thread is a frozen client.
Before choosing it for a new runtime, check the interpreter for a cancellation point and record what you found.
`sandbox`'s module doc holds that survey for the three above — including the one that looks like a cancellation point and is not: `steel`'s `with_interrupted` stores a flag the VM never reads.
Re-check each on a dependency bump.

## Adding a new language runtime

1. Add a `lang-<name>` feature in `Cargo.toml`.
2. Create `runtimes/<name>.rs` with one struct + one `impl Runtime`.
3. Register it in `RuntimeRegistry::with_builtins` behind the feature.
3.5. **Answer the two questions in [What a block may reach, and for how long](#what-a-block-may-reach-and-for-how-long)**: name the host surface explicitly (allowlist, never the interpreter crate's default constructor) and say what happens at `ctx.timeout`.
   Add both to `tests/sandbox.rs`.
   Both holes this section describes — `lua` in issue 278 and `lisp` found reviewing its fix — were an embedder default nobody re-read, so this step is the one that would have caught them.
4. Add aliases to `KNOWN_ALIASES` in `crates/outl-md/src/lang.rs` **and** the TS mirror at `crates/outl-frontend-shared/src/highlight/aliases.ts`.
5. If the runtime needs workspace access, override `needs_workspace_index()` to `true`, read `ctx.index` first, and only build one from `ctx.workspace_root` when it is `None`.
6. If the runtime should auto-run on page load, override `auto_run()` to return `true`.

See `runtimes/query.rs` for the most advanced example (workspace access, embed output, auto-run).

## The `query` runtime

The `query` runtime (`runtimes/query.rs`) is a special case:

- **Returns `OutputFormat::Embeds`**: each stdout line becomes an embed child (`!((blk-XXXXXX))`) under the result header.
  This makes query results **live references** to the original blocks, not copies.
- **Overrides `auto_run()` to `true`**: query blocks always re-run on page load, without needing `gx` or `auto-run::`.
- **Overrides `needs_workspace_index()` to `true`** — the only runtime that does, pinned by `tests/query_uses_injected_index.rs`.
- **Uses `ctx.index` when the caller supplied one**, and builds a `WorkspaceIndex` from `ctx.workspace_root` otherwise — the fallback keeps every existing caller working but re-reads the whole workspace per fence.
  `run_query_dsl_with_index` is the injected-index entry point.
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

- **`run_query_dsl(dsl, root)`** — user-facing DSL string → `Vec<QueryHit>`. Builds an index off disk; used by `QueryRuntime::execute` only when the caller injected none.
- **`run_query_structured(params, root)`** — plugin-facing struct → `Vec<QueryHit>`. Exposed to JS as `outl.query({ ... })`.
- **`run_query_dsl_with_index(dsl, &index)`** — the DSL path against an index the caller already holds. Prefer it; a page with several ` ```query ` fences otherwise pays a full workspace read per fence.
  There is deliberately **no** structured counterpart. The only caller that would want one is the `outl.query()` JS binding, and Boa stores a capturing native fn in the `Context`, which outlives the call, so a borrowed index cannot travel into it. Owning it behind an `Arc` would reinstate the per-fence clone this field removes, to serve the path that is not the hot one.

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
   The TUI collector (`exec.rs`) and both GUI clients (the `run_auto_run_blocks` command, registered by `exec_commands!` on desktop and mobile alike) honor this — query blocks auto-run on every page load and after every save.

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
