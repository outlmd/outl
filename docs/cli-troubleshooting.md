# Troubleshooting the CLI

Find your symptom, read what it means, run the next command.
Errors are quoted exactly as `outl` prints them.
For a workspace that behaves strangely rather than a command that fails, run [`outl doctor`](doctor.md) first.

## Which workspace did that touch?

**Symptom:** A command succeeded, but your notes folder did not change.
Or a page you never meant to touch appeared somewhere else.

**Cause:** The workspace is not always the current directory.
With no `-w` and no path argument, outl uses `[workspace] last` from `~/.config/outl/config.toml`, which the desktop app rewrites whenever you open a workspace.
Only if that is unset, or points at a folder that no longer exists, does it fall back to the current directory.

**Fix:** Name the workspace every time it matters:

```bash
outl -w ~/notes workspace info      # prints "root:" so you can see which one was used
```

`outl workspace info` is the quickest check: its first line is the root the command resolved.
Scripts should always pass `-w`.
The full order is in [Quick start → which workspace](cli-quickstart.md#2-tell-outl-which-workspace-to-use).

## Exit codes

| Code | Meaning | What to do |
|---|---|---|
| `0` | Success | Nothing. |
| `1` | You gave it something it could not use: bad input, not found, conflict, a refusal | Read `error.code` and `error.message`, fix the input, run again. Nothing was written unless the command says otherwise (`batch` keeps the ops that ran before the failure). |
| `2` | An internal error, or a command line the parser rejected (unknown flag, missing argument) | For a usage error the message names the missing piece. For an `INTERNAL` error, retry once, then run `outl doctor`. |
| `3` | `outl sync` only: nothing was flushed, and that is not a failure | Another outl process on this machine holds the sync endpoint and will push your ops, or sync is off, or no device is paired. |

`outl doctor` exits `1` when its report contains an error.
Its `--json` envelope still has `"ok": true`, because the check itself ran: read `data.error_count`.

## Error codes

These appear as `error (CODE): message` in the terminal and as `error.code` in `--json` and MCP replies.
Codes are stable strings and are never renumbered.

| Code | What it means | Next step |
|---|---|---|
| `NO_WORKSPACE` | The path has no `.outl/` folder. Also raised when the workspace config cannot be read. | `outl init <path>`, or point `-w` at the right folder. Opening the TUI (`outl` with no subcommand) on a terminal offers to create the workspace. Every other command just fails. |
| `PAGE_NOT_FOUND` | No page has that slug. Slugs, not titles. | `outl page list` and copy the slug column. Titles and aliases do not resolve on the command line. |
| `BLOCK_NOT_FOUND` | The id is well formed but no block has it. | The block may have been deleted: `outl trash list`. Or the id belongs to another workspace. |
| `INVALID_BLOCK_ID` | Not a 26-character block id. | Use the full id a command printed. The short `blk-evy6my` handle is only accepted by `outl backlinks embed`. |
| `INVALID_DATE` | The date did not parse. | Use `2026-11-25`, `yesterday`, `tomorrow` or `"April 22nd, 2026"`. |
| `INVALID_ARG` | A value failed validation: a blank title, a property value with a newline, `--page` together with `--daily`, a file over `[assets] max_bytes`, `--raw` (not implemented). | The message names the argument. |
| `CONFIRM_REQUIRED` | A destructive command needs `--confirm` (`page delete`, `page merge`, `block delete`). Over MCP and in `batch`, `confirm: true`. | Re-run with the flag if you meant it. Nothing was changed. |
| `SLUG_CONFLICT` | `page rename` onto a slug another page owns. | Pass `--merge` to fold the page into the existing one, or run `outl page merge <source> <target> --confirm`. |
| `PROP_NOT_FOUND` | The page or block has no property with that key. | `outl page prop list <page>` shows what it has. |
| `CYCLE_REJECTED` | `block move` would put a block inside its own subtree. The op is recorded as a no-op and the tree is unchanged. | Move it under a block that is not one of its descendants. |
| `NOT_TRASHED` | `trash restore` on a block that is not in the trash. | `outl trash list`. |
| `TRASH_PAGE_UNSUPPORTED` | The id is a deleted page. Only blocks restore today. | `outl block tree <id>` prints the content so you can re-create it. |
| `TRASH_PARENT_TRASHED` | The block's old parent is in the trash too. | Restore the parent first. The message names it. |
| `TRASH_PARENT_MISSING` | The old parent is not in the tree at all. | `outl doctor`. |
| `TRASH_ORIGIN_UNKNOWN` | The log does not say where this block came from, or the place cannot be reached from here. | `outl doctor`, then `outl block tree <id>` to read the content. |
| `TRASH_ORIGIN_INSIDE_SUBTREE` | The block's old parent has since moved inside the subtree you are restoring. | `outl block move <parent-id> --parent <live-block-id>` first, then restore. |
| `PAGE_MARKDOWN_AHEAD_OF_LOG` | The write was refused: the page's `.md` holds lines no op recorded, and overwriting would delete them. The reply carries `path`, `lines`, `sample` and `recovery_command`. | Run the command it names, normally `outl reconcile --ahead-of-log`. Your text is safe on disk until you do. |
| `INTERNAL` | Storage, filesystem or an unexpected state. Also what you see when another process holds the device's write lock and the command needs it. | Retry once. If a GUI or `outl serve` is running, close it or wait. Then `outl doctor`. |

## The terminal is full of log lines

**Symptom:** `INFO boot phases: ...` or `INFO initial scan: ...` lines on every command.

**Cause:** The CLI logs at `INFO` to stderr by default, and it ignores `RUST_LOG`.
The only dial is `-v`: once for `DEBUG`, twice for `TRACE`.

**Fix:** Logs are on stderr and results are on stdout, so redirect stderr when you only want results:

```bash
outl -w ~/notes page list 2>/dev/null
```

`--json` output is never mixed with logs.

## A page file has spaces or capitals in its name

**Symptom:** After `outl page create "My New Page"` the file is `pages/My New Page.md`.

**Cause:** The positional argument is the slug, used literally.

**Fix:** Pass `--slugify` to derive a clean slug from a name (`my-new-page`).
Rename the existing one with `outl page rename "My New Page" my-new-page`.
Rules for slugs: [Markdown format](markdown-format.md).

## "another outl process holds the device actor's write lock"

**Symptom:** A write fails with `INTERNAL` and a message about the device actor, while reads still work.

**Cause:** One process at a time writes under this device's actor.
The desktop app, the TUI, `outl serve` or `outl mcp serve` has it, and the command you ran refuses to invent a second actor.

**Fix:** Close the other process or wait for it to exit, then repeat.
Leave long-lived helpers on `outl serve --no-watch`, which takes no write lock: [outl serve](cli.md#outl-serve--the-background-daemon).

## Many `ops-<ulid>.jsonl` files keep appearing

**Symptom:** `ops/` grows a new `ops-01...jsonl` every time you run a write command.

**Cause:** A command that loses the race for the device actor gets a throwaway actor of its own, and each one leaves a file.
Typical trigger: a GUI or `outl serve` (with the watcher on) running while you script writes.

**Fix:** Run write commands when nothing else holds the workspace, or use `outl serve --no-watch` as your always-on helper.
`outl doctor` reports the actor count and warns past a threshold.
Read-only commands (`search`, `query`, `page get`, `workspace info`) never mint an actor.

## `$OUTL_DEVICE_DIR is set` warning

**Symptom:** `WARN $OUTL_DEVICE_DIR is set: this process is a SEPARATE device from ~/.outl ...`

**Cause:** The variable moves this process onto its own device identity and its own sync node id.
It is meant for tests and throwaway sandboxes.
Peers paired with your real identity show this process as offline.

**Fix:** Unset it for normal use (`unset OUTL_DEVICE_DIR`).
If you use it on purpose, keep the directory: deleting it rotates the node id, and every device paired with it then refuses to sync.
More: [Device store](device-store.md).

## `outl sync` exits 3

Nothing is wrong.
See [Exit codes](#exit-codes).
To see why, run `outl peer status`, and check whether `outl serve`, the TUI or the desktop app is already running.

## `outl doctor` says errors but `--json` says `ok: true`

`ok` reports that the check ran.
Read `data.error_count` and `data.findings[]`, or use the exit code.
What each finding means and which ones `--repair` will fix: [outl doctor](doctor.md).

## An import stopped halfway

Re-run the exact same command.
The marker `.outl/import-in-progress.json` tells outl that everything in the destination came from the unfinished run, so no `--force` is needed.
To start over, delete the destination folder and import into a fresh one: [Importing a graph](import.md).

## `error: unrecognized subcommand` or `unexpected argument`

The parser rejected the line and exited `2`.
Two spellings that trip people up: block commands take `--page`, not a positional page (`outl block append --page ideas --text "..."`), and ids are positional (`outl block update <id> --text "..."`).
`outl <command> --help` prints the exact usage, and the [CLI reference](cli.md) mirrors it.

## Still stuck

1. `outl -w <DIR> doctor --json > doctor.json`
2. Re-run the failing command with `-vv 2> trace.log`.
3. Open an issue at [github.com/outlmd/outl](https://github.com/outlmd/outl/issues) with the command, both files, and `outl --version`.
   Check them first: they name paths and may quote note text.
