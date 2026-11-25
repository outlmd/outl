# Properties

A property is a `key:: value` line.
Written at the top of a file it describes the **page**.
Written under a bullet it describes that **block**.

```markdown
title:: Interview notes
type:: meeting
pinned:: true

- TODO send feedback
  due:: 2026-12-01
  remind:: 3pm every 1h until DONE
```

Any key you invent is stored, synced and shown as a chip on the block.
A handful of keys also **do something**.
This page lists those, says where each one works, and says which keys outl writes for itself so you do not trip over them.

The syntax rules (what ends the page header, empty values, the alphabetical order outl writes them in) live in [Markdown dialect](markdown-format.md#page-properties-top-of-file).

## Keys that do something

| Key | On | Value | What it does |
|---|---|---|---|
| `title` | page | text | The name shown everywhere. The file name is the [slug](concepts.md#slugs); the title keeps your spelling, accents and `/` namespaces |
| `alias` | page | comma-separated names; `[[A, b]]` keeps a comma inside one name | Other names the page answers to. `[[ref]]`, `#tag`, `@mention` and autocomplete all match them. See [Page](concepts.md#page) |
| `type` | page | `person`, or any word | `type:: person` puts the page in the `@` mention picker. Other values are free-form labels today |
| `icon` | page | one emoji or a short string | The glyph shown beside the page in lists and the sidebar. Also set with `outl page update --icon` |
| `pinned` | page | `true`, `yes`, `1` or `on` | Pins the page to the sidebar (TUI: `/pin` or `g P`). Journals cannot be pinned. Any other value, or no key, means not pinned |
| `tags` | page | text | Plain page metadata. `outl export hugo` carries it into the front matter, and a page merge unions it |
| `template` | page | the template's name | Makes the page a [template](templates.md). The value is what you type after `/template` |
| `params` | page | comma-separated names | The parameters a callable template accepts. See [Callable templates](templates.md#type-2-callable-templates) |
| `from-template` | block | template slug | Written by outl on the root of a stamped template, so the template page can list where it was used. Do not edit it by hand |
| `remind` | block | `3pm every 1h until DONE` | Turns the block into a notification. The one property the parser validates. See [Reminders](reminders.md) |
| `auto-run` | block (a code block's bullet) | anything | Re-runs the code block when its source changes. TUI only. See [Auto-run](exec.md#auto-run) |
| `source-hash` | block (a result child) | `sha256:…` | outl's own cache key for the auto-run check. Do not edit it by hand |

A key is matched as written: `Remind::` is not `remind::`.
Keys cannot contain spaces.

## Keys outl keeps out of the file

These exist on a page but are never written to the `.md`, never shown as chips and never offered in the property editor.
They are internal bookkeeping, listed so you know why they do not appear:

| Key | Holds |
|---|---|
| `page-slug`, `page-kind` | The page's identity and whether it is a journal or a regular page |
| `page-source` | For a page made by opening a file outside the workspace, where the file lives. A local path, so it stays out of the `.md` and out of an `outl export hugo` |
| `page-frontmatter` | A YAML fence another tool wrote. Kept byte for byte. See [YAML frontmatter](markdown-frontmatter.md) |

Fold state is not a property either.
Older versions wrote `collapsed:: true` into the file.
Today whether a block is folded syncs through the op log and is not in the `.md`, so folding on your phone folds it on your laptop.
There is no `id::` in a file, ever: block ids live in the [sidecar](concepts.md#sidecar).

An import can write three keys of its own: `roam-uid::` (with `--keep-source-ids`), `created::` and `edited::` (with `--preserve-timestamps`).
They stay in the `.md`.
Clients hide them from the property chips when the value is a machine timestamp, so a `created:: by Roberta` you wrote yourself still shows.
See [Importing a graph](import.md).

## Edit a property

| Client | How |
|---|---|
| **TUI** | `g p` opens the property editor for the selected block: it lists what the block has and completes new keys from the keys already in your workspace. `/prop <key> <value>` sets one directly (an empty value deletes it), and `/prop-page <key> <value>` does the same on the page |
| **Desktop** | Click a chip to edit it. `Cmd+Shift+P` (`Ctrl+Shift+P` on Linux and Windows) in view mode adds a property to the selected block |
| **Mobile** | Tap a chip to edit it |
| **Any editor** | Type the line. `key:: value` under a bullet is a block property, at the top of the file it is a page property |
| **CLI** | `outl block prop set <block-id> key=value`, `outl page prop set <slug> key=value`, and `get`, `list`, `delete` beside them. See [CLI](cli.md) |
| **MCP** | `outl_block_prop_*` and `outl_page_prop_*`. See [MCP](mcp.md) |

Setting a value to nothing removes the key, in every client.

## Property or tag

Use a **property** for a fact about one block or page that you want to filter or read as a field: `due`, `owner`, `status`.
Use a **[tag](concepts.md#tag)** for a topic you want to browse by.
A [query](query.md) can filter on both: `prop: status: done` and `tag: ops`.

Two things to know when you query properties:

- A query reads **block** properties.
  `prop: type: person` finds blocks carrying `type:: person`, not pages.
  `outl query --prop type=person` reads **page** properties.
- `prop:` compares case-insensitively; `outl query --prop` compares case-sensitively.

## Troubleshooting

| You see | Why | Fix |
|---|---|---|
| A `key:: value` line shows as plain text | It has a space in the key, or sits after prose in the same bullet | Properties come right after the bullet line, before any continuation text. Write `due:: …`, not `due date:: …` |
| A page property is treated as a block | The header has a blank line or a bullet before it | Page properties are the first lines of the file, ending at the first blank line |
| Your properties changed order after a save | outl writes them alphabetically by key | Nothing to fix. See [Property order](markdown-format.md#property-order) |
| `pinned:: true` and the page is not in the sidebar | The page is a journal | Journals cannot be pinned |
| `remind::` does not fire | The rule did not parse | `outl doctor` names the line. See [When a rule doesn't parse](reminders.md#when-a-rule-doesnt-parse) |
