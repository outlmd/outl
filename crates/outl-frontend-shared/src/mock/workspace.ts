import type { BlockNode, PageMeta, PageView, TodoState } from "../api/types";
import { tokenize } from "./tokenize";

/**
 * An in-memory workspace for the browser mock. It is a fixture, not a
 * model of the CRDT: edits replace text in place and there is no op
 * log, no undo, no sync. Enough to render every screen with believable
 * content and to click around.
 */

interface Seed {
  text: string;
  children?: Seed[];
  /**
   * `key:: value` pairs, so the property line has something to draw.
   *
   * The mock exists so a client runs in a plain browser for
   * screenshots, which means a surface with no fixture data cannot be
   * looked at — and the property line is the surface that most needed
   * looking at. One seed carries an import stamp on purpose: it must
   * render *nothing*, so a regression that unhides it is visible here
   * rather than only in a unit test.
   */
  props?: Array<[string, string]>;
}

interface Page {
  meta: PageMeta;
  blocks: BlockNode[];
}

let nextId = 1;
const newId = () => `mock-${(nextId++).toString(36).padStart(6, "0")}`;

function block(seed: Seed): BlockNode {
  const m = /^(TODO|DOING|DONE) /.exec(seed.text);
  const todo = (m?.[1] as TodoState | undefined) ?? null;
  const text = m ? seed.text.slice(m[0].length) : seed.text;
  return {
    id: newId(),
    text: seed.text,
    todo,
    tokens: tokenize(text),
    collapsed: false,
    properties: seed.props ?? [],
    children: (seed.children ?? []).map(block),
  };
}

export function journalSlug(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export function shiftDay(slug: string, by: number): string {
  const [y, m, d] = slug.split("-").map(Number);
  return journalSlug(new Date(y, m - 1, d + by));
}

export class MockWorkspace {
  readonly pages = new Map<string, Page>();

  constructor(today = new Date()) {
    const t = journalSlug(today);
    this.add("journal", t, [
      { text: "Planning the [[outl]] UI pass with a fresh look at #design", children: [
        { text: "Journal title reads as a day, not as an ISO date" },
        { text: "One popover surface everywhere" },
      ] },
      {
        text: "TODO Review the command palette sketch",
        props: [
          ["created", "2026-11-12T09:14:07.690+00:00"],
          ["priority", "high"],
          ["remind", "9am every 1h"],
          ["status", "waiting on Ana"],
        ],
      },
      { text: "DONE Ship the browser mock for screenshots" },
      {
        text: "Call with [[Ana]] about **sync** on iOS",
        props: [
          ["owner", "Mario"],
          ["roam-uid", "callana-20261112-0611"],
        ],
      },
    ]);
    this.add("journal", shiftDay(t, -1), [
      { text: "Read about daily notes in [[outl]]" },
      { text: "TODO Write the RFC for the journal stream #design" },
    ]);
    this.add("journal", shiftDay(t, -3), [
      { text: "DOING Sketch the week strip for mobile" },
      { text: "Coffee with [[Ana]], talked about quick capture" },
    ]);
    this.add("journal", shiftDay(t, -4), [{ text: "Weekly review" }]);
    this.add("page", "outl", [
      { text: "Local-first outliner. Markdown on disk, CRDT underneath." },
      { text: "Clients", children: [{ text: "TUI" }, { text: "Desktop" }, { text: "Mobile" }] },
    ], true);
    this.add("page", "design", [{ text: "Notes on the visual system" }]);
    this.add("page", "ana", [{ text: "type:: person" }], false, "Ana");
  }

  private add(kind: PageMeta["kind"], slug: string, seeds: Seed[], pinned = false, title = slug) {
    this.pages.set(slug, {
      meta: { id: newId(), slug, title, kind, ...(pinned ? { pinned } : {}) },
      blocks: seeds.map(block),
    });
  }

  ensureJournal(slug: string): Page {
    if (!this.pages.has(slug)) this.add("journal", slug, [{ text: "" }]);
    return this.pages.get(slug)!;
  }

  bySlug(slug: string): Page | undefined {
    const key = slug.toLowerCase();
    return this.pages.get(key) ?? [...this.pages.values()].find((p) => p.meta.title.toLowerCase() === key);
  }

  byId(pageId: string): Page | undefined {
    return [...this.pages.values()].find((p) => p.meta.id === pageId);
  }

  view(page: Page): PageView {
    return { page: page.meta, outline: page.blocks, backlinks: [], backlinks_order: "newest", page_properties: [] };
  }

  /** Every block, depth first, with the page that owns it. */
  *walk(): Generator<[Page, BlockNode, BlockNode[]]> {
    for (const page of this.pages.values()) {
      const go = function* (nodes: BlockNode[], path: BlockNode[]): Generator<[Page, BlockNode, BlockNode[]]> {
        for (const n of nodes) {
          yield [page, n, path];
          yield* go(n.children, [...path, n]);
        }
      };
      yield* go(page.blocks, []);
    }
  }

  /** The sibling list holding `id`, and its index there. */
  locate(page: Page, id: string): [BlockNode[], number] | undefined {
    const go = (nodes: BlockNode[]): [BlockNode[], number] | undefined => {
      const i = nodes.findIndex((n) => n.id === id);
      if (i >= 0) return [nodes, i];
      for (const n of nodes) {
        const hit = go(n.children);
        if (hit) return hit;
      }
      return undefined;
    };
    return go(page.blocks);
  }

  setText(node: BlockNode, text: string) {
    const fresh = block({ text });
    node.text = fresh.text;
    node.todo = fresh.todo;
    node.tokens = fresh.tokens;
  }

  insertAfter(page: Page, afterId: string | null, text = ""): BlockNode {
    const node = block({ text });
    const at = afterId ? this.locate(page, afterId) : undefined;
    if (at) at[0].splice(at[1] + 1, 0, node);
    else page.blocks.push(node);
    return node;
  }
}
