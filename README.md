<p align="center">
  <img src="assets/logo.png" alt="outl" width="160" height="160" />
</p>

<h1 align="center">outl</h1>

<p align="center">
  Local-first outliner. Markdown is the source of truth. Sync that
  doesn't corrupt your tree when two devices edit offline.
</p>

Inspired by [Roam Research](https://roamresearch.com) and [Logseq](https://logseq.com).
Tree CRDT sync ([Kleppmann et al. 2022][paper]), per-device append-only op log, IDs in a sidecar so the `.md` you see is the `.md` you wrote.

- **Why outl?** → [outl.app/docs/why-outl.html](https://outl.app/docs/why-outl.html)
- **Sync, done right:** → [outl.app/docs/sync.html](https://outl.app/docs/sync.html)
- **CRDT walkthrough:** → [outl.app/docs/crdt.html](https://outl.app/docs/crdt.html)

[paper]: https://martin.kleppmann.com/papers/move-op.pdf

## Install

```bash
# macOS / Linux via Homebrew (beta channel — every push to main)
brew tap avelino/outl https://github.com/avelino/outl
brew trust avelino/outl # one-time, third-party tap
brew install outl-beta # TUI/CLI/MCP
brew install --cask outl-desktop-beta # GUI
```

iOS beta on TestFlight: [join here](https://testflight.apple.com/join/P2GdWAMd). Point the TUI at the same iCloud Drive container _(`<container>/Documents/`)_ and both clients share a workspace.

Android: a signed arm64 APK (`outl-android-arm64.apk`) rides every [release](https://github.com/avelino/outl/releases).
Sideload-only for now — not on Play, and each release is signed with a fresh key, so a new build means uninstalling the old one first ([#171](https://github.com/avelino/outl/issues/171)).

- **From source / channels:** → [getting started](https://outl.app/docs/getting-started.html), [homebrew](https://outl.app/docs/homebrew.html)

## Quick start

```bash
outl init ~/notes              # scaffold a workspace
outl --workspace ~/notes       # opens the TUI on today's journal
```

Press `?` for keymap, `:` for the command palette, `Ctrl+P` to fuzzy-jump.

- [Tutorial (15 min)](https://outl.app/docs/tutorial.html)
- [TUI manual](https://outl.app/docs/tui.html)
- [CLI reference](https://outl.app/docs/cli.html)
- [Markdown dialect](https://outl.app/docs/markdown-format.html)
- [Shortcuts](https://outl.app/docs/shortcuts.html)

## Coming from Logseq or Roam?

```bash
outl import logseq ~/path/to/logseq-graph ~/notes
outl import roam ~/Downloads/backup.json ~/notes
```

The importers (Roam, Logseq, Obsidian — or `auto` to detect) resolve `((uid))` block refs and `{{embed}}`s into real outl block references, keep folded state, and translate each dialect.
Everything touched is counted in the import report — run with `--dry-run` first to measure fidelity against your backup.
Anything unresolvable stays as `((unresolved:UID))` for manual triage.

## Contributing

- [Developer setup](https://outl.app/docs/development.html)
- [Contributing guide](https://outl.app/docs/contributing.html)
- [Architecture](https://outl.app/docs/architecture.html)
- [Roadmap](https://github.com/users/avelino/projects/2/views/1) — where the project is going

## Background reading

The engineering decisions behind outl on [avelino.run](https://avelino.run):

- **[File sync isn't trivial](https://avelino.run/file-sync-isnt-trivial/)** — why concurrent file moves are a distributed-systems problem that Dropbox and Google Drive still get wrong.
- **[From paper to outliner](https://avelino.run/from-paper-to-outliner/)** — the gap between "the CRDT converges" and "the app ships": projections, content-addressable reconciliation, surviving iCloud's lazy materialisation.

## License

[MIT](LICENSE).
