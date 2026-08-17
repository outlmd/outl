# Query code blocks

` ```query ` is a fence language that runs a declarative DSL against the workspace and renders matching blocks as **live embed references** (`!((blk-XXXXXX))`), not copies.
Toggling a TODO on the original block is reflected everywhere the query result appears.

It solves the "tasks scattered across notes" problem ([issue #139](https://github.com/outlmd/outl/issues/139)) without introducing a separate task manager — every block is already a potential task via the `TODO` / `DONE` prefix.

> **Why a line-oriented DSL in a code fence and not datalog:** [RFC 0139](rfcs/0139-query-language.md).

## How it works

1. You write a ` ```query ` fence inside any page.
2. On page load, the query runtime scans the workspace's block index (`WorkspaceIndex`) — every block in every page and journal.
3. Blocks matching **all** directives (implicit AND) are collected, sorted, and optionally limited.
4. Each match is rendered as a child bullet containing `!((blk-XXXXXX))` — an embed reference to the original block.
5. The result is a **live view**: edit the original block and every query that surfaces it updates on the next run.

Query blocks **auto-run on every page load** — they never need `gx` or the `auto-run::` property.
Results depend on workspace state, not the fence body, so caching by source-hash would be incorrect.

## Syntax

Each line is a `key: value` directive.
Lines are implicitly ANDed — a block must match every directive to appear in the results.
Blank lines and `#`-prefixed comments are ignored.

````markdown
- ```query
  # all open tasks tagged #ops, sorted by page
  status: todo
  tag: ops
  sort: page
  limit: 50
  ```
````

### Directives

| Key | Example | Description |
|-----|---------|-------------|
| `status` | `status: todo` | Filter by task state: `todo` (not started), `doing` (started), `done` (completed), or `open` (**any** task, DONE included) |
| `tag` | `tag: ops` | Block text contains `#ops` (partial match — `#ops/deploy` matches `tag: ops`) |
| `kind` | `kind: journal` | Hosting page kind: `journal` or `page` |
| `since` | `since: 7d` | Journal within N days. Units: `d` (days), `w` (weeks), `m` (months) |
| `text` | `text: deploy` | Substring in block text (case-insensitive) |
| `sort` | `sort: page, status` | Sort criteria, applied left-to-right. Keys: `page`, `status`, `text` |
| `limit` | `limit: 50` | Maximum number of results |

## Examples

### All open tasks across the workspace

````markdown
- ```query
  status: todo
  sort: page
  ```
````

### What I'm in the middle of

````markdown
- ```query
  status: doing
  sort: page
  ```
````

`status: todo` does **not** include these — once a task is started it answers to `doing`, and `open` is the filter that covers every task regardless of state.

### Today's open tasks in journals

````markdown
- ```query
  status: todo
  kind: journal
  since: 1d
  ```
````

### Tasks tagged #sprint-planning, grouped by page

````markdown
- ```query
  status: open
  tag: sprint-planning
  sort: page, status
  limit: 100
  ```
````

### Blocks mentioning "deploy" in the last week

````markdown
- ```query
  text: deploy
  kind: journal
  since: 7d
  ```
````

### All completed tasks

````markdown
- ```query
  status: done
  sort: page
  ```
````

## How results render

The query runtime returns `OutputFormat::Embeds`, which tells the orchestrator to render each result as a child bullet with an embed reference instead of dumping stdout text.

The rendered structure under the ` ```query ` block looks like:

````markdown
- ```query
  status: todo
  ```
  - > **result:** (3 blocks)
    - !((blk-abcdef))
    - !((blk-ghijkl))
    - !((blk-mnopqr))
````

When the page is opened in the TUI or desktop, each `!((blk-…))` expands to show the original block's text and subtree.
Because these are embeds — not copies — toggling a TODO on the original block updates the query result on the next page load.

## Architecture

| Component | Location | Role |
|-----------|----------|------|
| DSL parser | `crates/outl-exec/src/runtimes/query.rs` (`dsl` module) | Line-by-line `key: value` parse into `Query` struct |
| Execution engine | `crates/outl-exec/src/runtimes/query.rs` (`engine` module) | Filter + sort + limit against `WorkspaceIndex` |
| Runtime | `crates/outl-exec/src/runtimes/query.rs` (`QueryRuntime`) | Implements `Runtime`, returns `OutputFormat::Embeds`, `auto_run() == true` |
| Orchestrator | `crates/outl-exec/src/orchestrate.rs` | Detects `Embeds` format, calls `upsert_result_embeds` |
| Result rendering | `crates/outl-exec/src/result_block.rs` (`upsert_result_embeds`) | Creates child bullets from stdout lines |
| Feature flag | `crates/outl-exec/Cargo.toml` (`lang-query`) | On by default in the workspace |
| Language aliases | `crates/outl-md/src/lang.rs` (`KNOWN_ALIASES`) | `query` and `tasks` both resolve to `query` |

The runtime builds a `WorkspaceIndex` from `ctx.workspace_root` on every execution — no incremental index today.
For workspaces under ~1000 pages this is sub-second; the per-page op log shards plan ([`docs/sync.md`](sync.md)) will be needed before 10k pages.

## Relationship to `{{query: ...}}`

The inline token `{{query: ...}}` is a **legacy Roam-ism** — the parser treats it as opaque text.
The ` ```query ` code block supersedes it: it's standard CommonMark (no magic tokens), reuses the existing exec infrastructure, and produces live embeds instead of static text.

There are no plans to implement the inline `{{query: ...}}` DSL.
If a future need arises, it would be a separate parser token, not a reuse of the code block runtime.

## Extensibility

The DSL is designed to grow without breaking existing queries.

Planned filters (not yet implemented):

| Key | Description |
|-----|-------------|
| `prop` | Filter by block property (`prop priority: high`) — requires the block index to expose properties |
| `page` | Filter by hosting page slug (`page: inbox`) |
| `group` | Group results by field (`group: page`) |

New filters are `enum Filter` variants in `crates/outl-exec/src/runtimes/query.rs` — one match arm per filter, no parser change needed beyond recognizing the key.

## Plugin SDK API (`outl.query`)

The query engine is also available as a **structured API** inside JS code blocks and plugins.
Instead of the DSL string, pass a plain object — both paths converge on the same engine.

```js
// Inside a ```js block or plugin:
const tasks = outl.query({
  status: "todo",
  tag: "ops",
  sort: "page",
  limit: 50,
});

for (const t of tasks) {
  const mark = t.status === "done" ? "[x]" : "[ ]";
  console.log(`${mark} ${t.text} — (${t.page})`);
}
```

### Parameters

| Field | Type | Description |
|-------|------|-------------|
| `status` | `"todo"` \| `"doing"` \| `"done"` \| `"open"` | Filter by task state (`"open"` is any task, DONE included) |
| `tag` | `string` | Block contains `#tag` (partial match) |
| `kind` | `"journal"` \| `"page"` | Hosting page kind |
| `since` | `string` | Duration: `"7d"`, `"2w"`, `"3m"` |
| `text` | `string` | Substring search (case-insensitive) |
| `sort` | `string` | Sort key: `"page"`, `"status"`, `"text"` |
| `limit` | `number` | Max results |

All fields are optional — `outl.query({})` returns every block.

### Result shape

Each hit is an object:

```ts
interface QueryHit {
  handle: string;   // "blk-XXXXXX"
  text: string;     // block text, task prefix stripped
  status: string | null;  // "todo", "doing", "done", or null (not a task)
  page: string;     // hosting page slug
}
```

### Rust API

The same engine is available as `outl_exec::run_query_structured` for code that runs outside the JS runtime:

```rust
use outl_exec::{QueryParams, run_query_structured};

let params = QueryParams {
    status: Some("todo".into()),
    tag: Some("ops".into()),
    ..Default::default()
};
let hits = run_query_structured(&params, &workspace_root)?;
```
