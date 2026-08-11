# Why outl

This page is for the person who's already using Roam or Logseq and is wondering whether outl is worth the switch.
Short answer: if the way your tool handles sync, file ownership, or markdown cleanliness has ever annoyed you, yes.

## The two-paragraph version

Roam Research is the original block-based outliner.
It nailed the feel — bidirectional links, daily journal, dense queries, every line a first-class thing you can reference.
It also locked your data in their cloud and never really solved offline merge.

Logseq took the next step: put files on disk so you actually own your notes.
But to make it work, they pollute every block with `id::` lines, paid their sync as an add-on built on file rsync (no real merge), and spent a year pivoting to a database backend that fragmented the community.

outl picks the parts that worked — the outliner UX, the bi-directional graph, the daily journal — and rebuilds the parts that didn't: a [proper sync algorithm](sync.md), markdown that stays clean, and storage that's an interface — one append-only file per device, syncable by any filesystem-level transport.

## Feature-by-feature

| | Roam | Logseq | outl |
|---|------|--------|------|
| **Files on disk?** | No — cloud only | Yes (`.md` files) | Yes (`.md` files) |
| **Markdown stays clean?** | N/A | No — `id::` lines on every block | Yes — IDs in dotfile sidecar |
| **Offline editing?** | Limited | Yes | Yes |
| **Multi-device sync** | Cloud sync (paid plan, no merge surfaced) | File rsync (paid plan, last-write-wins) | Tree CRDT, P2P over iroh (default); file/iCloud opt-in |
| **Conflict on concurrent moves?** | Silent loss | Silent loss | Deterministic resolution, no loss |
| **Time travel / history** | Paid tier | Per-file git, optional | Issue [#1][i1] tracks ChronDB |
| **Open source** | No | Yes (frontend) | Yes (MIT) |
| **Plugin system** | Yes (JS) | Yes (JS, complex) | Issue [#4][i4] tracks `rhai` |
| **Mobile** | Native, fine | Native, known-bad | iOS (Tauri 2), TestFlight beta; Android APK per release |
| **Desktop** | Electron | Electron | Issue [#2][i2] tracks Tauri |
| **TUI** | No | No | Yes — first-class |
| **Daily journal** | Yes | Yes | Yes |
| **`[[refs]]` / `#tags`** | Yes | Yes | Yes |
| **Block refs `((blk-XXXXXX))` + embeds `!((blk-XXXXXX))`** | Yes (long uids) | Yes (long uids) | Short, sidecar-backed handles; clean `.md` |
| **Queries** | `{{query: ...}}` rich | Datalog-ish | `{{query: ...}}` DSL — planned |

[i1]: https://github.com/avelino/outl/issues/1
[i2]: https://github.com/avelino/outl/issues/2
[i3]: https://github.com/avelino/outl/issues/3
[i4]: https://github.com/avelino/outl/issues/4

## What outl is **not**

Be honest about what we're not building:

- **Not a Notion replacement.** No database views, no kanban boards, no team workspaces with permissions. outl is for one human (or a few) thinking through nested bullets.
- **Not a web app.** There's a Tauri desktop and an iOS app; no browser-based version is planned.
- **Not a federation protocol.** Sync keeps your notes flowing between *your* devices.
  It's not Mastodon for notes — there's no public graph, no following, no shared spaces.
- **Not opinionated about your workflow.** No templates beyond the optional `journal.md`.
  No required tags.
  No mandatory daily review modal.
  The TUI gets out of your way.

## Who outl is for

- People who keep daily notes and want them on disk in plain markdown.
- People who use more than one device and have lost work to bad sync at least once.
- People who got tired of `id::` lines.
- People who want to inspect their notes with `grep`, `awk`, whatever — without first parsing a proprietary format.
- People comfortable with a keyboard-driven TUI.
  (The desktop and iOS apps ship too; the TUI is the most battle-tested.)

## Who outl is **not** for, yet

- Visual thinkers who need a mind-map or a graph view.
  The clients are text/outline first; a graph view is not yet built.
- Teams that need shared editing today.
  Sync is between *your* devices, not a shared workspace.

## The pitch in one paragraph

We took everything Roam and Logseq taught us about how outlines feel, threw away the parts where they cut corners on storage and sync, and rebuilt the foundation with a CRDT that has a formal proof and 170+ tests covering it.
The markdown on your disk is exactly what you'd write by hand.
The sync won't lose your work when two devices edit offline.
The architecture is layered so the same engine drives the TUI, the Tauri desktop, and the iOS app.
If you want to know exactly how the algorithm works, [we wrote a whole page on it](sync.md).
