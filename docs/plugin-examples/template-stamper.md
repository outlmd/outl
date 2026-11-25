# Template Stamper

> **Capability:** `slash-command` + `config-schema` · **Permissions:** `read-page`, `write-page` · **Source:** this page (there is no `examples/template-stamper/` folder yet)

A `stamp` command that copies a structural template under the first block of a page you name in the settings.

## What it demonstrates

- `ctx.template.list()` reads the templates in the workspace (any page with a `template::` property, see [Templates](../templates.md)).
- `ctx.template.instantiate(name, blockId)` deep-copies the template's blocks under a block.
- Why the target is a setting and not "the block under the cursor": a plugin has no cursor.
  It sees a snapshot of the workspace and addresses blocks by id, so a plugin that needs a place to write has to name one.

## The files

Make a folder `template-stamper/` with these four files.

`plugin.json`

```json
{
  "$schema": "https://outl.app/schemas/plugin-v1.json",
  "id": "app.outl.examples.template-stamper",
  "name": "Template Stamper",
  "version": "1.0.0",
  "api": "^1.0",
  "main": "index.js",
  "description": "A `stamp` command that stamps a structural template under the first block of a page.",
  "license": "MIT",
  "category": "productivity",
  "capabilities": ["slash-command", "config-schema"],
  "permissions": ["read-page", "write-page"],
  "contributes": {
    "commands": [{ "id": "stamp", "title": "Stamp a template" }],
    "configSchema": "config.schema.json"
  }
}
```

`config.schema.json`

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "Template Stamper config",
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "template": {
      "type": "string",
      "title": "Template",
      "description": "Name of the template to stamp (the value of `template::`). Empty means the first template found."
    },
    "page": {
      "type": "string",
      "title": "Target page",
      "description": "Slug of the page to stamp into. The template lands under that page's first top-level block.",
      "default": "inbox"
    }
  }
}
```

`src/index.ts`

```ts
import { definePlugin, type PluginContext } from "@outl/plugin-sdk";

interface StamperConfig {
  template?: string;
  page?: string;
}

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.commands.register("stamp", async () => {
      const cfg = ctx.config.get<StamperConfig | null>() ?? {};
      const page = cfg.page?.trim() || "inbox";

      const templates = await ctx.template.list();
      const tpl = cfg.template
        ? templates.find((t) => t.name === cfg.template)
        : templates[0];
      if (!tpl) {
        ctx.ui.notify("No template found. Add `template:: <name>` to a page.");
        return;
      }

      // A plugin has no cursor, so it names its target: the first top-level
      // block of a configured page.
      const blocks = await ctx.blocks.query({ page });
      const target = blocks.find((b) => b.parent === null);
      if (!target) {
        ctx.ui.notify(`Page "${page}" has no block to stamp under.`);
        return;
      }

      ctx.template.instantiate(tpl.name, target.id);
      ctx.ui.notify(`Stamped "${tpl.name}" under the first block of "${page}".`);
    });
  },
});
```

`package.json` and `tsconfig.json` are the ones from the [tutorial](../plugin-tutorial.md#step-4--set-up-the-build-packagejson--tsconfigjson).
Set `"@outl/plugin-sdk": "latest"` in `package.json`.

## Build, install, run

```sh
bun install && bun run build
outl -w ~/notes plugin install . --yes

# a template: any page with a `template::` property
outl -w ~/notes page create interview --title interview
outl -w ~/notes page prop set interview template=interview
outl -w ~/notes block append --page interview --text "**Candidate:**"
outl -w ~/notes block append --page interview --text "**Role:**"

outl -w ~/notes plugin run app.outl.examples.template-stamper stamp
```

```text
Stamped "interview" under the first block of "inbox".
Applied 1 change(s).
```

The first top-level block of `inbox` now has the two template blocks as children, each tagged `from-template:: interview`.
In the TUI and the desktop app the command is `/stamp`.
Change the target or the template with `outl plugin config set app.outl.examples.template-stamper page journal-notes`.

If there is nothing to stamp, the command says so and writes nothing: ``No template found. Add `template:: <name>` to a page.`` or `Page "inbox" has no block to stamp under.`

## Adapting it

- **Stamp into today's journal:** journal slugs are dates (`2026-11-25`), so build the slug from `new Date()` and use it as `page`.
- **Stamp on a trigger:** use `ctx.ops.onOp` (needs the `op-hook` capability) and call `instantiate` when a `Create` op lands.
- **Callable templates** (the ones that run code) are not exposed to plugins.
  See the surface table in [Templates](../templates.md#surface-scope).
