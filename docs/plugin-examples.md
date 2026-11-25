# Plugin examples

A gallery of small, self-contained example plugins, **one per capability**, so you can copy a working starting point instead of building from scratch.
Each lives in [`examples/`](https://github.com/outlmd/outl/tree/main/examples) and has its own page below with the code, the manifest and how to run it.
All twelve folders in `examples/` install cleanly with the CLI, and the ones with a plain command (`stats`, `greet`, `pick`, `pulse`, `todo-archive-done`) were run end to end.

New to plugins?
Read the [tutorial](plugin-tutorial.md) first, then the [API reference](plugin-api.md).

## By capability

| Capability | Example | What it does |
|---|---|---|
| `op-hook` | [Word Count](plugin-examples/word-count.md) | Notifies word-count milestones as you type |
| `slash-command` | [Workspace Stats](plugin-examples/workspace-stats.md) | Counts blocks / TODO / DONE / pages |
| `slash-command` | [Template Stamper](plugin-examples/template-stamper.md) | Stamps a structural template under the first block of a configured page (page only, no `examples/` folder) |
| `config-schema` | [Greeter](plugin-examples/greeter.md) | Greets you using a configurable name |
| `keybinding` | [Random Task](plugin-examples/random-task.md) | A chord that picks a random TODO to focus on |
| `toolbar-button` | [Page Pulse](plugin-examples/page-pulse.md) | A chrome button that shows page stats |
| `ui-render` | [Confetti](plugin-examples/confetti.md) | Throws confetti when a block is marked DONE |
| `content-transformer:text` | [Box](plugin-examples/box.md) | A ` ```box ` fence wraps text in an ASCII box |
| `content-transformer:rich` | [Bars](plugin-examples/bars.md) | A ` ```bars ` fence renders a mini bar chart |
| `network` | [Inspire](plugin-examples/inspire.md) | Fetches a quote from an API |
| `sync-transport` | [Echo Sync](plugin-examples/echo-sync.md) | A push/pull transport skeleton |
| _several at once_ | [TODO Archiver](plugin-examples/todo-archiver.md) | op-hook + slash-command + keybinding + config-schema in one plugin |
| _several at once_ | [Excalidraw](plugin-examples/excalidraw.md) | Draws an Excalidraw scene inline: rich transformer + network + secrets + storage |

## Real integrations (external plugins)

The examples above live in this repo so you can read them next to the source.
A production plugin lives in **its own repo** and depends on the published `@outl/plugin-sdk` from npm — that's the shape you ship.

| Plugin | Capabilities | What it shows |
|---|---|---|
| [outl-plugin-ouraring](https://github.com/avelino/outl-plugin-ouraring) | `secrets` + `network` + `config-schema` | Syncs Oura Ring health metrics into daily pages — the canonical template for an **authenticated external integration**: an API token in the OS keychain (`ctx.secrets.get`), plaintext config in the lockfile, a gated `ctx.net.fetch`, and `appendTree` to write the results in one turn. |

Building your own integration?
Start from that repo, and see [Plugin API → Secrets, the full flow](plugin-api.md#secrets--the-full-flow) for the token contract.

## Which one do I need

| I want to… | Start from |
|---|---|
| Add a command that reads the workspace | [Workspace Stats](plugin-examples/workspace-stats.md) |
| Add a command that writes (move, create, edit) | [TODO Archiver](plugin-examples/todo-archiver.md) |
| Add a setting the user can change | [Greeter](plugin-examples/greeter.md) |
| Add a keyboard chord | [Random Task](plugin-examples/random-task.md) |
| Add a button to the toolbar | [Page Pulse](plugin-examples/page-pulse.md) |
| React to every edit | [Word Count](plugin-examples/word-count.md) |
| Draw something when something happens | [Confetti](plugin-examples/confetti.md) |
| Render a code fence as text (works in the TUI) | [Box](plugin-examples/box.md) |
| Render a code fence as HTML (desktop and mobile) | [Bars](plugin-examples/bars.md) |
| Call a web API | [Inspire](plugin-examples/inspire.md), then [Excalidraw](plugin-examples/excalidraw.md) for secrets and a cache |
| Build a sync backend | [Echo Sync](plugin-examples/echo-sync.md) (no client drives it yet) |

## The shape of an example

Every example follows the same layout (the dev shape — see the [tutorial](plugin-tutorial.md)):

```
examples/<name>/
├── plugin.json        # manifest: capabilities, permissions, contributes
├── package.json       # build script (esbuild → index.js)
├── tsconfig.json
├── src/index.ts       # the plugin (imports @outl/plugin-sdk)
├── index.js           # the bundled output (what installs)
└── README.md
```

Install any of them into a workspace and run:

```sh
outl -w <workspace> plugin install ./examples/<name> --yes
outl -w <workspace> plugin list
```

The bundle ships in the repo, so you can install an example without a build step.
To change one, edit `src/index.ts` and rebuild (`bun run build` inside the example, after `bun install` at the repo root).

A run of the TODO Archiver against a page holding `DONE ship it` and `TODO later`:

```sh
outl -w <workspace> plugin run app.outl.examples.todo-archiver todo-archive-done
```

```text
Archived 1 block(s) to "archive"
Applied 2 change(s).
```

`Applied 2` is the page create and the move.
`DONE ship it` is now the only block in `archive`.

Every example's id is `app.outl.examples.<folder name>`.
Run `outl plugin list` to see the command ids it contributes.
