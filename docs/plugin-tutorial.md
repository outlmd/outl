# Build your first outl plugin

This is a hands-on tutorial.
By the end you'll have written, built, installed, and run a real outl plugin — from an empty folder to a working command you trigger from the CLI, the TUI, the desktop app, and mobile.

It's long on purpose.
Nothing is skipped, nothing is assumed.
If you've never written a line of JavaScript for outl, start at the top and follow it straight through.

If you want the dry reference instead of the walkthrough, read [Plugin API](plugin-api.md).
If you want to understand the runtime internals (the Boa engine, the describe→apply model under the hood), read [Plugin architecture](plugin-architecture.md).

## Quickstart — a running plugin in 60 seconds

If you just want to see a plugin run before reading the whole thing, scaffold one and install it:

```sh
outl plugin init hello                       # writes ./hello (manifest + build + a `hello` command)
cd hello
# the scaffold pins "@outl/plugin-sdk": "^1.0.0", which npm does not have yet (see the note below):
perl -pi -e 's/"\^1\.0\.0"/"latest"/' package.json
bun install && bun run build                 # bundles src/index.ts → index.js  (npm works too)
outl -w ~/notes plugin install . --yes       # install into your workspace
outl -w ~/notes plugin run com.example.hello hello
```

That last line prints the toast the starter command fires; in the TUI / desktop, type `/hello` in a block instead.
Verified output: `👋 Hello from hello!` then `Applied 0 change(s).`
`outl plugin init` writes a complete, buildable project — manifest, `package.json`, `tsconfig`, `src/index.ts`, README — so you have a real plugin to grow from.

> **The SDK version pin.**
> `outl plugin init` writes `"@outl/plugin-sdk": "^1.0.0"`, and the npm registry only has `0.x` releases of the SDK (the `latest` tag points at a `0.13.0` beta).
> `bun install` fails with `No version matching "^1.0.0" found`.
> Change the dependency to `"latest"` as above and it resolves.
> The plugin API is `1.0`, and that is a different number from the SDK package version.

The rest of this page builds a plugin **from an empty folder by hand**, so you touch every field and every host call once and nothing is magic.
If you'd rather start from the scaffold and read the reference alongside, that's the [Plugin API](plugin-api.md).

## What we're going to build

We'll build **`tag-counter`** — a small but genuinely useful plugin that exercises both day-zero capabilities you'll reach for most:

- An **op-hook** (`ctx.ops.onOp`) that watches edits as they land and logs whenever a block gains a `#tag`.
- A **slash command** (`ctx.commands.register`) called `count-tags` that scans the workspace, tallies every `#tag`, and writes a tidy report into a `tag-summary` page.

It's a real plugin: it reads blocks, mutates the workspace, reads its own config, logs, and notifies the user.
That covers most of what you'll do in any plugin.

> We deliberately don't reuse the shipped `todo-archiver` example.
> Building something new from scratch is the point — you'll touch every moving part once.

### Prerequisites

You need three things on your machine:

1. **The `outl` CLI**, built and on your `PATH`.
   If you can run `outl --version`, you're set.
   If not, build it from the repo: `cargo build --release -p outl-cli` and use the binary in `target/release/`.
2. **A JavaScript runtime + `esbuild`.**
   We use [`bun`](https://bun.sh) in this tutorial (it's fast and bundles `esbuild`-style builds out of the box), but plain `node` + `npm` works identically.
   Any of these is fine:
   - `bun` installed → `bunx esbuild …`
   - `node` + `npm` installed → `npx esbuild …`
3. **A text editor.**
   TypeScript is recommended (you get types from the SDK), but you can write plain JavaScript if you prefer — the build step is the same.

That's it.
No outl account, no server, no registry — plugins are local files.

## Step 1 — Lay out the folder

A plugin during development has two halves: the **source** you edit (`src/`, TypeScript) and the **bundle** the host actually runs (`index.js`, a single self-contained file).
Create this layout anywhere on disk — call the folder `tag-counter`:

```
tag-counter/
├── plugin.json            # the manifest — what the host reads first
├── package.json           # your build script + the SDK dependency
├── tsconfig.json          # TypeScript config (skip if writing plain JS)
├── config.schema.json     # user-editable settings, described as JSON Schema
└── src/
    └── index.ts           # the plugin code you write
```

After we build, `index.js` (the bundle) lands next to `plugin.json`.
That bundle — plus `plugin.json` and `config.schema.json` — is the *installed shape*: what gets copied into a workspace.
Your `src/` and `node_modules/` never travel with it.

Don't create the files yet — we'll fill each one in below, explaining every field.

## Step 2 — Write the manifest (`plugin.json`)

The manifest is the contract.
The host reads it before it runs a single line of your code: it tells outl who you are, what you plug into, and what you're allowed to touch.
It's validated against [`plugin-v1.json`](schemas/plugin-v1.json) at install time and on every load — point your editor's `$schema` at it for autocomplete.

Create `tag-counter/plugin.json`:

```json
{
  "$schema": "https://outl.app/schemas/plugin-v1.json",
  "id": "app.outl.examples.tag-counter",
  "name": "Tag Counter",
  "version": "1.0.0",
  "api": "^1.0",
  "engines": {
    "outl": ">=0.7.0"
  },
  "main": "index.js",
  "capabilities": [
    "op-hook",
    "slash-command",
    "config-schema"
  ],
  "permissions": [
    "read-page",
    "write-page",
    "read-op-log",
    "submit-op"
  ],
  "contributes": {
    "commands": [
      {
        "id": "count-tags",
        "title": "Count #tags into a summary page"
      }
    ],
    "configSchema": "config.schema.json"
  },
  "description": "Tallies every #tag in the workspace into a summary page.",
  "author": "you",
  "license": "MIT",
  "category": "productivity"
}
```

Field by field:

- **`id`** — your stable, reverse-DNS identity.
  It never changes across versions.
  It's the install directory name *and* the op-log actor stamp (`plugin:app.outl.examples.tag-counter@<device>`), so pick it once and keep it.
  The pattern is enforced: lowercase labels separated by dots, at least two labels.
- **`name`** — the human-readable display name shown in menus.
- **`version`** — semver.
  Bump it when you publish a new build.
- **`api`** — the *plugin API* range you target, **not** the binary version.
  `^1.0` means "any host whose plugin API major is 1".
  Write it honestly.
  The host does not compare it at load time yet (see [Plugin API → Versioning](plugin-api.md#versioning)), but it will.
- **`engines.outl`** — the minimum outl *binary* version.
  Separate axis from `api`: this tracks the app, `api` tracks the plugin surface.
  Recorded, not enforced yet.
- **`main`** — the bundled entry file, relative to the plugin root.
  Always a single `.js` file — no `node_modules`, no runtime module resolution.
- **`capabilities`** — what you plug into.
  The loader **intersects** this with what the current client implements; a capability the client can't honor loads partially with a warning, never a crash.
  We declare the three day-zero ones we use.
  (`keybinding` is also declarable — see the note at the end of this step.)
- **`permissions`** — what you're allowed to do.
  The user approves these once on install, and they're frozen in the lockfile.
  Writes, storage, secrets and network are checked against this set when the plugin runs, deny by default.
  We ask for `read-page` + `read-op-log` to read blocks and watch ops, and `write-page` + `submit-op` to write our summary.
  The read-side permissions are listed at install but not checked at run time yet, see [Permissions: what is enforced](plugin-api.md#permissions-what-is-enforced).
  Declare them anyway: the user reads the list to decide.
- **`contributes.commands`** — the commands you surface in the slash menu / palette.
  Each `id` here must match the id you pass to `ctx.commands.register(...)` at runtime, and the `title` is the label the user sees.
  A command is only listed when the plugin also declares the `slash-command` capability.
- **`contributes.configSchema`** — a path to a JSON Schema file describing your user-editable settings.
  Day-zero accepts a path only, never an inline schema.
- **`description`, `author`, `license`, `category`** — descriptive, non-load-bearing fields for discovery UI.
  They sit at the top level of `plugin.json`, as in every example in `examples/`.

> **A note on `keybinding`.**
> You *can* declare `contributes.keybindings` (each entry is `{ "command": "<id>", "key": "Ctrl+Shift+T" }`).
> Chord dispatch is **live on the TUI** (a chord fires the command from Normal mode, single or two-chord, unless a native binding already claims it) and on the **desktop** (a native binding always wins).
> Mobile has no keyboard, so it doesn't apply there.
> Declare the `keybinding` capability too, or the chord never binds.
> A chord that does not parse, or has more than two parts, is skipped without a message.
> We leave keybindings out of this tutorial to keep it focused on the two capabilities you'll reach for first.

## Step 3 — Describe your config (`config.schema.json`)

Our plugin lets the user choose which page the summary lands on.
That's one setting, described as a JSON Schema (draft 2020-12).
Create `tag-counter/config.schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Tag Counter config",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "summaryPage": {
      "type": "string",
      "title": "Summary page",
      "description": "Slug of the page the tag tally is written to.",
      "default": "tag-summary"
    }
  }
}
```

A page slug is kebab-case: no `/`, no `.`.
(`tags/summary` is rejected with `invalid page slug`, and the write is dropped.)
You read this config at runtime with `ctx.config.get()`.

> **Where the config value actually comes from.**
> `ctx.config.get()` reads the `config` field for your plugin from the workspace lockfile (`installed.json`).
> The user edits it on **every client**: `outl plugin config set <id> <key> <value>` on the CLI, the settings form in the desktop / mobile plugin browser, or the TUI `plugin-settings` overlay.
> A plugin in `_dev/` has no lockfile entry, so there `ctx.config.get()` returns `null`: our `cfg?.summaryPage` guard handles that.
> All of them render this JSON Schema as a form and coerce each value to its field type, so the schema you write here *is* the settings UI.
> A field marked `"x-outl-secret": true` is routed to the OS keychain instead of the lockfile and read with `ctx.secrets.get()` — see [Plugin API → Secrets, the full flow](plugin-api.md#secrets--the-full-flow).

## Step 4 — Set up the build (`package.json` + `tsconfig.json`)

Create `tag-counter/package.json`:

```json
{
  "name": "tag-counter",
  "version": "1.0.0",
  "private": true,
  "type": "module",
  "scripts": {
    "build": "esbuild src/index.ts --bundle --format=iife --platform=neutral --target=es2022 --outfile=index.js",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "@outl/plugin-sdk": "latest"
  },
  "devDependencies": {
    "esbuild": "^0.24.0",
    "typescript": "^6.0.3"
  }
}
```

Two notes on the dependency line:

- `@outl/plugin-sdk` is **types plus one helper** (`definePlugin`) and nothing else — zero runtime code.
  Outside the repo, depend on the published package: `"latest"` tracks the newest build cut from `main` (the `release` workflow publishes it on every release).
  Inside this repo's bun workspace you can use `"workspace:*"` to resolve it from `plugin-sdk/`.

If you're using TypeScript, create `tag-counter/tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "lib": ["ES2022", "DOM"],
    "moduleResolution": "bundler",
    "skipLibCheck": true,
    "isolatedModules": true,
    "strict": true,
    "noEmit": true
  },
  "include": ["src"]
}
```

We'll come back to the **build command** in Step 6 — it has the single biggest gotcha in the whole tutorial.
For now, just install dependencies:

```sh
cd tag-counter
bun install      # or: npm install
```

## Step 5 — Write the plugin (`src/index.ts`)

Here's the whole plugin.
Create `tag-counter/src/index.ts`, then read the walkthrough below — every API call is explained.

```ts
import { definePlugin, type LogOp, type PluginContext } from "@outl/plugin-sdk";

/** Our config shape, mirrored by config.schema.json. */
interface TagCounterConfig {
  summaryPage: string;
}

const DEFAULT_SUMMARY_PAGE = "tag-summary";

/** Pull every #tag out of a block's text. Returns lowercase tag names, no `#`. */
function extractTags(text: string): string[] {
  const out: string[] = [];
  const re = /#([a-z0-9][a-z0-9/_-]*)/gi;
  let m: RegExpExecArray | null;
  while ((m = re.exec(text)) !== null) {
    out.push(m[1].toLowerCase());
  }
  return out;
}

export default definePlugin({
  activate(ctx: PluginContext) {
    // 1) op-hook — fires once for every op applied to the log.
    //    We only react to text edits, and only log when a tag appears.
    ctx.ops.onOp((op: LogOp) => {
      if (op.kind !== "Edit" || !op.text) {
        return;
      }
      const tags = extractTags(op.text);
      if (tags.length > 0) {
        ctx.log.info(`block ${op.node} now has tags: ${tags.join(", ")}`);
      }
    });

    // 2) command — fired from the slash menu / palette / `outl plugin run`.
    ctx.commands.register("count-tags", async () => {
      const cfg = ctx.config.get<Partial<TagCounterConfig>>();
      const summaryPage =
        cfg?.summaryPage?.trim() || DEFAULT_SUMMARY_PAGE;

      // --- READ PHASE: gather everything from the snapshot, up front. ---
      const blocks = await ctx.blocks.query({}); // empty filter = every block
      const counts = new Map<string, number>();
      for (const b of blocks) {
        // Skip the summary page itself so we don't count our own output.
        if (b.page === summaryPage) {
          continue;
        }
        for (const tag of extractTags(b.text)) {
          counts.set(tag, (counts.get(tag) ?? 0) + 1);
        }
      }

      // Sort by count desc, then name, for a stable report.
      const ranked = [...counts.entries()].sort(
        (a, b) => b[1] - a[1] || a[0].localeCompare(b[0]),
      );

      // --- WRITE PHASE: now emit the mutations. ---
      if (ranked.length === 0) {
        ctx.ui.notify("No #tags found in the workspace");
        return;
      }

      // `appendTree` seeds the whole page in a single turn: it creates
      // `summaryPage` if it's missing and threads the new block ids through
      // internally, so we never need a parent id we couldn't obtain mid-turn.
      ctx.page.appendTree(
        summaryPage,
        ranked.map(([tag, n]) => ({ text: `#${tag} — ${n}` })),
      );

      ctx.ui.notify(`Counted ${ranked.length} distinct tag(s)`);
    });
  },
});
```

### Walking through every API call

**`ctx.ops.onOp(cb)`** — registers a hook the host calls once for every op applied to the log, local edits and synced ops alike.
The `op` you receive is `{ kind, node, text?, todo? }`:

- `kind` is one of `"Create" | "Move" | "Edit" | "SetProp" | "SetCollapsed"`.
- `node` is the block id the op acted on (a string).
- `text` and `todo` are populated **only** when `kind === "Edit"` — that's the op that carries a block's new text and TODO state.

So our hook bails unless it's an `Edit` with text, then scans for tags.
This needs the `read-op-log` permission and the `op-hook` capability — both of which we declared.

**`ctx.commands.register(id, handler)`** — wires the `count-tags` handler to the command we declared in `contributes.commands`.
The id **must** match.
The handler is `() => void | Promise<void>`.
It is `async` here because the SDK types `ctx.blocks.query` as a `Promise`, and `tsc` rejects iterating it without an `await`.
At run time the call returns right away from the turn snapshot, and `await` on it costs nothing.
It fires when the user picks the command from the slash menu, the palette, or runs `outl plugin run`.

**`ctx.config.get()`** — returns your plugin's config object (read from the lockfile, as noted in Step 3).
We fall back to the default page when it's unset.

**`ctx.blocks.query(filter)`** → `Block[]` — finds blocks.
The filter is `{ page?, todo?, textContains? }`, all optional and ANDed; an empty `{}` matches every block.
Each `Block` is `{ id, text, parent, todo?, page }` — note `text` is **clean**, with no `TODO `/`DONE ` prefix, and `parent` is the parent block id or `null` for a top-level block.
Needs `read-page`.

**`ctx.page.appendTree(slug, tree)`** — appends a whole `TreeNode[]` (`{ text, children? }`, recursive) under a page, creating the page if it's missing.
This is the call that lets us seed a brand-new `tag-summary` page in a single run: it creates the page and all the tally blocks in one turn, threading the new block ids through internally.
Needs `write-page`.

**`ctx.ui.notify(msg)`** and **`ctx.log.info(msg)`** — user-facing toast / status line, and a line in the host's plugin log, respectively.
Neither needs a permission.

### The one thing that trips everyone: describe → apply

This is the single most important mental model in the plugin runtime, so read it twice.

**Reads come from a snapshot taken at the start of the turn.**
**Writes are buffered and applied by the host *after* your handler returns.**

Concretely:

- `ctx.blocks.query(...)`, `ctx.blocks.get(...)`, `ctx.page.list()`, `ctx.config.get()` all read from a frozen snapshot of the workspace as it was when your command started.
- `ctx.blocks.edit/create/move/...`, `ctx.page.create(...)` don't take effect immediately.
  They're queued, and the host drains the queue and applies each one (permission-gated) only once your handler has finished.

The practical consequence: **a mutation you make is NOT visible to a `query` later in the same command.**
The snapshot never changes mid-turn.
If you `create` a block and then `query`, the new block won't be in the results.

The rule that follows: **collect everything you're going to change first (the read phase), then emit all your mutations (the write phase).**
Our `count-tags` does exactly that — it builds the full `counts` map from the snapshot before it creates a single block.
Don't interleave reads and writes expecting to see your own changes; you won't.

### Why `appendTree` and not `create`

`ctx.blocks.create(parentId, text)` needs a **parent block id**, and describe→apply is exactly why that bites here.
A page you create this turn has no block you can address yet — it doesn't materialize until after your handler returns.
`ctx.page.appendTree(slug, tree)` is the escape hatch: the host applies the tree *and* creates the page if missing in one pass, threading the new ids internally, so you seed a brand-new page in a single run — no "run it twice" rough edge.

The other write calls are still the right tool when you already hold block ids:

- `ctx.blocks.create(parentId, text)` / `createAfter(afterId, text)` when you have a real parent or sibling id (e.g. one you got from `query`).
- `ctx.blocks.move(id, { toPage })` to relocate existing blocks onto a page — it takes a page slug and creates the page if missing (this is what the shipped `todo-archiver` example does).

Rule of thumb: **new content on a fresh page → `appendTree`; editing or relocating blocks you already found → `create` / `move`.**

The host API you have to work with, in full:

| Call | What it does | Permission |
|---|---|---|
| `ctx.ops.onOp(cb)` | hook every applied op | `read-op-log` |
| `ctx.commands.register(id, h)` | wire a command handler | — |
| `ctx.blocks.query(filter)` → `Block[]` | find blocks | `read-page` |
| `ctx.blocks.get(id)` → `Block \| null` | one block by id | `read-page` |
| `ctx.blocks.edit(id, text)` | replace text (include `TODO `/`DONE ` to keep a prefix) | `write-page` + `submit-op` |
| `ctx.blocks.create(parentId, text)` | new last child | `write-page` + `submit-op` |
| `ctx.blocks.createAfter(afterId, text)` | new sibling after a block | `write-page` + `submit-op` |
| `ctx.blocks.appendTree(parentId, tree)` | append a nested `TreeNode[]` under a block, one turn | `write-page` + `submit-op` |
| `ctx.blocks.move(id, target)` | `{ toPage }` (creates page) or `{ toParent }` | `write-page` + `submit-op` |
| `ctx.blocks.toggleTodo(id)` | cycle none, `TODO`, `DOING`, `DONE`, none | `write-page` + `submit-op` |
| `ctx.blocks.delete(id)` | move to trash (restorable) | `write-page` + `submit-op` |
| `ctx.page.list()` → `Page[]` | `{ slug, title, kind }` | `read-page` |
| `ctx.page.create(slug)` | idempotent page create (slug is kebab-case, no `/` or `.`) | `write-page` |
| `ctx.page.appendTree(slug, tree)` | append a `TreeNode[]`, creating the page if missing | `write-page` |
| `ctx.config.get()` | your config object | — |
| `ctx.log.info/warn/error(m)` | host log | — |
| `ctx.ui.notify(m)` | user toast / status line | — |

> **Also available** (beyond the table above): `ctx.storage.{get,set,delete}` is per-plugin local KV (gated by `storage:local`, stored at `<workspace>/.outl/plugins/<id>/storage.json`, never syncs), and `ctx.net.fetch(url, opts)` does a **blocking** HTTP request gated by `network:<domain>` (a denied domain returns `{ ok: false, error }` instead of throwing).
> `ctx.content.register(lang, fn)` registers a renderer for a fenced block language, and `ctx.sync.register({ push, pull })` registers a sync transport (the core path is live; client polling is still roadmap).
> See [Plugin API → Host API](plugin-api.md#host-api--plugincontext) for the full signatures.

## Step 6 — Build the bundle (read this carefully)

The host runs your bundle as a **plain script**, not as an ES module.
That means the build **must** emit an IIFE, not ESM.

Run this from inside `tag-counter/`:

```sh
bunx esbuild src/index.ts --bundle --format=iife --platform=neutral --target=es2022 --outfile=index.js
# or, with npm:
npx esbuild src/index.ts --bundle --format=iife --platform=neutral --target=es2022 --outfile=index.js
# or just use the script you defined:
bun run build
```

> **⚠️ Gotcha #1 — `--format=iife` is mandatory.**
> If you build with `--format=esm`, esbuild emits `export` statements.
> The outl engine evaluates the bundle as a script with no module loader, so an `export` is a syntax error in that context and your plugin **silently fails to load** (or loads with a confusing error).
> The symptom is "my command never shows up."
> The fix is always: rebuild with `--format=iife`.
> This is the number-one mistake people hit.

How `definePlugin` reaches the host through the IIFE: the SDK's `definePlugin(def)` calls `globalThis.__outl_register(def)`, which the engine installs *before* it evaluates your bundle.
The IIFE runs top-to-bottom, hits your `export default definePlugin({...})`, the helper registers your definition with the host, and the host then calls `activate(ctx)` with the real context.
You never `export` anything the host consumes — registration happens through that global, which is exactly why ESM's `export` machinery is unnecessary (and breaks things).

After a successful build you'll have `tag-counter/index.js`.
That, plus `plugin.json` and `config.schema.json`, is your installable plugin.

## Step 7 — Install it

Pick (or create) a workspace to install into.
If you don't have one:

```sh
outl init ~/notes-test
```

Then install your plugin from its directory (a `github:owner/repo` source works too, see [Plugins → Install](plugins.md#install)):

```sh
outl -w ~/notes-test plugin install ~/path/to/tag-counter --yes
```

What happens:

- outl parses `plugin.json` and validates it against the schema.
- It computes the bundle hash (stored for integrity — revalidated on every load).
- It shows you the permissions the plugin requests and asks for approval.
  `--yes` approves everything non-interactively (required when stdin isn't a TTY, e.g. in scripts).
  Drop `--yes` to see the prompt and approve by hand.
- It copies the installed shape into `~/notes-test/.outl/plugins/app.outl.examples.tag-counter/` and freezes the approved permission set in the lockfile (`installed.json`).

Confirm it's there:

```sh
outl -w ~/notes-test plugin list
```

You should see your plugin, its version, `enabled`, and the `/count-tags` command it contributes.

## Step 8 — Run it

Let's give it something to count.
Seed a couple of blocks with tags:

```sh
outl -w ~/notes-test page create inbox --title Inbox
outl -w ~/notes-test block append --page inbox --text "ship the thing #work #urgent"
outl -w ~/notes-test block append --page inbox --text "read the paper #research"
```

Now run the command from the CLI:

```sh
outl -w ~/notes-test plugin run app.outl.examples.tag-counter count-tags
```

`plugin run` executes the command and then **re-renders every `.md`** so the mutation lands on disk (the op log is the source of truth; the `.md` files are a projection of it).
You'll see the notification text and the change count in the output:

```text
Counted 3 distinct tag(s)
Applied 1 change(s).
```

`tag-summary.md` now holds `- #research — 1`, `- #urgent — 1`, `- #work — 1`.
If you see `Applied 0 change(s).` followed by an `error:` line, the write was refused: read the error.

### Where the command shows up in the GUI clients

The same plugin, same command, surfaces everywhere a person edits:

- **TUI** — press `/` to open the slash menu; `/count-tags` appears alongside the built-ins.
- **Desktop** — click the plugins button (`⧉`, in the top-right capsule) to open the plugin palette and pick the command, or type `/count-tags` at the start of a block.
- **Mobile** — tap the plugin glyph in the header to open the plugin sheet.

`op-hook` and `slash-command` both work end-to-end on all four clients (TUI, desktop, mobile, CLI), so your `onOp` log line and your `count-tags` command behave identically everywhere.

## Step 9 — Iterate fast with dev mode

Reinstalling after every code change is tedious.
For authoring, drop your plugin into the workspace's `_dev/` directory instead:

```
~/notes-test/.outl/plugins/_dev/tag-counter/
├── plugin.json
├── index.js
└── config.schema.json
```

A plugin in `_dev/`:

- **Loads without the hash check** — edit `index.js`, reload, no reinstall.
- **Runs with all permissions implicit** — you're not prompted on every iteration.
- **Is never recorded in the lockfile** — no approval, no saved config (`ctx.config.get()` returns `null`).

There is no file watcher and no "sandbox relaxed" banner yet.
The loop becomes: edit `src/index.ts` → `bun run build` → copy `index.js` into `_dev/tag-counter/` → run the CLI command again, or restart the client.
(You can symlink or script that copy.)
Remove any installed copy with the same id first, or both load.
This is for authoring only — ship the real, hash-pinned install when you're done.

## Step 10 — Debug when it doesn't work

Where output goes:

- **`ctx.log.info/warn/error`** and any `console.log` in your code go to the **host's plugin log** (and surface in each client's log view).
  This is your primary debugging channel.
- **`ctx.ui.notify`** shows the user-facing message (status line / toast) — good for confirming a run happened, not for debugging detail.
- **Load and runtime errors** surface as a warning in `outl plugin list` (a plugin that fails to load is reported, never hidden) and in the client's plugin log.

Common failures and what they mean:

| Symptom | Likely cause | Fix |
|---|---|---|
| Command never appears | Bundle built as ESM (`export` in `index.js`) | Rebuild with `--format=iife` |
| Command never appears | `contributes.commands[].id` ≠ the id passed to `ctx.commands.register` | Make the two ids identical |
| ``denied `write-page` for intent`` / write does nothing | You wrote without `write-page` approved | Add the permission to `plugin.json`, reinstall (you'll re-approve) |
| Install rejected: invalid id | `id` doesn't match reverse-DNS (`a.b`, lowercase, ≥2 labels) | Fix the `id` pattern |
| `install failed: io: No such file or directory` | `main` (`index.js`) does not exist: you installed before building | `bun run build`, then install again |
| `invalid page slug` in the errors, `Applied 0 change(s).` | You wrote to a page slug with `/` or `.` in it | Use kebab-case slugs (`tag-summary`) |
| `bun install` fails: `No version matching "^1.0.0"` | The scaffold pins an SDK version npm does not have | Set `"@outl/plugin-sdk": "latest"` |
| `tsc`: `Promise<Block[]>` must have a `[Symbol.iterator]` | The SDK types reads as `Promise` | `async` handler and `await` the read |
| `ctx.storage` / `ctx.net` call throws "permission denied" | You didn't declare `storage:local` / `network:<domain>` | Add the permission to `plugin.json`, reinstall (you'll re-approve) |
| Your own write doesn't show in a later `query` | describe→apply: reads are from the start-of-turn snapshot | Read everything first, then write |
| `onOp` never fires | Missing `op-hook` capability (`read-op-log` is listed too, but the capability is what gates it) | Declare both in `plugin.json` |
| Command never appears (manifest looks right) | Missing the `slash-command` capability | Declare it |
| A code fence of your language renders as plain source | `contributes.transformers` has no entry for the language, or your function threw or returned `null` | Declare it, and wrap the body in `try` with `ctx.log.error` while debugging |

When in doubt, sprinkle `ctx.log.info(...)` through your handler and watch the plugin log.

## Next steps

You've now built, installed, and run a plugin end to end.
Where to go from here:

- **[Plugin API](plugin-api.md)** — the full authoring reference: every host method, the manifest in detail, `definePlugin`, versioning, and the permission model.
- **[Plugins](plugins.md)** — the *user* side: installing, pinning, updating, approving permissions, and where plugins live on disk.
- **[`plugin-v1.json`](schemas/plugin-v1.json)** — the JSON Schema your `plugin.json` is validated against.
- **[Plugin architecture](plugin-architecture.md)** — how the runtime works under the hood (the Boa JS engine, the describe→apply host loop).
  The deepest internals live in the `outl-plugins` crate notes.

### A word on distribution

Three ways to share a plugin, from least to most public:

- **A built directory** — hand someone the folder (`plugin.json` + `index.js` + `config.schema.json`) and they run `outl plugin install <dir>`.
- **A GitHub repo** — `outl plugin install github:user/repo` clones it at the newest semver tag (or `…#v1.2.0` to pin one).
  No registry entry needed.
- **The official registry** — list it in [`outlmd/registry`](https://github.com/outlmd/registry) so it shows up in the in-app marketplace and `outl plugin search`.
  See [Plugins → Publish a plugin](plugins.md#publish-a-plugin) for the four-step flow.

Still roadmap: **`.outlpkg`** single-file packages and `outl plugin update`.
Reinstalling over an installed plugin works but resets its saved config, see [Plugins → Install](plugins.md#install).
