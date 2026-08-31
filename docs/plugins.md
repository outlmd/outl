# Plugins

outl ships a JavaScript plugin system so the ceiling of what you can do isn't pinned to what the maintainers ship.
That's the lesson from Roam and Logseq: the moment a tool can't be extended, every unmet need becomes a fork request that never lands.

The bet is simple: **a plugin is written once and runs on every client that renders or edits** — TUI, desktop, mobile, and the CLI.
Plugins are plain JavaScript, the largest ecosystem and the lowest barrier to entry.

> **Why JavaScript, and why iOS is what decided the engine:** [RFC 0025](rfcs/0025-plugin-system.md).

## Pick your path

By what you're here to do:

- **Use plugins** — you're on the right page; keep reading ([install](#installing), [permissions](#permissions), [where they live](#where-plugins-live)).
- **Write one** — start with the hands-on [Plugin tutorial](plugin-tutorial.md), keep the [Plugin API](plugin-api.md) open while you work.
- **Copy a working starting point** — grab one of the [example plugins](plugin-examples.md), one per capability.
- **Understand the internals** — read the [Plugin architecture](plugin-architecture.md) (the Boa engine, the describe→apply model, the safety model).

## How it works, in one paragraph

A plugin never touches your `.md` files or the CRDT directly.
Every mutation it performs flows `JS → host API → outl-actions → Workspace::apply → op log`, and every op a plugin produces is stamped with `actor = "plugin:<id>@<device>"`.
The op log stays the source of truth, your markdown stays 100% clean, and the log doubles as an audit trail of exactly what each plugin did.

## Installing

Plugins install per workspace from a **local directory** or a **`github:` source**:

```sh
outl plugin install ./outl-todo-archiver               # local dir
outl plugin install github:outlmd/outl/examples/todo-archiver   # github
outl plugin install github:user/repo#v1.2.0            # pin a tag
```

A `github:owner/repo[/subdir][#tag]` source is cloned at an **immutable semver tag** — the newest published tag when none is pinned, never a mutable branch like `main`.
Either way, install validates `plugin.json`, copies the installed shape into the workspace, computes the bundle hash, shows you the permissions the plugin requests, and asks for approval before anything is written.
The resolved version, source, approved permissions, and bundle hash are recorded in the lockfile (see [Where plugins live](#where-plugins-live)).

> **Scaffolding a new plugin:** `outl plugin init <name>` writes a buildable starter project (manifest + `package.json` + `tsconfig` + `src/index.ts`); run `bun install && bun run build` inside it for an installable bundle.
> Today's CLI surface is `init`, `search`, `list`, `install`, `run`, `config`, `secret`, `enable`, `disable`, `remove` — see [CLI → plugin](cli.md#workspace--admin) for every flag.
> `outl plugin update` and `.outlpkg` packaging are still roadmap.

### Running a command

A plugin command runs headless through the CLI:

```sh
outl plugin run <id> <command>
```

In the interactive clients the same command shows up on each client's command surface — see below.

### Command discovery per client

A `slash-command` plugin contributes a command that the user can run; where it surfaces depends on the client:

| Client | How you run a plugin command |
|---|---|
| TUI | Press `/` (Normal mode) → the slash menu lists built-ins **and** plugin commands, keyed by the command **id** (`/stats`); type to filter, `Enter` to run. |
| Desktop | Two ways: the `⧉` button (bottom-left chrome) opens the plugin palette, **and** typing `/` at the start of a block opens an inline slash menu (Notion-style) — `/stats` ranks the `stats` command on top, `Enter` runs it. |
| Mobile | The plugin sheet (header) lists the commands; tap to run. |
| CLI | `outl plugin run <id> <command>` (headless). |

The command **id** is the canonical name across every surface (the CLI's `outl plugin run … stats`, the TUI/desktop `/stats`), so a plugin author picks one id and it reads the same everywhere.
The inline desktop `/` menu only triggers **block-initial** (a mid-text slash is a path or URL, never a command).

### Enabling, disabling, listing, removing

```sh
outl plugin list
outl plugin enable <id>
outl plugin disable <id>
outl plugin remove <id>     # aliases: uninstall, rm
```

Enabled/disabled state is the `enabled` flag in the lockfile (`installed.json`).
Disabling keeps the plugin installed but stops it from loading.
`remove` is the opposite of `install`: it deletes the plugin's directory under `.outl/plugins/<id>/` and drops its lockfile entry.

## Permissions

A plugin declares the permissions it needs in its manifest.
You approve them **once, on install**, and the approved set is frozen in the lockfile.
Every host call is gated against that set — a plugin that didn't get `write-page` cannot write, no matter what its code tries.

| Permission | Grants |
|---|---|
| `read-page` | Read page and block content. |
| `write-page` | Create or edit page and block content. |
| `read-op-log` | Observe ops as they're applied (the `onOp` hook). |
| `submit-op` | Submit mutations to the op log. |
| `storage:local` | Per-plugin local key/value storage (this device only — does **not** sync). |
| `secrets` | Read the plugin's own secret from the OS keychain (`ctx.secrets.get`); the value is set out-of-band and never touches the workspace on disk. |
| `network:<domain>` | Network access scoped to one domain. |

Network is always scoped to a domain.
`network:api.openai.com` (exact) and `network:*.openai.com` (leading-label wildcard) are valid; a bare `network:*` is **rejected** — a plugin can never request the whole internet.

> **storage:local does not converge.**
> Per-plugin KV storage is local to the device on the day-zero release, to keep the op log from inflating.
> If a plugin needs state that syncs across your devices, that state has to be modeled as an op (not supported yet) — it won't silently appear on your other machines.

## Where plugins live

Installed plugins sit inside the workspace, next to your notes:

```
<workspace>/.outl/plugins/<id>/
├── plugin.json
├── index.js            # the single bundled file
├── index.js.map        # optional, for better errors
├── config.schema.json
└── README.md
```

Only the **build output** lands here — no `node_modules`, no source tree.
The rule is hard: a plugin survives deleting `node_modules`.

### The lockfile

Each workspace keeps an `installed.json` lockfile recording, per plugin:

- `version` and `source` (the install source ref — a local path today, an immutable `github:…#vX.Y.Z` tag once that source lands)
- `bundleHash` — sha256 of `index.js`, **revalidated on every load**
- `permissionsApproved` — the frozen approved set
- `installedBy` — the device that installed it
- `config` — your settings, stored outside the bundle so they survive a reinstall
- `enabled`

The `bundleHash` is the integrity check.
If `index.js` ever differs from the recorded hash — an out-of-band edit through iCloud or Finder, a half-finished sync — the load is **blocked** rather than silently running modified code.
The installed version never changes underneath you.

`installedBy` records the device that installed the plugin, so a synced workspace can tell "approved here" from "approved on another device".
(The cross-device re-confirm prompt that uses it is roadmap; today the hash check is the active gate.)

## Dev mode

While building a plugin, drop it in `.outl/plugins/_dev/<name>/` inside the workspace instead of installing it:

- **No bundle-hash check** — it loads straight from the directory, so you can rebuild and reload without reinstalling.
- **Permissions are implicit** — every permission the manifest declares is granted, no approval prompt.
- **Never recorded in the lockfile** and excluded from sync, so dev iterations don't leak to your other devices.

This is for authoring only — a `_dev` plugin runs with a relaxed sandbox.
A hot-reload-on-save watcher and an in-client "sandbox relaxed" banner are roadmap niceties; the load behavior above is what ships today.
See [Plugin API → Anatomy](plugin-api.md#anatomy-of-a-plugin) for the full dev layout.

## Distribution

### The registry

Discovery is a static index — `registry.json` — versioned in the [`outlmd/registry`](https://github.com/outlmd/registry) repo and served at **`https://plugins.outl.app/registry.json`** (Netlify, static, with CORS so any client can fetch it).
No server, no infrastructure.
It lists each plugin's id, name, `github:` repo, published versions, capabilities, permissions, and description — what powers the discovery list and search.
In-client discovery (`outl plugin search` + a browse/install screen in the desktop & mobile apps) reads this index.
A hosted registry (`registry.outl.app`) with full-text search and install counts is deferred until volume justifies it.

### Publishing your plugin (registering it)

Listing your plugin in the registry is what makes it show up in the in-app marketplace (the desktop/mobile browse-and-install screen) and in `outl plugin search`.
The registry stores **only metadata** — your code stays in your repo; the bundle host (`plugins.outl.app`) re-downloads it from there at build time.

Four steps:

1. **Host the built plugin in a public GitHub repo.**
   The repo's default branch must contain the **installed shape** at the path you'll point the registry at: `plugin.json`, the bundled `index.js` (run `bun run build`), and `config.schema.json` if you have one.
   Either at the repo root, or in a subdirectory (a monorepo of plugins works — point at `owner/repo/path/to/plugin`).

2. **Open a PR against [`outlmd/registry`](https://github.com/outlmd/registry)** adding one entry to `registry.json`:

   ```jsonc
   {
     "id": "dev.you.my-plugin",            // MUST equal your plugin.json `id`
     "name": "My Plugin",
     "description": "One sentence on what it does.",
     "author": "your-handle",
     "repository": "github:you/my-plugin", // or github:you/repo/subdir
     "category": "productivity",
     "keywords": ["..."],
     "capabilities": ["slash-command"],     // mirror plugin.json
     "permissions": ["read-page"],          // mirror plugin.json — users see the ask
     "latest": "1.0.0",
     "versions": ["1.0.0"]
   }
   ```

   The `id`, `capabilities`, and `permissions` **must** match your `plugin.json` (CI validates the entry against the schema; the install re-validates the manifest anyway).

3. **Merge → the Netlify build re-fetches your bundle** from the repo and serves it at `https://plugins.outl.app/p/<id>/`.

4. Your plugin now appears in **`outl plugin search`** and the **in-app marketplace**, installable with one tap.

> **Official vs. unofficial.**
> The in-app marketplace only installs plugins **listed in the registry** (so a tap-to-install is always something a human reviewed in the PR).
> A plugin that isn't listed yet — yours mid-development, a private one, a fork — installs via the CLI instead: `outl plugin install github:you/repo` or `outl plugin install ./dir`.
> Full reference for publishing (the schema, the build, pinning a tag) lives in the registry repo's [README](https://github.com/outlmd/registry#adding-a-plugin).

### `.outlpkg` (roadmap)

A `.outlpkg` will be the installed shape of a plugin — manifest, bundle, and assets, **no source** — packed as tar+gzip, named `<id>-<version>.outlpkg`, with its own extension (not `.zip`) so the OS can associate it with outl.
It is **not implemented yet**: today, install is from a local directory (and, once wired, a `github:` source).

## Capabilities per client

A capability is something a plugin plugs into; each client implements a subset.
The loader **intersects** the two — a capability your current client can't honor loads partially with a warning, never a crash.
The plugin still runs for everything else.

| Capability | TUI | Desktop | Mobile | CLI |
|---|:---:|:---:|:---:|:---:|
| `op-hook` | ✅ | ✅ | ✅ | ✅ |
| `slash-command` | ✅ | ✅ | ✅ | ✅ |
| `config-schema` (read) | ◑ | ◑ | ◑ | ◑ |
| `keybinding` | ✅ | ✅ | — | — |
| `toolbar-button` | ✅ (slash menu) | ✅ | ✅ | — |
| `content-transformer:text` | ✅ | ✅ | ✅ | — |
| `content-transformer:rich` | — | ✅ | ✅ | — |
| `sync-transport` | core only — client polling is roadmap | | | |

✅ implemented · ◑ partial · 🔜 planned (post day-zero) · — not applicable to this client.

`op-hook` and `slash-command` run identically on every client (the CLI exposes commands through `outl plugin run`).
**`config-schema` is live**: a plugin reads its config with `ctx.config.get()` (from the lockfile) and its secrets with `ctx.secrets.get()` (from the OS keychain, for fields marked `x-outl-secret`). Every client edits both — `outl plugin config` / `outl plugin secret` on the CLI, a settings form in the desktop / mobile plugin browser, and the TUI `plugin-settings` overlay — with values coerced to the field's schema type. The schema type is coerced but not otherwise validated on the stored value.
**`keybinding` is live on the TUI and the desktop.**
A `contributes.keybindings` chord fires the bound command — on the TUI from Normal mode (single- and two-chord sequences), on the desktop wherever a native binding doesn't claim it first.
Use a free chord like `Ctrl+G` or a two-chord sequence such as `Ctrl+G A`.
Mobile has no keyboard, so it doesn't apply there.
**`toolbar-button` is live**: desktop and mobile render a button in the chrome for the plugin's command, and the TUI surfaces that command in its slash menu (a terminal has no chrome bar).
**`content-transformer` is live** today: `ctx.content.register(lang, fn)` renders a fenced block — `:text` on every read surface (inline in the TUI), `:rich` as HTML in a sandboxed iframe on the GUIs (the TUI drops it).
A plugin that wants to be a query engine registers a transformer for the `query` fence; plugins can also call `outl.query({ … })` from JS code blocks to get structured results (see [Query code blocks → Plugin SDK API](query.md#plugin-sdk-api-outlquery)).
**`sync-transport` is core-ready**: `ctx.sync.register({ push, pull })` works and convergence is tested, but no client polls the transport on a timer yet — that wiring is roadmap.
The CLI is headless, so anything visual or chord-driven (`keybinding`, `toolbar-button`, `content-transformer:*`) doesn't apply to it.

## Permissions reference

| Wire string | Permission |
|---|---|
| `read-page` | Read page/block content |
| `write-page` | Create/edit page/block content |
| `read-op-log` | Observe applied ops |
| `submit-op` | Submit ops to the log |
| `storage:local` | Per-plugin local KV (no sync) |
| `secrets` | Read the plugin's own keychain secret (`ctx.secrets.get`) |
| `network:<domain>` | Network to one domain (`network:*` rejected) |

## See also

- [Plugin tutorial](plugin-tutorial.md) — build a plugin step by step.
- [Plugin API](plugin-api.md) — the authoring reference: manifest, host API, `definePlugin`, versioning.
- [Plugin architecture](plugin-architecture.md) — how the runtime works under the hood.
- [`plugin-v1.json`](schemas/plugin-v1.json) — JSON Schema for `plugin.json`.
- [CLI](cli.md) — the `outl plugin` subcommands.

---

## Desktop plugin surface

Moved here from `crates/outl-desktop/CLAUDE.md` (issue #216). How the desktop renders and runs plugins is plugin-system documentation; it was duplicated context on every desktop task.

JS plugins (`outl_plugins::PluginHost`) run on the desktop, but the host embeds a Boa `Context` that is **`!Send`**, so it can never live in the `Send + Sync` `AppState`.
The host therefore runs on a **dedicated plugin thread** (`src-tauri/src/plugin_service.rs`); `AppState` holds only a `PluginService` (a `Send + Sync` clone of a `std::sync::mpsc::Sender<PluginRequest>`).

Design:

- `spawn_plugin_service(workspace, storage_root, hlc)` (the desktop shim over `outl_tauri_shared::PluginService::spawn`, called once in `lib.rs::setup` after `open_today`/opener wiring) starts the thread.
  It is handed **clones of the same `Arc<Mutex<Option<Workspace>>>` and `Arc<Mutex<Option<PathBuf>>>` every Tauri command locks**, plus the per-device `HlcGenerator`.
  The `Workspace` is `Send`; the Boa `Context` never crosses a thread boundary.
- The thread owns the `PluginHost`.
  It loads plugins from `<root>/.outl/plugins/` lazily on the **first request after the workspace opens** (`ensure_loaded`), then `mark_synced` so pre-existing ops don't fire `onOp` hooks at boot.
  A workspace **swap** (different `storage_root`) rebuilds the host against the new root.
- Each request (`ListCommands` / `RunCommand` / `SyncHooks`) carries a one-shot `std::sync::mpsc::Sender` reply channel.
  The Tauri command sends the request, then **blocks on `recv()` with the workspace `Mutex` released** (never held across the reply) — the plugin thread is the one that locks the workspace to run the host.
  No `.await` ever holds the lock.
- After a plugin mutation (`run.applied > 0`), the plugin thread re-projects **every** page's `.md` via `outl_actions::apply_all_pages_md` before replying.
  A plugin can move blocks to any page — same rationale as the TUI's `reproject_after_plugin`.

Capabilities honored: `slash-command` + `op-hook` + `ui-render` + `keybinding` + `toolbar-button`.
The host filters `keybinding` / `toolbar-button` by declared capability **before** `keybindings("desktop")` / `toolbar_buttons("desktop")` return anything,
so both must be in `client_capabilities()` or the desktop sees an empty list.

Tauri commands live in `commands/plugin.rs`: `plugin_list`, `plugin_run`, `plugin_sync_hooks`, `plugin_keybindings`, `plugin_toolbar`, `plugin_transformers`, `plugin_transform`.
Return types and per-command behaviour (which ones re-project, which are read-only, what `view` / `views` carry): [`docs/plugin-architecture.md`](plugin-architecture.md#client-tauri-command-surface-desktop--mobile).

### `keybinding` + `toolbar-button` contributions

`lib/shortcuts.ts` loads `plugin_keybindings()` per `installShortcuts` (re-fetched on workspace swap, **not** module-cached) and folds the chords into the `keydown` dispatcher as a **Global overlay**.
The DTO's `chord` / `mode` serialize identically to the `outl-shortcuts` catalog, so the dispatcher reuses its `Chord` / `seqEq` machinery unchanged.
**Native always wins:** a plugin chord fires only after the native catalog matched nothing (match *and* prefix) and no native binding owns that chord in *any* mode (`nativeOwnsChord`) — a plugin can't shadow `Cmd+B` / `Cmd+P`.
`components/ChromeToggleBar.tsx` loads `plugin_toolbar()` on mount and renders one momentary button per entry in the native cluster (glyph = `icon`, tooltip = `title`, click = `plugin_run`).
Both paths run a command like the palette does: status-line output, re-render from `reply.view`, `playPluginViews(reply.views)`.

Op-hooks fire `pluginSyncHooks` at **two post-mutation points**: `OutlineView`'s `onCommit` (after an edit) and the `ToggleTodo` handler (`Cmd+T`).
`sync_hooks` dispatches **every** op since the host's last sweep, so one call also catches up structural ops (indent / move / delete) — mirrors the TUI's once-per-tick sweep.
Best-effort: a host with no op-hook plugins is a cheap no-op.
Both fire **fire-and-forget** (no `await`) — a slow plugin can't block the commit or next keystroke.

### `ui-render` overlays (sandboxed iframe)

A `ui-render` plugin emits HTML/JS via `ctx.ui.render(html)`.
The core gates these on the capability and surfaces them on `PluginRun.views`, propagated as `PluginRunReply.views` / `PluginSyncHooksReply.views`.
The desktop plays each as an **ephemeral, fully sandboxed `<iframe>` overlay**:

- `lib/plugin-views.ts` owns a Solid signal queue (`playPluginViews` enqueues, `dismissPluginView` pops).
- `components/PluginEffectLayer.tsx` (in `AppShell`) renders one fullscreen, click-through, `z-index: 9999` iframe per entry, auto-removed after 6s; multiple stack.
- **Security (load-bearing — never weaken):** the iframe is `sandbox="allow-scripts"` **without** `allow-same-origin`.
  The plugin JS runs in a null origin — no app DOM, cookies, `localStorage`, or credentialed fetch.
  HTML enters via `srcdoc`, never `innerHTML` on the host document.
  This is untrusted third-party code; the isolation is the whole point.

Played from three call sites: `PluginPalette` (after `pluginRun`), `OutlineView.onCommit`, and the `ToggleTodo` handler (after `pluginSyncHooks`).
The confetti example (`op-hook` + `ui-render`) rides this: mark a block DONE → `sync_hooks` → `onOp` emits confetti HTML → overlay.

Frontend pieces: plugin DTOs + wrappers from `@outl/shared/api` (`lib/api.ts` keeps only `pluginKeybindings`); `lib/plugin-views.ts` + `components/PluginEffectLayer.tsx` (overlay queue).
The `⧉` button in `ChromeToggleBar` toggles `appState.pluginsOpen`; `components/PluginPalette.tsx` lists + runs commands.

### Content transformers (inline code-fence rendering)

A plugin can declare a transformer for a code-fence language (`mermaid`, …); matching fences render through it in `CodeFenceView` (`components/BlockRow.tsx`).
Registry + cache glue: `@outl/shared/plugins/transformer-registry` (shared with mobile); keeps `BlockRow` a renderer.
It owns a `lang → PluginTransformer` registry (Solid signal via `loadTransformers`, re-run on `workspace-ready`; a boot fetch can be empty since plugins load lazily).
A `(blockId, body)` result cache (`runTransform`) re-runs plugin JS only when the body changes.
`kind: "text"` renders as plain whitespace-preserving text (no client-side markdown parse — a transformer wanting formatting emits `rich`).
`kind: "rich"` renders the HTML in an **inline** `<iframe>` (`RichFenceFrame`), sized via an optional `parent.postMessage({outlHeight})` handshake.
**Security (never weaken):** that iframe is `sandbox="allow-scripts"` **without** `allow-same-origin`, HTML via `srcdoc` — same isolation as the `ui-render` overlay, only inline + persistent instead of fullscreen + ephemeral.
`content-transformer:text` / `:rich` are in `client_capabilities()` (the host gates transformers by capability before listing them).

---

## TUI plugin surface

Moved here from `crates/outl-tui/CLAUDE.md` (issue #216), for the same reason as the desktop section above.

JS plugins are loaded at boot from `<root>/.outl/plugins/` into an `outl_plugins::PluginHost` held directly in `App.plugin_host` (`Option`, single-threaded — no `Arc`/`Mutex`, the Boa context is `!Send`).
Boot / slash / op-hook / content-transform wiring lives in `actions/plugins.rs`; keybinding dispatch lives in `input/plugin_chord.rs`.
The five touch points are:

- **Boot** (`App::load_plugins`, called at the end of `App::new`).
  Declares the client capabilities the TUI honors (`slash-command`, `op-hook`, `keybinding`, `content-transformer:text`, `toolbar-button`), runs `load_installed`, then `mark_synced` so pre-existing ops don't fire hooks on startup.
  A `toolbar-button` has no chrome bar in a terminal, so its command is surfaced in the **slash menu** instead (deduped against `slash-command` entries) — a runnable command is never dropped just because its only affordance was a GUI button.
  `ctx.net`, `ctx.storage`, and the gas limits are host-level (the engine), so they work in the TUI with no per-capability wiring — only HTML surfaces (`ui-render`, `content-transformer:rich`) stay undeclared, since a terminal can't draw them.
  Best-effort: a load failure toasts a warning and the TUI runs normally; a workspace with no plugins is unchanged.
  **`content-transformer:rich` is deliberately *not* declared** — `rich` output is HTML for a GUI iframe, meaningless in a terminal; the host filters those out of `host.transformers()` automatically.
- **Slash commands** (`App::slash_candidates` in `actions/overlay.rs`).
  The slash menu concatenates `host.commands()` onto the built-in registry list; each plugin command carries a `SlashOrigin::Plugin { plugin_id, command_id }` tag (vs `SlashOrigin::Builtin`).
  `accept_slash` routes a plugin pick to `App::run_plugin_command`, which surfaces `notify`/error output as toasts and re-projects if it mutated.
- **Keybindings** (`input/plugin_chord.rs::try_plugin_binding`, called first inside `handle_normal_key`).
  A plugin's `contributes.keybindings[].key` is parsed by `outl-plugins` into an `outl_shortcuts::ChordSequence`; `input/chord_adapter.rs` maps the live `crossterm::KeyEvent` into the same `outl_shortcuts::Chord` so we can compare them.
  A matching single-chord binding runs `App::run_plugin_command` immediately.
  A two-chord binding (`Ctrl+G A`) buffers the first chord in `App::pending_plugin_chord` (a **separate** field from the native `pending_chord` vim accumulator so the two never interfere) and fires on the second key.
  **Plugin chords are scoped to Normal mode** — they're `Mode::Global` in the catalog, but the TUI deliberately won't steal keys mid-edit.
  They **never shadow a native action**: `native_normal_chord` mirrors what `handle_normal_key` consumes, so a plugin can't rebind `j`, `dd`, `Ctrl+T`, `Ctrl+P`, etc. (use a free chord like `Ctrl+G` or a two-chord sequence).
  No host / no bindings / a key with no `Chord` form all short-circuit to native handling.
- **Op hooks** (`App::run_plugin_op_hooks`).
  Called once per iteration at the **single post-mutation point** in `runtime.rs`'s event loop (after the mode key handler, before the next draw).
  Deferred while in `Mode::Insert` (same reason as `pending_reload`: a hook-driven `load_current` would clobber the in-flight buffer; the edit isn't in the op log until commit anyway).
- **Content transformers** (`App::recompute_transforms`).
  **Pre-compute, not render-time.**
  When a block's text is a single closed code fence (`` ```<lang> `` … `` ``` ``) whose language a loaded `text` transformer claims, its body runs through `host.transform_block` *at load time*.
  The result is cached in `App::transform_cache`, keyed by `NodeId`.
  The render walk (`view/outline.rs`) only has `&App`, and `transform_block` is `&mut self` (it runs Boa) — so the transform **cannot** happen during render.
  It's done in `recompute_transforms`, called from `load_current_no_autorun` (every reparse), after `load_plugins` at boot, and on the reproject paths (plugin + peer mutations).
  The render path is then a pure `HashMap` lookup: a read-only block with a cache hit renders the transformed text/markdown (`RenderMode::Transformed`) in place of the raw fence; the bullet stays.
  **A block under the cursor (Insert / Normal-selected) always renders the raw fence source** so the user edits what they see — the cursor cases win over a cache hit.
  Lang match: the fence's raw info-string first (custom langs like `mermaid`), then the canonical alias via `outl_md::lang::canonical` (so a transformer registered as `rust` fires on `` ```rs ``).
  Best-effort: a plugin error or `Ok(None)` (declined) leaves the block to render as a raw fence — never crashes.

A plugin mutation lands in the op log via `outl-actions` but does **not** write `.md`, so `reproject_after_plugin` runs `outl_actions::apply_all_pages_md` (a plugin can touch any page) then `load_current`.
If a plugin declares a capability the TUI lacks, the host filters it; `host.missing_capabilities(id)` lists the gap.
