# TUI troubleshooting

Find your symptom, read the cause, do the fix.
Messages are quoted as they appear in the footer, a toast or a banner, so you can search this page for them.
When nothing here matches, the TUI log is the next place to look: `<workspace>/.outl/tui.log` (run `RUST_LOG=debug outl` for more detail, then `tail -f` the file).

## Starting

**`outl-tui requires an interactive terminal (stdout is not a TTY)`**
The TUI draws on a terminal and stdout is a pipe, a file or a CI job.
Run it from a real terminal window.
For scripts use the `outl` subcommands in [cli.md](cli.md).

**`no outl workspace at <path> — run outl init <path> first`**
The folder has no `.outl/` directory and outl could not ask you, because stdin or stderr is not a terminal.
Run `outl init <path>`, or run `outl` from a terminal to get the `Initialize a new workspace here? [y/N]` prompt.

**It opens a different workspace than the folder I am in.**
`cd` is the last step of the lookup, not the first.
A `--workspace` flag wins, then `[workspace] last` in `~/.config/outl/config.toml`.
Run `outl --workspace .` to force the current folder.
The full order: [config.md → Workspace path](config.md#workspace-path).

**A screen that says `Opening <path>` and nothing else.**
The workspace is being read.
A big op log (hundreds of thousands of ops) can take several seconds on a cold start.
It is not hung.
The next start is faster, because the TUI writes a snapshot while it runs.

**The keyboard does nothing for about two seconds right after launch.**
The TUI asks your terminal whether it supports the kitty keyboard protocol, and a terminal that never answers makes it wait.
Keys work again by themselves.
It costs nothing on terminals that answer (kitty, WezTerm, Alacritty, Ghostty, foot, recent iTerm2).

**The screen is garbled and the shell shows no cursor after a crash.**
Type `reset` and press `Enter`, blind if you have to.
The TUI restores the terminal on a panic, so this only happens on a hard kill (`kill -9`).

**`<path> could not be read (…) — every preference is running on defaults`**
Your `config.toml` has a syntax error.
The TUI starts anyway on default settings and does not save settings until the file parses.
Nothing was overwritten.
Fix the file named in the message.
`outl doctor` also reports it.

## Keys that do nothing

**`Shift+Enter` or `Ctrl+Enter` acts like plain `Enter`.**
Your terminal does not report those keys (Terminal.app, tmux without `extended-keys`, plain xterm).
Use `Alt+Enter` or `Ctrl+J` for a new line inside a block, and `Ctrl+T` for the task cycle.

**`Alt+Enter`, `Alt+↑` or `Alt+↓` do nothing on macOS.**
Terminal.app and iTerm2 only send Alt chords when "Use Option as Meta key" is on.
Turn it on in the profile settings, or use `Ctrl+J`, `K` and `J` (Normal mode), which need no setting.

**`Ctrl+Shift+↑` does nothing.**
Most terminals cannot tell it from `Ctrl+↑`.
`Alt+↑` and, in Normal and Visual mode, `K` do the same move.

**`B` does not toggle the backlinks.**
It is `Ctrl+B`.
The help popup still says `B` in one row, which is the bug, and this page is right.

**`Ctrl+A`, `Ctrl+W`, `Ctrl+K` do nothing in Insert mode.**
Readline editing is not implemented.
The keys are ignored on purpose, so they never type a stray letter into your block.
Use the arrow keys, `Home`, `End`, `Backspace` and `Delete`.

**`c` does nothing on a block.**
Folding stores the fold on the saved block, and a block you just created has not been saved yet.
Wait for `saved` in the footer and press `c` again.
`c` also does nothing on a block with no children, since there is nothing to hide.

**`/date-today only works in Insert mode (would insert [[2026-11-25]])`.**
The date commands write at the caret, and Normal mode has no caret.
Press `i`, then type `/date-today` inside the block and pick it from the popup.
`/upload` has the same rule.

**`unknown command: <name>`**
Nothing in the registry or in an installed plugin has that name or alias.
Press `/` with an empty query to see the whole list, or open [TUI commands](tui-commands.md).

**`no undo for this page`**
Undo history belongs to the page where the edit happened.
Go back to that page and press `u` again, the history is still there.
Merges, renames and page deletes are not undoable with `u`: rename back, or restore from `:trash`.

**`z` or `g` shows `z…` and the next key does something else.**
`z`, `g`, `d`, `y` and `q` wait for a second key.
Any key that does not complete the chord is read as itself, so press `Esc` or the key you meant.

## Copy and paste

**`yanked 2 blocks (clipboard unavailable)`**
Neither the system clipboard nor the OSC 52 escape worked.
Typical causes:

- You are on SSH or in a container with no display.
  Your terminal has to accept OSC 52 for the copy to reach your local clipboard.
  iTerm2, kitty, WezTerm, Alacritty, Ghostty and Windows Terminal do, with a setting on in some of them.
- You are inside tmux.
  Add `set -g set-clipboard on` to `tmux.conf`.
- A Wayland session without XWayland.

The copy still works inside outl: `p` pastes it in the same session.

**`y r` or `/refer` say `yanked … (clipboard unavailable)` while `y y` works.**
Those two use the system clipboard only, with no OSC 52 fallback.
The reference is still kept inside outl for the session.
Type `((` in Insert mode and pick the block instead.

**`no ref handle yet — save and retry`**
The block has not been written yet.
Wait for `saved`, then try again.

**`clipboard unavailable` on `p`.**
outl could not read the system clipboard, which is common over SSH.
Paste with your terminal instead (`Cmd+V` or `Ctrl+Shift+V` while editing a block), which hands outl the text directly.

**A multi-line paste in Insert mode split into several blocks.**
Your terminal did not wrap the paste in bracketed-paste markers, so each line break arrived as `Enter`.
Press `Esc`, then use `p` from Normal mode.

**Pasting a list inside a code block turned it into blocks.**
The TUI has no code-block guard yet.
See [Paste → In the TUI](paste.md#in-the-tui) for the workaround.

## Saving and syncing

**`cannot read <path>: … — editing disabled`**
The page's file could not be read: permissions, a disk error, invalid text encoding, or an iCloud file that has not downloaded yet.
outl refuses to save the page so an empty read cannot overwrite your notes.
Fix the cause (let iCloud finish, `chmod`, restore the file), then `Ctrl+L`.

**Banner `This page isn't syncing`**
The file holds lines that outl never recorded, so the page stopped converging with your other devices in both directions.
Run the command the banner shows, in the workspace folder:

```bash
outl reconcile --ahead-of-log
```

Reload with `Ctrl+L`.
Your text is safe on this device in the meantime.
Background and the other recovery paths: [doctor.md](doctor.md).

**A yellow banner `N line(s) outside outl dialect — preserved as blocks`.**
The file has content outl's parser had to adapt: a `# heading`, a paragraph between bullets, markdown from another tool.
Nothing was removed.
The next save rewrites the file into bullets.
No action is needed.

**An edit from another program does not show up.**
The TUI re-reads the open page about once a second when you are idle.
In Insert mode it waits, and warns `…` in the footer instead of replacing your text.
Press `Esc`, then `Ctrl+L`.

**Changes from my phone or laptop do not appear while I type.**
By design: incoming changes wait until you leave Insert mode, so your text is never replaced mid-sentence.
Press `Esc`.

**`iroh sync off here: …`**
Another outl process on this device (the desktop app or `outl serve`) owns the connection and passes your changes on.
That is a working state.
If the reason names a file or permission, fix that.
`:sync` shows what each device is waiting for.

**Toast `iroh sync unavailable, using filesystem sync: …`**
P2P could not start.
Sync still runs through the shared folder if you use one.
Read the rest of the toast for the reason, and see [sync.md](sync.md).

**The same page edited on two devices has a merged paragraph.**
Concurrent edits to one block are merged.
`:sync` lists them, and `u` there puts your earlier text back as a normal edit.

**A capture I made with `outl capture` is missing.**
The TUI lands captures when it starts.
A capture made while it is running waits for the next start.
`outl capture --list` shows what is waiting.

## What I see on the screen

**Backlinks says `indexing…`, or search finds nothing right after launch.**
The workspace index is building in the background.
The `indexing` chip is visible in the header while it runs.
It finishes within seconds on most workspaces.

**Backlinks are missing.**
The panel may be hidden: `Ctrl+B`.
Zooming into a block (`z i`) hides it too, since the panel answers for the page, not the block.
`z o` zooms out.

**Nested pages cannot be selected.**
Right, the list is read-only in the TUI.
Press `Ctrl+P` and type the page name.

**A table's columns do not line up.**
A table wider than the terminal wraps instead of scrolling.
Widen the window.
While the cursor is on a table you see its raw pipes, by design.

**A `mermaid` block shows as text.**
The terminal does not draw diagrams.
Open the page in the desktop or mobile app.

**Icons show as boxes or question marks.**
You set `[tui] icons = "nerd-font"` without a Nerd Font in the terminal.
Remove the line, or install and select a Nerd Font.
The default emoji set needs no special font.

**Colors look wrong or unreadable.**
Try another preset: `outl --theme light` or `:theme light`.
List them with `outl theme list`.
A theme file with an error is ignored, and that command says which and why ([theming.md](theming.md)).

## Mouse

**I cannot select text with the mouse.**
You turned on `[tui] mouse_capture = true`, so outl owns the mouse.
Hold your terminal's override key (often Shift) while dragging, or set it back to `false`.
Keyboard copy (`y y`) works either way.

**The mouse does nothing.**
`mouse_capture` is off by default.
Add it under `[tui]` in `~/.config/outl/config.toml` and restart.

## Reminders

**A reminder never fired.**
Reminders fire only while an outl app is running, and a TUI session counts only while it is open.
Check `g n` to see the rule and its next fire time.
Check `[reminders] enabled` is not `false` ([reminders.md](reminders.md)).
A quiet window can push the fire to its end.
The toast always appears, and the desktop banner only in terminals that support OSC 9.

## Plugins

**A plugin command is missing from `/`.**
The plugin failed to load, and the TUI showed a toast at start-up.
Check the plugin id and API version, then `outl plugin list`.
`/plugin-settings` says `No configurable plugins installed` when none declares settings.

## Still stuck

Open an issue at <https://github.com/outlmd/outl/issues> with:

- the exact message from the footer or banner,
- the last 50 lines of `<workspace>/.outl/tui.log`,
- your terminal and whether you are inside tmux or SSH,
- `outl --version`.

Do not paste your whole log if you are not sure what is in it.
Block text is never written to it, but file paths are.
