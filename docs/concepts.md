# Workspace anatomy

A workspace is a directory.
Everything else is a convention on top.

## Layout

```
~/notes/                            # workspace root
├── .outl/
│   ├── config.toml                 # workspace identity + settings
│   ├── peers.toml                  # P2P peers
│   └── orphans.log                 # blocks that lost their ID during external edits
├── ops/
│   ├── ops-<this-actor>.jsonl      # this device's append-only op log
│   └── ops-<peer-actor>.jsonl      # mirror of another device, synced via iCloud / Syncthing
├── pages/
│   ├── avelino.md                  # clean markdown
│   ├── avelino.outl                # JSON sidecar with stable IDs
│   ├── meu-projeto.md
│   └── meu-projeto.outl
├── journals/
│   ├── 2026-05-25.md
│   └── 2026-05-25.outl
└── templates/
    └── journal.md                  # applied to new journals
```

## The concepts

### Workspace

The top-level directory.
Holds everything.
`outl init <path>` creates one.
There's no concept of "switching workspaces" inside the TUI — each `outl` process is bound to one workspace.

### Page

A named container for an outline.
One `.md` file in `pages/` is one page.
The filename is the [slug](#slugs); the human-visible name lives in the `title::` property.

```markdown
title:: Avelino
type:: person

- works on outl
- can be reached at [[email]]
```

The `type::` property on a page is read by surfaces that filter pages by role.
The canonical case is `type:: person`: every client opens a person picker on a word-initial `@` keystroke and lists only pages whose `type:: person` is set.
Accepting a candidate inserts `[[@name]]`, a regular wikilink whose `@` is purely visual (the page identity is still `name`, without the `@`).
See [the markdown format spec](markdown-format.md#mentions-name) for the full contract.

### Journal

A page keyed by date.
Files live in `journals/YYYY-MM-DD.md`.
Created automatically when you reference a date — typing `[[2026-05-25]]` and pressing `Enter` over the link makes the file if it doesn't exist.

The TUI opens on today's journal by default.
`[` / `]` navigate days.

### Block

A node in the outline tree.
One bullet line:

```markdown
- this is a block
  priority:: high       ← this is a property OF the block above
  - this is a child block
```

Every block has a stable ULID.
The ID is **never** in the `.md` — it's in the sidecar.

### Property

A `key:: value` pair attached to a page (when at the top of the file) or a block (when nested under one).

```markdown
title:: My project       ← page property
status:: active          ← page property

- objective              ← block
  priority:: high        ← block property
  owner:: [[avelino]]    ← block property
```

Properties drive queries (the `{{query: ...}}` DSL is planned) and influence display.

### Tag

A page reference with classification semantics.
`#urgent` resolves to the same underlying file as `[[urgent]]`, and both forms count as backlinks: a block mentioning `#urgent` shows up in the `urgent` page's "Linked from" panel exactly like a block mentioning `[[urgent]]`.
The remaining difference is presentational — tags additionally appear in filter sidebars and counts.

### Sidecar

A JSON file paired with each `.md`.
Stores the stable block IDs and content hashes:

```json
{
  "version": 2,
  "page_id": "01J...",
  "last_synced_hash": "sha256:...",
  "last_synced_at": "2026-05-25T...",
  "blocks": [
    {"id": "01J...", "line": 3, "indent": 0, "content_hash": "sha256:...", "ref_handle": "blk-r6s4a1"},
    {"id": "01J...", "line": 4, "indent": 1, "content_hash": "sha256:...", "ref_handle": "blk-r6s4a2"}
  ]
}
```

Filename is a dotfile: `pages/avelino.md` ↔ `pages/.avelino.outl`.
Hidden from `ls` by default; gitignorable if you want (but you'd lose ID stability across devices).

`ref_handle` is the short, stable handle used by inline block references (`((blk-XXXXXX))`) and embeds (`!((blk-XXXXXX))`).
See [`docs/markdown-format.md`](markdown-format.md#block-refs-and-embeds).

### Op log

The sequence of mutations that produced the current state.
Lives in `ops/ops-<actor>.jsonl` — one append-only JSONL file per device.
Every block creation, every move, every text edit is one line.
The tree is a projection over the merged log of every actor's file.

This is the **source of truth** — if your markdown gets corrupted, `outl doctor --repair` regenerates the pages from the log (a bare `outl doctor` reports without changing anything).

## Slugs

> **Why a page answers to three identities — its slug on disk, its title on screen, and the date that decides both:** [RFC 0107](rfcs/0107-page-identity.md).

`[[Avelino]]` → `pages/avelino.md`.
The slug rule:

- Lowercase
- Strip accents: `[[São Paulo]]` → `pages/sao-paulo.md`
- Non-alphanumeric → `-`, collapsed
- Empty result → `untitled`

The original name is preserved in `title::`.
The autocomplete on `[[` searches by title (not slug), so users type the way they think and outl figures out the filename.

That last point is what makes **namespaces** work: `[[os/linux]]` folds to `pages/os-linux.md` (a slug is one path component — no directory), but the `/` survives in `title::`, and every namespace question is asked of the title.
So `os` can list its nested pages and collect their mentions without the filesystem knowing anything about it.
See [markdown-format.md → Nested tags and page namespaces](markdown-format.md#nested-tags-and-page-namespaces).

## What's NOT in a workspace

- **Trash isn't a directory.** Deleted blocks are moved to a `TRASH_ROOT` node in the op log, not deleted from any file.
- **No `archive/` folder.** Archived pages are just pages you stopped referencing — they're still in `pages/`.
- **No per-workspace config beyond `config.toml`.** Plugins (JavaScript, via the Boa engine) are global, not workspace-scoped.

## Sharing a workspace

By default: `outl peer pair` exchanges a pairing ticket between devices and P2P sync (iroh) starts converging them over QUIC — no shared folder required.
The sidecar files carry the IDs, the op log carries the history.

Opt-in alternative: set `transport = "file"` and point each device at the same iCloud Drive / Syncthing / shared folder, or drag the directory between devices and reopen.

Neither path changes the file layout.
The transport just keeps the two directories converging.
