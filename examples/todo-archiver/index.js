"use strict";
(() => {
  // ../../plugin-sdk/src/index.ts
  function definePlugin(def) {
    if (def === null || typeof def !== "object") {
      throw new TypeError("definePlugin: expected a plugin definition object");
    }
    if (typeof def.activate !== "function") {
      throw new TypeError("definePlugin: `activate` must be a function");
    }
    if (def.deactivate !== void 0 && typeof def.deactivate !== "function") {
      throw new TypeError(
        "definePlugin: `deactivate` must be a function when provided"
      );
    }
    const host = globalThis;
    host.__outl_register?.(def);
    return def;
  }

  // src/index.ts
  var DEFAULT_ARCHIVE_PAGE = "archive";
  var index_default = definePlugin({
    activate(ctx) {
      ctx.ops.onOp((op) => {
        if (op.actor?.startsWith("plugin:app.outl.examples.todo-archiver")) {
          return;
        }
        if (becameDone(op)) {
          ctx.log.info(`block ${op.node} marked DONE`);
        }
      });
      ctx.commands.register("todo-archive-done", async () => {
        const archivePage = resolveArchivePage(ctx);
        await ctx.page.create(archivePage);
        const done = await ctx.blocks.query({ todo: "DONE" });
        const movable = done.filter((b) => b.page !== archivePage);
        for (const block of movable) {
          await ctx.blocks.move(block.id, { toPage: archivePage });
        }
        ctx.ui.notify(
          movable.length === 0 ? "No DONE blocks to archive" : `Archived ${movable.length} block(s) to "${archivePage}"`
        );
      });
    }
  });
  function becameDone(op) {
    if (op.todo === "DONE") {
      return true;
    }
    return op.kind === "TextUpdate" && (op.text?.startsWith("DONE ") ?? false);
  }
  function resolveArchivePage(ctx) {
    const cfg = ctx.config.get();
    const page = cfg?.archivePage?.trim();
    return page && page.length > 0 ? page : DEFAULT_ARCHIVE_PAGE;
  }
})();
//# sourceMappingURL=index.js.map
