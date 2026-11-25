# Command line quick start

Ten minutes from an empty folder to a workspace you wrote in, searched, queried, exported and backed up, all from the terminal.
Every command and output on this page was run against a real build.
Block ids and timestamps differ on your machine, the shape does not.

You need the `outl` binary on your `PATH`.
Install options are in [Getting started](getting-started.md); `outl --version` tells you it worked.

The full flag reference is [cli.md](cli.md).
Copy-paste jobs for scripts and cron are in [cli-recipes.md](cli-recipes.md).
When something prints an error, start at [cli-troubleshooting.md](cli-troubleshooting.md).

## 1. Create a workspace

```console
$ outl init notes
Initialized outl workspace at notes
  ops:      notes/ops
  config:   notes/.outl/config.toml
  journal:  notes/journals/2026-11-25.md
  scope:    global
```

`init` needs a path.
It will not guess the current directory, so you cannot create a workspace somewhere by accident.
The folder holds plain markdown (`pages/`, `journals/`) plus the op log (`ops/`) and a small `.outl/` directory.
What each part is: [Storage](storage.md).

## 2. Tell outl which workspace to use

Every command picks its workspace in this order, first match wins:

1. a path given after the subcommand, where the command takes one (`outl doctor notes`)
2. the global flag `-w <DIR>` / `--workspace <DIR>`
3. `[workspace] last` in `~/.config/outl/config.toml`, which the desktop app updates whenever you open a workspace there
4. the current directory

**Rule 3 beats rule 4.**
If you have ever opened a workspace in the desktop app, `cd notes && outl page list` still reads the workspace the app opened last, not `notes`.
Pass `-w` whenever the answer matters, and always in scripts:

```bash
alias n='outl -w ~/notes'
n daily today
```

The rest of this page writes `-w notes` out in full.

## 3. Write

Add a line to today's journal.
The date comes from your clock, and the journal is created if it does not exist:

```console
$ outl -w notes daily append --text "TODO call the dentist #health"
appended block 01M897XMCFQA0EGYEK21EVY6MY to 2026-11-25
```

Create a page and put blocks on it:

```console
$ outl -w notes page create ideas --title "Ideas"
created: ideas (Ideas)
$ outl -w notes block append --page ideas --text "TODO ship the docs #docs"
appended: 01M897WPZV1ZMEX2C9XXRC6H60  TODO ship the docs #docs
$ outl -w notes block append --page ideas --text "see also [[inbox]]"
appended: 01M897XMA3DQVQVWTJT095RBXP  see also [[inbox]]
```

Three things worth knowing before you script this:

- **The positional argument of `page create` is the slug, used literally.**
  `outl page create "My New Page"` makes a file called `pages/My New Page.md`.
  Add `--slugify` to turn a human name into a clean slug (`my-new-page`).
- **Block ids are the 26-character ULIDs the command prints.**
  Keep them: `block update`, `block move`, `block delete` and `trash restore` take them.
  The short `blk-evy6my` handle you see in search output and `((blk-evy6my))` references is only accepted by `backlinks embed`.
- **Dates are links.**
  `[[2026-11-25]]` is a link to that day's journal, and `daily` commands accept `yesterday`, `tomorrow` and `"April 22nd, 2026"` as well as ISO dates.

## 4. Read

```console
$ outl -w notes daily today
2026-11-25
- 
- [ ] call the dentist #health
$ outl -w notes page get ideas
Ideas  (ideas, page)
- [ ] ship the docs #docs
- see also [[inbox]]
```

`page get` is the outline as outl understands it.
`page render` is the `.md` file as it will sit on disk:

```console
$ outl -w notes page render ideas
title:: Ideas

- TODO ship the docs #docs
- see also [[inbox]]
```

Find things:

```console
$ outl -w notes search dentist
# blocks
  [2026-11-25] ((blk-evy6my))  TODO call the dentist #health
$ outl -w notes tag list
#health                          1
#docs                            1
$ outl -w notes tag pages health
#health
  2026-11-25  2026-11-25
```

`search` is full text over blocks and page titles.
`query` filters pages by tag, property, kind and date, and every filter has a `--not-` twin:

```console
$ outl -w notes query --not-tag docs --kind page
page      ideas                           Ideas
page      templates-journal               templates/journal
```

`outl query` answers about **pages**.
To filter **blocks** (open tasks, for example) write a ` ```query ` fence inside a page: [Queries](query.md).

## 5. Change and undo

Tasks cycle one step per call: no marker, `TODO`, `DOING`, `DONE`, no marker.

```console
$ outl -w notes block toggle-todo 01M897WPZV1ZMEX2C9XXRC6H60
toggled: 01M897WPZV1ZMEX2C9XXRC6H60
```

`block update` replaces the whole text, so write the `TODO` / `DONE` prefix yourself if the block has one.
An update with plain text turns the task into an ordinary block.

Deleting asks you to mean it, and nothing is ever destroyed:

```console
$ outl -w notes block delete 01M897XMA3DQVQVWTJT095RBXP
error (CONFIRM_REQUIRED): refusing to delete block `01M897XMA3DQVQVWTJT095RBXP` without --confirm
$ outl -w notes block delete 01M897XMA3DQVQVWTJT095RBXP --confirm
deleted: 01M897XMA3DQVQVWTJT095RBXP
$ outl -w notes trash list
1 deletion(s) in the trash
  01M897XMA3DQVQVWTJT095RBXP  see also [[inbox]]

restore one with `outl trash restore <id>`
$ outl -w notes trash restore 01M897XMA3DQVQVWTJT095RBXP
restored 01M897XMA3DQVQVWTJT095RBXP: see also [[inbox]]
```

Want to know what happened to a block or a page?
The op log remembers every edit:

```console
$ outl -w notes block history 01M897WPZV1ZMEX2C9XXRC6H60 --limit 2
history of block 01M897WPZV1ZMEX2C9XXRC6H60 (in `ideas`)
  2026-11-25 08:37  edited
      - DONE ship the docs #docs
      + ship the docs (updated)
  2026-11-25 08:37  edited
      - DOING ship the docs #docs
      + DONE ship the docs #docs

showing the 2 most recent of 5 events — `--limit` for more
```

## 6. Ask for JSON

Add `--json` to any command and the output is one envelope on stdout.
Logs go to stderr, so a pipe stays clean:

```console
$ outl -w notes search dentist --json | jq -r '.data.blocks[] | "\(.page)\t\(.text)"'
2026-11-25	TODO call the dentist #health
```

The envelope, the exit codes and the error codes are in [cli.md → JSON envelope](cli.md#json-envelope).

## 7. Check health and keep a copy

```console
$ outl -w notes doctor
...
integrity OK
$ outl -w notes backup init
backup repository ready at notes
run `outl backup now` to take the first snapshot
$ outl -w notes backup now -m "end of the quick start"
90a7724  end of the quick start
```

`doctor` is read-only unless you add `--repair`: [outl doctor](doctor.md).
Backups are git snapshots stored outside the notes folder, and a restore never writes into your live workspace: [Backups and restore](backups.md).

## Where to go next

| You want to | Read |
|---|---|
| Bring notes over from Roam, Logseq or Obsidian | [Importing a graph](import.md) |
| Run scripts, cron jobs, a daily capture hotkey | [CLI recipes](cli-recipes.md) |
| Let Claude or another agent use the workspace | [MCP server](mcp.md) |
| Sync two devices | [Syncing devices](sync-devices.md) |
| Keep a server replica running | [Self-hosting an always-on peer](self-hosting.md) |
| See every flag | [CLI reference](cli.md) |
