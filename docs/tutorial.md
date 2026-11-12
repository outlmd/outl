# Your first week with outl

This is a tutorial, not a reference.
We're going to take a real week's worth of notes, and at each step I'll point at the outl feature that handles it.
By the end you'll know enough to live in outl daily.

If you want the dense version, jump to [Getting started](getting-started.md) and the [TUI manual](tui.md).
Otherwise, keep reading.

---

## Day 0 — Install and open

```bash
git clone https://github.com/outlmd/outl.git
cd outl
cargo build --release
cp target/release/outl ~/.local/bin/   # or wherever your PATH leads
```

Pick a directory for your notes.
Doesn't matter where:

```bash
outl init ~/notes
outl --workspace ~/notes
```

Today's journal opens.
The screen looks like this:

```
┌Pages────────┬─outl · default-dark · ws:notes pages:0 blocks:1 ──────────┐
│             │ Journal · Monday, 2026-05-25                              │
│             ├───────────────────────────────────────────────────────────┤
│             │ │ - ▏                                                    │
│             │                                                          │
│             │                                                          │
│             │                                                          │
│             ├──── NORMAL  i edit  o new  h/l cursor  …  q quit ────────┤
└─────────────┴───────────────────────────────────────────────────────────┘
```

You're in **Normal mode**.
The cursor is on the empty first block.
Time to type.

---

## Day 1 — First thoughts

Press `i` to enter Insert mode.
The header changes to `[INSERT]`.
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

Notice: each `Esc` or `Enter` from Insert mode wrote your `.md` to disk and reconciled it.
You can `cat ~/notes/journals/2026-05-25.md` right now in another shell and see it.

`.md` is clean — no `id::`, no UUIDs:

```bash
$ cat ~/notes/journals/2026-05-25.md
- read kleppmann's tree CRDT paper today
- the move-with-cycle case is the interesting one
- I wonder how Yrs handles char-level concurrent edits
```

The IDs live in `.2026-05-25.outl` (dotfile).
You'll basically never look at it.

---

## Day 2 — Linking

Today you have a meeting with someone named Avelino.
Press `]` to navigate to tomorrow's journal — wait, that's tomorrow.
Press `[` to go back.
Press `t` to go to *today*.

Press `o` for a new block, `i` to type, then:

```
met with [[Avelino]]
```

The moment you start typing `[[Av` an autocomplete popup appears.
It's empty because you don't have a page for Avelino yet.
Type `elino]]` to close the brackets, press `Esc`.

Move the cursor onto the link (the cyan `Avelino`).
Press `Enter`.

Boom: outl created `pages/avelino.md` with `title:: Avelino` at the top, and put you on the new page.
Type the meeting notes here.
When you're done, press `q` then `outl --workspace ~/notes` again — or just press `[` to go back to today's journal.

The link in your journal still points at the page.
Open today again: `t`.
The block with `[[Avelino]]` now renders the link cyan-underlined because the target exists.

---

## Day 3 — Tags

You realize you want to group reading lists.
Add a block:

```
read MIT 6.824 lecture 4 #lecture #distsys
```

`#tags` work the same as `[[refs]]` — they resolve to pages (`pages/lecture.md`, `pages/distsys.md`), but the syntax is one token without brackets.

Open one: cursor over `#distsys`, press `Enter`. outl creates the page and lands you on it.
Add a block:

```
list of papers and lectures grouped under this tag
```

Press `B`.
The right pane opens: **Backlinks**.
It shows every block in the workspace that references `distsys`.
So far, just the line in today's journal.

Press `B` again to hide.
Press `q` to go back, then `t` to land on today.

---

## Day 4 — Hierarchy

You want to nest some bullets.
In Normal mode, hover the block:

```
read MIT 6.824 lecture 4 #lecture #distsys
```

Press `i`, position cursor at the end, then `Enter` (or `Esc` + `o`) to make a new block, type:

```
key concept: vector clocks
```

Press `Tab` to indent — it becomes a child of the lecture block.
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

## Day 5 — Editing prose with markdown

Add a block, press `i`, type:

```
**important:** the merge algorithm is *provably* convergent. see `apply_op`.
```

Press `Esc`.
The block now renders with markdown applied: `important` in bold, `provably` in italic, `apply_op` in code green.
The asterisks and backticks **disappear** in display.

Move to that block again (`j` / `k`).
Press `i`.
The asterisks *come back* — outl shows the raw source while you're editing so the cursor columns line up with bytes.
Press `Esc` and they vanish again.

Links work too: `[outl](https://outl.app)` becomes a blue underlined `outl` when you're not on the block, raw text when you are.

---

## Day 6 — Finding stuff

Your workspace now has a journal entry per day, three named pages (`Avelino`, `lecture`, `distsys`), and a bunch of blocks.
Finding things:

- **`Ctrl+P`** — quick switcher.
  Fuzzy match by title or filename.
  Type `dist` and `distsys` jumps to the top.
- **`/`** — slash command menu.
  Notion-style filterable list of every registered command.
  Type `/search` and `Enter` for workspace-wide block search; after the search popup closes, `n` and `N` walk through the rest of the matches.
- **`:`** — command palette (vim-style).
  Same registry as `/`, same aliases.
  `:open Avelino` opens by title, `:today` jumps to today's journal, `:theme nord` swaps the theme, `:q` quits.

Try them.
Press `?` if you forget a key — the help popup lists everything.

---

## Day 7 — Power moves

A few things that compound:

- **`u` / `Ctrl+R`** — undo / redo.
  Bounded at 200 steps.
  Cursor position is restored too.
- **`K` / `J`** (or `Alt+↑/↓`) — move the current block (with its subtree) up or down among its siblings.
- **`yy`** — yank the current block (and its subtree) to the OS clipboard.
  `p` pastes from the OS clipboard **with formatting** (outline structure / paragraph split preserved).
  `P` pastes **without formatting** (raw text, single block — useful when the clipboard contains identifiers with underscores or brackets).
  Works in Visual mode too: press `V`, extend with `j`/`k`, then `y`.
- **`Ctrl+Enter`** (or **`Ctrl+T`** as a portable fallback) — cycle the block's TODO/DONE/none prefix.
  In Insert mode it cycles inline without moving your cursor relative to the text.
  Use `Ctrl+T` on terminals/multiplexers (tmux, Terminal.app) that collapse `Ctrl+Enter` into plain `Enter`.
- **`Ctrl+L`** — re-read the workspace from disk.
  Useful when another editor changed a `.md` behind your back.
- **`Ctrl+S`** — force save.
  (Edits auto-save on every commit, but this is for muscle memory.)

You're now using outl roughly the way it's meant to be used.

---

## Day 8 — Date & time inserters

Journal-first means a lot of cross-linking between days.
Typing `[[2026-05-26]]` by hand gets old fast, so outl ships slash commands for it.

In Insert mode, type `/`, filter, pick:

- **`/date-today`** → `[[2026-05-26]]` (also `/dt`)
- **`/date-tomorrow`** → `[[2026-05-27]]` (also `/dtm`)
- **`/date-yesterday`** → `[[2026-05-25]]` (also `/dy`)
- **`/date-next-monday`** → next Monday's journal ref (also `/dnmon`, and one per weekday: `dntue`, `dnwed`, `dnthu`, `dnfri`, `dnsat`, `dnsun`)
- **`/date-next-week`** / **`/date-last-week`** → today ±7 days (`/dnw`, `/dlw`)
- **`/date +3d`** / **`/date -2w`** / **`/date +1m`** / **`/date 2026-06-15`** → flexible offset or absolute
- **`/time-now`** → `14:32` (plain text, no brackets — for "I started at" notes)
- **`/datetime-now`** → `[[2026-05-26]] 14:32` (stamp this moment)
- **`/iso-date-today`** → `2026-05-26` without brackets, for property values like `due:: 2026-05-26`
- **`/week-num`** → `#2026-W21` (ISO week as a tag — clusters weekly notes via the tag index)

All of these write at the cursor without committing your in-flight edit — type away.

---

## Coming from Logseq or Roam?

You don't have to retype anything:

```bash
# Logseq graph directory:
outl import logseq ~/path/to/logseq-graph ~/notes

# Roam JSON backup:
outl import roam ~/Downloads/avelino-backup.json ~/notes
```

The importer strips `id::` lines (Logseq), resolves `((uid))` block refs to `[[Page Title]]` links, and converts Roam's `{{[[TODO]]}}` to outl's `TODO ` prefix.
Filenames are slugified so `[[Meu Projeto]]` lands at `pages/meu-projeto.md` with `title:: Meu Projeto` preserved.

Anything that can't be resolved stays in the file as the source wrote it, `((UID))`, so you can `grep` and fix it manually.

---

## What's next

- **TUI manual** ([docs/tui.md](tui.md)) — full keymap, every overlay, every command, persistence behavior.
- **Theming** ([docs/theming.md](theming.md)) — six presets, how to add your own.
- **Sync, done right** ([docs/sync.md](sync.md)) — what makes the CRDT interesting.
  Sync runs over iroh P2P by default; file/iCloud is an opt-in transport.

If something feels off, [open an issue][issues].
If you wrote a patch, [the contributing guide][contrib] tells you what reviewers look at.

[issues]: https://github.com/outlmd/outl/issues
[contrib]: https://github.com/outlmd/outl/blob/main/CONTRIBUTING.md
