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
  var QUOTE_URL = "https://api.quotable.io/random";
  var index_default = definePlugin({
    activate(ctx) {
      ctx.commands.register("inspire", async () => {
        const r = await ctx.net.fetch(QUOTE_URL, { timeoutMs: 8e3 });
        if (!r.ok) {
          const reason = `HTTP ${r.status}`;
          ctx.log.error(`[inspire] fetch failed: ${reason}`);
          ctx.ui.notify(`Could not reach the quote service (${reason})`);
          return;
        }
        const quote = await r.json();
        ctx.ui.notify(`\u{1F4AC} ${quote.content} \u2014 ${quote.author}`);
      });
    }
  });
})();
