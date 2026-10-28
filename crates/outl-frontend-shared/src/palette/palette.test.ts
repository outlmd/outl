import { describe, expect, it } from "vitest";

import type { BlockHit } from "../api/commands";
import type { PageMeta } from "../api/types";
import { paletteResults, sectionOf, type PaletteActionRow } from "./index";

const page = (slug: string): PageMeta => ({ id: slug, slug, title: slug, kind: "page" });
const hit = (text: string): BlockHit => ({ handle: text, id: text, text, source_slug: "p" });
const act = (label: string): PaletteActionRow => ({ kind: "action", action: label, label, chord: null });

describe("paletteResults", () => {
  it("lists pages, tasks, blocks and actions in that order", () => {
    const r = paletteResults({
      query: "wr",
      pages: [page("writing")],
      blocks: [hit("TODO write"), hit("wrote it")],
      actions: [act("Open today's journal")],
    });
    expect(r.items.map((i) => i.kind)).toEqual(["page", "task", "block"]);
  });
  it("matches an action by label", () => {
    const r = paletteResults({ query: "today", pages: [], blocks: [], actions: [act("Open today's journal")] });
    expect(r.items.map((i) => i.kind)).toEqual(["action"]);
  });
  it("matches an action by keyword", () => {
    const row = { ...act("Devices and sync"), keywords: "pending merged peers" };
    const r = paletteResults({ query: "peers", pages: [], blocks: [], actions: [row] });
    expect(r.items).toHaveLength(1);
  });
  it("lists only actions after the command prefix", () => {
    const r = paletteResults({ query: "> side", pages: [page("side")], blocks: [], actions: [act("Toggle sidebar")] });
    expect(r.items.map((i) => i.kind)).toEqual(["action"]);
  });
  it("shows every action for a bare command prefix", () => {
    const r = paletteResults({ query: ">", pages: [], blocks: [], actions: [act("a"), act("b")] });
    expect(r.items).toHaveLength(2);
  });
  it("hides blocks until the query is long enough", () => {
    const r = paletteResults({ query: "a", pages: [], blocks: [hit("alpha")], actions: [] });
    expect(r.items).toHaveLength(0);
  });
  it("returns pages alone for an empty query", () => {
    const r = paletteResults({ query: "", pages: [page("a")], blocks: [hit("b")], actions: [act("Sync now")] });
    expect(r.items.map((i) => i.kind)).toEqual(["page"]);
  });
  it("honours per-section limits", () => {
    const pages = [page("pa1"), page("pa2"), page("pa3")];
    const r = paletteResults({ query: "pa", pages, blocks: [], actions: [], limits: { pages: 2 } });
    expect(r.items).toHaveLength(2);
  });
  it("hidden_counts_what_the_limit_dropped", () => {
    const pages = [page("pa1"), page("pa2"), page("pa3")];
    const r = paletteResults({ query: "pa", pages, blocks: [], actions: [], limits: { pages: 2 } });
    expect(r.hidden.pages).toBe(1);
    expect(r.hidden.blocks).toBe(0);
  });
  it("caps tasks and blocks independently and counts each drop", () => {
    const many = Array.from({ length: 9 }, (_, i) => hit(`TODO milk ${i}`));
    const r = paletteResults({ query: "milk", pages: [], blocks: [...many, hit("milk")], actions: [] });
    expect(r.items.filter((i) => i.kind === "task")).toHaveLength(5);
    expect(r.items.filter((i) => i.kind === "block")).toHaveLength(1);
    expect(r.hidden.tasks).toBe(4);
  });
  it("counts block hits past the blocks limit", () => {
    const blocks = Array.from({ length: 8 }, (_, i) => hit(`milk note ${i}`));
    const r = paletteResults({ query: "milk", pages: [], blocks, actions: [] });
    expect(r.items.filter((i) => i.kind === "block")).toHaveLength(6);
    expect(r.hidden.blocks).toBe(2);
  });
  it("hides nothing when a section sits exactly at its limit", () => {
    const pages = [page("pa1"), page("pa2")];
    const blocks = Array.from({ length: 6 }, (_, i) => hit(`pa note ${i}`));
    const tasks = Array.from({ length: 5 }, (_, i) => hit(`TODO pa ${i}`));
    const actions = Array.from({ length: 5 }, (_, i) => act(`pa action ${i}`));
    const r = paletteResults({ query: "pa", pages, blocks: [...tasks, ...blocks], actions, limits: { pages: 2 } });
    expect(r.items).toHaveLength(2 + 5 + 6 + 5);
    expect(r.hidden).toEqual({ pages: 0, tasks: 0, blocks: 0, actions: 0 });
  });
  it("reports actions past the limit after the command prefix", () => {
    const actions = Array.from({ length: 7 }, (_, i) => act(`Do ${i}`));
    const r = paletteResults({ query: ">", pages: [], blocks: [], actions });
    expect(r.items).toHaveLength(5);
    expect(r.hidden.actions).toBe(2);
  });
  it("splits task hits from block hits by the shared taskState", () => {
    const r = paletteResults({
      query: "milk",
      pages: [],
      blocks: [hit("notes on milk"), hit("TODO buy milk"), hit("[x] milk run")],
      actions: [],
    });
    expect(r.items.map(sectionOf)).toEqual(["Tasks", "Tasks", "Blocks"]);
    expect(r.items[0]).toMatchObject({ kind: "task", state: "todo" });
    expect(r.items[1]).toMatchObject({ kind: "task", state: "done" });
  });
  it("sectionOf names each kind", () => {
    expect(sectionOf({ kind: "task", hit: hit("TODO x"), state: "todo" })).toBe("Tasks");
  });
});
