# Why outl

This page is for the person who's already using Roam, Logseq or Obsidian and is wondering whether outl is worth the switch.
Short answer: if the way your tool handles sync, file ownership, or markdown cleanliness has ever annoyed you, yes.

## The two-paragraph version

Roam Research is the original block-based outliner.
It nailed the feel: bidirectional links, daily journal, dense queries, every line a first-class thing you can reference.
It also locked your data in their cloud and never really solved offline merge.

Logseq took the next step: put files on disk so you actually own your notes.
But to make it work, they pollute every block with `id::` lines, paid their sync as an add-on built on file rsync (no real merge), and spent a year pivoting to a database backend that fragmented the community.

outl picks the parts that worked (the outliner UX, the bi-directional graph, the daily journal) and rebuilds the parts that didn't.
The rebuilt parts are a [proper sync algorithm](sync.md), markdown that stays clean, and storage that's an interface: one append-only file per device, syncable by any filesystem-level transport.

## Feature-by-feature

| | Roam | Logseq | outl |
|---|------|--------|------|
| **Files on disk?** | No, cloud only | Yes (`.md` files) | Yes (`.md` files) |
| **Markdown stays clean?** | N/A | No, `id::` lines on every block | Yes, IDs in a `.outl` sidecar beside the `.md` |
| **Offline editing?** | Limited | Yes | Yes |
| **Multi-device sync** | Cloud sync (paid plan, no merge surfaced) | File rsync (paid plan, last-write-wins) | Tree CRDT, P2P over iroh (default); file/iCloud opt-in |
| **Conflict on concurrent moves?** | Silent loss | Silent loss | Deterministic resolution, no loss |
| **Time travel / history** | Paid tier | Per-file git, optional | Page history from the op log (desktop, phone), automatic git snapshots of the workspace ([backups](backups.md)), and a trash that reaches any deletion. A queryable history backend is tracked in [#1][i1] |
| **Open source** | No | Yes (frontend) | Yes (MIT) |
| **Plugin system** | Yes (JS) | Yes (JS, complex) | Yes (JS, [RFC 0025](rfcs/0025-plugin-system.md)) |
| **Mobile** | Native, fine | Native, known-bad | iOS on TestFlight, Android APK on every release |
| **Desktop** | Electron | Electron | Tauri 2: macOS, Linux and Windows |
| **TUI** | No | No | Yes, first-class |
| **Daily journal** | Yes | Yes | Yes |
| **`[[refs]]` / `#tags`** | Yes | Yes | Yes |
| **Block refs `((blk-XXXXXX))` + embeds `!((blk-XXXXXX))`** | Yes (long uids) | Yes (long uids) | Short, sidecar-backed handles; clean `.md` |
| **Page embeds** | `{{embed: [[page]]}}` | `{{embed [[page]]}}` | `![[page]]`: one token, no `{{…}}` wrapper, and still a page reference ([spec](markdown-format.md#page-embeds-page)) |
| **Queries** | `{{query: ...}}` rich | Datalog-ish | A ` ```query ` block with one `key: value` filter per line, results are live references ([Query code blocks](query.md)) |

[i1]: https://github.com/outlmd/outl/issues/1

## What outl is **not**

Be honest about what we're not building:

- **Not a Notion replacement.**
  No database views, no kanban boards, no team workspaces with permissions.
  outl is for one human (or a few) thinking through nested bullets.
- **Not a web app.**
  There's a terminal client, a Tauri desktop app, and iOS and Android apps; no browser-based version is planned.
- **Not a federation protocol.**
  Sync keeps your notes flowing between *your* devices.
  It's not Mastodon for notes: there's no public graph, no following, no shared spaces.
- **Not opinionated about your workflow.**
  Templates are optional pages you can ignore ([Templates](templates.md)).
  No required tags.
  No mandatory daily review modal.
  The TUI gets out of your way.

## Who outl is for

- People who keep daily notes and want them on disk in plain markdown.
- People who use more than one device and have lost work to bad sync at least once.
- People who got tired of `id::` lines.
- People who want to inspect their notes with `grep`, `awk`, whatever, without first parsing a proprietary format.
- People comfortable with a keyboard-driven TUI.
  (The desktop, iOS and Android apps ship too; the TUI is the most battle-tested, and [Which client does what](clients.md#which-client-does-what) lists where each one differs.)

## Who outl is **not** for, yet

- Visual thinkers who need a mind-map or a graph view.
  The clients are text/outline first; a graph view is not yet built.
- Teams that need shared editing today.
  Sync is between *your* devices, not a shared workspace.

## Moving over: what changes

If you come from Roam, Logseq or Obsidian, three things feel different on day one.

- **Your notes are files, and they are yours.**
  Open `~/notes/pages/` in any editor.
  There are no `id::` lines to strip before you leave, because they were never there.
- **A page exists once you open it.**
  `[[Some Page]]` in a block does not create a file until you follow it.
- **Sync needs pairing, once.**
  Devices pair with a code, then converge on their own.
  No account, no cloud folder required ([Sync your devices](sync-devices.md)).

Bringing your graph is one command, and `--dry-run` shows the damage before you commit:

```bash
outl import auto ~/path/to/your/graph ~/notes --dry-run
outl import auto ~/path/to/your/graph ~/notes
```

It reads Roam JSON backups, Logseq graphs and Logseq DB exports, and Obsidian vaults.
Block references become real outl references, folded blocks stay folded, and anything it cannot resolve stays in the text where you can `grep` it.
What each dialect turns into: [Importing a graph](import.md).
The first hour, end to end: [Getting started](getting-started.md).

## The pitch in one paragraph

We took everything Roam and Logseq taught us about how outlines feel and threw away the parts where they cut corners on storage and sync.
Then we rebuilt the foundation on a published tree-CRDT algorithm with a machine-checked proof ([Kleppmann et al. 2022](https://martin.kleppmann.com/papers/move-op.pdf)), plus a test suite that holds the implementation to it.
The markdown on your disk is exactly what you'd write by hand.
The sync won't lose your work when two devices edit offline.
The architecture is layered so the same engine drives the TUI, the Tauri desktop, and the iOS and Android apps.
If you want to know exactly how the algorithm works, [we wrote a whole page on it](sync.md).
