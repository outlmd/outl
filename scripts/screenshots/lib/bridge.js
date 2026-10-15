// Injected before the client boots under `?mock=1`. The mock backend in
// outl-frontend-shared answers every Tauri command with a tiny built-in
// fixture; this wraps its invoke so the commands that carry workspace
// content are answered from the CLI dump in `window.__OUTL_SHOTS__`
// instead. Everything else (settings, themes, shortcuts) stays on the mock.
(() => {
  const fx = window.__OUTL_SHOTS__;
  if (!fx) return;
  const view = (slug) => {
    const v = fx.views[slug];
    if (!v) throw new Error(`shots: no page ${slug}`);
    return { ...structuredClone(v), backlinks: [], backlinks_order: "newest" };
  };
  const bySlugOrTitle = (target) => {
    const t = String(target).toLowerCase();
    return fx.pages.find((p) => p.slug === t || p.title.toLowerCase() === t);
  };
  const filterPages = (q) => {
    const s = String(q ?? "").toLowerCase();
    return fx.pages.filter((p) => p.title.toLowerCase().includes(s) || p.slug.includes(s));
  };
  const walk = function* (nodes) {
    for (const n of nodes) {
      yield n;
      yield* walk(n.children ?? []);
    }
  };
  const tasks = () => {
    const out = [];
    for (const [slug, v] of Object.entries(fx.views)) {
      if (slug.startsWith("templates-")) continue;
      for (const n of walk(v.outline)) {
        if (n.todo === "TODO" || n.todo === "DOING") {
          out.push({ id: n.id, source_slug: slug, status: n.todo.toLowerCase(), text: n.text.replace(/^(TODO|DOING) /, "") });
        }
      }
    }
    return { total: out.length, tasks: out };
  };
  const ours = {
    today_slug_cmd: () => fx.today,
    open_today_journal: () => view(fx.today),
    open_journal_for: (a) => view(a.slug),
    open_page_by_slug: (a) => view(a.slug),
    open_ref: (a) => {
      const p = bySlugOrTitle(a.target);
      if (!p) throw new Error(`shots: no page ${a.target}`);
      return view(p.slug);
    },
    resolve_ref: (a) => bySlugOrTitle(a.target) ?? null,
    list_all_pages: () => fx.pages,
    search_pages: (a) => filterPages(a.query),
    search_persons: (a) => filterPages(a.query).filter((p) => p.page_type === "person"),
    search_blocks: (a) => fx.searches[String(a.query ?? "").toLowerCase()] ?? [],
    list_open_tasks: tasks,
    journal_days_with_content: (a) =>
      fx.pages
        .filter((p) => p.kind === "journal" && p.slug >= a.from && p.slug <= a.to)
        .filter((p) => fx.views[p.slug]?.outline.some((b) => b.text.trim() !== ""))
        .map((p) => p.slug)
        .sort(),
    page_backlinks: (a) => ({
      backlinks: fx.backlinks[a.slug] ?? [],
      backlinks_order: "newest",
      namespace_backlinks: [],
      namespace_backlinks_total: 0,
      namespace_children: [],
    }),
    list_templates_cmd: () => fx.templates,
    // A shot that needs a page other than today asks for it the way a
    // cold-start `outl://` link would, so no click path is involved.
    take_pending_deep_link: () => {
      const link = window.__OUTL_SHOTS_LINK__ ?? null;
      window.__OUTL_SHOTS_LINK__ = null;
      return link;
    },
    known_property_keys: () => [...new Set(Object.values(fx.views).flatMap((v) => v.page_properties.map(([k]) => k)))],
  };
  const internals = (window.__TAURI_INTERNALS__ ??= {});
  let inner;
  Object.defineProperty(internals, "invoke", {
    configurable: true,
    get: () => (cmd, args = {}, opts) => {
      const h = ours[cmd];
      if (h) {
        try {
          return Promise.resolve(h(args));
        } catch (e) {
          return Promise.reject(e instanceof Error ? e.message : String(e));
        }
      }
      return inner(cmd, args, opts);
    },
    set: (fn) => {
      inner = fn;
    },
  });
})();
