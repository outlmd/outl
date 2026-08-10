# MCP

outl ships an [MCP][mcp] (Model Context Protocol) server as a subcommand of the same binary you already have: `outl mcp serve`.

Claude Desktop, Cursor, Zed, and anything else that speaks MCP can reach the workspace through it — no extra install, no daemon, no parallel codebase.
Every tool you see in the host's tools panel maps 1:1 to a CLI subcommand.
The wiring lives in [`docs/cli.md`](cli.md); this page is just about plugging the server into a host.

[mcp]: https://modelcontextprotocol.io

## Wiring it up

### Claude Desktop

Edit `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "outl": {
      "command": "outl",
      "args": ["--workspace", "/Users/avelino/notes", "mcp", "serve"]
    }
  }
}
```

`--workspace` (short `-w`) is the global flag every subcommand honours; `mcp serve` targets whichever workspace it points at.

Restart Claude Desktop.
The outl tools and resources show up under the server name; calling any tool is exactly equivalent to running the matching CLI command with `--json`.

### Cursor / Zed / other MCP hosts

The shape is the same.
Any host that lets you register an MCP server with a command + args wants:

```
command: outl
args:    ["--workspace", "<absolute path to workspace>", "mcp", "serve"]
```

Run `outl mcp serve --help` to see all flags.

### From a script (smoke test)

```bash
printf '%s\n%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | outl --workspace ~/notes mcp serve
```

If both lines come back with valid JSON-RPC responses, the server is healthy.

## What the host sees

### Tools

Every CLI subcommand documented in [`docs/cli.md`](cli.md#commands-by-domain) is registered as an MCP tool.
Names are `outl_<command>_<verb>` (e.g.
`outl_page_get`, `outl_block_append`, `outl_daily_today`, `outl_search`, `outl_query`).
Input schema mirrors the CLI flags; the response is the JSON envelope's `data` field wrapped in MCP's `content` shape.

Destructive tools (`outl_page_delete`, `outl_block_delete`) require `confirm: true` in the input.
Without it they return a recoverable `CONFIRM_REQUIRED` error and the workspace is untouched.

For multi-block authoring, prefer the composite tools over a chain of single-op calls:

- `outl_block_append_tree` — append a root block + its recursive children in one shot.
- `outl_page_create` — accepts an optional `content` forest so a brand-new page lands with its full outline in a single call.
- `outl_batch` — apply a sequence of `{op, args}` writes in one workspace session.
  Stops on first error and reports `failed_at` / `applied` so the caller can pick up the suffix that never ran.
  Supported ops cover every other write tool.
  See [`docs/cli.md` → Batch](cli.md#batch) for the payload shape.

For file uploads, `outl_asset_add` imports a file into `<workspace>/assets/` and appends its markdown link as a new block (today's journal by default, or a `page` slug).
Because MCP is stdio, it takes the file as a `path` argument — a filesystem path on the machine running the server, not base64 bytes.
See [`docs/cli.md` → Asset](cli.md#asset).

### Resources

URIs the host can attach as context without an explicit tool call.
Almost all of these are read-only; the one exception (`outl://daily/today`) lazily materialises today's journal on first access, the same way `outl daily today` does in the CLI.

| URI                       | Type               | Body                                          |
|---------------------------|--------------------|-----------------------------------------------|
| `outl://workspace/info`   | `application/json` | path, actor id, counts, ops                   |
| `outl://daily/today`      | `text/markdown`    | today's journal projection (creates on first read) |
| `outl://page/{slug}`      | `text/markdown`    | page projection (template URI)                |

Useful pattern: tell Claude Desktop "you are the assistant for my second brain" and attach `outl://daily/today` so it sees the day's context without having to call a tool.

> Looking to build a skill, slash command, or custom agent that pulls workspace context from outl?
> See [MCP recipes](mcp-recipes.md) — pattern, tool-naming rules (direct vs. proxy), porting matrix across hosts (Claude Code / Claude Desktop / Cursor / Zed / Continue.dev / anything with an MCP client), and a worked `/standup` example.

### Prompts

Slash-style shortcuts the host renders in the prompt picker:

| Prompt                       | Arguments        | What it does                          |
|------------------------------|------------------|---------------------------------------|
| `outl-summarize-day`         | `date?` (ISO)    | pulls daily, asks for a summary       |
| `outl-blog-from-block`       | `block_id`       | expands a block into a blog draft     |

Prompts are nice-to-have.
Same surface works through tools (`outl_daily_today`
+ a free-form prompt) — they're just keyboard shortcuts.

## Architecture in one paragraph

`outl mcp serve` is a 200-line stdio loop on top of the same Rust handlers the CLI subcommands call.
There is no `outl-mcp` crate, no parallel logic, no JSON-RPC framework dependency (we speak the protocol directly).
Every new feature lands once, as a function in `outl-actions`, and is exposed in both surfaces — see [`crates/outl-cli/CLAUDE.md`](../crates/outl-cli/CLAUDE.md) for the exact "add a new tool" walkthrough.

## Sync: edits made through MCP reach your other devices

`outl mcp serve` is **long-lived**, so it can be this device's peer — no GUI needs to be open.
When the device has paired peers (`outl peer pair`), the server brings the iroh transport up on first use and, after every mutating tool, wakes connected peers so they pull the change in real time.
So an edit you make through Claude lands on your laptop/phone the same way an edit in the desktop app would.
With no paired peers it stays fully local (nothing to sync) and never touches the network.

**One process per device holds the endpoint.**
iroh routes one endpoint per device identity, so if a GUI is already running here it keeps the endpoint and the MCP server writes its ops to the shared `ops/` dir instead — the GUI pushes them out on its own pass, and everything still converges.
Whichever process started first wins; nothing is lost either way.
This is what a headless setup (a machine running only `outl mcp serve`) depends on: with no GUI to defer to, the MCP server *is* the peer.

A short-lived `outl <subcommand>` CLI call can never take the endpoint (it exits before a connection is even established); for scripts that must flush, run `outl sync` after the mutations.
See [`crates/outl-sync-iroh/CLAUDE.md`](../crates/outl-sync-iroh/CLAUDE.md) → "One endpoint per identity, elected not assigned".

## Troubleshooting

**The server starts but the host shows zero tools.** Check stderr (`outl mcp serve --workspace … 2> /tmp/outl-mcp.log` and tail it).
Almost always it's a permission error reading the workspace path.

**`workspace at … is locked by another outl process`.** As of 0.5.1 the workspace lock (`.outl/.lock`) is shared, so this error is no longer raised on simple co-tenancy: TUI + MCP server + a subprocess CLI all coexist.
The remaining contention point is the per-actor write lock at `ops/.lock-<actor>`.
The opener tries this device's actor first and, if taken, mints an `ActorId::new()` ephemeral and locks that one instead — so the second `outl mcp serve` of the day usually just writes to a fresh `ops-<ephemeral>.jsonl` without telling you.
You only see a hard error when both the device actor AND the ephemeral path can't be acquired (e.g. a stale `.lock-<actor>` left by a killed process, or a filesystem that doesn't support `flock`).
Recovery: delete the dangling `ops/.lock-*` files for processes that are no longer running, or move the workspace off the unsupported filesystem.

**Tool calls return `INTERNAL` errors.** Run the same command on the CLI (`outl <command> --json`) — same code path, same error, easier to read.
If CLI works and MCP doesn't, file a bug.

**Path quoting on macOS.** If the workspace path has spaces, the JSON in `claude_desktop_config.json` must escape them.
Use a path without spaces (`~/notes`, `~/Documents/outl`) — easier than fighting JSON escaping in two layers.

## What's NOT exposed over MCP

By design:

- `outl init`, `outl serve`, `outl reconcile` — interactive or long-running, wrong shape for a tool call.
- `outl import logseq|obsidian|roam` — one-time migration, not a workspace op.
- `outl mcp serve` itself — the host already booted you.

These stay CLI-only.
Run them from a terminal when you need them.
