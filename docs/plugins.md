# Plugins

A plugin is a small piece of JavaScript that adds a command, a keyboard chord, a toolbar button, a code-block renderer or a reaction to your edits.
You install it into a workspace, approve what it may touch, and it works the same on the TUI, the desktop app, the mobile app and the CLI.

This page is for **using** plugins.
Writing one starts at the [Plugin tutorial](plugin-tutorial.md).

> **Why JavaScript, and why iOS decided the engine:** [RFC 0025](rfcs/0025-plugin-system.md).

## In one minute

```sh
outl -w ~/notes plugin install ./examples/workspace-stats --yes
outl -w ~/notes plugin list
outl -w ~/notes plugin run app.outl.examples.workspace-stats stats
```

```text
Installing Workspace Stats (app.outl.examples.workspace-stats) v1.0.0
Requests these permissions:
  - read-page
Installed app.outl.examples.workspace-stats v1.0.0.
app.outl.examples.workspace-stats  1.0.0  (enabled)
    /stats  Workspace statistics
📊 0 blocks · 0 TODO · 0 DONE · 2 pages
Applied 0 change(s).
```

Run from a checkout of the repo, `./examples/workspace-stats` ships with its bundle, so there is no build step.
In the TUI or the desktop app, the same command is `/stats`.
Drop `--yes` to see the permission prompt and answer it by hand.

## What a plugin can never do

- **Edit your `.md` files or the CRDT directly.**
  Every change a plugin makes becomes an op in the workspace log, stamped `plugin:<id>@<device>`, then your markdown is re-rendered from the log.
  You can tell which plugin wrote what, and undo it like any other edit.
- **Run code you did not install.**
  The bundle is hashed at install and re-checked on every load.
  If the file changes underneath (Finder, iCloud, a half-synced copy), the plugin refuses to load.
- **Reach the network without a grant.**
  Network access is per domain, approved by you, and re-checked on every redirect.
- **Read another plugin's secrets.**
  Secrets live in your OS keychain, namespaced by plugin id.
- **Hang the app.**
  A loop or recursion that runs away is stopped by the engine and shows up as an error.

Plugins are still code written by someone else.
Read the permission list before you approve it.
If a plugin asks for `network:` access and `secrets`, it can send a token you stored to the domain you approved, and nowhere else.

## Find a plugin

| Where | How |
|---|---|
| Registry, from a terminal | `outl plugin search [query]` matches name, id, description and keywords, and prints the `install:` line for each hit. |
| Marketplace, desktop and mobile | Open the plugin browser and search. One tap installs. |
| Repo examples | [One plugin per capability](plugin-examples.md), each a working starting point. |
| Anywhere on GitHub | `outl plugin install github:owner/repo`. |

The TUI has no marketplace screen (the [client parity table](client-parity.md) lists it as unsupported).
Install from a terminal instead: `outl plugin install <source>`.

The marketplace only lists plugins a maintainer reviewed in a pull request against the registry.
A plugin that is not listed (a private one, a fork, one you are still writing) installs with the CLI.

## Install

```sh
outl plugin install ./my-plugin                          # a local directory
outl plugin install github:owner/repo                    # newest semver tag
outl plugin install github:owner/repo#v1.2.0             # pinned tag
outl plugin install github:owner/repo/sub/dir#v1.2.0     # plugin in a subdirectory
```

A `github:` source is cloned at an immutable semver tag.
With no `#tag`, outl picks the newest published tag.
It never follows a branch like `main`, so installing the same source twice gives you the same code.

Install does five things, in this order:

1. Validates `plugin.json` (id shape, paths stay inside the plugin folder, every keybinding and toolbar button points at a declared command).
2. Prints the permissions the plugin asks for and waits for a yes.
   `--yes` answers for you, and is required when stdin is not a terminal.
3. Copies `plugin.json`, the bundle named by `main` and the `configSchema` file into `<workspace>/.outl/plugins/<id>/`.
   A README, a source map or anything else in the source folder is not copied.
4. Computes the sha256 of the bundle.
5. Writes the entry to `installed.json` (the [lockfile](#the-lockfile)): version, source, hash, approved permissions, `enabled: true`.

**Installing over an existing plugin resets it.**
The new entry replaces the old one, so your saved config goes back to empty and a disabled plugin turns back on.
Keys you stored in the keychain and `storage.json` are untouched.
Run `outl plugin config show <id>` first and write down your settings.
There is no `outl plugin update` yet.

> Install needs a workspace.
> In a folder that is not one, it prints the permission list and then fails with `NO_WORKSPACE: no outl workspace at <dir> — run `outl init <dir>` first`.

## Run a plugin's commands

A plugin contributes commands.
Each command has an **id** (`stats`) and a **title** (`Workspace statistics`).
The id is the same on every client.

| Client | How to run a command |
|---|---|
| CLI | `outl plugin run <plugin-id> <command-id>`. Prints notifications and `Applied N change(s).`, then re-renders every page's `.md`. |
| TUI | `/` in Normal mode opens the slash menu. Plugin commands sit next to the built-ins, described as `plugin · <plugin-id>`. Type to filter, `Enter` to run. |
| Desktop | Type `/` at the **start** of a block for the inline slash menu, or click the plugins button (`⧉`) for the plugin palette. A `/` in the middle of text is a path, never a command. |
| Mobile | Tap the plugin glyph in the header (aria label "Plugin commands") to open the plugin sheet, then tap a command. |

Three more ways a command can be reached, if the plugin declares them:

- **A chord.**
  `Ctrl+Shift+R` or a two-chord sequence like `Ctrl+G A`.
  On the TUI it fires from Normal mode only, never in the middle of an edit.
  On the desktop it fires anywhere, unless a built-in shortcut owns that chord.
  A plugin can never take over `Cmd+B`, `dd`, `Ctrl+P` and friends.
  Mobile has no keyboard, so chords do not apply there.
  A terminal usually cannot tell `Ctrl+Shift+R` from `Ctrl+R`, so on the TUI a two-chord sequence such as `Ctrl+G A` is the reliable spelling.
- **A toolbar button.**
  Desktop and mobile draw it in the chrome.
  The TUI has no toolbar, so that command shows up in the slash menu.
- **A reaction to your edits.**
  An `op-hook` plugin runs after each change, whether you typed it or a peer synced it.
  The TUI defers hooks while you are in Insert mode and runs them when you leave it.

A command that needs the plugin's output visible (a toast, an HTML overlay) shows it on the client's status line or as a toast.
HTML overlays and rich code blocks are drawn only by the desktop and mobile apps.

## Configure a plugin

Some plugins have settings (a page name, a number of days) and some need a secret (an API token).
Both are described by the plugin's `config.schema.json`, so every client shows the same fields.

Plain settings live in `installed.json`.
Secrets live in your OS keychain and never touch the workspace, so they do not sync and do not end up in a git backup of your notes.

```sh
outl plugin config show app.outl.examples.greeter
```

```text
Settings for app.outl.examples.greeter:
  name  [string]  (default "friend")
      Name the greeting addresses you by.

Edit: outl plugin config set app.outl.examples.greeter <key> <value>
      outl plugin secret set app.outl.examples.greeter <key>
```

```sh
outl plugin config set app.outl.examples.greeter name Ana     # Set app.outl.examples.greeter/name.
outl plugin secret set run.avelino.ouraring token             # hidden prompt
outl plugin secret set run.avelino.ouraring token --value ... # non-interactive; lands in shell history
outl plugin secret remove run.avelino.ouraring token          # alias: rm
```

The value is coerced to the field's schema type (`14` becomes an integer for an `integer` field, `true` a boolean).
A key the schema does not list is refused (`has no config field`), and a secret field is refused by `config set` (use `secret set`).
`secret set` needs a terminal for the hidden prompt, otherwise pass `--value`.

| Client | Where the settings form is |
|---|---|
| CLI | `outl plugin config` and `outl plugin secret`, above. |
| TUI | `/plugin-settings` (alias `/plugins`). `Up` / `Down` pick a field, `Enter` edits it (a boolean flips), `Enter` again saves, `Esc` cancels or closes. Shows "No configurable plugins installed" when no plugin has a schema. |
| Desktop | The settings form of each installed plugin in the plugin browser. |
| Mobile | The settings form of each installed plugin in the plugin browser. |

A secret field never shows its value on any client, only whether it is set.

## Turn off, turn on, remove

```sh
outl plugin list                  # every plugin, version, enabled or disabled, and its commands
outl plugin disable <id>          # keeps it installed, stops it from loading
outl plugin enable <id>
outl plugin remove <id>           # aliases: uninstall, rm
```

`remove` deletes `<workspace>/.outl/plugins/<id>/` and the lockfile entry (your saved config goes with it).
It does not delete the plugin's keychain secrets, so run `outl plugin secret remove <id> <key>` for each one you want gone.
Removing a plugin that is not installed prints `Plugin `<id>` is not installed.` and exits cleanly.

Everything a plugin wrote stays in your notes.
Removing the plugin does not undo its edits.
Its ops carry the `plugin:<id>@<device>` actor if you need to find them.

## Permissions

A plugin lists what it needs in `plugin.json`.
You approve the list once, at install, and the approved set is frozen in the lockfile.

| Permission | What it lets the plugin do | Enforced when the plugin runs |
|---|---|---|
| `read-page` | Read blocks and pages. | Shown at install. Not checked on reads today (see below). |
| `write-page` | Create, edit, move, delete blocks and pages. | **Yes.** A write without it is dropped and reported as an error. |
| `read-op-log` | Observe each applied op (`onOp`). | Shown at install. The hook runs when the plugin declares the `op-hook` capability. |
| `submit-op` | Companion to `write-page` for mutations. | Shown at install. The check is on `write-page`. |
| `storage:local` | A key/value store for this plugin on this device. It does not sync. | **Yes.** Without it `ctx.storage` throws. |
| `secrets` | Read the plugin's own keychain secrets. | **Yes.** Without it `ctx.secrets.get` throws. |
| `network:<domain>` | HTTP(S) to one domain. | **Yes.** |

Network grants are always scoped.
`network:api.openai.com` matches that host only.
`network:*.openai.com` matches any subdomain (`api.openai.com`) but not `openai.com` itself and not `evil-openai.com`.
A bare `network:*` is rejected when the manifest is parsed.

What the network gate does for you:

- Only `http` and `https`.
- A redirect to a host you did not approve is refused, and the refusal is logged with the plugin id (the plugin can hide the error from itself, not from the log).
- An `https` to `http` downgrade is refused.
- Custom headers, such as a token in `X-Api-Key`, are dropped when a redirect changes host, port or scheme.
- A request stops after 60 seconds at most, redirects included.
- A response body over the size cap is refused.

What it does not do: it matches host **names**.
A plugin author who controls DNS for an approved domain can point it at `127.0.0.1`.
Approve `network:` grants only for authors you would trust with a local service.

> **Read the read-side honestly.**
> Today `read-page` and `read-op-log` are listed and approved but not enforced at run time: a plugin without `read-page` can still query blocks, and `onOp` is gated by the `op-hook` capability, not by `read-op-log`.
> Treat a plugin as able to read your whole workspace if it asks for any read or write permission, and decide on that basis.
> The enforced writes, storage, secrets and network checks are the ones that stop it from changing or sending anything.

## Where plugins live

```
<workspace>/.outl/plugins/
├── installed.json          # the lockfile
├── <plugin-id>/
│   ├── plugin.json
│   ├── index.js            # the bundle named by `main`
│   ├── config.schema.json  # only if the plugin has settings
│   └── storage.json        # only if the plugin used ctx.storage
└── _dev/<name>/            # plugins under development, see below
```

Only build output lives here.
No `node_modules`, no source tree.
A plugin survives deleting `node_modules` in its repo.

### The lockfile

`installed.json` records, per plugin:

- `version` and `source` (`local:./path` or the `github:` ref).
- `bundleHash`: the sha256 of the bundle, checked on **every** load.
- `permissionsApproved`: the frozen set from install.
- `installedBy`: the actor of the device that installed it.
- `config`: your plaintext settings, `null` until you set one.
- `enabled`.

If the bundle on disk no longer matches `bundleHash`, that plugin is blocked and the others load normally:

```text
warning: plugin `app.outl.examples.workspace-stats` failed to load: bundle integrity check failed for app.outl.examples.workspace-stats: expected sha256:9b4c…, got sha256:6cf8…
```

To recover, `outl plugin remove <id>` and install it again from the source you trust.
Do not edit `bundleHash` by hand to silence the warning, that is the check doing its job.

## Troubleshooting

| What you see | Cause | Fix |
|---|---|---|
| `bundle integrity check failed for <id>` | `index.js` differs from the installed hash: an edit, a sync that did not finish, a disk problem. | Remove and install again. |
| `running <id>/<cmd>: invalid manifest: no such plugin `<id>`` | The id is wrong, **or** the plugin failed to load (look for the warning line above it), or it is disabled. | `outl plugin list`, fix the id, `outl plugin enable <id>`. |
| `Applied 0 change(s).` and nothing else | The command id does not exist (a typo is not an error), or the command only reads and shows nothing. | Compare against `outl plugin list`, which prints each `/command-id`. |
| The command is missing from `outl plugin list` and the slash menu | The plugin did not declare the `slash-command` capability, or the id in `contributes.commands` is not the one it registers. | A plugin author fix. Ask the author, or see the [tutorial](plugin-tutorial.md#step-10--debug-when-it-doesnt-work). |
| `permission denied` / a write does nothing | The plugin never asked for `write-page`, or you approved a smaller set earlier. | Remove and install again; the new prompt shows the full list. |
| `invalid manifest: invalid id` | The id is not lowercase reverse-DNS with at least two labels. | Author fix. |
| `invalid manifest: main must be a relative path inside the plugin` | A `main` or `configSchema` path with `..`, a leading `/` or `~`, or a `:`. | Author fix. |
| Install: `reading plugin.json from <source>` | The folder has no `plugin.json`, usually because you pointed at the repo root of a plugin whose output is in a subfolder. | Use `github:owner/repo/subdir`. |
| `not a TTY — pass --value` | `plugin secret set` without a terminal. | Pass `--value`, mind your shell history. |
| A chord does nothing | Another binding owns it (built-ins always win), the plugin is missing the `keybinding` capability, the chord is longer than two parts, or it does not parse. A bad chord is skipped without a message. | Pick a free chord such as `Ctrl+G A`. |
| A code fence shows raw source | No plugin claims that language on this client. The TUI never draws `rich` output. Your cursor is on the block (the TUI shows source while you edit it). | Move off the block, or use the desktop or mobile app. |
| The toolbar button is missing on the TUI | A terminal has no toolbar. | Use `/` and the command id. |

The plugin's `ctx.log` lines and any `console.log` go to the host log of each client.
Output a command wants you to see arrives as a toast or status line.

## Dev mode

A plugin folder dropped in `<workspace>/.outl/plugins/_dev/<name>/` loads straight from disk:

- No hash check, so rebuild and relaunch without reinstalling.
- Every permission the manifest asks for is granted, with no prompt.
- Not in the lockfile, so it has no saved config: `ctx.config.get()` returns `null` there.
  Write `cfg?.field ?? default` in the plugin.
- `outl plugin list` only prints plugins that have a lockfile entry, but a `_dev` plugin loads and its commands run (`outl plugin run <id> <command>` works, and its commands show under a lockfile entry with the same id).
  Disable and remove do not apply to it, delete the folder instead.
- If a lockfile entry with the same id exists, both load, so `plugin run` can hit the dev copy.
  Remove the installed one while you develop.

There is no file watcher and no "sandbox relaxed" banner yet.
After a rebuild, restart the client (or run the CLI command again).
Do not leave a `_dev` copy in a workspace you sync to other devices unless you want that code on them.

## Distribution (for authors)

### The registry

Discovery is a static index, `registry.json`, kept in [`outlmd/registry`](https://github.com/outlmd/registry) and served at `https://plugins.outl.app/registry.json`.
It lists each plugin's id, name, repo, versions, capabilities, permissions and description.
`outl plugin search` and the marketplace screens read it.

### Publish a plugin

1. **Host the built plugin in a public GitHub repo.**
   The default branch must contain the installed shape at the path you register: `plugin.json`, the bundled `index.js` and `config.schema.json` if you have one.
   A subdirectory works, so one repo can hold several plugins.
2. **Open a pull request** against `outlmd/registry` adding one entry:

   ```jsonc
   {
     "id": "dev.you.my-plugin",            // must equal plugin.json `id`
     "name": "My Plugin",
     "description": "One sentence on what it does.",
     "author": "your-handle",
     "repository": "github:you/my-plugin", // or github:you/repo/subdir
     "category": "productivity",
     "keywords": ["..."],
     "capabilities": ["slash-command"],     // mirror plugin.json
     "permissions": ["read-page"],          // mirror plugin.json, users see this ask
     "latest": "1.0.0",
     "versions": ["1.0.0"]
   }
   ```

   `id`, `capabilities` and `permissions` must match your manifest.
   CI validates the entry, and install re-validates the manifest anyway.
3. **Merge.** The registry build fetches your bundle and serves it at `https://plugins.outl.app/p/<id>/`.
4. Your plugin appears in `outl plugin search` and the in-app marketplace.

The full schema and how to pin a tag live in the registry repo's [README](https://github.com/outlmd/registry#adding-a-plugin).

### `.outlpkg` (roadmap)

A single-file `<id>-<version>.outlpkg` (tar and gzip, no source) is planned and not implemented.
Today you distribute a directory or a `github:` source.

## Capabilities per client

A **capability** is a thing a plugin plugs into.
Each client implements some of them.
The loader keeps the intersection: a capability your client cannot honor loads partially with a warning, and the rest of the plugin still runs.

| Capability | TUI | Desktop | Mobile | CLI |
|---|:---:|:---:|:---:|:---:|
| `slash-command` | yes | yes | yes | yes (`plugin run`) |
| `op-hook` | yes | yes | yes | yes |
| `config-schema` | yes (`/plugin-settings`) | yes | yes | yes (`plugin config`) |
| `keybinding` | yes, Normal mode | yes | not applicable | not applicable |
| `toolbar-button` | in the slash menu | button in the chrome | button in the header | not applicable |
| `content-transformer:text` | yes, inline | yes | yes | not applicable |
| `content-transformer:rich` | not drawn | yes, sandboxed iframe | yes, sandboxed iframe | not applicable |
| `ui-render` | not drawn | yes, overlay | yes, overlay | not applicable |
| `sync-transport` | core only | core only | core only | core only |

`sync-transport` works in the host (`ctx.sync.register` is live and convergence is tested), but no client calls it on a timer yet.
Treat it as roadmap when you pick a plugin.
`query-provider` appears in the manifest schema and is not wired to anything.

A query engine is not its own capability.
A plugin claims the ` ```query ` fence with a `content-transformer`, and can call `outl.query({ … })` for structured results (see [Query code blocks → Plugin SDK API](query.md#plugin-sdk-api-outlquery)).

Fence rendering order on the GUI clients: a plugin transformer for the language, then the built-in Pele diagram (only for `mermaid`, only when `[editor] diagrams` is `"pele"`), then highlighted source.
A plugin that claims `mermaid` replaces Pele for that language.

## See also

- [Plugin tutorial](plugin-tutorial.md): build one step by step.
- [Plugin API](plugin-api.md): manifest, host API, versioning.
- [Plugin examples](plugin-examples.md): one working plugin per capability.
- [Plugin architecture](plugin-architecture.md): how the runtime works.
- [CLI → plugin](cli.md#workspace--admin): every `outl plugin` subcommand.
- [`plugin-v1.json`](schemas/plugin-v1.json): the manifest JSON Schema.

---

## Contributor reference: client plugin surfaces

The next two sections are for people working on the clients, not for plugin users.
The headings below are linked from other docs, so they stay as they are.

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
- After a plugin mutation (`run.applied > 0`), the plugin thread re-projects **every** page's `.md` + sidecar via `outl_actions::apply_all_pages_md` before replying. One refused page does not skip later pages; the sweep returns every failure separately from the already-committed plugin mutation.
  A plugin can move blocks to any page — same rationale as the TUI's `reproject_after_plugin`.
  Each page uses the post-mutation ahead-of-log guard, so the sweep refuses rather than overwriting unlogged content.

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

A fenced block is rendered in this order, on both GUI clients:

1. A plugin content transformer whose `lang` matches the fence.
2. The built-in Pele diagram, only for a `mermaid` fence, and only when `[editor] diagrams` is `"pele"` (the default).
3. Highlighted source.

`"off"` skips step 2.
The TUI has no diagram renderer, so a mermaid fence stays source there unless a plugin transformer claims it.
A plugin that claims `mermaid` replaces Pele for that language.
The edit affordance on a Pele fence matches a transformed fence: Edit on the desktop, tap the diagram (not a link inside it) to edit on mobile.

A node opens a note when it carries the class `internal-link` (`class A internal-link`, or `A[Journal]:::internal-link`).
The click target is the note name, the node's text, not a path.
A relative `href` (`click A "pages/journal"`) is dropped.
`http`, `https` and `mailto` open outside the app.
The diagram has no zoom buttons.
A left-to-right flowchart wider than the page is drawn top-to-bottom.

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
  Called once per iteration at the **single post-mutation point** in `runtime/event_loop.rs` (after the mode key handler, before the next draw).
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

A plugin mutation lands in the op log via `outl-actions` but does **not** write `.md`, so `reproject_after_plugin` runs `outl_actions::apply_all_pages_md` (a plugin can touch any page, and every page uses the post-mutation ahead-of-log guard) then `load_current`.
If a plugin declares a capability the TUI lacks, the host filters it; `host.missing_capabilities(id)` lists the gap.
