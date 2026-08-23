# CLI

outl ships one binary — `outl` — that does everything a workspace needs from outside the TUI: scripts, cron jobs, CI, editor integrations, and LLM agents.
This document is the surface contract.

Looking for how to plug outl into Claude Desktop / Cursor / other MCP hosts? → [docs/mcp.md](mcp.md).
The two share the same handlers under the hood, so this page stays the source of truth for what each command does.

## The bet — one binary, one surface

Knowledge bases get integrated everywhere: editors, shell scripts, LLM agents, automation.
The wrong move is to grow a separate API per host (REST for one, MCP for another, library for a third).
Each new client doubles the surface and drifts.

outl's bet: **everything reachable from outside the TUI is reachable through the `outl` binary**, with a stable JSON envelope.
Other protocols (MCP today, anything that comes next) are thin shims that shell out to the same commands.
There is one place where logic lives, and that place is `outl-actions`.

## The stack

```text
┌────────────────────────────────────────────────────────────┐
│ Hosts                                                       │
│   shell · cron · CI · editors · Claude Code · Claude Desktop│
└────────────────────────────────────────────────────────────┘
                  │                            │
                  │ subprocess                 │ MCP / stdio
                  ▼                            ▼
┌──────────────────────────────┐  ┌──────────────────────────┐
│ outl <subcommand>            │  │ outl mcp serve           │
│  page · block · daily ·      │  │ (thin shim, declares     │
│  search · query · export …   │  │  tools, calls into the   │
│                              │  │  same handlers below)    │
└──────────────────────────────┘  └──────────────────────────┘
                  │                            │
                  └──────────────┬─────────────┘
                                 ▼
┌────────────────────────────────────────────────────────────┐
│ outl-actions                                                          │
│   block · tree · todo · journal · page · backlinks · sync · template   │
└────────────────────────────────────────────────────────────┘
```

The MCP server is a subcommand of the same binary.
There is no `outl-mcp` crate, no separate distribution, no parallel logic.
A new feature lands once: as a function in `outl-actions`, exposed by one subcommand and one tool, sharing the same handler.

## JSON envelope

Every command that produces machine output emits the same shape so downstream consumers (jq, LLMs, scripts) cache one parser.

Success:

```json
{
  "ok": true,
  "data": { "...": "command-specific payload" },
  "error": null
}
```

Failure:

```json
{
  "ok": false,
  "data": null,
  "error": {
    "code": "PAGE_NOT_FOUND",
    "message": "page 'foo' does not exist"
  }
}
```

Error codes are stable strings, listed alongside each command below when relevant.
Exit codes follow:

| Code | Meaning                                       |
|------|-----------------------------------------------|
| 0    | Success                                       |
| 1    | User error (bad input, not found, conflict)   |
| 2    | Internal error (bug, broken invariant, panic) |
| 3    | Nothing was done, and that is not a failure    |

Exit 3 exists for `outl sync`, which can legitimately do nothing: another process on this device holds the sync endpoint (or the endpoint lease could not be claimed at all), P2P is off, or no device is paired.
A script can then tell "I flushed" from "someone else will" without reading either as an error.

Add `--json` to any command to force JSON.
Without the flag, output is human-readable (tables, colored).
MCP tools wrap the same envelope in the MCP tool-result shape: `structuredContent` carries the full `{ ok, data, error }` envelope and `content[].text` carries either a pretty-printed payload or, for markdown-first tools (`outl_export_md`, `outl_page_render`, `outl_daily_today`, `outl_daily_get`), the raw `.md` string.
Clients should read `structuredContent.data` for typed access.

## Commands by domain

The CLI column is what you type at the terminal.
The MCP tool column is the name Claude Desktop (or any MCP host) sees.

### Page

| CLI                                                            | MCP tool             |
|----------------------------------------------------------------|----------------------|
| `outl page get <slug> [--json]`                                | `outl_page_get`      |
| `outl page create <slug> --title=… [--icon=…] [--content=<JSON\|->] [--slugify]` | `outl_page_create`   |
| `outl page update <slug> [--title=…] [--icon=…]`               | `outl_page_update`   |
| `outl page delete <slug> [--confirm]`                          | `outl_page_delete`   |
| `outl page list [--filter=tag:foo] [--json]`                   | `outl_page_list`     |
| `outl page rename <old-slug> <new-slug>`                       | `outl_page_rename`   |
| `outl page render <slug>`                                      | `outl_page_render`   |
| `outl page history <slug> [--limit=N] [--json]`                 | —                    |

`page get` returns page meta plus the outline tree.
`page render` returns the projected `.md` string (clean, no sidecar fields).
`page rename` updates the `page-slug` property and renames the on-disk `.md`/`.outl` — it does **not** rewrite `[[old_slug]]` references in other pages.
Affected blocks come back in `affected_refs` so the caller can decide whether to bulk-rewrite.

`page create --content` accepts a forest of `[{text, children?}, ...]` (or a single `{text, children?}` for ergonomics) so a brand-new page lands with its full outline in one op-log session instead of a chain of `block append` calls.
Pass `--content -` to read the JSON from stdin.
The returned `content` array mirrors the input and carries the freshly minted block ids, so the caller can keep referencing them in follow-ups.

`page create --slugify` treats the positional argument as a human name and derives the slug from it through the shared `outl_md::slugify` rule (lowercase, fold Latin accents, non-alphanumeric → `-`, collapse + trim).
It is opt-in and idempotent on an already-clean slug, so the default path — and the `outl_page_create` MCP tool — stay literal, keeping hierarchical slugs like `ai-agent/learning` verbatim.
The flag exists so external clients (the Raycast extension's "New Page") can ask the user for a name only and let the one owner of the rule generate the slug, instead of re-implementing slugify.

### Block

| CLI                                                            | MCP tool                |
|----------------------------------------------------------------|-------------------------|
| `outl block get <blk-XXX> [--json]`                            | `outl_block_get`        |
| `outl block append <page> --text=… [--parent=blk-YYY]`         | `outl_block_append`     |
| `outl block append-tree --page=… --tree=<JSON\|->`              | `outl_block_append_tree`|
| `outl block insert --after=<blk-XXX> --text=…`                 | `outl_block_insert`     |
| `outl block update <blk> --text=…`                             | `outl_block_update`     |
| `outl block move <blk> --parent=<blk-YYY> [--after=<blk-ZZZ>]` | `outl_block_move`       |
| `outl block delete <blk> [--confirm]`                          | `outl_block_delete`     |
| `outl block toggle-todo <blk>`                                 | `outl_block_toggle_todo`|
| `outl block tree <blk> [--json]`                               | `outl_block_tree`       |
| `outl block history <blk> [--limit=N] [--json]`                | —                       |

`block move` is the one user-visible name for `Op::Move`.
Cycle detection still applies: a move that would create a cycle returns `{ "code": "CYCLE_REJECTED" }` and the op still goes into the log (see [docs/crdt.md](crdt.md)).
`block toggle-todo` walks `None → TODO → DOING → DONE → None`, same as `outl_actions::cycle_todo`.
One call is one step, so reaching `DONE` from an unmarked block takes three.

`block append-tree` writes a root block plus its recursive children in one op-log session.
`--tree` accepts the JSON shape `{"text": "...", "children": [{"text": "...", "children": [...]}]}`, or `--tree -` to read the JSON from stdin.
The response mirrors the input shape with `id` at every node so the caller can map back to anything they wrote.
Prefer this over chained `outl block append` calls when authoring structured content from a script or agent.

### Daily / Journal

| CLI                                                | MCP tool             |
|----------------------------------------------------|----------------------|
| `outl daily today [--json]`                        | `outl_daily_today`   |
| `outl daily get <date> [--json]`                   | `outl_daily_get`     |
| `outl daily append --text=… [--date=…]`            | `outl_daily_append`  |
| `outl daily range --from=… --to=… [--json]`        | `outl_daily_range`   |

`<date>` accepts ISO (`2026-05-31`) and natural (`"April 22nd, 2026"`, `"yesterday"`, `"tomorrow"`).
Range is inclusive on both sides and emits one entry per day in the interval — days that have no materialised journal come back as `{ exists: false }` placeholders so the caller can spot gaps.

### Asset

| CLI                                                         | MCP tool          |
|-------------------------------------------------------------|-------------------|
| `outl asset add <file> [--page=<slug>] [--daily] [--json]`  | `outl_asset_add`  |

Copies `<file>` into `<workspace>/assets/<hash>.<ext>` (content-addressed, so re-importing identical bytes is idempotent) and appends its markdown link as a new block.
The link is `[name](assets/<hash>.<ext>)` for a document, `![name](…)` for an image.
Target defaults to today's journal (`--daily`); pass `--page=<slug>` to append to an existing page instead — `--page` and `--daily` are mutually exclusive (`INVALID_ARG`), and an unknown slug returns `PAGE_NOT_FOUND`.
A file over `[assets] max_bytes` in `config.toml` (default 100 MiB, `0` = unbounded) is rejected before the copy with `INVALID_ARG`.
The MCP tool takes the file as a `path` argument (MCP is stdio — it receives a filesystem path, not bytes) plus optional `page` / `daily`.
Only the link enters the op log; the asset's bytes are a plain blob replicated alongside the `.md` projections, never through the CRDT.

### Search / Query

| CLI                                                              | MCP tool          |
|------------------------------------------------------------------|-------------------|
| `outl search "<query>" [--in=blocks\|pages] [--json]`            | `outl_search`     |
| `outl query --tag=foo [--priority=p1] [--since=7d] [--json]`     | `outl_query`      |

`search` is full-text and lives today as the TUI's workspace search.
`query` is the structured filter (tag, property, date range, kind).

`search` and `backlinks` answer from the **op log**, not by reading `pages/` and `journals/`.
Practical consequence: a line that exists in a `.md` but in no op is not found.
That happens when a file was edited outside outl and no reconcile has run yet, or when the page is in the state `outl doctor` reports as ahead of the log — `outl reconcile --ahead-of-log` is what turns that content into ops.
In exchange, a block the log holds stays findable even while its sidecar is stale, which the previous disk-walking implementation could not promise: it dropped every block after the first one whose hash disagreed.
The `--raw='…'` flag is reserved for the not-yet-implemented query DSL and currently rejects with `INVALID_ARG` — when the DSL lands it folds into the same `outl_query` tool, not a new one.

### Backlinks / Refs

| CLI                                | MCP tool              |
|------------------------------------|-----------------------|
| `outl backlinks page <slug> [--json]`             | `outl_backlinks`   |
| `outl backlinks block <blk-XXX> [--json]`         | `outl_block_refs`  |
| `outl backlinks embed <blk-XXX\|handle> [--json]` | `outl_block_embed` |

`block embed` resolves `!((blk-XXX))` recursively, returning the source block plus children — the same expansion the TUI does inline.

### Tags / Properties

| CLI                                              | MCP tool             |
|--------------------------------------------------|----------------------|
| `outl tag list [--json]`                         | `outl_tag_list`      |
| `outl tag pages <tag> [--json]`                  | `outl_tag_pages`     |
| `outl page prop set <page> <key>=<value>`        | `outl_page_prop_set` |
| `outl page prop get <page> <key>`                | `outl_page_prop_get` |
| `outl page prop list <page> [--json]`            | `outl_page_prop_list`|

Properties stay in the `key:: value` lines at the top of the page; the CLI never invents a new place to put metadata (see [docs/markdown-format.md](markdown-format.md)).

### Export

| CLI                                                  | MCP tool          |
|------------------------------------------------------|-------------------|
| `outl export hugo <page> --out=./content/posts/`     | `outl_export_hugo`|
| `outl export md <page>`                              | `outl_export_md`  |
| `outl export json <page>`                            | `outl_export_json`|

`export hugo` is the pipeline that drives avelino.run: frontmatter from page properties, block refs flattened, code blocks preserved.
`export md` is the same string `page render` returns.
`export json` is the full AST plus sidecar — the format an external tool would ingest.

### Template

| CLI                                                                        | MCP tool                |
|----------------------------------------------------------------------------|-------------------------|
| `outl template list [--json]`                                              | `outl_template_list`    |
| `outl template apply <name> --page <slug> [--block <id>]`                  | `outl_template_apply`   |
| `outl template resolve <name> [--json]`                                    | `outl_template_resolve` |
| `outl template run <name> --page <slug> --block <id> [--params k=v …]`     | `outl_template_run`     |

`template list` finds every page with a non-empty `template::` property.
`template apply` deep-copies the template's subtree under a target block with variable substitution.
`template resolve` returns a callable template's code block language, source, and declared params.
`template run` executes a callable template — injects `--params`, runs its code block through the shared runtime, and writes the `> **result:**` subtree under `--block`.
The MCP tool takes `params` as an object (`{ "k": "v" }`).
`--block` (and `apply`'s optional `--block`) must belong to `--page`; a cross-page block returns `INVALID_ARG` (reprojection only touches `--page`, so a foreign anchor would silently drop the new blocks from disk).
See [Templates](templates.md) for the full guide.

### Batch

| CLI                                  | MCP tool      |
|--------------------------------------|---------------|
| `outl batch [--ops=<JSON\|->] [--json]` | `outl_batch`  |

`batch` runs a list of write ops sequentially in one workspace session.
Input shape:

```json
{
  "ops": [
    { "op": "page_create",       "args": { "slug": "ideas" } },
    { "op": "block_append_tree", "args": { "page": "ideas",
                                           "tree": { "text": "root",
                                                     "children": [{ "text": "child" }] } } },
    { "op": "page_prop_set",     "args": { "page": "ideas", "key": "icon", "value": "💡" } }
  ]
}
```

Supported `op` names: `page_create`, `page_update`, `page_delete`, `page_rename`, `block_append`, `block_append_tree`, `block_insert`, `block_update`, `block_move`, `block_delete`, `block_toggle_todo`, `daily_append`, `page_prop_set`.
Each op's `args` mirror the matching standalone tool.

**Semantics: stop-on-first-error.** When an op fails, earlier ops stay in the op log (they're already CRDT ops; we don't roll them back) and the response carries `failed_at`, `failed_op`, and `error` so the caller can decide what to do with the suffix that never ran.
CLI exit code is `1` in that case; MCP returns the payload via the normal envelope.

### Workspace / Admin

| CLI                                          | MCP tool                |
|----------------------------------------------|-------------------------|
| `outl init <path>`                           | —                       |
| `outl serve [<path>] [--once] [--no-watch] [--no-sync]` | —           |
| `outl doctor [--json] [--repair] [--force]`  | `outl_workspace_doctor` |
| `outl reconcile [--ahead-of-log] [--allow-bulk-delete]` | —             |
| `outl recover [--apply] [--min-lines=N]`     | —                       |
| `outl mcp serve [--workspace=…]`             | —                       |
| `outl peer pair\|list\|remove\|status`        | —                       |
| `outl plugin init\|search\|list\|install\|run\|config\|secret\|enable\|disable\|remove` | — |
| `outl sync`                                  | —                       |
| `outl workspace info [--json]`               | `outl_workspace_info`   |
| `outl import roam\|logseq\|obsidian\|auto <src> <dst> [--dry-run] [--json] [--preserve-timestamps] [--no-assets] [--force]` | — |

`init`, `serve`, `reconcile`, `recover`, `import`, `mcp serve`, `peer`, `plugin`, and `sync` are CLI-only on purpose — they're either interactive, long-running, or bootstrap commands that don't fit a tool-call shape.


`outl import` runs the adapter-based pipeline in the `outl-import` crate for every source (`roam` = JSON backup file, `logseq` = graph directory, `obsidian` = vault directory; `auto` detects from the source's shape).
`((uid))` block refs and `{{embed}}`s resolve to real `((blk-XXXXXX))` handles, not page-link fallbacks.
Folded blocks (Roam `open: false`, Logseq `collapsed:: true`) land as `Op::SetCollapsed`.
Each dialect is translated on the way in.
Roam: `__italic__` → `*italic*`, flat `{{[[query]]}}` → ` ```query ` fences.
Logseq: `DOING` and `NOW` → outl's `DOING` prefix (`NOW` also keeps a `state:: now` property, the nuance outl has no separate state for).
`LATER`/`WAITING` → `TODO` + `state::` property, `CANCELED` → `DONE` + `state::`, `[#A]` → `priority::`, `SCHEDULED:`/`DEADLINE:` → `[[date]]` links, `:LOGBOOK:` drawers dropped and counted.
A `DOING` block imported before outl had the state was flattened to `TODO ` + `state:: doing` and is indistinguishable from a real `TODO` in every query and count; re-importing that graph is what fixes it.
Obsidian: frontmatter → `key:: value` properties, wiki-link variants collapse to `[[Note]]`.
Referenced files are pulled into the workspace's `assets/` dir, content-addressed.
A local attachment (`![](../assets/pic.png)`) is copied and a remote image (Roam's firebase URLs) is downloaded; either way the link is rewritten to `[name](assets/<hash>.<ext>)`.
A file that can't be pulled (missing, download failed, over `[assets] max_bytes`) keeps its original link and is counted in `assets missing` — never fatal.
`--no-assets` skips all of that, keeping every original relative/remote link verbatim.
A real (non-dry) import paints a live progress line on stderr — phase, page counter, percentage, current page, elapsed — TTY-only, so piped output stays clean.
`--dry-run` parses and reports without writing a byte — run it against a real backup to measure fidelity before migrating.
`--json` prints the full report (per-feature counts, warnings with location) as JSON.
`--preserve-timestamps` keeps source create/edit times as `created::`/`edited::` block properties (dropped and counted by default).

**Importing twice is destructive, so it's opt-in.**
An import overwrites every `.md` it emits and reconciles the result through the op log, so a second run against a workspace you've been using erases whatever you wrote there since the first import — there is no undo.
`outl import` therefore refuses a destination that already holds content, naming what it found and pointing at the escape hatch.
Pass `--force` when overwriting is exactly what you want; import into a fresh directory otherwise.
`--dry-run` writes nothing and is never blocked.

What counts as "already holds content" is read from the **op log's materialized tree**, not from the `.md` files on disk.
A device paired over iroh receives every op through sync, but only projects a page's `.md` when that page is opened.
A freshly-paired laptop therefore holds your whole graph with an empty `pages/` directory, and a file-counting guard would wave the import straight into it.
Two extra signals round it out: markdown dropped into `pages/` by hand (no sidecar beside it, so the tree cannot see it) also blocks, and so does any `.md` in a destination that isn't an outl workspace at all.

The output of `outl init` is **not** content.
`init` seeds a journal-template page and today's (empty) journal, so `outl init ./notes && outl import roam backup.json ./notes` — the documented migration flow — runs with no flags.
A page only counts once it holds a block with real text.
That distinction matters: `--force` is the flag that destroys, and a guard that fires on the normal flow just teaches you to type it by reflex.

**A failed import is resumable, without `--force`.**
The pipeline writes page by page, so a failure at page 40k of 66k leaves the destination half-populated.
For the duration of a real import, `outl import` keeps a marker at `<workspace>/.outl/import-in-progress.json` (adapter, source path, start time) and deletes it on success.
If a run dies, the marker survives and the error message says exactly how to recover.
Re-running the same command then imports again **without** `--force`: everything in that destination came from the run that never finished, so there is nothing of yours to protect.
Delete the destination instead if you'd rather start clean.
A marker that is missing or unparseable is treated as "no unfinished import", so a corrupt file is never a free pass.

**Reconciliation: what the source held vs. what landed.**
The per-feature counts only describe what the pipeline knows it produced — a block lost in the parse would show up in neither the numerator nor the denominator.
So the report also carries a `reconciliation` block (Roam today; other adapters as they start reporting source counts) whose denominators are counted straight off the parsed source:

```text
  reconciliation:
    pages:             3/4 emitted (1 merged, 0 skipped)
    blocks:            12/15 emitted (2 lifted to page props, 1 in skipped pages)
    in the op log:     12/12 emitted blocks confirmed on disk after reconcile
```

Every legitimate reducer is subtracted by name — pages merged onto the same journal date, pages skipped (each listed under `skipped:` with the blocks that went down with it), and blocks promoted into page properties (`blocks_lifted_to_props`).
Whatever is left over is unexplained loss, and the human output says so in a block you can't miss (`UNACCOUNTED CONTENT — the import does not add up`), with the same numbers available under `reconciliation` in `--json`.

The `in the op log` line closes the other half of the contract.
Every other counter in the report is incremented in memory during rendering, before a byte reaches disk — they prove the parser and the renderer agree about your graph, not that your graph is in a workspace.
A page that fails to write, fails to reconcile, or loses blocks in the matcher is invisible to all of them.
So a real import also sums the block entries in each page's sidecar (written by `reconcile_md` straight off the materialized tree) and reports that as `landed_blocks`.
A gap prints as `CONTENT NEVER REACHED THE OP LOG` and makes `balanced` false.
`--dry-run` writes nothing, so it reports the landing as *not measured* rather than as zero-loss: there, `balanced: true` means only that parse and render agree.

Warnings are listed in full under `--json`.
The human output prints the first 20 and states how many it hid.

One counted loss worth knowing about on Roam graphs: a `{{[[TODO]]}}` / `{{[[DONE]]}}` marker in the *middle* of a block keeps its literal `TODO`/`DONE` word but not its task state.
outl models one task per block, driven by the marker at the block's head, so such a block won't answer `outl query --kind=task`.
It's reported as `mid-block tasks` plus a single aggregate warning — one per import, not one per marker.

`outl plugin` manages the workspace's JS plugins (under `<workspace>/.outl/plugins/`), wrapping `outl-plugins`.
`init <NAME> [--id <ID>] [--dir <PATH>]` scaffolds a buildable starter project (manifest + `package.json` + `tsconfig` + `src/index.ts` + README); run `bun install && bun run build` inside it for an installable bundle.
`search [QUERY]` lists installable example plugins from the repo (filtered by `QUERY` when given).
`list` loads every installed plugin and prints each one's version, enabled state, and the slash commands it contributes.
`install <SOURCE>` takes a local directory holding a `plugin.json` plus its bundle (the installed shape), **or** a `github:owner/repo[/subdir][#tag]` source.
A `github:` source is cloned at an immutable semver tag — the newest published tag when none is pinned, never a mutable branch.
It prints the permissions the manifest requests and asks for approval before copying the plugin in and freezing those permissions in the lockfile.
Pass `--yes` to approve non-interactively (required when stdin is not a TTY).
`run <PLUGIN_ID> <COMMAND_ID>` runs a contributed command and re-renders every page's `.md` afterwards, because the op log is the source of truth and the files are a projection.
`config show <ID>` / `config set <ID> <KEY> <VALUE>` read and write the plugin's plaintext config in the lockfile (value coerced to the field's schema type).
`secret set <ID> <KEY> [--value <V>]` / `secret remove <ID> <KEY>` (alias `rm`) manage the plugin's secrets in the **OS keychain** — never in the workspace on disk.
`secret set` prompts for the value on a hidden line when `--value` is omitted (prefer that: a `--value` on the command line lands in your shell history).
`enable <ID>` / `disable <ID>` flip the plugin's `enabled` flag in the lockfile without uninstalling it.
`remove <ID>` (aliases `uninstall`, `rm`) deletes the plugin's directory and its lockfile entry.
`remove <ID>` (aliases `uninstall`, `rm`) deletes the plugin's directory and its lockfile entry.

`outl peer pair` takes an optional `--name <NAME>` — the label this device advertises to the other (shown in the peer's `outl peer list`).
It defaults to the machine hostname; the GUI clients default it to "desktop" / "mobile" and let the user edit it before pairing.

`outl sync` forces a one-shot P2P sync pass (bring the iroh transport up, exchange ops with every paired device, exit).
It's for scripts that mutate via the CLI and must flush to peers before the process dies — a normal short-lived CLI mutation can't keep a connection alive long enough.
The long-lived surfaces (`outl serve`, `outl mcp serve`, the desktop/TUI apps) sync continuously and don't need it.
If one of them is running on this machine, `outl sync` says so and exits without doing anything: a device binds one sync endpoint at a time, and that process is already pushing your ops out.

#### `outl serve` — the background daemon

Two halves, both on by default:

- the **file watcher** reconciles external `.md` edits into the op log (`--no-watch` turns it off);
- the **sync supervisor** holds this device's iroh endpoint so paired peers converge continuously (`--no-sync` turns it off).

`--once` reconciles every `.md` and exits; it implies neither half and conflicts with both flags.
Turning both halves off is a usage error rather than a process that runs and does nothing.

**The sync half defers.**
One endpoint per device identity, elected not assigned — so the supervisor asks for the lease every 30s and treats a refusal as a normal state.
A desktop or TUI that already holds the endpoint keeps it; the daemon takes over when that process exits, and hands it back the next time one wins.
It also stands down entirely when no devices are paired, since holding the endpoint to sync with nobody only denies it to a GUI that could be using it to pair.
It re-reads `.outl/peers.json` and rebuilds the transport when that file changes, so a device paired *after* the daemon started is actually synced with.

**Which flag to run permanently.**
The watcher emits ops, so it takes the exclusive per-actor write lock, and any process that loses the race for the device actor gets a fresh ephemeral actor and its own `ops-<ulid>.jsonl`.
That is cheap for an occasional overlap and expensive for a daemon: `outl serve` running forever holds the device actor, so every later GUI or TUI launch mints one more op-log file.

The sync half has no such cost — the transport writes peer ops to `ops-<peer>.jsonl` and uses your actor id only for the vector clock it offers.
So:

- **`outl serve --no-watch`** — the mode to leave running beside a GUI you also use. No write lock, no ephemeral actors.
- **`outl serve`** — both halves, for a headless box where nothing else opens the workspace.

**What it converges is the op log, not the `.md` files.**
The daemon never re-projects: peer ops land in `ops/`, the materialised tree is reloaded in memory, and the `.md` on that machine keeps whatever text it had.
They catch up the next time a client opens the workspace, or on the next `outl reconcile`.
So a headless box stays a correct *replica*; it is not a place to read current notes off disk.
Re-projecting from a daemon has to clear [invariant 8](../CLAUDE.md) first — overwriting a `.md` that holds content the log never saw is the one failure this project treats as unrecoverable — so it is deliberately not done here.

SIGTERM and SIGINT both shut down cleanly, releasing the endpoint lease.
That release matters: a lease left held by a killed process locks every outl process on the device out of an endpoint.


### `outl doctor`

The integrity check you run before trusting a migration, and after any sync weirdness.
**Read-only by default** — it reports, it never fixes, unless you pass `--repair`.
Exit code is `1` when the report carries any error, so it drops straight into a script or CI step.

Full check list, what `--repair` is allowed to touch, and the ceilings that make it stand down: **[doctor.md](doctor.md)**.

### `outl recover`

Recovers block text an `Op::Edit` truncated — the mirror of `outl reconcile --ahead-of-log`, reading a different source.
Both close the same gap ([RFC 0210](rfcs/0210-md-content-outside-op-log.md)), and neither replaces the other:

- `reconcile --ahead-of-log` reads the **`.md`**.
  It recovers content still on disk but in no op.
  It cannot help a page whose `.md` was already overwritten.
- `recover` reads the **op log**.
  The producer bug did not lose text into thin air: the reconcile that followed it emitted an `Op::Edit` carrying the *truncated* text, and the edit before that one — carrying the full text — is still in the append-only log.
  So a page whose `.md` was already overwritten before the guard existed is unreachable by the first and recoverable by the second.

**Read-only by default.**
A plain `outl recover` lists what it found and writes nothing.
`--apply` is the only writing mode, and what it writes is a **new** `Op::Edit` per block — the log is never rewritten (root `CLAUDE.md` invariant 1).

```
outl recover [<path>] [--apply] [--min-lines=N]
```

- `--min-lines` (default `1`) — only report a block that lost at least this many non-blank lines.
  A one-line loss is often ordinary editing, so the default is noisy on purpose: a false positive costs a line of output, a false negative costs the content.
  Raise it when the listing is too long to read.
- `--apply` — write the recovered text back.
  Additive by construction: a block only qualifies when its current text is a *prefix* of the revision being restored, so nothing the block shows today is dropped.
  `outl_actions::restore_truncated_block` re-checks that at write time rather than trusting the scan, and refuses a block that changed in between.

A `--apply` run re-projects the `.md` of every page it touched, best-effort.
The common, harmless outcome there is the page's `.md` already carrying the full text (only the *log* was truncated).
The re-projection guard reports and skips that page rather than overwrite it — nothing is lost, since the `.md` is the party that is ahead.
What it leaves behind is a **stale sidecar**: it still lists the truncated text, so the guard keeps refusing that page's future re-projections, and `outl serve --once` does not clear it (measured: 0 ops applied, the `.md` unchanged).
`outl doctor` names each such page and points at the fix, which is `outl reconcile --ahead-of-log` — that clears `last_synced_hash` and rebuilds the sidecar.

**Run `reconcile --ahead-of-log` first, then `recover`.**
Reconcile is the wider net (it recovers whole blocks the log never saw, not just truncated ones) and it refreshes sidecars, so running it first leaves `recover` with only the cases the `.md` genuinely cannot answer.
Measured on the workspace that surfaced issue #210: reconcile-first recovered 40 pages / 623 ops and left `recover` reporting 4 blocks holding 4 lines, all of them one-line editing artifacts.
Recover-first restored the same content as 4 blocks / 77 lines but parked two pages behind the guard until a reconcile ran anyway.

Cost is one op-log read per node in the workspace, since there is no cheaper prefilter that wouldn't itself be a second opinion about which blocks deserve a look.
Measured at ~4.7s over 67,213 nodes / 214k ops on the workspace that surfaced issue #210.

### `outl page history` / `outl block history`

What the op log says happened, newest first.
Read-only — neither writes an op.

```
$ outl page history buser-cto --limit 3
history of `buser-cto`
  2026-08-07 11:44  edited
      - **Volta do Slack**
      + **Volta do Slack** :slack:
  2026-07-16 12:24  deleted
      - 32 dias, até o abril/23
  2026-07-16 12:24  created

showing the 3 most recent of 174 events — `--limit` for more
```

`--limit` (default 50) caps the **listing**, never the count: the last line always names the total, so a truncated history can't read as a complete one.

Two rules worth knowing before you read the output, both owned by `outl_actions::timeline`:

- **A page's history includes the blocks deleted out of it**, with the text each one held when it went. That is usually why someone opens a history at all. A block moved to a *different* page goes with it and shows up there; `outl block history` follows one block wherever it has lived.
- **Not everything in the log is a change.** Folding a block, snoozing a reminder, a `page-slug` write, an `Op::Edit` that re-emitted a block's existing text, and a re-emitted `Create` / `Move` that moved nothing are all skipped. A reconcile produces those in volume — on the reference workspace one block's history was six rows of them around a single real edit.

For scripting, `--json` gives one flat object per event: `change` is `created` / `edited` / `deleted` / `restored` / `moved` / `property`, with `from` / `to` / `text` / `key` beside it, plus `physical_ms` and `logical` if you need the real HLC ordering rather than the rendered local time.

The desktop has the same read behind the `⏱` button in the page header.

## MCP

Every machine-shaped command above is also exposed as an MCP tool through `outl mcp serve` — same binary, same handler, same JSON shape.
Claude Desktop, Cursor, and any other MCP host plug straight into it.

→ [docs/mcp.md](mcp.md) covers the wiring, resources, prompts, and troubleshooting.
This document stays focused on the surface; how to attach it to a host lives over there.

## What does not map 1:1 (and that's fine)

- **Interactive commands** (`init`, `reconcile`, `recover`, `mcp serve`) stay CLI-only.
  A wizard inside a tool call is the wrong shape.
- **Long-running watchers** (`serve`) stay CLI-only.
  MCP tools are request/response; the file watcher is a process, not a tool.
- **Destructive commands** (`page delete`, `block delete`) accept `--confirm` on the CLI and require `confirm: true` in the MCP input.
  Without it, the tool returns `{ "code": "CONFIRM_REQUIRED" }` and the operation is a no-op.
- **Importers** (`outl import …`) stay CLI-only — they're one-time migrations, not workspace ops.
- `page history` / `block history` — read-only, and their handlers print rather than returning a `Value`, so they need the ordinary `fn(ctx, …) -> Result<Value, ApiError>` extraction before they can be registered. Tracked with [#241](https://github.com/outlmd/outl/issues/241).

## Layout

The CLI and shim are siblings inside `outl-cli`.
Everything below delegates to `outl-actions`.

```text
outl-cli/
└── src/
    ├── main.rs              # clap entry, dispatches to commands/
    ├── output.rs            # JSON envelope, --json flag, exit codes
    ├── commands/
    │   ├── page.rs
    │   ├── block.rs
    │   ├── daily.rs
    │   ├── search.rs
    │   ├── query.rs
    │   ├── tag.rs
    │   ├── export.rs
    │   ├── workspace.rs
    │   └── mcp.rs           # `outl mcp serve` shim
    └── mcp/
        ├── server.rs        # stdio transport
        ├── tools.rs         # tool registry → handlers
        ├── resources.rs     # outl:// URIs
        └── prompts.rs       # /outl-* prompts
```

`commands/*.rs` and `mcp/tools.rs` both reach into `outl-actions`.
No business logic lives in either layer — they format input and output, that's it.

## Status

Shipping today:

- `outl init`, `outl serve`, `outl doctor`, `outl reconcile`, `outl recover`, `outl import logseq|obsidian|roam`, `outl theme`.
- `outl` (no subcommand) opens the TUI.
- `outl page get|create|update|delete|list|rename|render` (`create` accepts `--content` to seed the outline in one call)
- `outl block get|append|append-tree|insert|update|move|delete|toggle-todo|tree`
- `outl daily today|get|append|range`
- `outl search "<query>" [--in=blocks|pages|all]`
- `outl query [--tag] [--priority] [--since=Nd] [--kind] [--prop k=v]`
- `outl backlinks page|block|embed`
- `outl tag list|pages`
- `outl page prop set|get|list`
- `outl export hugo|md|json`
- `outl template list|apply|resolve|run`
- `outl batch` — stream `{ops: [...]}` from stdin (or `--ops=…`)
- `outl workspace info`
- `outl mcp serve` — full MCP protocol surface (tools, resources, prompts) over stdio.

Still ahead:

- Richer `outl query --raw='…'` DSL (today returns `INVALID_ARG`).
- Per-page block-level property surface beyond the well-known keys the `prop list` probe enumerates.

The order of landing matched the order of unlocking real workflows (scripts → LLM agents in Claude Code → Claude Desktop → blog publishing pipeline).
