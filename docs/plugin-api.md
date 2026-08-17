# Plugin API

This is the **authoring** reference: the manifest, the host API your code talks to, and the versioning contract.
If you just want to install and run plugins, read [Plugins](plugins.md) instead.

The single biggest predictor of a plugin ecosystem's survival is a stable, versioned API surface.
So this document leads with a changelog and treats every entry as a promise.

---

## API changelog

### API 1.0

The day-zero surface.

- **Capabilities (live):** `op-hook`, `slash-command`, read-only `config-schema`, `keybinding`, `toolbar-button`, `ui-render`, and `content-transformer:text` / `:rich`.
  `keybinding` fires the bound command from a chord on the **TUI** (Normal mode, single + two-chord, never overriding a native binding) and the **desktop** (a native binding always wins); mobile has no keyboard.
  `toolbar-button` renders a chrome button on **desktop** and **mobile**, and surfaces the command in the **TUI** slash menu (a terminal has no chrome bar).
  `ui-render` and `content-transformer:rich` run author-written HTML/JS in a sandboxed iframe on the **GUI clients** (desktop + mobile); the TUI/CLI drop HTML.
  `content-transformer:text` renders on every read surface (inline in the TUI).
  `sync-transport` is **core-ready** (the host serializes and applies ops through a registered `{push, pull}`) but no client polls it yet — see [Roadmap](#roadmap-not-yet-available).
- **Host namespaces (live):** `ctx.ops`, `ctx.blocks`, `ctx.page`, `ctx.commands`, `ctx.config`, `ctx.content`, `ctx.storage`, `ctx.secrets`, `ctx.net`, `ctx.sync`, `ctx.log`, `ctx.ui`. (`ctx.sync.register` is live; the client polling that drives it is roadmap.)
  `ctx.blocks.appendTree` / `ctx.page.appendTree` build a nested tree in one turn (the describe→apply escape hatch for seeding fresh content), and `ctx.blocks.query` results carry `parent` for reconstructing hierarchy.
- **Permissions:** `read-page`, `write-page`, `read-op-log`, `submit-op`, `storage:local`, `secrets`, `network:<domain>`.
- **Entry contract:** `definePlugin({ activate(ctx), deactivate?() })`.

Anything not listed here is **not** part of API 1.0, even if a capability string for it exists in the manifest schema (see [Versioning](#versioning)).

> **The SDK types describe the API 1.0 *target*; the runtime ships a subset.**
> `@outl/plugin-sdk` types every read/write as `async` / `Promise`.
> The runtime today is **synchronous** (describe → apply: reads return from a turn snapshot, writes are buffered and applied when your handler returns — see [Plugin architecture](plugin-architecture.md#execution-model-describe--apply)).
> Write against the typed contract; the [Host API](#host-api--plugincontext) table below is the authority on what actually runs.

> **Gas: the engine can't be wedged by a runaway plugin.**
> Boa runs under `RuntimeLimits` — a loop-iteration cap (~20M), a recursion cap (~2000), and a stack-depth cap.
> An infinite loop or unbounded recursion surfaces as a JS error, not a hung thread.
> This is cooperative gas against a misbehaving plugin, not a wall-clock timeout.

---

## Anatomy of a plugin

A plugin has two shapes: the **dev** layout (your repo) and the **installed** layout (what lands in a workspace).

### Dev layout (your repo)

```
outl-todo-archiver/
├── plugin.json             # contract with the host (manifest)
├── package.json            # build deps + SDK (NOT shipped)
├── tsconfig.json
├── config.schema.json      # JSON Schema for user config
├── src/
│   ├── index.ts            # entry: calls definePlugin(...)
│   ├── commands/
│   │   └── archive-done.ts
│   └── hooks/
│       └── on-op.ts
├── README.md
└── LICENSE
```

You build with `esbuild` / `tsup` into **one bundled `index.js`** — no `node_modules`, no runtime resolution.

### Installed layout

```
.outl/plugins/com.avelino.todo-archiver/
├── plugin.json
├── index.js                # bundled, single JS file
├── index.js.map            # optional, for errors
├── config.schema.json
└── README.md
```

Only the build output ships.
The rule is hard: a plugin survives deleting `node_modules`.

Dev mode lives in `_dev/<name>/` — hot reload, implicit permissions, a "sandbox relaxed" banner, and excluded from sync.

Scaffolding all of the above is one command:

```sh
outl plugin init
```

It generates the manifest, `tsconfig`, the SDK wiring, an example `src/index.ts`, and the bundler config.

---

## The manifest — `plugin.json`

A single JSON file at the plugin root.
It's validated against [`plugin-v1.json`](schemas/plugin-v1.json) at install and on every load; point your editor at that `$id` for autocomplete.

| Field | Required | Description |
|---|:---:|---|
| `id` | ✅ | Reverse-DNS identity (`com.avelino.todo-archiver`). Never changes across versions. Used as the install directory and the op-log actor stamp `plugin:<id>@<device>`. |
| `name` | ✅ | Human-readable display name. |
| `version` | ✅ | Plugin version (semver). Resolved from the install tag, frozen in the lockfile. |
| `api` | ✅ | Plugin API range this plugin targets, e.g. `^1.0`. Matched against the host's plugin API, **not** the binary version. |
| `main` | ✅ | Path to the bundled entry file, e.g. `index.js`. |
| `engines.outl` | — | Minimum binary version (semver range), e.g. `>=0.8.0`. Independent of `api`. |
| `capabilities[]` | — | What the plugin plugs into (see below). |
| `permissions[]` | — | What it requests against the host (see below). |
| `contributes.commands[]` | — | `{ id, title, description? }` — surfaced in the slash menu / palette. |
| `contributes.keybindings[]` | — | `{ command, key, when? }` — default chord bindings for those commands. |
| `contributes.configSchema` | — | Path to a JSON Schema file (`config.schema.json`) for the user-editable config form. |
| `metadata` | — | `author`, `license`, `repo`, `funding`, `locales`, `category`, `description`. Descriptive only. |

### `id` — reverse-DNS

The `id` is identity, not a label.
It's the directory name under `.outl/plugins/`, and it's what stamps the op log, so it must be stable forever and unique across the ecosystem.
Pattern: lowercase reverse-DNS, e.g. `com.avelino.todo-archiver`.

### `capabilities[]`

The loader intersects what you declare with what the running client implements (see [Plugins → Capabilities per client](plugins.md#capabilities-per-client)).
A capability the client can't honor lands in a warning, and the plugin still loads for the rest.

Live capabilities: `op-hook`, `slash-command`, `keybinding`, `config-schema` (read), `toolbar-button`, `ui-render`, and `content-transformer:text` / `:rich`.
`sync-transport` is core-ready (the host serializes/applies ops through a registered transport) but no client polls it yet.
A plugin that wants to be a query engine registers a `content-transformer` for the `query` fence language (` ```query `).
Plugins and JS code blocks can also call `outl.query({ status: "todo", … })` to get structured `QueryHit[]` results — see [Query code blocks → Plugin SDK API](query.md#plugin-sdk-api-outlquery).

### `permissions[]`

Declared here, approved by the user on install, frozen in the lockfile.
Every host call is gated against the approved set, so a permission you didn't request is a permission your code can't use.

```json
"permissions": ["read-page", "submit-op", "network:*.openai.com"]
```

Network must be scoped to a domain.
`network:api.openai.com` and `network:*.openai.com` are valid; a bare `network:*` is rejected at parse time.

---

## `definePlugin`

The entry file exports a single `definePlugin(...)` call.
Metadata redundant with `plugin.json` stays in the manifest; the entry file is behavior only.

```ts
// src/index.ts
import { definePlugin, type PluginContext, type LogOp } from "@outl/plugin-sdk";

export default definePlugin({
  activate(ctx: PluginContext) {
    // 1) op-hook — runs on every applied op (local or from sync)
    ctx.ops.onOp((op: LogOp) => {
      if (op.kind === "Edit" && op.text?.startsWith("DONE ")) {
        ctx.log.info(`block ${op.node} completed`);
      }
    });

    // 2) command — fired by the "/" slash menu / palette (CLI: `plugin run`)
    ctx.commands.register("todo-archive-done", () => {
      const cfg = ctx.config.get<{ archivePage: string }>();
      const done = ctx.blocks.query({ todo: "DONE" });
      for (const b of done) {
        ctx.blocks.move(b.id, { toPage: cfg.archivePage });
      }
      ctx.ui.notify(`${done.length} blocks archived`);
    });
  },

  // optional: cleanup on disable / update
  deactivate() {},
});
```

`activate(ctx)` runs once when the plugin loads.
`deactivate()` is optional and runs when the plugin is disabled or updated.

Mutation never happens in JS.
`ctx.blocks.move` becomes a host call → `outl-actions::block::*` → `Workspace::apply` → an op log entry stamped `plugin:<id>@<device>`.
You think in blocks and ops; the host owns the CRDT and the `.md`.

Hooks and command handlers run under a re-entrancy guard (the host tracks how far into the op log it has dispatched), so a plugin that triggers ops that trigger the plugin again can't spin into an infinite loop.
There is no ambient I/O — only what a permission grants.

---

## Host API — `PluginContext`

**This table is the canonical owner of the runtime host-API surface.**
The [architecture](plugin-architecture.md) and [tutorial](plugin-tutorial.md) link here rather than restating it.
Every namespace is gated by the permission in the right-hand column.
The typed signatures live in `@outl/plugin-sdk`; the runtime ships the subset below (the SDK types reads/writes as `Promise`, but the runtime resolves them synchronously within the turn — see the [describe → apply note](#api-changelog)).

| Namespace | Functions (what runs today) | Permission |
|---|---|---|
| `ctx.ops` | `onOp(cb: (op: LogOp) => void)` | `read-op-log` |
| `ctx.blocks` | `query(filter) → Block[]`, `get(id) → Block`, `edit(id, text)`, `create(parentId, text)`, `createAfter(afterId, text)`, `appendTree(parentId, tree)`, `move(id, { toPage } \| { toParent })`, `toggleTodo(id)`, `delete(id)` | `read-page` (reads) · `write-page` (writes) |
| `ctx.page` | `list() → { slug, title, kind }[]`, `create(slug)`, `appendTree(slug, tree)` | `read-page` (`list`) · `write-page` (`create`, `appendTree`) |
| `ctx.template` | `list() → { name, slug, params? }[]`, `instantiate(name, blockId)` — stamp a structural template under a block (see [Templates](templates.md)) | `read-page` (`list`) · `write-page` (`instantiate`) |
| `ctx.commands` | `register(id, handler)` | — (declared in `contributes.commands`) |
| `ctx.config` | `get<T>() → T` | — |
| `ctx.content` | `register(lang, fn)` — `fn(body) → { kind: "text" \| "rich", content } \| null` renders a fenced block of language `lang` (e.g. ` ```query `) | capability `content-transformer:text` / `:rich` |
| `ctx.storage` | `get(k) → v \| null`, `set(k, v)`, `delete(k)` — per-plugin KV at `<workspace>/.outl/plugins/<id>/storage.json`, local-only (never converges) | `storage:local` |
| `ctx.secrets` | `get(k) → string \| null` — read the plugin's own secret from the **OS keychain** (namespaced `outl-plugin:<id>`, never on disk in the workspace). Set out-of-band by the user; the plugin only reads | `secrets` |
| `ctx.net` | `fetch(url, { method?, headers?, body?, timeoutMs? }) → { ok, status, headers, text(), json() }` (or `{ ok: false, error }` on a denied domain — it returns, it doesn't throw); **blocking** on the plugin thread | `network:<domain>` |
| `ctx.sync` | `register({ push(opsJsonl), pull() → jsonl \| null })` — register a sync transport (core live; client polling is roadmap) | capability `sync-transport` |
| `ctx.log` | `info(m)` / `warn(m)` / `error(m)` | — |
| `ctx.ui` | `notify(m)` | — |
| `ctx.ui` | `render(html)` — run author-written HTML/JS in a sandboxed iframe overlay (GUI only) | capability `ui-render` |

`Block` is `{ id, text, parent, todo?, page }` — `text` has the task prefix stripped, `todo` is `"TODO" \| "DOING" \| "DONE"` or absent, and `parent` is the parent block id or `null` for a top-level child of the page (use it to reconstruct hierarchy from a `query`).
Branch on the state you care about and treat the rest as unfinished, rather than assuming `todo !== "DONE"` means `"TODO"`: a plugin written against two states counts every started block as neither.

`TreeNode` (what `appendTree` takes) is `{ text, children? }`, recursive.
`LogOp` (what `onOp` receives) is `{ kind, node, text?, todo? }`, where `kind` is one of `"Create" | "Move" | "Edit" | "SetProp" | "SetCollapsed"`; `text` and `todo` are present only on `"Edit"`.

A few load-bearing notes:

- **`ctx.ops.onOp`** receives a `LogOp` that has **already been applied** — local edits and ops arriving from sync alike.
  Use it to react, not to gate; you can't veto an op.
  Your own writes never re-fire your hook (the host advances its log mark past them).
- **`ctx.blocks` splits by permission** — reading (`query`/`get`) needs `read-page`; every mutating call (`edit`/`create`/`createAfter`/`move`/`toggleTodo`/`delete`) needs `write-page`.
  A plugin granted only `read-page` can read but every write is dropped with a recorded error, never a crash.
- **Reads are a turn snapshot, writes are deferred.**
  `query`/`get` read the workspace as it was at the start of the turn; a write you emit is *not* visible to a later read in the same handler — it lands on the next turn.
  This is the describe → apply model ([architecture](plugin-architecture.md#execution-model-describe--apply)).
- **`ctx.page.appendTree(slug, tree)` / `ctx.blocks.appendTree(parentId, tree)`** build a whole nested `TreeNode[]` in one turn.
  This is the escape hatch for the describe→apply limitation above: `create` needs a parent id, but a plugin can't obtain the id of a block (or a fresh page's first block) it just created this turn.
  `appendTree` sidesteps it — the host threads the new ids through internally as it applies, so you can **seed a brand-new page in a single run** (`page.appendTree` creates the page if missing) instead of running the command twice.
- **`ctx.config.get<T>()`** returns the plugin's config from the lockfile's `config` field as-is.
  Schema *validation* of that config (against `configSchema`) is not enforced yet, so treat `T` as a shape you trust, not one the host guarantees.
- **`ctx.commands.register`** wires a handler to a `contributes.commands[].id`, surfaced in the slash menu / palette / mobile sheet, or run headless via `outl plugin run <id> <command>`.
- **`ctx.content.register(lang, fn)`** registers a transformer for a fenced block language: `fn(body)` returns `{ kind: "text" | "rich", content }` or `null` to decline.
  `text` renders on every read surface (inline in the TUI); `rich` is HTML in a sandboxed iframe inline in the block and runs on the GUI clients only — the TUI/CLI drop a `rich` descriptor.
  A query engine plugs in here by registering for the `query` language.
- **`ctx.storage.{get,set,delete}`** is per-plugin key/value persisted at `<workspace>/.outl/plugins/<id>/storage.json`.
  It's **local-only and deliberately outside the op log** — it does not converge between devices.
  Without `storage:local` the call throws a clear error.
- **`ctx.secrets.get(key)`** reads the plugin's own secret from the **OS keychain** — never from the workspace on disk, so a token is not synced or committed with your notes.
  Secrets are namespaced by plugin id (`outl-plugin:<id>`), so a plugin can only read its own; without the `secrets` permission the call throws.
  The plugin **only reads** — the value is set out-of-band by the user via `outl plugin secret set <id> <key>` or a client's plugin settings, and `get` returns `null` until then (prompt the user to configure it).
  Mark the field in your `config.schema.json` with `"x-outl-secret": true` so the settings form routes it to the keychain instead of the lockfile.
- **`ctx.net.fetch(url, opts)`** is **blocking** on the plugin thread (on the TUI it blocks the UI for the duration of the request, bounded by `timeoutMs`).
  A domain the manifest didn't grant **returns `{ ok: false, error }`** rather than throwing.
  `network:<domain>` gates it; a bare `network:*` is rejected at parse time (use `domain` or `*.domain`).
- **`ctx.sync.register({ push, pull })`** registers a sync transport: the host serializes local ops into `push(opsJsonl)` and applies whatever `pull()` returns through `Workspace::apply`.
  The core path is live and convergence is tested; what's missing is a client that calls `push`/`pull` on a timer — see [Roadmap](#roadmap-not-yet-available).

### Secrets — the full flow

A secret (an API token, a webhook key) should never live in the lockfile or the
synced workspace. `ctx.secrets` keeps it in the OS keychain; the plugin only
reads it. Four steps wire it end to end.

**1. Request the permission** in `plugin.json`:

```json
"permissions": ["network:api.ouraring.com", "secrets"]
```

**2. Mark the field as a secret** in `config.schema.json` with the
`x-outl-secret` extension, so a client's plugin settings routes it to the
keychain instead of the plaintext config:

```json
{
  "type": "object",
  "properties": {
    "token": {
      "type": "string",
      "title": "API token",
      "x-outl-secret": true
    }
  }
}
```

**3. The user stores the value** out-of-band — it never passes through the
plugin. From the CLI (hidden prompt), or a client's plugin-settings form:

```sh
outl plugin secret set run.avelino.ouraring token
# Value for run.avelino.ouraring/token: ••••••••
```

**4. The plugin reads it** at run time and uses it — e.g. as a bearer token on a
gated `ctx.net.fetch`. `get` returns `null` until the user sets it, so guard for
that and tell them how:

```ts
export default definePlugin({
  activate(ctx) {
    ctx.commands.register("oura-sync", async () => {
      const token = await ctx.secrets.get("token");
      if (!token) {
        ctx.ui.notify(
          "No Oura token. Run: outl plugin secret set run.avelino.ouraring token",
        );
        return;
      }

      const r = await ctx.net.fetch("https://api.ouraring.com/v2/usercollection/daily_sleep", {
        headers: { Authorization: `Bearer ${token}` },
        timeoutMs: 10_000,
      });
      if (!r.ok) {
        ctx.ui.notify(`Oura request failed (HTTP ${r.status})`);
        return;
      }
      // …write the data to pages…
    });
  },
});
```

The token stays in the keychain the whole time — it is in memory only for the
duration of the turn, never written to the workspace, never synced, never in the
op log. A plugin without the `secrets` permission throws on `ctx.secrets.get`,
and each plugin is namespaced (`outl-plugin:<id>`), so it can only read its own.

### Roadmap (not yet available)

These are typed in `@outl/plugin-sdk` and/or enumerated in the manifest schema so the contract is forward-stable, but the runtime does **not** drive them end to end yet.

| Surface | State today | Notes |
|---|---|---|
| `sync-transport` client polling | **Core live, no client driver** | `ctx.sync.register` works and convergence is tested, but no client calls `push`/`pull` on a timer yet. |
| `ctx.page.open(slug)` / `ctx.page.today()` | **Not present** | Typed in the SDK; the runtime `ctx.page` exposes only `list`/`create`. |
| `{{query}}` inline | **Parser defers it** | A fenced ` ```query ` block works natively (auto-run, embeds). Plugins can call `outl.query({ … })` for structured results. Inline `{{query}}` needs a new parser token the project defers. |

`github:` install and `outl plugin init` ship today (see [Plugins → Installing](plugins.md#installing)).
The remaining tooling roadmap (`outl plugin update`, `.outlpkg` pack, dev hot-reload, a dev console, and discovery / marketplace / signing in the clients) is tracked in [Plugins](plugins.md). A config-editing surface now ships on every client: `outl plugin config` / `outl plugin secret` on the CLI, and a settings form per installed plugin in the desktop / mobile plugin browser and the TUI `plugin-settings` overlay.

---

## Versioning

There are **two independent semver axes**, and keeping them separate is what lets the plugin API stay stable while the binary moves fast.

- **`api`** — the plugin's required range against the **plugin API surface** (the host API + capabilities described here), e.g. `^1.0`.
  A plugin asking for `api: "^2.0"` on a host that only implements API 1.x **does not load**, with an error pointing the user at "update outl or use the previous plugin version".
- **`engines.outl`** — the minimum **binary** version, e.g. `>=0.8.0`.
  This tracks the fast-moving binary; `api` tracks the slow, long-lived contract.

A plugin built against API 1.0 keeps working on every host that still implements API 1.x, regardless of how many binary releases ship in between.

### Changelog discipline

The [API changelog](#api-changelog) at the top of this page is the contract.
When the host adds a namespace, capability, or permission, it gets a new entry there before it ships, and anything not yet listed is not part of the stable surface.
Treat each entry as a promise: things in API 1.0 don't break under API 1.x.

---

## See also

- [Plugin tutorial](plugin-tutorial.md) — build the TODO-archiver end to end.
- [Plugin architecture](plugin-architecture.md) — describe → apply, the host, permission gating, the lifecycle.
- [Plugins](plugins.md) — installing, permissions, distribution, the lockfile.
- [`plugin-v1.json`](schemas/plugin-v1.json) — the manifest JSON Schema.
- [CLI](cli.md) — the `outl plugin` subcommands.
