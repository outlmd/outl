# Confetti

> **Capability:** `ui-render` (+ `op-hook`) · [Source on GitHub](https://github.com/outlmd/outl/tree/main/examples/confetti)

Throws a confetti burst every time you mark a block **DONE**.

## What it demonstrates

`ui-render` lets a plugin hand a GUI client a chunk of **author-written HTML/JS** that it runs in a sandboxed iframe overlay.
The host knows nothing about "confetti" — the plugin produces the markup, the client only runs it (isolated, `sandbox="allow-scripts"` with no same-origin).
Want fireworks instead?
Rewrite `CONFETTI_HTML` — it's your creativity, not a fixed catalog of effects.

It also shows `op-hook`: the burst is triggered from `ctx.ops.onOp`, watching for a block that transitions into DONE.
On the TUI/CLI the render is dropped (no webview); the op-hook still fires.

## The code

```ts
import { definePlugin, type LogOp, type PluginContext } from "@outl/plugin-sdk";

const CONFETTI_HTML = `<!doctype html>... a full-screen <canvas> + a tiny
particle simulation, fully inline (the iframe has no network, no imports) ...`;

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.ops.onOp((op: LogOp) => {
      // A TODO→DONE toggle lands as an `Edit` op the host projects `todo: "DONE"` onto.
      if (op.kind === "Edit" && op.todo === "DONE") {
        ctx.ui.render(CONFETTI_HTML);
      }
    });
  },
});
```

See [`src/index.ts`](https://github.com/outlmd/outl/tree/main/examples/confetti/src/index.ts) for the full `CONFETTI_HTML`.

## Manifest

```jsonc
"capabilities": ["op-hook", "ui-render"],
"permissions":  ["read-op-log"]
```

## Try it

```sh
outl -w <workspace> plugin install ./examples/confetti --yes
```

Open the workspace in the **desktop** or **mobile** app, mark any block DONE (`Cmd+T` cycles a block through `TODO`, `DOING`, `DONE`, so press it until it reads DONE) → 🎉.

## Adapting it

`ctx.ui.render(html)` runs whatever HTML/JS you give it in an isolated iframe.
Swap `CONFETTI_HTML` for any self-contained visual — a toast, an SVG badge, a celebratory GIF — and trigger it from any hook or command.
Keep it inline: the sandbox has no network and no imports.
