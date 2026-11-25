# MCP recipes: skills and slash commands on top of outl

Once `outl mcp serve` is wired into a host (see [MCP](mcp.md)), every workspace tool is callable from whatever skill, command or agent mechanism that host has.
That covers a Claude Code slash command, a Claude Desktop skill, a Cursor rule, a Zed assistant, anything with an MCP client.

The shape that keeps working is **read context from outl, then generate something that depends on it**.
The recipes below use Claude Code syntax because it is the most compact way to show the wiring.
The prompt body ports to any other host unchanged.

- [The pattern](#the-pattern)
- [Tool naming](#tool-naming)
- [Which tools are safe to allow](#which-tools-are-safe-to-allow)
- [Recipe: `/standup`](#recipe-standup) (read only)
- [Recipe: `/capture`](#recipe-capture) (one small write)
- [Recipe: `/meeting`](#recipe-meeting) (builds a page)
- [Recipe: `/weekly-review`](#recipe-weekly-review) (read, then one write)
- [Tips](#tips)

## Set up Claude Code

```bash
claude mcp add outl --scope user -- outl --workspace /Users/you/notes mcp serve
claude mcp list        # outl: ... ✓ Connected
```

The `--` is required.
Without it `claude mcp add` captures `--workspace` and registration fails.
For a team repo, put the equivalent `mcpServers` block in `.mcp.json` at the repo root.
The other hosts are in [MCP → Set it up](mcp.md#set-it-up).

## The pattern

1. **Pull context.**
   Call read tools: `outl_search`, `outl_page_get`, `outl_daily_*`, `outl_backlinks`.
2. **Structure it.**
   Bucket the blocks into something the model can reason about (done, in flight, blocked).
3. **Generate.**
   Write the artifact: a summary, a draft, a briefing.

Writes are rarer than reads.
When you write, use a composite tool (`outl_block_append_tree`, `outl_page_create` with `content`, `outl_batch`) over a chain of single operations.

## Tool naming

A host names a tool `mcp__<server>__<tool>`.
outl's tools are already called `outl_<thing>_<verb>`, so registered directly as `outl` the full name **repeats the word**:

```text
mcp__outl__outl_daily_today
mcp__outl__outl_search
```

If your host runs an MCP proxy that fans out to several servers, the proxy adds its own prefix in front: `mcp__<proxy>__outl__outl_daily_today`.
Copy the name verbatim from the host's tools panel.
Most hosts match exactly and fail without a hint on a typo, and a wrong name in `allowed-tools` just means the tool is never allowed.

## Which tools are safe to allow

Reads never change the workspace and are fine in any drafting command:

- `outl_workspace_info`, `outl_workspace_doctor`
- `outl_search`, `outl_query`
- `outl_page_get`, `outl_page_list`, `outl_page_render`
- `outl_daily_range`
- `outl_backlinks`, `outl_block_refs`, `outl_block_embed`
- `outl_tag_list`, `outl_tag_pages`
- `outl_block_get`, `outl_block_tree`
- `outl_page_prop_get`, `outl_page_prop_list`, `outl_block_prop_get`, `outl_block_prop_list`
- `outl_template_list`, `outl_template_resolve`
- `outl_export_md`, `outl_export_json`
- `outl_trash_list`

Two look like reads and are not quite: `outl_daily_today` and `outl_daily_get` **create the journal page** if the day has none yet.
After the first call of a day they only read.
Allow them freely; just know that "what did I do on a day I never opened" leaves an empty page behind.

`outl_trash_restore` writes but destroys nothing: it puts a deleted block back.
It takes no `confirm`, so an agent that deleted something can undo it without asking twice.

Keep these out of a drafting command: `outl_page_delete`, `outl_page_merge`, `outl_block_delete` (all need `confirm: true`), plus `outl_page_rename`, `outl_asset_add`, `outl_export_hugo` and `outl_template_run`.
The reasons are in [MCP → What an agent can and cannot touch](mcp.md#what-an-agent-can-and-cannot-touch).

## Recipe: `/standup`

Reads the last seven journals and drafts the morning standup.
Pure read and summarise, no writes.

Save as `.claude/commands/standup.md` (or `~/.claude/commands/standup.md` for every project).
The `allowed-tools` line limits the command to the two tools it needs, so a stray hallucination cannot reach `Bash`, `Write` or `outl_page_delete`:

````markdown
---
description: Draft today's standup from the last week of outl daily notes.
argument-hint: "[YYYY-MM-DD]"
allowed-tools: mcp__outl__outl_daily_today, mcp__outl__outl_daily_range
---

You are drafting the standup for $1 (today if empty).
The source is the user's outl workspace, through MCP.

## 1. Pull context

1. Call `mcp__outl__outl_daily_today`. Its reply has `date`, which is the real today. Use it as the end date when `$1` is empty.
2. Call `mcp__outl__outl_daily_range` with `{ "from": <end date minus 6 days, ISO>, "to": <end date> }`.

`outl_daily_range` returns `{ "journals": [...] }`, one entry per day.
A day with no journal is `{ "date": "...", "exists": false }`. Skip it silently.
A day with one has `outline`: a tree of blocks `{ id, text, todo?, children }`.
The task state is the `todo` field (`TODO`, `DOING` or `DONE`), not part of `text`.

## 2. Structure

Walk every `outline` recursively and bucket:

- **Done**: `todo: "DONE"` in the last six days.
- **In flight**: `todo: "TODO"` or `"DOING"` in an earlier day that is not `DONE` later.
- **Today**: `todo: "TODO"` or `"DOING"` in the end date's journal.
- **Blocked / open**: text containing `#blocked` or `#blocker`, ending in `?`, or saying "stuck on".

Ignore meeting notes, link dumps and bare references unless they sit under a task.

## 3. Draft

Four short sections, in this order:

- **Yesterday**: 1 to 3 bullets, past tense, concrete verbs.
- **Today**: 1 to 3 bullets of what the user will actually do.
- **Blockers**: only if real. Omit the section when empty.
- **Notes**: only if a decision or question deserves surfacing.

Never invent items. If a day was empty, the standup is short, and that is the right answer.
Keep the user's wording when it is already clean.
````

What flows at runtime is two calls:

```jsonc
{ "tool": "mcp__outl__outl_daily_today", "input": {} }
// → { "date": "2026-11-25", "meta": {...}, "outline": [...], "md": "..." }

{ "tool": "mcp__outl__outl_daily_range",
  "input": { "from": "2026-11-19", "to": "2026-11-25" } }
// → { "journals": [
//      { "date": "2026-11-19", "exists": false },
//      { "date": "2026-11-20", "exists": true, "meta": {...},
//        "outline": [ { "id": "01M8…", "text": "ship emoji shortcodes", "todo": "DONE", "children": [] } ] }
//    ] }
```

The reply is JSON with structure, not markdown, on purpose: a rendered page has no block ids, and a command that later ticks or edits a block needs one.

## Recipe: `/capture`

Drops one line into today's journal.
The smallest useful write, and safe to allow without a prompt because it only appends.

````markdown
---
description: Capture a thought into today's outl journal.
argument-hint: "<text>"
allowed-tools: mcp__outl__outl_daily_append
---

Append `$ARGUMENTS` to today's journal with `mcp__outl__outl_daily_append` and `{ "text": "$ARGUMENTS" }`.
Prefix it with `TODO ` only if the user's text is clearly a task.
Do not rephrase. Reply with one line: the date and the text you wrote.
````

The reply is `{ "block_id", "date", "text" }`.
Keep `block_id` if a follow-up might edit the block.

## Recipe: `/meeting`

Turns a transcript or rough notes into a page with a real outline.
One call builds the whole page, so there is no half-written state to clean up.

````markdown
---
description: Turn meeting notes into an outl page.
argument-hint: "<slug> then paste the notes"
allowed-tools: mcp__outl__outl_page_list, mcp__outl__outl_page_create, mcp__outl__outl_daily_append
---

1. Pick a slug: kebab-case, no `/` and no `.` (for example `2026-11-25-roadmap-sync`).
   Call `mcp__outl__outl_page_list` and make sure it is not taken.
   `outl_page_create` on an existing slug reuses the page and **appends** `content` to it, so a retry or a clash duplicates blocks.
2. Call `mcp__outl__outl_page_create` with `slug`, a human `title`, and `content`: a forest of `{ "text", "children" }`.
   Use the groups **Decisions**, **Actions** (each a `TODO ` block), **Open questions**.
   Date links are ISO: `[[2026-11-25]]`.
3. Add one line to today's journal with `mcp__outl__outl_daily_append`: `Meeting: [[<slug>]]`.
4. Reply with the slug and the number of action items.
````

Rules the dialect enforces, because the server does not rewrite input: ISO date links, 2-space nesting (handled for you by `children`), and a literal `TODO ` / `DONE ` prefix for tasks.
See [Markdown format](markdown-format.md).

## Recipe: `/weekly-review`

Reads a week, finds what slipped, and writes a review page.
Shows the read-then-write shape: everything is read first, and the write is a single call.

````markdown
---
description: Review last week and write it up as an outl page.
allowed-tools: mcp__outl__outl_daily_today, mcp__outl__outl_daily_range, mcp__outl__outl_query, mcp__outl__outl_page_create
---

1. `mcp__outl__outl_daily_today` for the real date. Compute the Monday to Sunday of last week.
2. `mcp__outl__outl_daily_range` over that week.
3. `mcp__outl__outl_query` with `{ "tag": "project", "not_tags": ["someday"] }` to list active project pages.
4. Group the week's `todo: "DONE"` blocks by project page mention (`[[page]]`).
   List `TODO` blocks that appeared early in the week and never reached `DONE` under **Slipped**.
5. Write one page with `mcp__outl__outl_page_create`, slug `review-<monday-iso>`, with `content` holding **Done by project**, **Slipped**, **Next week**.
   Never invent items. An empty section is omitted.
````

`outl_query` filters **pages** (by tag, property, date, kind, each with a `not_` twin), not blocks.
To find blocks, use `outl_search`, or a `query` fence on a page ([Queries](query.md)).

## Same pattern, different goal

The `voice-humanize` skill in the author's dotfiles uses the same three steps for text: it samples the user's writing through `outl_search` over blocks and pages, extracts tone, and drafts a message that sounds like them.
What to read, how to bucket it and what to produce change per skill.
The tools do not.

## Tips

- **Writes follow the markdown dialect.**
  ISO date links (`[[2026-04-22]]`), 2 spaces per nesting level if you send raw text, literal `TODO` / `DONE` prefixes.
  The server does not transform what you send.
- **Best-effort reads.**
  If one call fails or comes back empty, continue with the rest.
  Do not block a whole command on one miss.
- **Check `failed_at` after `outl_batch`.**
  A batch that stops halfway still reports success at the protocol level.
  Details in the [batch section of the tool reference](mcp-tools.md#batch).
- **Block ids come from `outline`.**
  `outl_daily_*` and `outl_page_get` return JSON because a rendered `.md` has no ids, and `outl_block_update`, `_move`, `_delete` and `_toggle_todo` need one.
- **Reply shape.**
  Success is content only: compact JSON in `content[0].text`, or raw markdown from `outl_page_render` and `outl_export_md`.
  Errors set `isError: true` and keep `structuredContent: { ok: false, error }`, so `error.data` is readable.
- **A wrong workspace is the host's problem, not the prompt's.**
  If every tool replies `NO_WORKSPACE`, tell the user to fix `--workspace` in the host config.
  The command cannot recover from it.

## See also

- [MCP](mcp.md): wiring `outl mcp serve` into every host, safety, troubleshooting.
- [MCP tool reference](mcp-tools.md): every tool, argument and error code.
- [CLI](cli.md): each tool has a CLI twin for debugging.
- [Markdown format](markdown-format.md): the dialect a skill must respect when it writes back.
