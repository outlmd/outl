# Your first week with outl

This is a tutorial, not a reference.
We take a week's worth of notes, and at each step it points at the outl feature that handles it.
By the end you can live in outl daily.

It uses the terminal client because the keys are the same on every day of the week.
The desktop and phone apps do the same things with a mouse and touch.
Where they differ it says so, and [Client parity](client-parity.md) has the full table.

If you want the dense version, jump to [Getting started](getting-started.md) and the [TUI manual](tui.md).
Otherwise, keep reading.

---

## Day 0: Install and open

```bash
brew tap outlmd/outl https://github.com/outlmd/outl
brew trust outlmd/outl
brew install outl-beta
```

On Windows use Scoop, on Nix use the flake, and from source `cargo build --release`.
All of it is in [Getting started](getting-started.md#install-the-cli-and-tui).

Pick a directory for your notes.
Doesn't matter where:

```bash
outl init ~/notes
outl --workspace ~/notes
```

Today's journal opens.
The screen looks like this:

```text
┌ outl · outl ─────────────────────────────────────────────────────────┐
│ outl ▸ notes ▸ 📅 Journal · Wednesday, 2026-11-25         ⟳ just now │
└──────────────────────────────────────────────────────────────────────┘
┌Outline───────────────────────────────────────────────────────────────┐
│  - ▏                                                                 │
│  press i to write · ? for keys                                       │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────────────┐
│ NORMAL   ◌ notes    iroh sync enabled           🕐 08:36   💾  ? help │
└──────────────────────────────────────────────────────────────────────┘
```

You're in **Normal mode**.
The cursor is on the empty first block.
Time to type.

---

## Day 1: First thoughts

Press `i` to enter Insert mode.
The status bar changes from `NORMAL` to `INSERT`.
Type:

```
read kleppmann's tree CRDT paper today
```

Press `Esc`.
You're back in Normal.
Press `o` to make a new block below:

```
- read kleppmann's tree CRDT paper today
- the move-with-cycle case is the interesting one
- I wonder how Yrs handles char-level concurrent edits
```

Notice: each `Esc` or `Enter` from Insert mode wrote your `.md` to disk.
You can `cat ~/notes/journals/$(date +%F).md` right now in another shell and see it.

The `.md` is clean, with no `id::` and no UUIDs:

```bash
$ cat ~/notes/journals/2026-11-25.md
- read kleppmann's tree CRDT paper today
- the move-with-cycle case is the interesting one
- I wonder how Yrs handles char-level concurrent edits
```

The block ids live in `2026-11-25.outl`, right next to it.
It is not a dotfile, on purpose: iCloud Drive silently drops dotted paths when it syncs.
You will basically never look at it.

---

## Day 2: Linking

Today you have a meeting with someone named Avelino.
Press `[` to go to yesterday's journal and `]` to come back.
Press `t` to jump to *today* from anywhere.

Press `o` for a new block, and type:

```
met with [[Avelino]]
```

The moment you type `[[` an autocomplete popup appears with the pages you already have.
Keep typing `Avelino]]` and press `Esc`.

Move the cursor onto the link with `w` (next word) or `l` (one character right).
Press `Enter`.
Pressing `Enter` with the cursor anywhere else in the block starts editing it instead, which is why the cursor has to be on the link first.

outl creates the page `Avelino` (a file at `pages/avelino.md` with `title:: Avelino`) and opens it.
The status line says `created page "Avelino"`.
Type the meeting notes there.

A page only exists once you open it.
`[[Avelino]]` in a block, on its own, creates nothing, so a typo never litters your workspace.

To go back, press `t` for today, or `Ctrl+P` and type a few letters of any page.
The link in your journal now points at a real page.

On the desktop and phone the same thing is a click or a tap on the link.

---

## Day 3: Tags

You realize you want to group reading lists.
Add a block:

```
read MIT 6.824 lecture 4 #lecture #distsys
```

`#tags` work the same as `[[refs]]`: they resolve to pages (`pages/lecture.md`, `pages/distsys.md`), but the syntax is one token without brackets.

Open one: cursor over `#distsys`, press `Enter`. outl creates the page and lands you on it.
Add a block:

```
list of papers and lectures grouped under this tag
```

Look at the bottom of the page.
The **Backlinks** section lists every block in the workspace that references `distsys`.
So far, just the line in today's journal.
It is on by default in the terminal.
`Ctrl+B` hides it and shows it again.

Backlinks are editable where they sit: select a row and press `i`, and you edit the block on the page it lives on.
Press `t` to land on today.

---

## Day 4: Hierarchy

You want to nest some bullets.
In Normal mode, hover the block:

```
read MIT 6.824 lecture 4 #lecture #distsys
```

Press `i`, position cursor at the end, then `Enter` (or `Esc` + `o`) to make a new block, and type:

```
key concept: vector clocks
```

Press `Tab` to indent.
It becomes a child of the lecture block.
Press `Esc`, `o`, type:

```
contrast with HLC (what outl uses)
```

`Tab` again.
Now you have:

```
- read MIT 6.824 lecture 4 #lecture #distsys
  - key concept: vector clocks
  - contrast with HLC (what outl uses)
```

You can see the indent guides (`│`) on the left of the nested blocks.
That's the outline rendering.
Indents work to any depth.

---

## Day 5: Editing prose with markdown

Add a block, press `i`, type:

```
**important:** the merge algorithm is *provably* convergent. see `apply_op`.
```

Press `Esc`.
The block now renders with markdown applied: `important` in bold, `provably` in italic, `apply_op` in code green.
The asterisks and backticks **disappear** in display.

Move to that block again (`j` / `k`).
Press `i`.
The asterisks *come back*, because outl shows the raw source while you're editing so the cursor columns line up with bytes.
Press `Esc` and they vanish again.

Links work too: `[outl](https://outl.app)` becomes a blue underlined `outl` when you're not on the block, raw text when you are.

---

## Day 6: Finding stuff

Your workspace now has a journal entry per day, three named pages (`Avelino`, `lecture`, `distsys`), and a bunch of blocks.
Finding things:

- **`Ctrl+P`**: quick switcher.
  Fuzzy match by title or filename.
  Type `dist` and `distsys` jumps to the top.
- **`/`**: slash command menu.
  Notion-style filterable list of every registered command.
  Type `/search` and `Enter` for workspace-wide block search; after the search popup closes, `n` and `N` walk through the rest of the matches.
- **`:`**: command palette (vim-style).
  Same registry as `/`, same aliases.
  `:open Avelino` opens by title, `:today` jumps to today's journal, `:theme nord` swaps the theme, `:q` quits.

Try them.
Press `?` if you forget a key: the help popup lists everything.

---

## Day 7: Power moves

A few things that compound:

- **`u` / `Ctrl+R`**: undo / redo.
  Bounded at 200 steps.
  Cursor position is restored too.
- **`K` / `J`** (or `Alt+↑/↓`, or `Ctrl+Shift+↑/↓`), move the current block (with its subtree) up or down among its siblings.
- **`yy`**: yank the current block (and its subtree) to the OS clipboard.
  `p` pastes from the OS clipboard **with formatting** (outline structure / paragraph split preserved).
  `P` pastes **without formatting** (raw text, single block, useful when the clipboard contains identifiers with underscores or brackets).
  Works in Visual mode too: press `V`, extend with `j`/`k`, then `y`.
- **`Ctrl+Enter`** (or **`Ctrl+T`** as a portable fallback), cycle the block's task state: none, `TODO`, `DOING`, `DONE`, none.
  In Insert mode it cycles inline without moving your cursor relative to the text.
  Use `Ctrl+T` on terminals/multiplexers (tmux, Terminal.app) that collapse `Ctrl+Enter` into plain `Enter`.
- **`Ctrl+L`**: re-read the workspace from disk.
  Useful when another editor changed a `.md` behind your back.
- **`Ctrl+S`**: force save.
  (Edits save on every commit, but this is for muscle memory.)

You're now using outl roughly the way it's meant to be used.

---

## Day 8: Date & time inserters

Journal-first means a lot of cross-linking between days.
Typing `[[2026-05-26]]` by hand gets old fast, so outl ships slash commands for it.

In Insert mode, type `/`, filter, pick:

- **`/date-today`** → `[[2026-05-26]]` (also `/dt`)
- **`/date-tomorrow`** → `[[2026-05-27]]` (also `/dtm`)
- **`/date-yesterday`** → `[[2026-05-25]]` (also `/dy`)
- **`/date-next-monday`** → next Monday's journal ref (also `/dnmon`, and one per weekday: `dntue`, `dnwed`, `dnthu`, `dnfri`, `dnsat`, `dnsun`)
- **`/date-next-week`** / **`/date-last-week`** → today ±7 days (`/dnw`, `/dlw`)
- **`/date +3d`** / **`/date -2w`** / **`/date +1m`** / **`/date 2026-06-15`** → flexible offset or absolute
- **`/time-now`** → `14:32` (plain text, no brackets, for "I started at" notes)
- **`/datetime-now`** → `[[2026-05-26]] 14:32` (stamp this moment)
- **`/iso-date-today`** → `2026-05-26` without brackets, for property values like `due:: 2026-05-26`
- **`/week-num`** → `#2026-W21` (ISO week as a tag, clusters weekly notes via the tag index)

All of these write at the cursor without committing your in-flight edit, so type away.

---

## Day 9: Take it with you

Install outl on a second device and pair them.
The device that already has your notes is the host; the new one joins.

```bash
# host
cd ~/notes && outl peer pair

# joiner, from an empty folder
mkdir ~/notes && cd ~/notes
outl peer pair --ticket <ticket from the host>
outl sync
```

On a phone you scan the host's QR instead (the desktop shows one under **Settings → Sync**).
From then on, anything you write on either device shows up on the other whenever both are online, and edits made while one was offline merge when they meet.
There is no account and no server in the middle.
Full walkthrough, including what each client can do: [Getting started → Sync a second device](getting-started.md#sync-a-second-device).

---

## Coming from Logseq or Roam?

You don't have to retype anything:

```bash
# See what would happen first. Writes nothing.
outl import auto ~/path/to/source ~/notes --dry-run

# Logseq graph directory:
outl import logseq ~/path/to/logseq-graph ~/notes

# Roam JSON backup:
outl import roam ~/Downloads/backup.json ~/notes
```

`auto` detects Roam, Logseq, Logseq DB exports and Obsidian vaults, so you can use it for any of them.
The importer strips Logseq's `id::` lines, turns `((uid))` block references into real outl block references, and converts Roam's `{{[[TODO]]}}` to outl's `TODO ` prefix.
Filenames are slugified, so `[[Meu Projeto]]` lands at `pages/meu-projeto.md` with `title:: Meu Projeto` preserved.

Anything that can't be resolved stays in the file as the source wrote it, `((UID))`, so you can `grep` and fix it by hand.
Details for each dialect: [Importing a graph](import.md).

---

## What's next

- **A shorter start** ([tui-quickstart.md](tui-quickstart.md)): ten guided minutes in the TUI.
- **TUI manual** ([tui.md](tui.md)): full keymap, every overlay, every command, persistence behaviour.
- **Shortcuts** ([shortcuts.md](shortcuts.md)): the same chords for the desktop and phone.
- **Query blocks** ([query.md](query.md)): a ` ```query ` block that lists every open task in the workspace.
- **Theming** ([theming.md](theming.md)): eleven built-in presets, and how to write your own.
- **Sync, done right** ([sync.md](sync.md)): what makes the CRDT interesting.
  Sync runs over iroh P2P by default; file and iCloud folders are an opt-in transport.
- **Something broke?** [Troubleshooting](troubleshooting.md).

If something feels off, [open an issue][issues] or ask on [Discord](https://discord.gg/fDDXRtFhA6).
If you wrote a patch, [the contributing guide][contrib] tells you what reviewers look at.

[issues]: https://github.com/outlmd/outl/issues
[contrib]: https://github.com/outlmd/outl/blob/main/CONTRIBUTING.md
