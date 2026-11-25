# CLI recipes

Copy-paste answers for the jobs people automate with `outl`: capture hotkeys, cron, CI, publishing, bulk edits.
Each recipe says what it does, shows the command, and names the one thing that goes wrong.

New to the command line?
Start with the [quick start](cli-quickstart.md).
Every flag lives in the [CLI reference](cli.md), and errors are decoded in [Troubleshooting the CLI](cli-troubleshooting.md).

Two habits apply to every recipe here:

- **Pass `-w <DIR>` in anything that runs unattended.**
  Without it, outl falls back to the workspace the desktop app opened last, which is not necessarily the one your script means ([which workspace is used](cli-quickstart.md#2-tell-outl-which-workspace-to-use)).
- **Read stdout, ignore stderr.**
  Logs go to stderr.
  The `--json` envelope goes to stdout.

## Capture a thought from anywhere

You want a hotkey, a shell alias or a Shortcut that drops a line into your notes in a few milliseconds, even with a large graph.

```bash
outl -w ~/notes capture "ring the dentist"
pbpaste | outl -w ~/notes capture -        # the clipboard, as one block
outl -w ~/notes capture --list             # what is still waiting
```

`capture` writes one small file into `.outl/captures/` and exits.
It does not open the workspace.
The line becomes a block in the journal of the day you captured it the next time a client opens: the desktop and phone apps check on launch and on every return to the foreground, the TUI at launch.

Pick the other command when the note has to be durable and syncing before the process exits:

```bash
outl -w ~/notes daily append --text "TODO renew passport #admin"
```

`daily append` opens the workspace, which costs seconds on a big graph, and writes the op straight into the log.
The trade-offs side by side are in [cli.md → Capture](cli.md#capture).
Pairing `capture` with a macOS Service, Raycast or Alfred: [Capturing from outside the app](client-outside-the-app.md#capturing-from-outside-the-app).

## Append to a specific day

```bash
outl -w ~/notes daily append --date yesterday --text "forgot to log: met Ana about pricing"
outl -w ~/notes daily append --date 2026-11-20 --text "backfill"
```

`--date` takes ISO dates, `yesterday`, `tomorrow` and `"April 22nd, 2026"`.
Anything else fails with `INVALID_DATE` and writes nothing.

## Put today's open tasks in a morning email

`outl query` filters pages, not blocks, so use `search` for a plain-text sweep:

```bash
outl -w ~/notes search "TODO" --in blocks --limit 200 --json \
  | jq -r '.data.blocks[] | "\(.page)\t\(.text)"'
```

`search` is a substring match over the op log, capped at 50 hits per category unless you pass `--limit`.
For a real task list with filters, sorting and live updates, write a ` ```query ` fence in a page: [Queries](query.md).

## A weekly review

```bash
outl -w ~/notes daily range --from 2026-11-16 --to 2026-11-22 --json \
  | jq -r '.data.journals[] | select(.exists) | .date'
```

The range is inclusive, and a day with no journal comes back as `{"date": "...", "exists": false}`, so gaps are visible rather than skipped.

## Write many things in one go

`batch` applies a list of ops in one workspace session and stops at the first failure.
Earlier ops stay applied: there is no rollback, because every op is already in the CRDT log.

```bash
cat <<'JSON' | outl -w ~/notes batch --json
{"ops": [
  {"op": "page_create",       "args": {"slug": "launch-plan", "title": "Launch plan"}},
  {"op": "block_append_tree", "args": {"page": "launch-plan",
      "tree": {"text": "Milestones", "children": [{"text": "Beta"}, {"text": "GA"}]}}},
  {"op": "page_prop_set",     "args": {"page": "launch-plan", "key": "status", "value": "draft"}}
]}
JSON
```

On success: exit `0` and `data.applied` is the op count.
On failure: exit `1`, and the reply carries `failed_at`, `failed_op` and `error`, so the script can decide what to do with the ops that never ran.
Argument names per op are in [cli.md → Batch](cli.md#batch).

A single page with its outline in one call, no batch needed:

```bash
echo '[{"text":"Agenda","children":[{"text":"Status"},{"text":"Risks"}]}]' \
  | outl -w ~/notes page create standup --title "Standup" --content -
```

The reply lists the id of every block it created.

## Publish a page to a Hugo site

```bash
outl -w ~/notes export hugo my-post --out ./content/posts/
```

That writes `./content/posts/my-post.md` with TOML front matter (`title`, `slug`, and every page property) followed by the outline as markdown.
For plain markdown on stdout use `export md`, for the full AST plus sidecar use `export json`.

A property you want on the post goes on the page first:

```bash
outl -w ~/notes page prop set my-post draft=false
```

## Rename a page without breaking links

```bash
outl -w ~/notes page rename old-name new-name
```

Every `[[old-name]]` and `#old-name` in the workspace is rewritten in the same step, and the reply lists the blocks it touched (`rewritten_refs`).
If `new-name` already exists the command refuses with `SLUG_CONFLICT`.
To fold one page into another on purpose:

```bash
outl -w ~/notes page merge old-name new-name --confirm
```

Read both pages with `page get` first: there is no `--dry-run`.
Details and limits: [cli.md → Merging pages](cli.md#merging-pages).

## Attach a file

```bash
outl -w ~/notes asset add ~/Downloads/contract.pdf --page legal
outl -w ~/notes asset add ./screenshot.png          # today's journal
```

The file is copied to `assets/<hash>.<ext>` (identical bytes are stored once) and a markdown link is appended as a block.
Images render inline, other files are links.
Files above `[assets] max_bytes` (100 MiB by default) are refused before anything is copied: [Configuration](config.md).

## Edit the markdown files in your own editor

The `.md` files are plain text and yours to edit.
outl reads them back into the op log whenever something is watching or a client opens:

```bash
outl -w ~/notes serve --once     # read every changed .md into the op log, then exit
outl -w ~/notes doctor           # confirm: no "unreconciled external edit" line
```

A running desktop app, TUI or `outl serve` picks edits up on its own.
`outl serve --once` is for the case where nothing is running, for example after a `git pull` or a bulk find-and-replace.
Plain `outl reconcile` does not apply edits, it lists blocks it could not match.
Keep the dialect: one `- ` bullet per block, two spaces per indent level: [Markdown format](markdown-format.md).

## Checkpoint before anything risky

```bash
outl -w ~/notes backup now -m "before roam import"
outl -w ~/notes doctor
```

If `backup now` prints `nothing changed since the last snapshot`, you are already covered.
Getting something back: [Backups and restore](backups.md).

## Nightly backup and health check (cron)

```cron
30 2 * * *  outl -w $HOME/notes backup now -m "nightly" >> $HOME/.outl-backup.log 2>&1
45 2 * * *  outl -w $HOME/notes doctor --json > /tmp/outl-doctor.json || mail -s "outl doctor failed" me@example.com < /tmp/outl-doctor.json
```

`doctor` exits `1` when its report holds an error, so the `||` fires.
Read `data.error_count`, not `ok`: the envelope's `ok` only says the check ran, and stays `true` when the report found errors.
Desktop and TUI users already get an automatic snapshot at most every 30 minutes; the cron line is for headless boxes.

## Flush a script's writes to your other devices

A short-lived `outl` command does not stay online long enough to push to peers.
End the script with one explicit pass:

```bash
outl -w ~/notes daily append --text "deploy finished"
outl -w ~/notes sync
echo "sync exit: $?"
```

Exit `0` means a pass ran.
Exit `3` means nothing was flushed, and that is not a failure: another outl process on this machine holds the sync endpoint and will push the ops, or sync is off, or no device is paired.
Exits `1` and `2` are real failures.

## Keep a headless replica

```bash
outl init ~/replica --bare
outl peer pair --ticket - -w ~/replica   # paste the ticket `outl peer pair` printed on your main device
outl serve --no-watch ~/replica          # leave this one running
```

`--no-watch` holds only the sync endpoint, so it takes no write lock and never makes your desktop mint a new actor file.
The full walkthrough, with Docker, is [Self-hosting an always-on peer](self-hosting.md).
Pairing from scratch: [Syncing devices](sync-devices.md).

## Move a graph in

```bash
outl import roam ~/Downloads/backup.json ~/notes --dry-run   # measure first, writes nothing
outl import roam ~/Downloads/backup.json ~/notes
```

The dry run prints what would land and the reconciliation table.
Read the table before the real run: [Importing a graph](import.md).

## Let an agent drive it

`outl mcp serve` exposes the same operations as MCP tools, with the same JSON.
Setup for Claude Desktop, Claude Code and Cursor: [MCP server](mcp.md).
Prompts that work well against a workspace: [MCP recipes](mcp-recipes.md).
