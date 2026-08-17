# Summary

* [Welcome](README.md)
* [Getting started](getting-started.md)
* [Homebrew tap](homebrew.md)
* [Your first week with outl](tutorial.md)
* [Why outl](why-outl.md)

## Sync — done right

* [The problem with how the others do it](sync.md)
* [Relay & NAT traversal](relay.md)
* [iroh transport internals](iroh-internals.md)
* [Tree CRDT walkthrough](crdt.md)

## Editor

* [TUI manual](tui.md)
* [Paste](paste.md)
* [Theming](theming.md)
* [Configuration](config.md)
* [Shortcuts](shortcuts.md)
* [Reminders](reminders.md)
* [Deep links (`outl://`)](deep-links.md)

## Integrations

* [CLI](cli.md)
* [outl doctor](doctor.md)
* [Embedding outl as a Rust library](embedding.md)
* [MCP](mcp.md)
* [MCP recipes (skills & commands)](mcp-recipes.md)
* [Plugins](plugins.md)
* [Build your first plugin](plugin-tutorial.md)
* [Plugin API](plugin-api.md)
* [Plugin architecture](plugin-architecture.md)
* [Plugin examples](plugin-examples.md)

## Format

* [Markdown dialect](markdown-format.md)
* [Query code blocks](query.md)
* [Templates](templates.md)
* [Workspace layout](concepts.md)

## Under the hood

* [Architecture](architecture.md)
* [Storage trait](storage.md)
* [Shared primitives catalog](shared-primitives.md)
  * [Core state, sync, and durability](primitives-core.md)
  * [Markdown pipeline](primitives-markdown.md)
  * [Editing actions and client features](primitives-actions.md)

## Project

* [Development guide](development.md)
* [iOS platform integration](ios-platform.md)
* [Android platform integration](android-platform.md)
* [Contributing & code review](contributing.md)

## RFCs

* [What an RFC is and how one lands](rfcs/README.md)
  * [Template](rfcs/0000-template.md)
  * [0002 — Every GUI client is Tauri 2 over one Rust surface](rfcs/0002-tauri-for-every-gui-client.md)
  * [0008 — What it costs to add a token to the outl dialect](rfcs/0008-markdown-dialect-and-sidecar-tokens.md)
  * [0025 — iOS bans JIT, so the plugin runtime is an interpreter](rfcs/0025-plugin-system.md)
  * [0038 — iroh is the default transport, and a workspace is an id the joiner adopts](rfcs/0038-sync-transport-and-workspace-identity.md)
  * [0044 — Copy-out and paste-in are one pair](rfcs/0044-clipboard-and-paste.md)
  * [0070 — One catalog owns every chord, and the desktop has no character cursor](rfcs/0070-keybinding-ownership-and-vim-parity.md)
  * [0107 — A page has three identities: slug, title, and the date that decides both](rfcs/0107-page-identity.md)
  * [0128 — Boot and memory at scale](rfcs/0128-boot-and-memory-at-scale.md)
  * [0129 — An acknowledged op must survive the crash, the reader, and the rebuild](rfcs/0129-op-log-durability.md)
  * [0137 — Storage scale: constant RSS, then constant boot/sync](rfcs/0137-storage-scale.md)
  * [0139 — A line-oriented query DSL in a code fence, not datalog](rfcs/0139-query-language.md)
  * [0146 — A template is a page with a property, not a new op](rfcs/0146-template-engine.md)
  * [0155 — A paired peer is not a trusted peer](rfcs/0155-peer-trust.md)
  * [0169 — Backlinks: one definition of a mention, one index, four clients](rfcs/0169-backlinks.md)
  * [0202 — Asset bytes are content-addressed blobs, deliberately outside the op log](rfcs/0202-file-assets.md)
  * [0210 — A sidecar hash match is not evidence the `.md` came from the op log](rfcs/0210-md-content-outside-op-log.md)
  * [0211 — State that leaves a boundary arrives somewhere with different rules](rfcs/0211-state-that-leaves-a-boundary.md)
