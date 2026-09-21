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
| `tag` | `tag: ops` | Block carries `#ops` or a tag nested under it (`#ops/deploy` matches; `#opsec` does not) |
| `prop` | `prop: status` | Block carries the property `status::`. `prop: status: done` narrows it to one value |
| `kind` | `kind: journal` | Hosting page kind: `journal` or `page` |
| `since` | `since: 7d` | Journal within N days. Units: `d` (days), `w` (weeks), `m` (months) |
| `text` | `text: deploy` | Substring in block text (case-insensitive) |
| `sort` | `sort: page, status` | Sort criteria, applied left-to-right. Keys: `page`, `status`, `text` |
| `limit` | `limit: 50` | Maximum number of results |

Every **filter** above also exists as `not-<key>`, taking the same values: `not-status`, `not-tag`, `not-prop`, `not-kind`, `not-since`, `not-text`.
`sort` and `limit` are not filters and have no negative — `not-sort` is rejected as an unknown key.

Every directive is repeatable and every line ANDs, positives included: two `tag:` lines mean "carries both", two `not-tag:` lines mean "carries neither".

### Negative filters

`not-<key>` is the exact complement of `<key>`, and not by convention: the parser reads `not-tag: x` by parsing `tag: x` and wrapping the result, and the engine answers it with one `!`.
There is no second matcher to disagree with the first, so a fence carrying both `tag: x` and `not-tag: x` returns nothing — for every key, including ones added later.

Repeated lines **exclude more**, they do not widen.

````markdown
- ```query
  status: todo
  tag: work
  not-tag: research
  not-tag: future
  not-tag: someday
  sort: page, status
  limit: 100
  ```
````

`not-since:` is the one that reads oddly, and it reads oddly because it is honest: `since: 7d` means "a journal dated within 7 days", so `not-since: 7d` means everything else — including every ordinary page, which was never a journal.
Read it as `!since`, not as "older than". `not-since: 7d` plus `kind: journal` is the one you probably wanted.

Matching stops at the tag boundary on both sides.
`not-tag: work` drops `#work` and `#work/ops`, and leaves `#workflow` alone — a substring negative would hide live work behind a filter written to hide something else, with nothing on screen to notice.

`prop: key` asks only whether the property is present; `prop: key: value` narrows to one value.
A dangling colon (`not-prop: status:`) is a **parse error**, not a wildcard: reading it as "any status" would drop far more than the query asked for.
The `.md` spelling is accepted too, so `prop: status:: done` works.

The same rule covers tags. `not-tag:` with no name is rejected, and so is a name outside the tokenizer's alphabet (letters, digits, `-`, `_`, `/`) — the DSL has no trailing comments, so `not-tag: research # parked stuff` would otherwise build a filter that can never equal a tag and quietly exclude nothing.
A leading `#` is fine: `not-tag: #research` and `not-tag: research` are the same filter.

Keys and values are matched **case-insensitively**, like every other directive here.

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

### Open work tasks, minus the parking lot

````markdown
- ```query
  status: todo
  tag: work
  not-tag: someday
  not-prop: status: parked
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
| DSL parser | `crates/outl-exec/src/runtimes/query/dsl.rs` | Line-by-line `key: value` parse into `Query` struct |
| Execution engine | `crates/outl-exec/src/runtimes/query/engine.rs` | Filter + sort + limit against `WorkspaceIndex` |
| Runtime + public API | `crates/outl-exec/src/runtimes/query/mod.rs` | `QueryRuntime` (returns `OutputFormat::Embeds`, `auto_run() == true`) plus `QueryParams` / `run_query_*` |
| Tag boundary predicate | `crates/outl-md/src/tag.rs` (`text_contains_tag_or_child`) | Single owner of "does this text carry `#tag` or a child of it" |
| Orchestrator | `crates/outl-exec/src/orchestrate.rs` | Detects `Embeds` format, calls `upsert_result_embeds` |
| Result rendering | `crates/outl-exec/src/result_block.rs` (`upsert_result_embeds`) | Creates child bullets from stdout lines |
| Feature flag | `crates/outl-exec/Cargo.toml` (`lang-query`) | On by default in the workspace |
| Language aliases | `crates/outl-md/src/lang.rs` (`KNOWN_ALIASES`) | `query` and `tasks` both resolve to `query` |

The runtime uses the index its caller injected (`ExecContext::index`) and falls back to building one from `ctx.workspace_root` when there is none — no incremental index today.
On the fallback path every fence execution re-reads and re-parses the whole workspace, so a caller that already holds a `Workspace` should derive the index once (`outl_actions::index::derive`) and pass it in.
Every shipping caller now does: the TUI's auto-run loop passes the index it already maintains, and the desktop / mobile paths derive one per sweep.
The fallback remains for an embedder that has neither.
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
| `page` | Filter by hosting page slug (`page: inbox`) |
| `group` | Group results by field (`group: page`) |

There is no general boolean grouping, and no OR across positive filters — "tagged `#meeting` **or** `#code-review`" cannot be said in one fence ([issue #323](https://github.com/outlmd/outl/issues/323), open question 3).
That is a grammar change, not another `Filter` variant, so it is deliberately not bolted onto the flat directive list.

New filters are `enum Filter` variants in `crates/outl-exec/src/runtimes/query/dsl.rs` — one match arm in `engine.rs` per filter, no parser change needed beyond recognizing the key.
**The negative comes for free and must stay that way.** `not-<key>` parses `<key>` and wraps it in `Filter::Not`, which the engine answers with one `!`, so a new directive ships negatable without extra work. A hand-written `NotFoo` variant is the thing to refuse in review: it is a second opinion about what `foo` means, and the direction it drifts is the one that silently removes results.

## Plugin SDK API (`outl.query`)

The query engine is also available as a **structured API** inside JS code blocks and plugins.
Instead of the DSL string, pass a plain object — both paths converge on the same engine.

```js
// Inside a ```js block or plugin:
const tasks = outl.query({
  status: "todo",
  tag: "ops",
  notTag: ["someday", "future"],
  notProp: "status: parked",
  notText: "wontfix",
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
| `tag` | `string` | Block carries `#tag` or a tag nested under it |
| `prop` | `string \| string[]` | Require each property: `"key"`, or `"key: value"` |
| `kind` | `"journal"` \| `"page"` | Hosting page kind |
| `since` | `string` | Duration: `"7d"`, `"2w"`, `"3m"` |
| `text` | `string` | Substring search (case-insensitive) |
| `sort` | `string` | Sort key: `"page"`, `"status"`, `"text"` |
| `limit` | `number` | Max results |

And one negative per filter, taking the same values: `notStatus`, `notTag`, `notProp`, `notKind`, `notSince`, `notText`.
`prop`, `notTag` and `notProp` accept a string or an array of strings; the rest take one string.
A non-string array entry is an error, not a silent drop — dropping one quietly hands back the rows the caller asked to exclude.

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
    not_tag: vec!["someday".into()],
    not_prop: vec!["status: parked".into()],
    ..Default::default()
};
let hits = run_query_structured(&params, &workspace_root)?;
```

## Not the same thing as `outl query`

The CLI's `outl query` filters **pages**; this DSL filters **blocks**.
They share a name and not a matcher: `outl query --tag=ops` asks whether the page's subtree mentions `#ops` *exactly and case-sensitively*, while `tag: ops` here also answers for `#ops/deploy` and ignores case.
A property filter is spelled `--prop key=value` there and `prop: key: value` here, and the CLI's reads the page's **own** `key::` property rather than properties on blocks inside it.
Each negative is the exact complement of the positive **on its own surface**, which is the guarantee that actually matters — see [`cli.md`](cli.md) for the page-level flags.
