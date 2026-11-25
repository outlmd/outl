# TUI commands

Every command the TUI ships, what it does, and how to reach it.
The screen, modes and keys are in the [TUI manual](tui.md).

## The two doors

Two doors, one list.
`/` opens a filterable menu with a description next to every command, and groups them (Actions, Navigation, Search, Settings, Dates and time).
`:` is the vim-style command line.
Both run the same registry with the same arguments and aliases, so `/quit` and `:q` are the same thing.
A name that matches nothing says `unknown command: <name>` in the footer.

Inside a block, typing `/` at the start of a word opens the same list inline.
Pick a command with `Enter` or `Tab` and the `/query` text disappears as it runs.
A command that needs an argument (`/open`, `/rename`) opens the `:` line pre-filled with its name.
Commands marked `…` in the menu take an argument.

## Workspace and navigation

| Command | Aliases | What it does |
|---|---|---|
| `open <name>` | `o`, `new`, `n` | Open the page called `<name>`, creating it (with `title::` and one empty bullet) when it does not exist. The footer says `created page "name"` |
| `today` | | Jump to today's journal |
| `search` | `s`, `find` | Workspace search |
| `write` | `w`, `save` | Save the page now |
| `refresh` | `r`, `reload` | Re-read the workspace from disk and rebuild the index (same as `Ctrl+L`) |
| `theme <preset>` | | Swap the theme for this session. `unknown theme: <name>` if it does not exist |
| `help` | `h` | Toggle the help popup |
| `quit` | `q`, `exit` | Close the TUI. An edit in progress is committed first |
| `sync` | | Open the sync panel |
| `trash` | `deleted` | Open the trash |
| `plugin-settings` | `plugins` | Open the plugin settings |

## Properties

| Command | Aliases | What it does |
|---|---|---|
| `prop-block <key> <value>` | `prop` | Set a `key:: value` property on the block. An empty value deletes it |
| `prop-page <key> <value>` | | Set a page property (`title::`, `icon::`, `type::`, ...) |
| `pin` | `pinned` | Toggle `pinned::` on the page. Same as `g P` |

`:prop remind <rule>` is how you edit a reminder after `g r` writes the starter rule.
Setting `title` on a page is a rename, with every link respelled, not a plain property write.
`page-slug` and `page-kind` are refused, since they are the page's identity.

## Pages

| Command | What it does |
|---|---|
| `rename <new name>` | Rename the open page and respell every `[[old]]` and `#old` in the workspace |
| `merge <page name>` | Move the open page into another existing page, after a `y/n` |

**Rename.**
`/rename Beta` on the page `Alpha` renames it in the same batch as every mention.
The slug comes from the name you type and the title is the name itself, so `/rename Café Notes` lands on `pages/cafe-notes.md` titled `Café Notes`.
A name that only changes capitalisation or punctuation keeps the slug, so the file stays put.
The toast says what the rewrite touched, for example `renamed 'Alpha' to 'Beta': links updated in 3 blocks`.
It also names `[[Beta/child]]` mentions it left alone, because those belong to a sub-page that was not renamed.
Renaming onto a free name runs on the keystroke, since renaming back is the undo.
When another page already has the name, nothing is written and the footer asks `'Beta' already has that name, merge 'Alpha' into it? can't be undone here. y/n`.
`y` runs the merge, anything else cancels and leaves both pages as they were.
A journal cannot be renamed, because its slug is its date.

**Merge.**
`/merge Beta` on `Alpha` asks `merge 'Alpha' into 'Beta'? can't be undone here. y/n`.
On `y`, the blocks of `Alpha` move under `Beta` with ids and subtrees intact, `Alpha`'s properties and names carry over as aliases, links to `Alpha` are rewritten to `Beta`, and `Alpha` goes to the trash.
You land on `Beta` and a toast says what moved, for example `merged 'Alpha' into 'Beta': 2 blocks, links updated in 1 block`.
Where both pages set the same property to different values, `Beta`'s value is kept and the toast says how many clashed.
The target must already exist, a journal cannot be the source, and a page cannot merge into itself.
`u` does not undo a merge or a rename, and the undo history of every page they touched is dropped so an old snapshot cannot undo half of it.
The same operations from a script: `outl page merge` and `outl page rename` in [cli.md](cli.md), and [RFC 0557](rfcs/0557-page-merge-and-rename.md) for the design.

## Block references

| Command | What it does |
|---|---|
| `refer` | Copy `((blk-XXXXXX))` for the current block. Same as `y r` |
| `refer-embed` | Copy `!((blk-XXXXXX))`, which shows the source block and its children where you paste it |

Both write through the OS clipboard only, with no OSC 52 fallback.
Over SSH the footer says `yanked … (clipboard unavailable)`.
The token is still kept inside outl for the session.
A block that has not been saved yet says `no ref handle yet — save and retry`.

## Files and code

| Command | Aliases | What it does |
|---|---|---|
| `upload` | `attach` | Pick a file, copy it into `<workspace>/assets/<hash>.<ext>`, and insert its `[name](assets/…)` link at the caret |
| `run` | `x`, `execute` | Run the code block under the cursor |
| `template [name key=value …]` | `tpl` | Open the template picker, or insert / run a template by name |

`/upload` is an Insert-mode command and says `/upload only works in Insert mode` anywhere else.
On macOS and Windows it opens the system file dialog.
On Linux it takes a path: `/upload` opens the `:` line pre-filled with `upload `, so `upload /path/to/file.pdf` works everywhere, including over SSH.
Files over `[assets] max_bytes` are refused with the reason in the footer.
outl never renders the file.
`g x` on the link opens it in the OS default app.
Link format: [markdown-format.md → Asset links](markdown-format.md#asset-links-nameassetshashext).

`/template` with no argument opens a picker: type to filter by name, `↑` / `↓` choose, `Enter` inserts.
A structural template is copied under the cursor, a callable one runs and its output becomes a block.
`template standup person=Ana` passes parameters.
`template <name> not found` means no page carries that `template::` property.
Writing templates: [templates.md](templates.md).

## Date and time inserters

These write text at the caret, so they need Insert mode.
From Normal they say `/date-today only works in Insert mode (would insert [[2026-11-25]])` and change nothing.
Type `/` at the start of a word inside a block to use them.
The names and the text they produce are shared with the desktop app, so `/date-today` writes the same string in both.

| Command | Aliases | Inserts |
|---|---|---|
| `date-today` | `dt` | `[[YYYY-MM-DD]]` for today |
| `date-tomorrow` | `dtm` | `[[YYYY-MM-DD]]` for tomorrow |
| `date-yesterday` | `dy` | `[[YYYY-MM-DD]]` for yesterday |
| `date-next-week` | `dnw` | `[[YYYY-MM-DD]]`, today + 7 |
| `date-last-week` | `dlw` | `[[YYYY-MM-DD]]`, today − 7 |
| `date-next-monday` ... `date-next-sunday` | `dnmon` `dntue` `dnwed` `dnthu` `dnfri` `dnsat` `dnsun` | The next such weekday as a journal ref |
| `date <arg>` | | Flexible, see below |
| `iso-date-today` | `isod` | `YYYY-MM-DD` with no brackets, for `due::` and friends |
| `iso-date-tomorrow` | `isodtm` | `YYYY-MM-DD` |
| `iso-date-yesterday` | `isody` | `YYYY-MM-DD` |
| `time-now` | `now`, `tn` | `HH:MM` |
| `datetime-now` | `dtn`, `stamp` | `[[YYYY-MM-DD]] HH:MM` |
| `week-num` | `wn`, `week` | `#YYYY-Www`, the ISO week as a tag |

`date-next-<weekday>` is always strictly in the future.
Run on a Monday, `date-next-monday` gives next Monday, seven days out, not today.
`week-num` uses the ISO week-year, so 2025-12-31 is `#2026-W01`.

| `/date …` | Resolves to |
|---|---|
| `+3d` | Today plus 3 days |
| `-2w` | Today minus 2 weeks |
| `+1m` | Today plus one month, clamped to the last day (Jan 31 + 1m is Feb 28 or 29) |
| `5d` | A bare `Nd`, `Nw` or `Nm` counts as positive |
| `2026-06-15` | That date |
| `April 22nd, 2026` | Any spelling the shared date parser accepts: `2026/04/22`, `22/04/2026`, `Sept 3rd, 2025`, `22 April 2026` |

Anything else (`/date nope`, `/date +3x`, an impossible date) shows `usage: date +Nd | -Nw | +Nm | YYYY-MM-DD`.

## Plugin commands

JavaScript plugins in `<workspace>/.outl/plugins/` load at start-up.
Each command a plugin contributes appears in the `/` menu under its id, with the title and `plugin` as the description.
A plugin toolbar button shows up the same way, labelled `plugin · <plugin-id>`, because a terminal has no toolbar.
Selecting one runs it.
Notices and errors it raises appear as toasts.
If the plugin changed the workspace, the touched pages are re-written and the page reloads.

Plugins can also bind keys.
A binding in the plugin's manifest (`contributes.keybindings`) fires from Normal mode, as a single chord or a two-chord sequence.
Those two-chord sequences show `plugin chord…` in the footer while they wait for the second key.
A plugin can never take a key outright: a chord the TUI already uses stays with the TUI.
Plugin bindings do not fire in Insert mode or inside an overlay, so they cannot steal keys while you type.
Plugins can also react to every change (`onOp`), including ones that arrive from another device, and a hook that edits the workspace does not re-trigger itself.
A plugin that fails to load (bad manifest, wrong API version, tampered bundle) is skipped with a toast and the TUI runs normally.
A workspace with no plugins is unaffected.

`/plugin-settings` lists every installed plugin that declares settings.
`↑` / `↓` move, `Enter` toggles a boolean or starts editing any other field (`Enter` saves, `Esc` cancels), `Esc` closes.
Values go to the plugin lockfile.
Fields marked secret go to the OS keychain, show `set` or `not set` and are never displayed.
With nothing configurable installed it says `No configurable plugins installed`.
Installing a plugin is not done here: `outl plugin install <id>` ([plugins.md](plugins.md)).

