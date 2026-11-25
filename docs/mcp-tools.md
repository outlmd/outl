# MCP tool reference

Every tool `outl mcp serve` registers, with its arguments, what it returns, and what it can break.
There are **49**, and `tools/list` is the authority: this page is checked against it.

New here?
Start at [MCP](mcp.md) to wire a host, then come back when you need an argument name.
Building a skill or slash command?
[MCP recipes](mcp-recipes.md) has worked examples that use the tools below.

## How to read this page

- **Mutates** means the call appends ops to the log (and reprojects the page `.md`).
  Everything else is a read.
  Read-only tools never need approval from a careful host, and the mutating ones are where you point an allowlist.
- **Confirm** means the tool refuses unless you pass `confirm: true`.
  Without it you get `CONFIRM_REQUIRED` and the workspace is untouched.
- Ids are ULIDs (`01M897XCT6KWK2D1D0Q8P9VZVW`) unless a column says slug.
  A slug is the page's kebab-case name (`ideas`, `2026-11-25`), never its title.
- Replies are compact JSON in `content[0].text`.
  Errors set `isError: true` and carry `{ ok: false, error: { code, message, data } }` in `structuredContent`.
  The error codes are listed at the bottom.
- A tool maps to the CLI command of the same shape, and the CLI page ([`cli.md`](cli.md#commands-by-domain)) has the long-form semantics.
  This page links there instead of repeating it.

## Pages

| Tool | Arguments | Returns | Notes |
|---|---|---|---|
| `outl_page_get` | `slug` | `{ icon, meta, outline }` | `outline` is the block tree with ids. `meta` is `{ id, kind, slug, title }`. |
| `outl_page_list` | `filter?` | `{ pages: [meta, …] }` | `filter` is `tag:foo` or `kind:journal` / `kind:page`. |
| `outl_page_create` | `slug`, `title?`, `icon?`, `content?` | `{ meta, … }` | **Mutates.** Idempotent on `slug`: an existing page is reused, not replaced. `content` is a forest of `{ text, children? }`, so a new page lands with its full outline in one call. On an existing page the forest is **appended again**, so do not retry a create blindly. |
| `outl_page_update` | `slug`, `title?`, `icon?` | `{ meta }` | **Mutates.** A blank title is `INVALID_ARG` and nothing is applied. |
| `outl_page_rename` | `old_slug`, `new_slug`, `merge?` | rename report | **Mutates.** Rewrites `[[old]]` and `#old` in every block and lists the edited blocks in `rewritten_refs`. If `new_slug` is another page it fails with `SLUG_CONFLICT`, unless `merge: true`, which folds into that page (see merge). |
| `outl_page_merge` | `source`, `target`, `confirm` | merge report | **Mutates. Confirm.** Moves the source's blocks, merges properties, rewrites `[[source]]` and `#source`, keeps the source's names as aliases, trashes the source. Safe to run twice (`nothing_changed: true`). No dry run. [RFC 0557](rfcs/0557-page-merge-and-rename.md). |
| `outl_page_delete` | `slug`, `confirm` | `{ id, slug }` | **Mutates. Confirm.** Moves the page root to the trash. |
| `outl_page_render` | `slug` | raw markdown | The clean `.md` projection, no ids. Returned as plain text, not JSON. |

Merge conflicts: when both pages hold a property with different values, the target's value wins and the pair is listed in `property_conflicts`.

## Blocks

| Tool | Arguments | Returns | Notes |
|---|---|---|---|
| `outl_block_get` | `id` | `{ id, parent, text, raw_text, todo, child_count }` | `text` has the `TODO` / `DONE` marker split off into `todo`. |
| `outl_block_tree` | `id` | the block plus all descendants | Same node shape as `outl_page_get`'s `outline`. |
| `outl_block_append` | `text`, `page?` or `parent?` | `{ id, parent, text }` | **Mutates.** Last child of the page or block. Give one of `page` / `parent`. |
| `outl_block_append_tree` | `tree`, `page?` or `parent?` | `{ parent, tree }`, the created blocks with ids | **Mutates.** `tree` is `{ text, children?: [tree, …] }`. `children` holds objects, not strings. Prefer it to chained appends. |
| `outl_block_insert` | `after`, `text` | `{ id, after, text }` | **Mutates.** New sibling right after block `after`. |
| `outl_block_update` | `id`, `text` | `{ id, text }` | **Mutates.** Replaces the whole text. Write `TODO ` / `DONE ` literally to set task state. |
| `outl_block_move` | `id`, `parent?`, `after?` | `{ id, parent }` | **Mutates.** A move that would put a block under its own descendant is refused with `CYCLE_REJECTED`. `after` must share the target `parent`, or it is `INVALID_ARG`. |
| `outl_block_toggle_todo` | `id` | `{ id, text, todo }` | **Mutates.** One step of the cycle: none, `TODO`, `DOING`, `DONE`, none. |
| `outl_block_delete` | `id`, `confirm` | `{ id }` | **Mutates. Confirm.** Moves the block (and its children) to the trash. Undo it with `outl_trash_restore`. |
| `outl_block_embed` | `id_or_handle` | `{ id, handle, page, text, children }` | Resolves `!((blk-xxxxxx))` recursively: the source block and its children. |
| `outl_block_refs` | `id` | blocks that cite it | Who writes `((blk-xxxxxx))` for this block. |

## Journals

| Tool | Arguments | Returns | Notes |
|---|---|---|---|
| `outl_daily_today` | none | `{ date, meta, outline, md }` | **Creates today's journal on first call.** Block ids are in `outline`, so use this (not a render) before editing a block. |
| `outl_daily_get` | `date` | `{ date, meta, outline, md }` | **Creates that day's journal if it does not exist.** `date` is ISO, `today` / `yesterday` / `tomorrow`, or `April 22nd, 2026`. |
| `outl_daily_append` | `text`, `date?` | `{ block_id, date, text }` | **Mutates.** One block at the end of the day (today by default). The fastest way to capture a thought. |
| `outl_daily_range` | `from`, `to` | `{ journals: [entry, …] }` | Inclusive. An entry is `{ date, exists: true, meta, outline }`, or `{ date, exists: false }` for a day with no journal. It does **not** create the missing days. `from` after `to` is `INVALID_ARG`. |

The first two are listed as mutating because the first call of a day writes the journal page.
Every call after that only reads.

## Search, query, links

| Tool | Arguments | Returns | Notes |
|---|---|---|---|
| `outl_search` | `query`, `in?` (`blocks` / `pages` / `all`), `limit?` | `{ blocks: [{ id, handle, page, path, text }], pages: […] }` | Full text. `in` defaults to `all`, `limit` to 50. |
| `outl_query` | filters below | `{ count, results: [meta, …] }` | Filters **pages**. See below. |
| `outl_backlinks` | `slug` | `{ backlinks: […] }` | Each carries the citing block, its ancestors and its page. |
| `outl_tag_list` | none | `{ tags: [{ tag, count, pages }] }` | Every tag with usage counts. |
| `outl_tag_pages` | `tag` | `{ tag, pages }` | Pages that mention `#tag` anywhere in their subtree. |

`outl_query` filters, all optional and ANDed:

| Filter | Meaning |
|---|---|
| `tag` | The page's subtree mentions `#tag`. Exact and case-sensitive: `ops/deploy` is not `ops`. |
| `priority` | The page's `priority::` equals this. |
| `props` | `key=value`, or bare `key` for "has it at all". String or array. Reads the page's own property. |
| `since` | Dated pages on or after the cutoff, plus every undated page. The cutoff is `Nd` (`7d`, days back from today) or an ISO date (`2026-11-01`). Anything else is `INVALID_ARG`. |
| `kind` | `page` or `journal`. |
| `not_tags`, `not_priority`, `not_props`, `not_since`, `not_kind` | The exact complement of the positive filter. `tag: x` with `not_tags: x` returns nothing. |

A wrong type (an array where a string is expected, a number in a list) is `INVALID_ARG` rather than a dropped filter, because a dropped filter returns the whole workspace.
`raw` is reserved for a future query DSL and is rejected today.
The block-level query language is a different thing: see [Queries](query.md).

`outl_search` and `outl_backlinks` answer from the **op log**, not from the `.md` files.
A line that exists only in a file edited outside outl is not found until `outl reconcile` turns it into ops.
Details in [`cli.md`](cli.md#search--query).

## Properties

Page properties are the `key:: value` lines at the top of a page.
Block properties are the same line indented under a block.

| Tool | Arguments | Returns |
|---|---|---|
| `outl_page_prop_set` | `page`, `key`, `value` | `{ page, key, value }` |
| `outl_page_prop_get` | `page`, `key` | `{ page, key, value }`, or `PROP_NOT_FOUND` |
| `outl_page_prop_list` | `page` | `{ page, properties }` |
| `outl_page_prop_delete` | `page`, `key` | `{ page, key }` |
| `outl_block_prop_set` | `id`, `key`, `value` | `{ id, key, value }` |
| `outl_block_prop_get` | `id`, `key` | `{ id, key, value }`, or `PROP_NOT_FOUND` |
| `outl_block_prop_list` | `id` | `{ id, properties: [{ key, value }] }` |
| `outl_block_prop_delete` | `id`, `key` | `{ id, key }` |

`set` and `delete` mutate.
`set` with an empty value removes the key, so the `.md` never keeps a bare `key::` line.
A block property value with a newline, a carriage return or a null byte is `INVALID_ARG` and the page is untouched.
None of these need `confirm`: the previous value stays in the op log.

## Trash

Deleting is a move to the trash, never a physical removal, so it can be undone.

| Tool | Arguments | Returns | Notes |
|---|---|---|---|
| `outl_trash_list` | none | `{ entries: [{ id, parent, preview, blocks, deleted_at_ms, restorable, refusal }] }` | `restorable: false` comes with a `refusal` sentence that says why. |
| `outl_trash_restore` | `id` | `{ id, parent, preview, blocks }` | **Mutates.** Puts the block back under the parent it was deleted from. No `confirm`: the worst case is a block reappearing. |

Restoring refuses a deleted **page** (`TRASH_PAGE_UNSUPPORTED`) and a block whose parent is also in the trash (`TRASH_PARENT_TRASHED`).
Nothing is lost in either case: the content is still in the log, and the refusal tells you which tool prints it.
The full code table is in [`cli.md` → Trash](cli.md#trash).

## Files in and out

These three reach past the workspace folder.
Read the [safety model](mcp.md#what-an-agent-can-and-cannot-touch) before allowing them without a prompt.

| Tool | Arguments | Returns | Notes |
|---|---|---|---|
| `outl_asset_add` | `path`, `page?` | `{ block_id, display_name, is_image, markdown, rel_path, target }` | **Mutates.** Copies the file at `path` (a path on the machine running the server, not bytes) into `assets/<hash>.<ext>` and appends its link as a block. Defaults to today's journal. A file over `[assets] max_bytes` (default 100 MiB) is `INVALID_ARG`. |
| `outl_export_md` | `slug` | raw markdown | Same text as `outl_page_render`. |
| `outl_export_json` | `slug` | `{ meta, blocks, … }` | The page's AST plus sidecar data. The interchange format. |
| `outl_export_hugo` | `slug`, `out_dir` | `{ slug, path, bytes }` | **Writes a file to `out_dir`**, anywhere the server's user can write. Frontmatter comes from page properties. |

`outl_asset_add` rejects `daily` together with `page` (`INVALID_ARG`); `daily` is the default and never needs setting.

## Templates

A template is a page with a non-empty `template::` property.
The user-facing guide is [Templates](templates.md).

| Tool | Arguments | Returns | Notes |
|---|---|---|---|
| `outl_template_list` | none | `{ count, templates: [{ name, slug, page_id }] }` | |
| `outl_template_apply` | `name`, `page`, `block?` | `{ template, page, count, created_blocks }` | **Mutates.** Deep-copies the template's blocks under `block` (or the page root), replacing `{{date}}`, `{{today}}`, `{{yesterday}}`, `{{tomorrow}}`, `{{page}}` and `{{time}}` (`HH:MM`). The copied root carries a `from-template::` property naming the template. |
| `outl_template_resolve` | `name` | `{ template_slug, language, source, params }` | Reads a callable template without running it. |
| `outl_template_run` | `name`, `page`, `block`, `params?` | `{ template, page, block, result }` | **Mutates and executes code.** Runs the template's code block and writes a `> **result:**` subtree under `block`. `block` must belong to `page`. `params` is `{ "k": "v" }`, strings only. |

`outl_template_run` runs the code block through the same runtime as the apps' run button.
Treat it as the one tool here that executes something.
It needs a build with the matching language runtime, and fails with `no runtime for <lang>` otherwise.

## Batch

`outl_batch` applies a list of `{ op, args }` writes in one session, in order.

```json
{ "ops": [
  { "op": "page_create", "args": { "slug": "ideas" } },
  { "op": "block_append_tree",
    "args": { "page": "ideas", "tree": { "text": "root", "children": [{ "text": "child" }] } } },
  { "op": "page_prop_set", "args": { "page": "ideas", "key": "status", "value": "draft" } }
] }
```

`op` is one of:

`page_create`, `page_update`, `page_delete`, `page_rename`, `page_merge`,
`block_append`, `block_append_tree`, `block_insert`, `block_update`, `block_move`, `block_delete`, `block_toggle_todo`,
`daily_append`, `page_prop_set`, `page_prop_delete`.

Each op's `args` are the arguments of the matching tool, and `confirm` rules carry over (`page_merge` needs `confirm: true` inside its `args`).

It **stops at the first error and does not roll back.**
The reply is a normal success (`isError: false`) that carries `applied` (how many ran), `failed_at` (zero-based index), `failed_op`, `error` and a `results` entry per op that ran.
So check for `failed_at` before telling the user it worked.
The ops that already ran stay in the log, and the suffix never ran: resend only that suffix.
There is no `block_prop_*` or `trash_*` op in a batch; call those tools directly.

## Workspace

| Tool | Arguments | Returns |
|---|---|---|
| `outl_workspace_info` | none | `{ root, actor, pages, journals, blocks, ops, backlinks, assets, properties, last_modified, … }` |
| `outl_workspace_doctor` | none | `{ actor, error_count, findings: [{ severity, message }], … }` |

`outl_workspace_doctor` is the read-only half of `outl doctor`: it checks the op log, sidecars and block refs, and never repairs.
`severity` is `ok`, `info`, `warn` or `error`.
Repair (`outl doctor --repair`) stays a terminal command.

## Error codes

| `error.code` | When | What to do |
|---|---|---|
| `CONFIRM_REQUIRED` | A destructive tool without `confirm: true`. | Ask the user, then resend with `confirm: true`. |
| `PAGE_NOT_FOUND` | The slug names no page. | Check `outl_page_list`. A title is not a slug. |
| `BLOCK_NOT_FOUND`, `INVALID_BLOCK_ID` | The id is not a block id, or no such block. | Re-read the page to get ids. Ids come from `outline` fields, never from rendered markdown. |
| `NO_WORKSPACE` | The `--workspace` path has no `.outl/` folder, or its `config.toml` does not load. | Fix the host config. Not retryable from inside the conversation. |
| `INVALID_ARG` | A missing or mistyped argument, bad date, unknown tool or batch op. | The message names the argument. |
| `INVALID_DATE` | A date the parser rejects. | Use ISO `YYYY-MM-DD`. |
| `SLUG_CONFLICT` | Renaming onto an existing page. | Pass `merge: true`, or pick another slug. |
| `PROP_NOT_FOUND` | `*_prop_get` on a missing key. | Not an error to retry. |
| `NOT_TRASHED`, `TRASH_*` | `outl_trash_restore` refused. | The `refusal` text in `outl_trash_list` says why. |
| `CYCLE_REJECTED` | A move that would nest a block in its own descendant. | Pick another parent. |
| `PAGE_MARKDOWN_AHEAD_OF_LOG` | The page's `.md` holds lines the log never recorded, and a write would delete them. | Stop retrying. Tell the user to run `outl reconcile --ahead-of-log`. `error.data` has `path`, `lines`, `sample`, `recovery_command`. |
| `INTERNAL` | Anything else. | Run the same command on the CLI with `--json` for a readable error. |

Protocol-level faults (malformed JSON, an unknown method) are JSON-RPC errors, not tool errors.
