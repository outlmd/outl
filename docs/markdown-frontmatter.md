# YAML frontmatter

What outl does with a `.md` that opens with a `---` fence.

Short version: it keeps it, byte for byte, and never reads it.

```markdown
---
aliases: [Note One, note-one]
tags:
  - project/alpha
cssclass: wide
---

- first bullet
  - nested
```

That file survives `outl serve`, `outl block append`, a GUI edit, a peer's sync and `outl doctor --repair` with the fence unchanged.

## Why preservation and not conversion

outl's own page metadata is [`key:: value`](markdown-format.md#page-properties-top-of-file), and the dialect has [no frontmatter delimiter](markdown-format.md#what-is-not-in-the-file).
A fence in the file therefore belongs to **another tool**.

`transport = "file"` exists so a workspace can live in iCloud Drive, Syncthing or a shared folder, which means pointing outl at a folder that is also an Obsidian vault is not misuse — it is the interop story.
Converting that vault's `aliases:`, `tags:`, `cssclass:`, `publish:` and Dataview fields into outl syntax would break the other tool; deleting them would break the user.
So outl carries them and stays out of the way.

Not supporting a construct and rewriting someone else's file are different decisions, and for a while they shared one code path.
The parser read the fence as ordinary content, so `---` became a bullet and the first write projected five bullets back over four lines of metadata ([issue #281](https://github.com/outlmd/outl/issues/281)).

## The rules

- **The fence is never outline.**
  Its delimiters and keys are not blocks: they do not appear in search, in backlinks, in a block reference, or in any client's outline.
- **It survives every write.**
  The fence travels the op log as one property on the page root, so it converges between devices and a projection from the tree re-emits it rather than dropping it.
  This is the same rule every other page-level fact follows — state that must converge goes through an `Op`, never through a file with last-write-wins semantics.
- **Deleting it sticks.**
  Remove the fence in an editor and the next reconcile removes it from the log too, so nothing grows it back.
- **It is not a place to put outl properties.**
  `title: My Note` inside the fence does **not** set the page title.
  Write `title:: My Note` as an outl page property, below the fence, when you want outl to act on a value.
  The two coexist in one file:

  ```markdown
  ---
  aliases: [Note One]
  ---
  title:: My Note
  type:: person

  - first bullet
  ```

- **It is not a chip you can edit.**
  The fence travels as a reserved page property (`page-frontmatter`), and no client offers it in the property panel, the "add a property" menu or a copied block.
  Editing YAML is the editor's job, and deleting the property would tell outl the fence is gone while the file still has it, which stops the page syncing.
  The page's history leaves frontmatter edits out for the same reason it leaves out `page-slug`: the fence is not part of what the page says.

- **An unterminated fence is not frontmatter.**
  `---` with no closing delimiter leaves every line as ordinary content, handled by [permissive parsing](markdown-format.md#permissive-parsing--warnings).
  A malformed fence must never swallow the rest of the file.

## What outl normalizes

Three changes on the first save, all deliberate, none lossy.

| You wrote | outl writes back | Why |
|---|---|---|
| `...` as the closing delimiter | `---` | Both end a YAML document, and carrying two spellings means the file never settles on one shape |
| no blank line between the closing delimiter and the first bullet | one blank line | The `key:: value` header run ends at the first blank line, so the separator has to be there for page properties to keep working |
| a UTF-8 BOM before the opening `---` | dropped | U+FEFF is an encoding artifact, not content, and no renderer emits one — preserving it would leave the file changing shape on every save |

Everything inside the fence — key order, indentation, comments, quoting, block lists, nested mappings — comes back exactly as written.

The BOM row is the one that used to bite.
A `.md` written by a Windows editor opens `\u{feff}---`, and for a while the parser skipped those three bytes while the scan that decides *which leading lines are metadata* did not.
So the fence was metadata to one and four lines of unlogged content to the other: outl refused to write the page, on every boot, and could never clear the state because the write that drops the BOM is the same write being refused.
Such a page recovers by itself on the next reconcile — no `--ahead-of-log` needed, since the op log always held the fence.

## Importing is a different operation

`outl import obsidian` **translates** frontmatter into `key:: value` properties ([import](import.md)), because an import is a one-way conversion into the dialect and the source file is left behind.

Pointing outl at a folder in place is not an import.
Nothing is converted, and the `.md` files stay usable by whatever else reads them.

## When a page stops syncing over this

A page whose `.md` carries a fence the op log has never seen is refused rather than overwritten, with the same error and the same recovery as any other content the log lacks:

```
outl reconcile --ahead-of-log
```

That reads the fence into the log and clears the refusal.
It is reachable in one situation — a workspace whose pages were last reconciled by a binary that predates this behaviour, before their first reconcile on the new one — and `outl doctor` names the page.
See [Surfacing a page that stopped syncing](clients.md#surfacing-a-page-that-stopped-syncing).
