/**
 * Excalidraw — render a drawing inside a note.
 *
 * A ```excalidraw fence either carries a `.excalidraw` file or references a
 * scene in Excalidraw+ by id; either way the block draws the scene as inline
 * SVG, hand-drawn strokes and all, with no network access from the render
 * surface and nothing but clean markdown left in the file.
 *
 * How the pieces fit:
 *
 *   - `content-transformer:rich` draws the fence. It is **synchronous** — a
 *     transformer returns its descriptor within the call — so it never
 *     fetches. It reads the local cache, or says what to run.
 *   - `excalidraw-sync` does the fetching, on demand, over the public API with
 *     a key from the OS keychain.
 *   - `excalidraw-scenes` writes the workspace's scenes into a page as
 *     ready-made fences, so the first drawing is two commands away.
 *
 * The capability split is worth naming: this is a **GUI surface**. `rich`
 * descriptors are HTML in a sandboxed iframe, and the TUI/CLI drop them — a
 * terminal has nowhere to draw an SVG, so there the fence stays as the text
 * you wrote. The commands work everywhere.
 */

import { definePlugin, type PluginContext, type TransformResult } from "@outl/plugin-sdk";
import { packFiles, readScene } from "./cache";
import { clearCache, listScenes, syncScenes } from "./commands";
import { parseFence, type FenceOptions } from "./fence";
import { caption, drawingPage, messagePage } from "./html";
import { renderScene } from "./render/scene";
import type { ExcalidrawElement, PluginConfig, Theme } from "./types";

/** Height ceiling when neither the fence nor the config picks one. */
const DEFAULT_MAX_HEIGHT = 480;

/** The command line that fills the cache, shown when a scene is missing. */
const SYNC_HINT = "outl plugin run app.outl.examples.excalidraw excalidraw-sync";

export default definePlugin({
  activate(ctx: PluginContext) {
    // Declared in plugin.json as `{ lang: "excalidraw", kind: "rich" }`.
    ctx.content.register("excalidraw", (body) => transform(ctx, body));

    ctx.commands.register("excalidraw-sync", () => syncScenes(ctx));
    ctx.commands.register("excalidraw-scenes", () => listScenes(ctx));
    ctx.commands.register("excalidraw-cache-clear", () => clearCache(ctx));
  },
});

/**
 * Render one fence. Never throws: a broken scene becomes a readable message.
 *
 * The `catch` is the only thing that makes that sentence true. A renderer
 * works on JSON somebody else wrote, and the host reads a throw as "no
 * descriptor" — which draws an empty block and explains nothing. Whatever
 * slips past the per-field guards lands here instead, named.
 */
function transform(ctx: PluginContext, body: string): TransformResult {
  try {
    return render(ctx, body);
  } catch (e) {
    return rich(messagePage("Could not draw this scene", String(e)));
  }
}

/** The body of `transform`, minus its safety net. */
function render(ctx: PluginContext, body: string): TransformResult {
  const fence = parseFence(body);
  if (fence.kind === "error") {
    return rich(messagePage("Excalidraw fence", fence.message));
  }

  const config = ctx.config.get<PluginConfig>() || {};

  if (fence.kind === "inline") {
    const elements = Array.isArray(fence.scene.elements) ? fence.scene.elements : [];
    const background =
      (fence.scene.appState && fence.scene.appState.viewBackgroundColor) || "";
    // No budget: a pasted scene's images are already in the markdown.
    return draw(
      elements,
      packFiles(fence.scene.files).files,
      background,
      fence.opts,
      config,
      "",
    );
  }

  const cached = readScene(ctx, fence.ref.id);

  if (cached.kind === "unavailable") {
    return rich(
      messagePage(
        "Cache unavailable",
        `Scenes are cached locally, and that cache could not be read: ${cached.reason}. A \`scene:\` fence needs the storage:local permission — a pasted .excalidraw file does not.`,
      ),
    );
  }

  if (cached.kind === "missing") {
    return rich(
      messagePage(
        "Not synced yet",
        `Scene ${fence.ref.id} is not in the local cache. Run “Excalidraw: sync scenes” from the command palette, or:`,
        SYNC_HINT,
      ),
    );
  }

  const scene = cached.scene;
  return draw(
    scene.elements,
    scene.files,
    scene.background,
    fence.opts,
    config,
    caption({
      name: fence.opts.title === undefined ? scene.name : fence.opts.title,
      syncedAt: scene.at,
      filesDropped: scene.filesDropped,
    }),
  );
}

/** Render elements into the framed page. */
function draw(
  elements: ExcalidrawElement[],
  files: Record<string, string>,
  background: string,
  opts: FenceOptions,
  config: PluginConfig,
  baseCaption: string,
): TransformResult {
  if (elements.length === 0) {
    return rich(messagePage("Empty scene", "This scene has no elements to draw."));
  }

  const rendered = renderScene(elements, files, background, opts.background === true);
  const theme: Theme = opts.theme || config.theme || "auto";

  // Whatever the caption already says, plus what this render left out. The
  // second half is the honest part: a truncated drawing that says nothing
  // looks exactly like a complete one.
  const notes = caption({
    truncated: rendered.truncated,
    unsupported: rendered.unsupported,
  });
  const text = [baseCaption, notes].filter((s) => s !== "").join(" · ");

  return rich(
    drawingPage(rendered.svg, {
      dark: theme === "dark",
      auto: theme === "auto",
      maxHeight: clampHeight(config.maxHeight),
      height: opts.height,
      width: rendered.width,
      intrinsicHeight: rendered.height,
      caption: text,
    }),
  );
}

/** Keep a configured height inside what the schema promises. */
function clampHeight(value: number | undefined): number {
  if (typeof value !== "number" || !Number.isFinite(value)) return DEFAULT_MAX_HEIGHT;
  return Math.min(2000, Math.max(80, Math.round(value)));
}

/** A `rich` descriptor: HTML for the host's sandboxed iframe. */
function rich(content: string): TransformResult {
  return { kind: "rich", content };
}
