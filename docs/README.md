# outl

> Local-first outliner.
> Markdown is the source of truth.
> Sync that doesn't corrupt your tree when two devices edit offline.

outl takes the parts of [Roam Research][roam] and [Logseq][logseq] that work (bi-directional links, dense queries, block-level thinking, journal-first) and rebuilds the part that doesn't: how your notes survive being on more than one device.
You write in a terminal, a desktop window or a phone.
Your notes are plain `.md` files in a folder you own.
Devices sync directly with each other, with no account and no server holding your graph.

[roam]: https://roamresearch.com
[logseq]: https://logseq.com

## Start here

New to outl?
Do these three, in order.
They take about twenty minutes.

1. [Getting started](getting-started.md): install, create a workspace, write a first note, pair a second device.
2. [Your first week with outl](tutorial.md): the editor, through a week of real notes.
3. [Troubleshooting](troubleshooting.md): keep it open in a tab; it answers the first-hour questions.

## Where to find things

| You want to... | Read |
|----------------|------|
| Install on macOS, Linux, Windows, iOS or Android | [Getting started](getting-started.md) |
| Decide which client to use | [Which client does what](clients.md#which-client-does-what) |
| Use the desktop app | [Using outl on the desktop](desktop.md) |
| Use the phone app | [outl on your phone](mobile.md) |
| Use the terminal editor fluently | [TUI manual](tui.md) |
| Find a key | [Shortcuts](shortcuts.md) |
| Know what each client can and cannot do | [Client parity](client-parity.md) |
| Put your notes on more than one device | [Sync your devices](sync-devices.md) |
| Run an *optional* always-on peer on hardware you own | [Self-hosting](self-hosting.md) |
| Bring a Roam, Logseq or Obsidian graph | [Importing a graph](import.md) |
| Undo a bad import or a deleted page | [Backups and restore](backups.md) |
| Change colors or write a theme | [Theming](theming.md) |
| Change a setting | [Configuration](config.md) |
| Fix something that broke | [Troubleshooting](troubleshooting.md), then [`outl doctor`](doctor.md) |
| Understand the pitch vs. Roam/Logseq | [Why outl](why-outl.md) |
| Know *exactly* how sync works | [Sync, done right](sync.md) |
| Script outl or plug it into Claude Code | [CLI](cli.md) |
| Connect outl to Claude Desktop, Cursor, etc. | [MCP](mcp.md) |
| Build an MCP-driven skill or slash command on top of outl | [MCP recipes](mcp-recipes.md) |
| Extend outl in JavaScript | [Plugins](plugins.md) |
| Install through a specific package manager | [Homebrew](homebrew.md), [Scoop](scoop.md), [Nix](nix.md) |
| Read what outl does with your data | [Privacy](privacy.md) |
| Send a PR and know what reviewers look at | [Contributing & code review](contributing.md) |

## What's locked in

The shape of outl is settled:

- **Markdown is on disk, untouched.**
  No `id::` lines.
  No HTML comments.
  No frontmatter delimiters.
  What you wrote is what's saved.
  Stable IDs live in a sidecar file (`foo.outl`, next to `foo.md`) you'll never have to look at.
- **The op log is the source of truth.**
  Not the file.
  Not the database.
  A sequence of [`Move` / `Edit` / `Create` / `SetProp`][crdt] ops with HLC timestamps.
  The tree you see is a projection.
- **Storage is a trait, not a struct.**
  JSONL (one append-only file per device) ships today; [ChronDB][chrondb] is tracked publicly for when you want git-style history with branches and time travel.
- **Every UI surface shares one core.**
  The TUI, the Tauri desktop, and the iOS and Android apps all reuse [`outl-core`][outl-core] and [`outl-md`][outl-md], including the tokens, the index, and the slugify rules.

[crdt]: crdt.md
[chrondb]: https://github.com/outlmd/outl/issues/1
[outl-core]: https://github.com/outlmd/outl/tree/main/crates/outl-core
[outl-md]: https://github.com/outlmd/outl/tree/main/crates/outl-md

## Status

| Area | State |
|---|---|
| Terminal client | Ships. Modes, undo and redo, autocomplete, backlinks, theming, fuzzy switcher, workspace-wide search, command palette |
| Desktop (macOS, Linux, Windows) | Ships, unsigned for now ([Getting started](getting-started.md#install-the-desktop-app)) |
| iOS | Public [TestFlight beta][testflight] |
| Android | Signed arm64 APK on every [release](https://github.com/outlmd/outl/releases), sideload only ([Android platform](android-platform.md)) |
| Sync | Peer to peer over iroh QUIC, end-to-end encrypted, the default. The `file` transport (iCloud Drive, Syncthing, a shared folder) is the opt-in alternative |
| Plugins | JavaScript, on the terminal, desktop and phone; which hooks each one honors is in [Plugins](plugins.md#capabilities-per-client) |

The CRDT algorithm is implemented and covered by the [test suite][tests].
Where it's headed: the [roadmap lives on the GitHub Project][roadmap].

[testflight]: https://testflight.apple.com/join/P2GdWAMd
[roadmap]: https://github.com/users/avelino/projects/2/views/1
[tests]: https://github.com/outlmd/outl/actions

## Get help

- **Something broke:** [Troubleshooting](troubleshooting.md), or run `outl doctor`.
- **A question, an idea, a show-and-tell:** [Discord](https://discord.gg/fDDXRtFhA6) or [GitHub Discussions](https://github.com/outlmd/outl/discussions).
- **A bug or a missing feature:** [open an issue](https://github.com/outlmd/outl/issues) with `outl --version`, the client, what you did and what you expected.

## Background reading

Long-form posts about the engineering behind outl, published on [avelino.run](https://avelino.run):

- **[File sync isn't trivial](https://avelino.run/file-sync-isnt-trivial/)**: the distributed-systems problem behind concurrent file moves, and what a formally-verified algorithm gives you that ad-hoc merge doesn't.
- **[From paper to outliner](https://avelino.run/from-paper-to-outliner/)**: the engineering between a CRDT proof and a shipped app: projections, reconciliation, transport edge cases, editor state.

## Contributing

The [README on GitHub](https://github.com/outlmd/outl) has the install bits and the dev workflow.
Before sending a PR, read [Contributing & code review](contributing.md); the rules of the game are written down so you know exactly what reviewers will look at.
Open issues to discuss design before sending big PRs; the sync algorithm in particular has a 100% coverage rule on its critical functions.

## License

[MIT](../LICENSE).
