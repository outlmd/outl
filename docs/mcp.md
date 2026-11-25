# MCP: let an AI assistant work in your notes

outl ships an [MCP][mcp] (Model Context Protocol) server inside the `outl` binary you already have.
Point Claude Desktop, Claude Code, Cursor, Zed or any other MCP host at it, and the assistant can read your journal, search your pages, capture a thought, tick a task or restructure a page, in the same graph you edit by hand.

```text
you:     what did I decide about the migration last week?
claude:  (searches, reads 3 journals) On Tuesday you wrote "keep the old queue until March"...

you:     add that to today's journal as a TODO
claude:  Done. Added "TODO drop the old queue after March" to 2026-11-25.
```

No extra install, no daemon to babysit, no second copy of your data.
The server runs on your machine, reads your workspace folder, and writes through the same op log as every other client, so an edit made by an assistant syncs to your phone like any other edit.

- [Set it up](#set-it-up) takes two minutes.
- [What an agent can and cannot touch](#what-an-agent-can-and-cannot-touch) is the part to read before you let one write.
- [Tool reference](mcp-tools.md) lists every tool and its arguments.
- [MCP recipes](mcp-recipes.md) builds slash commands and skills on top.

[mcp]: https://modelcontextprotocol.io

## Set it up

You need the `outl` binary on your `PATH`.
`outl --version` should print a version.
If it does not, install the CLI from the [README](../README.md#install) (`brew install outlmd/outl/outl-beta`, or Scoop on Windows).
MCP is a feature of the CLI binary: the desktop and mobile apps do not host a server, but anything an assistant writes reaches them through [sync](sync.md).

Every host wants the same three things: a command, its arguments, and an absolute workspace path.

```text
command: outl
args:    ["--workspace", "/Users/you/notes", "mcp", "serve"]
```

### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json` (on Windows, `%APPDATA%\Claude\claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "outl": {
      "command": "outl",
      "args": ["--workspace", "/Users/you/notes", "mcp", "serve"]
    }
  }
}
```

Quit and reopen Claude Desktop.
The outl tools, resources and prompts show up under the server name `outl`.

### Claude Code

One line, with `--scope user` so every project sees it:

```bash
claude mcp add outl --scope user -- outl --workspace /Users/you/notes mcp serve
claude mcp list        # outl: ... ✓ Connected
```

The `--` is required.
Without it `claude mcp add` swallows `--workspace` and the registration fails.
To share the setup with a team repo, put the same `mcpServers` block in `.mcp.json` at the repo root.

### Cursor, Zed and the rest

Register a stdio server with the command and args above.
The config file differs per host, the three values do not.

### Check that it works

Without any host, from a terminal:

```bash
printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | outl --workspace ~/notes mcp serve
```

Two JSON lines come back: the first names the server (`"name":"outl"`), the second lists 49 tools.
Logs go to stderr, never stdout, because stdout is the protocol channel.

Then in the host, ask something harmless: "list my pages tagged #project" or "what is in my journal today?".

### Which workspace does it open

In this order:

1. `--workspace <path>` (short `-w`) on the command line.
2. The last workspace you opened in any outl client (`workspace.last` in `config.toml`), if the folder still exists.
3. The folder the host launched the server from.

**Always pass `--workspace` in a host config.**
Leave it out and the server quietly opens whatever you used last, which is rarely what a config file should depend on.
There is no `OUTL_WORKSPACE` variable.

If the path has spaces, JSON needs them escaped inside two layers of quoting.
A path without spaces (`~/notes`) saves you the trouble.

## What an agent can do

49 tools, grouped by what you would ask for.
Arguments and return shapes are in the [tool reference](mcp-tools.md).

| You want to | Tools |
|---|---|
| Find things | `outl_search`, `outl_query`, `outl_tag_list`, `outl_tag_pages`, `outl_backlinks`, `outl_block_refs` |
| Read | `outl_page_get`, `outl_page_render`, `outl_page_list`, `outl_block_get`, `outl_block_tree`, `outl_block_embed`, `outl_daily_today`, `outl_daily_get`, `outl_daily_range` |
| Capture | `outl_daily_append`, `outl_block_append`, `outl_block_append_tree`, `outl_asset_add` |
| Edit | `outl_block_update`, `outl_block_insert`, `outl_block_move`, `outl_block_toggle_todo`, `outl_page_update` |
| Properties | `outl_page_prop_get`, `outl_page_prop_list`, `outl_page_prop_set`, `outl_page_prop_delete`, and the same four for blocks: `outl_block_prop_*` |
| Build a page in one call | `outl_page_create` (with `content`), `outl_block_append_tree`, `outl_batch` |
| Clean up | `outl_page_rename`, `outl_page_merge`, `outl_page_delete`, `outl_block_delete`, `outl_trash_list`, `outl_trash_restore` |
| Templates | `outl_template_list`, `outl_template_apply`, `outl_template_resolve`, `outl_template_run` |
| Publish | `outl_export_md`, `outl_export_json`, `outl_export_hugo` |
| Health | `outl_workspace_info`, `outl_workspace_doctor` |

Not every CLI command is a tool.
`outl capture`, `outl page history`, `outl peer`, `outl plugin` and the rest of the [admin commands](cli.md#workspace--admin) stay in the terminal, where a person is watching.

### Things worth knowing before you build on it

- **Block ids come from `outline`, not from markdown.**
  A rendered page has no ids, and every block-level write needs one.
  `outl_page_get` and the `outl_daily_*` reads return `outline`; `outl_page_render` does not.
- **Reading a journal can create it.**
  `outl_daily_today` and `outl_daily_get` write the day's page if it does not exist yet.
  `outl_daily_range` never does, and marks missing days `exists: false`.
- **A write is durable when the call returns.**
  The next read, from any session, sees it.
- **Search reads the op log.**
  Text sitting in a `.md` that outl has not ingested yet is invisible to `outl_search` and `outl_backlinks`.
  See [`cli.md`](cli.md#search--query).
- **Prefer composite writes.**
  `outl_block_append_tree`, `outl_page_create` with `content` and `outl_batch` do in one call what takes ten, with one disk sync instead of ten.

## What an agent can and cannot touch

The server has no login and no permission list of its own.
Its trust boundary is the process: whoever can start `outl mcp serve` as you is you.
That is the right model for a local tool, and it means the safety is in three places you control.

**1. Where it can reach.**

- It opens exactly one workspace folder.
- It speaks stdio.
  It opens no network port.
  (The embedded variant under [`outl serve`](#running-beside-outl-serve) listens on a Unix socket readable only by your user, and nothing else.)
- Three tools step outside the folder, and are the ones to gate:
  - `outl_asset_add` reads a file from any `path` you can read, and copies it into `assets/`.
  - `outl_export_hugo` writes a file into any `out_dir` you can write.
  - `outl_template_run` executes the code block of a callable template.
    It is the only tool that runs code.

**2. What a mistake costs.**

- Deleting a block or page is a move to the trash, not erasure.
  `outl_trash_list` shows what is there and `outl_trash_restore` puts a block back.
  Restoring a deleted **page** is not offered yet; its content stays in the log and the refusal says how to print it.
- The three destructive tools, `outl_page_delete`, `outl_page_merge` and `outl_block_delete`, refuse without `confirm: true`.
  The assistant has to say it on purpose, and a good host shows you the call first.
- Every write is an op in an append-only log.
  Nothing an assistant does is unrecorded, and [`outl page history` and `outl block history`](cli.md#outl-page-history--outl-block-history) list what changed and when, including blocks deleted out of a page.
- `outl_page_merge` and `outl_page_rename` with `merge: true` have **no dry run**.
  They are the closest thing to an irreversible-looking action, so give them a prompt, not an allowlist entry.

**3. What your host asks you.**

Most hosts ask per tool call, or let you allowlist tools.
A sensible split:

| Allow without asking | Ask every time |
|---|---|
| every read in the table above, `outl_daily_append`, `outl_block_append*`, `outl_trash_restore` | `outl_page_delete`, `outl_page_merge`, `outl_block_delete`, `outl_page_rename`, `outl_asset_add`, `outl_export_hugo`, `outl_template_run` |

In Claude Code the allowlist is `allowed-tools` in a command's frontmatter or `permissions.allow` in settings.
The tool names carry the host's prefix: see [tool naming](mcp-recipes.md#tool-naming).

**A write can refuse.**
If a page's `.md` holds lines outl never recorded (typically an edit from a plain text editor that no client has picked up), a write would delete them for good, so outl refuses with `PAGE_MARKDOWN_AHEAD_OF_LOG`.
The reply names the page, how many lines are at risk, one sample, and the fix: run `outl reconcile --ahead-of-log` in a terminal inside the workspace.
An assistant should stop and tell you, not retry.
The same refusal reaches the [other clients](clients.md#surfacing-a-page-that-stopped-syncing) in their own wording.

## Resources and prompts

### Resources

Context a host can attach to a conversation without a tool call.

| URI | Type | Body |
|---|---|---|
| `outl://workspace/info` | `application/json` | path, actor id, page / block / op counts |
| `outl://daily/today` | `text/markdown` | today's journal. **Creates it on first read** |
| `outl://page/{slug}` | `text/markdown` | the page's clean markdown (a template URI: fill in the slug) |

A good opener in Claude Desktop is to attach `outl://daily/today` and say "you are the assistant for my second brain".
The day's context is already there.

### Prompts

Two ready-made prompts show up in the host's prompt picker.

| Prompt | Arguments | What you get |
|---|---|---|
| `outl-summarize-day` | `date?` (ISO, default today) | the day's journal, with a request for decisions, open questions and next actions |
| `outl-blog-from-block` | `block_id` (required) | the block's subtree as JSON, with a request to expand it into a blog draft. The request is fixed to a direct, informal pt-BR voice, so for another voice use a plain message and the tools |

Prompts are shortcuts.
Anything they do, a plain message plus the tools does too.

## Running beside `outl serve`

On a machine that runs [`outl serve`](cli.md#workspace--admin) as its daemon, let it own the workspace.
`outl serve` answers MCP itself (`--no-mcp` turns that off, `--once` never serves it), and `outl mcp serve` becomes a thin bridge to it.
Your host config does not change.

- **One workspace, one writer.**
  Every connection is a session on the daemon's single open workspace, so sessions are serialized, the op log is replayed once, and all writes land under the device actor.
  No session leaves an `ops-<ulid>.jsonl` or a lock file behind.
- **Local only.**
  The socket is `<device dir>/run/mcp-<hash>.sock`, mode 0600 inside a 0700 directory.
  `<device dir>` is `~/.config/outl` unless `$OUTL_DEVICE_DIR` or `$XDG_CONFIG_HOME` says otherwise.
  Windows has no Unix sockets here, so `outl serve` runs without it and `outl mcp serve` owns the workspace itself.
- **How the bridge picks.**
  `outl mcp serve` looks for a live socket for the workspace.
  If one answers it pipes stdin and stdout to it and opens nothing.
  Otherwise it owns the workspace as usual.
  Which one it chose is logged to stderr.
  `--standalone` forces the second behaviour.
- **With `--no-watch`** the daemon holds no write lock while idle.
  A mutating tool takes the device actor on demand and releases it when the last session closes, so a desktop app opened later keeps the actor.
  If another process holds it, the tool says so, reads keep working, and no throwaway actor is minted.
- **A broken socket never stops the daemon.**
  A path too long, a directory that cannot be made private, or a second `outl serve` on the same workspace is a logged warning, and `outl serve` keeps running without MCP.
  A stale socket from a killed daemon is replaced on the next start.

## Sync: edits through MCP reach your other devices

A standalone `outl mcp serve` is long-lived, so it can act as this device's sync peer even with no app open.
When the device has paired peers ([`outl peer pair`](sync.md)), the server brings the transport up on first use and, after each write, tells connected peers to pull.
An edit made through an assistant lands on your phone the way a desktop edit does.
With no paired peers nothing touches the network.

One process per device holds the network endpoint.
If the desktop app is already running, it keeps it, and the MCP server writes its ops to the shared `ops/` folder for the app to ship.
Whoever started first wins, and nothing is lost either way.
A headless machine that runs only `outl mcp serve` (or `outl serve`) is its own peer.

A one-off `outl <command>` call exits too fast to hold the endpoint.
For scripts that must flush before they stop, run [`outl sync`](cli.md#workspace--admin) after the writes.

## Troubleshooting

**The host shows zero tools, or the server does not start.**
Run the smoke test above and read stderr.
Nearly always it is a wrong path in the config, or a binary that is not on the `PATH` the host sees.
GUI hosts do not read your shell profile: use the absolute path to `outl` (`which outl`) as `command`.

**Every tool replies `NO_WORKSPACE: no outl workspace at …`.**
The server started, but the path is not a workspace: a workspace has a `.outl/` folder inside it.
Fix the `--workspace` path in the host config, or create the workspace with `outl init <path>`.
The server never prompts to initialize one, because stdin is the protocol.
The same code also covers a `config.toml` that fails to parse: run `outl doctor` in the folder.

**A tool returns `INTERNAL`.**
Run the same action on the CLI with `--json` (`outl page get notes --json`).
Same code path, same error, easier to read.
If the CLI works and the tool does not, file a bug with both outputs.

**`PAGE_MARKDOWN_AHEAD_OF_LOG`.**
See [A write can refuse](#what-an-agent-can-and-cannot-touch).
Run `outl reconcile --ahead-of-log`.

**An edit I made in the app is missing from a tool's answer.**
Peer edits are picked up on the next call, not mid-call.
Ask again.
If it is still missing, the two devices have not synced yet: check `outl peer status`.

**The assistant edited the wrong block.**
Block ids are ULIDs and must come from `outline`.
A model that guesses an id gets `BLOCK_NOT_FOUND`; one that reuses an id from earlier in a long chat can hit a block that has since moved.
Have it re-read the page before a block edit.

**`workspace … is locked by another outl process`.**
Rare.
The workspace lock is shared, so the TUI, a GUI, the MCP server and CLI calls coexist.
What can collide is the per-actor write lock at `ops/.lock-<actor>`.
A second standalone `outl mcp serve` takes a throwaway actor and writes to its own `ops-<ulid>.jsonl`, which is harmless but untidy.
Running `outl serve` and letting `outl mcp serve` [bridge to it](#running-beside-outl-serve) avoids it.
You only get a hard error when neither actor can be locked: delete a `.lock-*` left by a killed process, or move the workspace off a filesystem without `flock`.

## What is not exposed

By design, these stay in the terminal:

- `outl init`, `outl serve`, `outl reconcile`, `outl recover`, `outl compact`: interactive, long-running or repairing, the wrong shape for a tool call.
- `outl import …`: a one-time migration, not a workspace operation.
- `outl peer …`, `outl plugin …`, `outl telemetry …`: they change what the device trusts or sends, so a person does them.
- `outl mcp serve` itself.

## How it fits together

`outl mcp serve` is a JSON-RPC 2.0 loop over stdio, newline-delimited, speaking MCP protocol version `2024-11-05`.
Each tool calls the same handler as its CLI twin, so there is one implementation and one set of bugs.
There is no separate `outl-mcp` crate and no JSON-RPC framework.
A session opens the workspace once and keeps it, so a tool call costs the call, not a replay of the op log.
Read tools answer from an index derived from the op log, cached for the session and dropped after any write or any peer push.

A success reply is content only: the handler's payload as compact JSON in `content[0].text`, with fields only a GUI draws removed.
`outl_page_render` and `outl_export_md` send raw markdown instead.
An error sets `isError: true` and keeps `structuredContent: { ok: false, error }` so `error.data` survives.
No tool declares an `outputSchema`.
The reasoning is in [RFC 0276](rfcs/0276-mcp-content-only-replies.md).

To add a tool, see the walkthrough in [`crates/outl-cli/CLAUDE.md`](../crates/outl-cli/CLAUDE.md).
