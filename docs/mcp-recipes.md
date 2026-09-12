# Building MCP-driven skills and commands on top of outl

Once `outl mcp serve` is wired into a host (see [`docs/mcp.md`](mcp.md)), every workspace tool becomes callable from whatever skill/command/agent mechanism the host exposes — a Claude Code slash command, a Claude Desktop skill, a Cursor custom mode, a Zed assistant, anything that speaks MCP.

The shape that keeps showing up: **read context from outl, then generate output that depends on it.**
Examples below use Claude Code syntax because it's the most compact way to show the wiring; the body of the prompt ports unchanged to any other host.

## Setup: register outl with Claude Code

[`docs/mcp.md`](mcp.md) covers Claude Desktop / Cursor / Zed.
For Claude Code, one line does it — use `--scope user` so the server shows up in every project:

```bash
claude mcp add outl --scope user -- outl --workspace /Users/you/notes mcp serve
```

The `--` is mandatory; without it `--workspace` is captured by `claude mcp add` and the registration fails.
Verify with `claude mcp list` — you should see `outl: ... ✓ Connected`.

> Prefer to check it into a repo?
> Drop the equivalent `mcpServers` block in `.mcp.json` at the repo root; Claude Code picks it up on the next session.

## The pattern

Every useful integration follows three steps:

1. **Pull context** — call read-only tools (`outl_search`, `outl_page_get`, `outl_daily_*`, `outl_backlinks`).
2. **Structure** — bucket the raw blocks/pages into something the model can reason on.
3. **Generate** — write the artefact (summary, draft, briefing).

Writes are rarer; when you do write, prefer composite tools (`outl_block_append_tree`, `outl_batch`) over chains of single ops.

## Tool naming

Tools are named `mcp__<server>__<tool>` — for a direct registration as `"outl"` that's `mcp__outl__daily_today`, `mcp__outl__search`, and so on.
If your host runs an MCP proxy that fans out to multiple servers, the proxy adds its own prefix (`mcp__<proxy>__outl__daily_today`).
Copy the name verbatim from your host's tools panel — most hosts don't do fuzzy matching.

## Safe-by-default read set

These never mutate the workspace and never need confirmation:

- `outl_workspace_info` — counts, actor id, path.
- `outl_search` — full-text over blocks/pages.
- `outl_page_get` / `outl_page_list` / `outl_page_render` — page reads.
- `outl_daily_today` / `outl_daily_get` / `outl_daily_range` — journal reads.
- `outl_backlinks` / `outl_block_refs` — graph reads.
- `outl_tag_list` / `outl_tag_pages` — tag discovery.
- `outl_block_get` / `outl_block_tree` — block reads.

Destructive tools (`outl_page_delete`, `outl_block_delete`) require `confirm: true` and should almost never appear in a drafting command.

## Worked example: a `/standup` slash command

`/standup` reads the last seven journals and drafts the morning standup.
Pure read-and-summarise — no writes.

Save the block below verbatim as `.claude/commands/standup.md` (or `~/.claude/commands/standup.md` for a global command).
The `allowed-tools` line restricts this command to *only* the three outl tools it needs — a stray hallucination can't fire `Bash`, `Write`, `mcp__outl__page_delete`, or anything else:

```markdown
---
description: Draft today's standup from the last week of outl daily notes.
argument-hint: "[date]"
allowed-tools: mcp__outl__daily_today, mcp__outl__daily_get, mcp__outl__daily_range
---

You are drafting the standup for $1 (defaults to today if empty).
The source of truth is the user's outl workspace, accessed via MCP.

## 1. Pull context

Read today's journal (or the date the user passed) and the six days before it.
Call the MCP tools registered under the `outl` server:

- `mcp__outl__daily_today` — when `$1` is empty.
- `mcp__outl__daily_get` with `{ "date": "$1" }` — when `$1` is set.
- `mcp__outl__daily_range` with `{ "from": "$1 - 6d", "to": "$1" }` — always, to pull the prior week.

`outl_daily_range` returns one entry per day in the interval.
Days with no materialised journal come back as `{ "exists": false }` — skip them silently, don't invent content.

## 2. Structure

Bucket the blocks you read into:

- **Done** — anything starting with `DONE ` in the last six days.
- **In flight** — `TODO ` items from yesterday that aren't `DONE` today.
- **Today** — `TODO ` items in today's journal.
- **Blocked / open questions** — blocks with `#blocked`, `#blocker`, `?` at end, or explicit "stuck on" / "esperando".

Ignore meeting notes, link dumps and pure references unless they appear under a TODO.

## 3. Draft

Output four short sections, in pt-BR, in this order:

- `**Ontem**` followed by 1-3 bullets (past tense, concrete verbs).
- `**Hoje**` followed by 1-3 bullets (what the user is actually going to do).
- `**Bloqueios**` — only if real; omit the section if empty.
- `**Notas**` — only if there's a decision/question worth surfacing; omit otherwise.

Rules:

- Never invent items.
  If a day was empty, the standup is short — that's the right answer.
- Keep the user's exact wording from the blocks when it's already clean; rewrite only when the block was a half-thought.
- No filler ("alinhado", "seguindo", "no caminho"). If you'd cut it in your own standup, cut it here.
```

### Porting to another host

Save the same prompt body (everything after the frontmatter) where your host expects it: a Claude Desktop `SKILL.md`, a Cursor `.cursor/rules/*.md`, a Continue.dev `slashCommand`, or a system message in any custom MCP client.
The wrapper changes; the body doesn't.

### What gets called at runtime

Two or three MCP calls per invocation:

```jsonc
// Today's journal (or the passed date)
{ "tool": "mcp__outl__daily_today", "input": {} }
// — or —
{ "tool": "mcp__outl__daily_get", "input": { "date": "2026-06-12" } }

// The prior six days
{ "tool": "mcp__outl__daily_range",
  "input": { "from": "2026-06-06", "to": "2026-06-12" } }
```

`outl_daily_range` hands back markdown per day, with explicit gaps:

```jsonc
{ "ok": true, "data": { "entries": [
  { "date": "2026-06-06", "exists": false },
  { "date": "2026-06-07", "exists": true,
    "markdown": "- DONE ship emoji shortcodes\n- TODO review PR #78\n" },
  { "date": "2026-06-12", "exists": true,
    "markdown": "- TODO standup doc\n" }
]}}
```

The model walks `entries[].markdown`, buckets the lines per the prompt, and writes the output.
No schema, no validation step — markdown is the contract.

## Same pattern, different goal: `voice-humanize`

The `voice-humanize` skill (in the user's dotfiles) uses the same three steps for a different output: it samples the user's writing style via `outl_search` over blocks and pages, extracts tone markers, and drafts a message that sounds like them.
The fixed pieces are *what to read, how to bucket it, what to produce* — the tools never change.

## Tips

- **Markdown dialect.** Writes must follow [`docs/markdown-format.md`](markdown-format.md): ISO date links (`[[2026-04-22]]`), 2-space indent per level, literal `TODO` / `DONE` prefixes. The server doesn't transform input.
- **Best-effort reads.** If a call fails or returns empty, continue with what the rest of the calls produced — don't block the whole command on one miss.
- **Output shape.** Success is content-only: compact JSON in `content[].text`, or raw `.md` from `outl_page_render` / `outl_export_md`. Errors set `isError: true` and keep `structuredContent: { ok: false, error }` so `error.data` is readable.
- **Block ids come from `outline`.** `outl_daily_*` returns JSON, not raw markdown, because a rendered `.md` carries no ids — and you need one to call `outl_block_update` / `_move` / `_delete` / `_toggle_todo`.
- **Workspace errors.** If `outl mcp serve` was launched without `--workspace` (and `OUTL_WORKSPACE` isn't set), every tool returns "no workspace". Tell the user to fix the host config — the prompt can't recover.

## See also

- [`docs/mcp.md`](mcp.md) — wiring `outl mcp serve` into every host.
- [`docs/cli.md`](cli.md) — full tool surface (each CLI subcommand maps 1:1 to an MCP tool).
- [`docs/markdown-format.md`](markdown-format.md) — the dialect skills must respect when writing back.
