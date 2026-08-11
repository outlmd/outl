# outl

> Local-first outliner.
> Markdown is the source of truth.
> Sync that doesn't corrupt your tree when two devices edit offline.

outl takes the parts of [Roam Research][roam] and [Logseq][logseq] that work — bi-directional links, dense queries, block-level thinking, journal-first — and rebuilds the part that doesn't: how your notes survive being on more than one device.

[roam]: https://roamresearch.com
[logseq]: https://logseq.com

## Where to start

| You want to... | Read |
|----------------|------|
| Install and try outl in a minute | [Getting started](getting-started.md) |
| Install via Homebrew (GA or beta) | [Homebrew tap](homebrew.md) |
| Understand the pitch vs. Roam/Logseq | [Why outl](why-outl.md) |
| Know *exactly* how sync works | [Sync, done right](sync.md) |
| Use the TUI fluently | [TUI manual](tui.md) |
| Change colors / write a theme | [Theming](theming.md) |
| Script outl or plug it into Claude Code | [CLI](cli.md) |
| Connect outl to Claude Desktop, Cursor, etc. | [MCP](mcp.md) |
| Build an MCP-driven skill or slash command on top of outl | [MCP recipes](mcp-recipes.md) |
| Send a PR and know what reviewers look at | [Contributing & code review](contributing.md) |

## What's locked in

The shape of outl is settled:

- **Markdown is on disk, untouched.** No `id::` lines.
  No HTML comments.
  No frontmatter delimiters.
  What you wrote is what's saved.
  Stable IDs live in a sidecar file (`foo.outl`, next to `foo.md`) you'll never have to look at.
- **The op log is the source of truth.** Not the file.
  Not the database.
  A sequence of [`Move` / `Edit` / `Create` / `SetProp`][crdt] ops with HLC timestamps.
  The tree you see is a projection.
- **Storage is a trait, not a struct.** JSONL (one append-only file per device) ships today; [ChronDB][chrondb] is tracked publicly for when you want git-style history with branches and time travel.
- **Every UI surface shares one core.** The TUI, the Tauri desktop, and the iOS app all reuse [`outl-core`][outl-core] and [`outl-md`][outl-md] —
  including the tokens, the index, the slugify rules.
  Android ships the same way — a signed APK on every release; see [android-platform.md](android-platform.md).

[crdt]: crdt.md
[chrondb]: https://github.com/avelino/outl/issues/1
[outl-core]: https://github.com/avelino/outl/tree/main/crates/outl-core
[outl-md]: https://github.com/avelino/outl/tree/main/crates/outl-md

## Status (May 2026)

- Single-device editor: **works**.
  Modes, undo/redo, autocomplete, backlinks, theming, fuzzy switcher, workspace-wide search, command palette.
- Cross-device sync: **works today** (macOS TUI ↔ iOS app).
  The iOS client is on public TestFlight beta — [join here][testflight].
- P2P transport: **iroh (default)**.
  QUIC, end-to-end encrypted, no central server; pairing via `outl peer pair`.
  The CRDT algorithm is implemented and tested ([170+ tests][tests]); the `file` transport (iCloud Drive / Syncthing / shared FS) is the opt-in alternative.
- Tauri desktop: **works** (macOS / Linux / Windows).

Where it's headed: the [roadmap lives on the GitHub Project][roadmap].

[testflight]: https://testflight.apple.com/join/P2GdWAMd

[roadmap]: https://github.com/users/avelino/projects/2/views/1

[tests]: https://github.com/avelino/outl/actions

## Background reading

Long-form posts about the engineering behind outl, published on [avelino.run](https://avelino.run):

- **[File sync isn't trivial](https://avelino.run/file-sync-isnt-trivial/)** — the distributed-systems problem behind concurrent file moves, and what a formally-verified algorithm gives you that ad-hoc merge doesn't.
- **[From paper to outliner](https://avelino.run/from-paper-to-outliner/)** — the engineering between a CRDT proof and a shipped app: projections, reconciliation, transport edge cases, editor state.

## Contributing

The [README on GitHub](https://github.com/avelino/outl) has the install bits and the dev workflow.
Before sending a PR, read [Contributing & code review](contributing.md) — the rules of the game are written down so you know exactly what reviewers will look at.
Open issues to discuss design before sending big PRs; the sync algorithm in particular has a 100% coverage rule on its critical functions.

## License

[MIT](../LICENSE).
