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
  var index_default = definePlugin({
    activate(ctx) {
      const transport = {
        // Called by the host with the JSONL of locally-authored ops to ship.
        // One op per line, so counting lines = counting ops.
        push(opsJsonl) {
          const count = countOps(opsJsonl);
          ctx.log.info(`[echo-sync] pushing ${count} op(s)`);
        },
        // Called by the host on a timer. Return JSONL of remote ops to apply, or
        // `null` when there's nothing new. The skeleton has no backend, so: null.
        pull() {
          ctx.log.info("[echo-sync] pull: no backend wired, nothing to apply");
          return null;
        }
      };
      ctx.sync.register(transport);
      ctx.log.info("[echo-sync] transport registered (skeleton \u2014 no backend)");
    }
  });
  function countOps(jsonl) {
    return jsonl.split("\n").filter((line) => line.trim().length > 0).length;
  }
})();
