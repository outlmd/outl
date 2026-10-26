import type { Backlink, Palette, PageMeta, TimelineEvent } from "../api/types";
import presets from "./presets.json";
import { journalSlug, MockWorkspace, shiftDay } from "./workspace";

type Args = Record<string, unknown>;
type Handler = (args: Args) => unknown;

const palettes = presets as Palette[];

/**
 * A handful of the catalog's chrome bindings, so the palette and the
 * keyboard have something to work with. The real catalog is
 * `outl_shortcuts::default_bindings`; this is not a copy of it.
 */
const META = 0b1000;
const CTRL = 0b0001;
const SHIFT = 0b0100;
const BASE = [
  { key: "p", mode: "global", action: { kind: "OpenPicker" }, description: "Open quick switcher" },
  { key: "k", mode: "normal", action: { kind: "OpenCommandPalette" }, description: "Command palette" },
  { key: "j", mode: "global", action: { kind: "OpenToday" }, description: "Open today's journal" },
  { key: "\\", mode: "global", action: { kind: "ToggleSidebar" }, description: "Toggle sidebar" },
  { key: ",", mode: "global", action: { kind: "OpenSettings" }, description: "Open settings" },
];
// Cmd first, then its Ctrl twin, the order the real catalog uses.
const BINDINGS = [
  ...BASE.flatMap(({ key, ...rest }) =>
    [META, CTRL].map((mods) => ({ chord: [{ mods, key: { kind: "Char", value: key } }], ...rest })),
  ),
  // Quick capture is the one Shift+Space chord the palette needs to show.
  ...[META, CTRL].map((mods) => ({
    chord: [{ mods: mods | SHIFT, key: { kind: "Space" } }],
    mode: "global",
    action: { kind: "QuickCapture" },
    description: "Quick capture to today",
  })),
];
const palette = (name: unknown) =>
  palettes.find((p) => p.name === name) ?? palettes[0];

/**
 * Answers the Tauri commands the clients call, from a {@link MockWorkspace}.
 * Anything missing here resolves to `null` and logs once, so a new
 * command shows up in the console instead of breaking the page.
 */
export function createMockBackend(ws = new MockWorkspace(), today = new Date()) {
  let mockMerges = [
    {
      id: "m1",
      page_id: "p-inbox",
      page_title: "Inbox",
      block_id: "b1",
      ours: "Call the venue about Friday",
      merged: "Call the venue about Friday at 3pm",
      peer_actor: "01JQ00000000000000009F3A2C",
      at_ms: Date.now() - 6 * 60_000,
    },
  ];
  const todaySlug = journalSlug(today);
  const settings = {
    last_workspace: "/mock/workspace",
    vim_mode: false,
    theme: "outl-light",
    theme_dark: "outl",
    theme_mode: "auto",
    font_size: 15,
    sync_transport: "iroh",
    backlinks_order: "newest",
    reminders_enabled: false,
    reminders_quiet_hours: "",
    diagrams: "pele",
  };

  const page = (args: Args) => {
    const p = ws.byId(String(args.pageId));
    if (!p) throw new Error(`mock: no page ${String(args.pageId)}`);
    return p;
  };
  const open = (slug: string) => {
    const p = ws.bySlug(slug) ?? (/^\d{4}-\d{2}-\d{2}$/.test(slug) ? ws.ensureJournal(slug) : undefined);
    if (!p) throw new Error(`mock: no page ${slug}`);
    return ws.view(p);
  };
  const metas = (): PageMeta[] => [...ws.pages.values()].map((p) => p.meta);
  const search = (query: unknown) => {
    const q = String(query ?? "").toLowerCase();
    return metas().filter((m) => m.title.toLowerCase().includes(q) || m.slug.includes(q));
  };

  const backlinks = (slug: string): Backlink[] => {
    const target = ws.bySlug(slug);
    if (!target) return [];
    const names = [target.meta.slug, target.meta.title.toLowerCase()];
    const out: Backlink[] = [];
    for (const [src, node, path] of ws.walk()) {
      if (src === target) continue;
      const hit = node.tokens.some((t) => (t.kind === "ref" || t.kind === "tag") && names.includes(t.value.toLowerCase()));
      if (!hit) continue;
      out.push({
        block_id: node.id,
        todo: node.todo,
        source_page: src.meta,
        ancestors: path.map((a) => ({ id: a.id, text: a.text })),
        source_block: node,
        source_block_path: [],
      });
    }
    return out;
  };

  const handlers: Record<string, Handler> = {
    current_workspace: () => settings.last_workspace,
    workspace_stats: () => ({ blocks: [...ws.walk()].length, ops: 0, actor: "mock", storage_root: settings.last_workspace, ready: true, ops_complete: true }),
    get_settings: () => settings,
    update_settings: (a) => Object.assign(settings, a.next),
    get_theme_config: () => ({ preset: settings.theme, preset_dark: settings.theme_dark, mode: settings.theme_mode }),
    get_theme: (a) => palette(a.name ?? settings.theme),
    list_themes: () => palettes.map((p) => p.name),
    list_shortcut_bindings: () => BINDINGS,
    list_action_support: () => [],

    today_slug_cmd: () => todaySlug,
    previous_day: (a) => shiftDay(String(a.slug), -1),
    next_day: (a) => shiftDay(String(a.slug), 1),
    date_title: (a) => {
      const [y, m, d] = String(a.slug).split("-").map(Number);
      return new Date(y, m - 1, d).toLocaleDateString("en-US", { weekday: "long", month: "long", day: "numeric", year: "numeric" });
    },
    open_today_journal: () => open(todaySlug),
    open_journal_for: (a) => open(String(a.slug)),
    open_page_by_slug: (a) => open(String(a.slug)),
    open_ref: (a) => open(String(a.target)),
    resolve_ref: (a) => ws.bySlug(String(a.target))?.meta ?? null,
    list_all_pages: metas,
    search_pages: (a) => search(a.query),
    search_persons: (a) => search(a.query).filter((m) => m.slug === "ana"),
    search_blocks: (a) => {
      const q = String(a.query ?? "").toLowerCase();
      return [...ws.walk()]
        .filter(([, n]) => n.text.toLowerCase().includes(q))
        .slice(0, 25)
        .map(([p, n]) => ({ handle: `blk-${n.id}`, id: n.id, text: n.text, source_slug: p.meta.slug }));
    },
    list_open_tasks: () => {
      const tasks = [...ws.walk()]
        .filter(([, n]) => n.todo === "TODO" || n.todo === "DOING")
        .map(([p, n]) => ({
          id: n.id,
          source_slug: p.meta.slug,
          status: n.todo === "DOING" ? "doing" : "todo",
          text: n.text.replace(/^(TODO|DOING) /, ""),
        }));
      return { total: tasks.length, tasks };
    },
    journal_days_with_content: (a) =>
      [...ws.pages.values()]
        .filter((p) => p.meta.kind === "journal" && p.meta.slug >= String(a.from) && p.meta.slug <= String(a.to))
        .filter((p) => p.blocks.some((b) => b.text.trim() !== ""))
        .map((p) => p.meta.slug)
        .sort(),
    page_backlinks: (a) => ({
      backlinks: backlinks(String(a.slug)),
      backlinks_order: settings.backlinks_order,
      namespace_backlinks: [],
      namespace_backlinks_total: 0,
      namespace_children: [],
    }),
    resolve_page_labels: (a) => (a.nodeIds as string[]).map(() => ""),
    resolve_embeds: () => [],
    known_property_keys: () => [],
    list_templates_cmd: () => [],
    page_timeline: (a) => mockTimeline(page(a)),
    device_activity: () => ({
      complete: true,
      actors: [
        { actor: "01JQ0000000000000000LOCAL1", ops: 1284, last_at_ms: Date.now() - 20_000, is_local: true },
        { actor: "01JQ00000000000000009F3A2C", ops: 3071, last_at_ms: Date.now() - 4 * 60_000, is_local: false },
        { actor: "01JQ00000000000000007B11E0", ops: 212, last_at_ms: Date.now() - 3 * 86_400_000, is_local: false },
      ],
    }),
    merged_edits: () => mockMerges,
    drain_capture_inbox: () => ({ landed: 0, skipped: 0, page_ids: [] }),
    undo_merged_edit: (a) => {
      mockMerges = mockMerges.filter((m) => m.id !== a.id);
      return null;
    },
    export_markdown: (a) => ({
      dir: `${String(a.destDir ?? "~/Documents")}/outl-export-2026-10-12-171500`,
      pages: ws.pages.size,
      assets: 0,
    }),
    outl_emoji_search: () => [],
    outl_peer_list: () => [
      { node_id: "9f3a2c1d", alias: "MacBook", added_at: "2026-09-01T10:00:00Z" },
      { node_id: "7b11e0aa", alias: "iPhone", added_at: "2026-09-02T10:00:00Z" },
    ],
    outl_peer_status: () => [
      { node_id: "9f3a2c1d", alias: "MacBook", online: true, rtt_ms: 38, pending_ops: 0 },
      { node_id: "7b11e0aa", alias: "iPhone", online: false, rtt_ms: null, pending_ops: 4 },
    ],
    list_reminders: () => [],
    plugin_list: () => [],
    plugin_toolbar: () => [],
    plugin_keybindings: () => [],
    plugin_transformers: () => [],
    reminder_action_catalog: () => [],
    reload_workspace: () => null,
    outl_sync_now: () => null,
    take_pending_deep_link: () => null,
    take_pending_open_file: () => null,

    edit_block: (a) => {
      const p = page(a);
      const at = ws.locate(p, String(a.id));
      if (at) ws.setText(at[0][at[1]], String(a.text));
      return ws.view(p);
    },
    create_block: (a) => {
      const p = page(a);
      const node = ws.insertAfter(p, (a.afterId as string | null) ?? null, (a.text as string | null) ?? "");
      return { view: ws.view(p), new_id: node.id };
    },
    split_block: (a) => {
      const p = page(a);
      const node = ws.insertAfter(p, String(a.id));
      return { view: ws.view(p), new_id: node.id };
    },
    toggle_todo: (a) => {
      const p = page(a);
      const at = ws.locate(p, String(a.id));
      if (at) {
        const n = at[0][at[1]];
        const body = n.text.replace(/^(TODO|DOING|DONE) /, "");
        ws.setText(n, n.todo === null ? `TODO ${body}` : n.todo === "DONE" ? body : `DONE ${body}`);
      }
      return ws.view(p);
    },
    delete_block: (a) => {
      const p = page(a);
      const at = ws.locate(p, String(a.id));
      if (at) at[0].splice(at[1], 1);
      return ws.view(p);
    },
    set_block_collapsed: (a) => {
      const p = page(a);
      const at = ws.locate(p, String(a.id));
      if (at) at[0][at[1]].collapsed = Boolean(a.collapsed);
      return ws.view(p);
    },
  };

  const missed = new Set<string>();
  return (cmd: string, args: Args = {}): unknown => {
    const h = handlers[cmd];
    if (h) return h(args);
    if (/^plugin:[a-z-]+\|/.test(cmd)) return null;
    if (!missed.has(cmd)) {
      missed.add(cmd);
      console.info(`[outl mock] no handler for "${cmd}", answering null`);
    }
    return null;
  };
}

/**
 * A believable history for whatever page is open, so the history panel
 * has edits, a deletion and a move to render under `?mock=1`. Newest
 * first, like the real command.
 */
function mockTimeline(p: { meta: PageMeta; blocks: { id: string; text: string }[] }) {
  const now = Date.now();
  const events: TimelineEvent[] = [];
  const base = { actor: "01JQ0000000000000000LOCAL1", block_deleted: false, text: null, key: null };
  p.blocks.slice(0, 3).forEach((b, i) => {
    const words = b.text.split(" ");
    const from = words.length > 2 ? [...words.slice(0, -1), "draft"].join(" ") : `${b.text} (draft)`;
    events.push({ ...base, at_ms: now - (i + 1) * 3_600_000, block: b.id, change: "edited", from, to: b.text });
  });
  events.push({
    ...base,
    actor: "01JQ00000000000000009F3A2C",
    at_ms: now - 26 * 3_600_000,
    block: "mock-deleted",
    block_deleted: true,
    change: "deleted",
    from: null,
    to: null,
    text: "Call the venue about the projector",
  });
  if (p.blocks[0]) {
    events.push({ ...base, at_ms: now - 50 * 3_600_000, block: p.blocks[0].id, change: "moved", from: null, to: null });
  }
  return { slug: p.meta.slug, total: events.length, truncated: false, events };
}
