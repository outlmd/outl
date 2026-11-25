# Your first 10 minutes in the TUI

Goal: write a journal entry, link two pages, check off a task, find a note again, and undo a mistake.
You will use about twenty keys.
Every step says what you should see, so you can tell it worked.

You need outl installed ([Getting started](getting-started.md)) and a terminal at least 80 columns wide.

## 1. Open a workspace

```bash
mkdir ~/notes && cd ~/notes
outl
```

outl does not find a workspace in an empty folder, so it asks `Initialize a new workspace here? [y/N]`.
Type `y` and press `Enter`.
You land on today's journal.

You should see a header with `Journal · <today's date>`, one empty bullet, and a footer that starts with a `NORMAL` badge.

Normal mode is where you move and give commands.
Nothing you type appears as text yet.
That is the vim part, and the only part you have to unlearn.

> If `outl` opens a different folder than the one you are in, a workspace is saved in your config as the last one used.
> Run `outl --workspace ~/notes` to be explicit.
> The lookup order is in [TUI manual → Start it](tui.md#start-it).

## 2. Write something

1. Press `i`.
   The badge changes to `INSERT`.
2. Type `call the dentist` and press `Enter`.
   A new block opens below.
3. Type `read the paper on tree CRDTs`.
4. Press `Esc`.
   The badge goes back to `NORMAL`.

`Esc` is what saves your text into the page.
The footer shows `saved` a moment later, and the header chip counts up from `just now`.

To add a block without entering a block first, use `o` (below the cursor) or `O` (above).

## 3. Make it a task and check it off

Move to the first block with `k` (up) and `j` (down).
Press `Ctrl+T`.
The block gets a `☐` checkbox, and the header shows `0/1` tasks.
Press `Ctrl+T` again for `◐` (doing), and once more for `☑` (done).
A fourth press clears it.

To jump straight to done, press `g` then `D`.

## 4. Nest and fold

1. Put the cursor on `read the paper on tree CRDTs`.
2. Press `o` and type `Kleppmann 2022`, then `Esc`.
3. Press `Tab`.
   The block moves under its parent.
   `Shift+Tab` brings it back.
4. Move to the parent and press `c`.
   The child hides and the bullet shows `▶`.
   Press `c` again to open it.

## 5. Link to another page

1. Press `A` to start typing at the end of the parent block.
2. Type ` see [[` (a space, then two opening brackets).
   outl adds the closing `]]` for you, and a popup lists your pages.
3. Type `CRDT notes`.
   There is no such page yet, so the popup may be empty, and that is fine.
   Do not type `]]`: it is already there, and typing it again adds a second pair.
4. Press `Esc`.
5. Make sure the cursor is on the link (`h` and `l` move it), then press `Enter`.

`Enter` on a link opens the page, and it creates it when it does not exist.
The footer says `created page "CRDT notes"`.
You are now on a new page with one empty bullet.
Press `i`, write a line, press `Esc`.

There is no "back" key.
Press `t` to return to today's journal, or `Ctrl+P` to pick any page.

## 6. See who links here

Press `Ctrl+P`, type `crdt` and press `Enter`.
You are on `CRDT notes`.
Under the outline you should see `Backlinks · 1 ref(s)` and the block from your journal that linked here.

The panel is on by default.
`Ctrl+B` hides it.
Press `i` on the backlink row to edit that block from here.
`Esc` to finish.

## 7. Find a note

Press `/`, type `search`, press `Enter`.
Type a few letters of something you wrote, such as `dent`.
Matching blocks appear with their page names.
`↓` to choose, `Enter` to jump.

Afterwards `n` and `N` walk through the other matches.

Closer than a search: put the cursor on a word and press `*` to search the whole workspace for it.

## 8. Undo, and the safety net

1. Press `d` twice (`d d`) on a block.
   It is deleted.
2. Press `u`.
   It is back.

`u` only remembers this session, per page.
For anything older, or a block deleted from another device, type `:trash` and press `Enter`.
Move to the row with `j` and press `r` to restore it.

## 9. Copy and paste

Put the cursor on a block with children and press `y` twice (`y y`).
The footer says `yanked 1 block → clipboard`.
The child blocks travel with their parent.
Open any other app and paste: you get a clean bullet list.

Back in outl, move somewhere else and press `p`.
The copied blocks land after the selected one, hierarchy intact.

## 10. Where to go from here

| Press | To |
|---|---|
| `?` | Open the help popup, with tabs for each mode |
| `/` | See every command with its description |
| `g p` | Edit a block's properties (`due::`, `type::`, anything) without remembering key names |
| `g r` | Add a reminder to a block |
| `V` then `j` | Select several blocks, then `Tab`, `d` or `y` acts on all of them |
| `Ctrl+E` | Open the sidebar with a calendar, pinned and recent pages |
| `q q` | Quit |

Next:

- [TUI manual](tui.md) for every key and screen.
- [TUI commands](tui-commands.md) for the `/` and `:` list, including the date inserters (`/date +3d`).
- [TUI troubleshooting](tui-troubleshooting.md) if something behaves oddly.
- [Your first week with outl](tutorial.md) for the ideas behind tags, journals and links.
