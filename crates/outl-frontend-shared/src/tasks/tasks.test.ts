/**
 * The grouping both task lists render. It was two client-local memos
 * until they were caught with the same bug — folding only adjacent rows,
 * so a page with a `DOING` and a `TODO` got two headings.
 *
 * The backend's order is the input here and is never re-derived: these
 * cases fix what grouping does to an order, not what the order is.
 */

import { describe, expect, it } from "vitest";

import { groupTasksByPage } from "./index";

const row = (source_slug: string, text: string) => ({ source_slug, text });

describe("groupTasksByPage", () => {
  it("gives each page one section, in order of first appearance", () => {
    const groups = groupTasksByPage([
      row("2026-10-23", "review PR"),
      row("2026-10-22", "write RFC"),
      row("2026-10-23", "reply to the thread"),
    ]);
    expect(groups.map((g) => g.slug)).toEqual(["2026-10-23", "2026-10-22"]);
  });

  it("collects a page's non-adjacent rows under its first section", () => {
    // The shape the backend really produces: all started tasks, then all
    // TODOs, so a page holding both arrives as two runs.
    const groups = groupTasksByPage([
      row("2026-10-23", "review PR"),
      row("2026-10-22", "write RFC"),
      row("2026-10-23", "reply to the thread"),
      row("2026-10-23", "book the flight"),
    ]);
    expect(groups).toHaveLength(2);
    expect(groups[0].tasks.map((t) => t.text)).toEqual([
      "review PR",
      "reply to the thread",
      "book the flight",
    ]);
    expect(groups[1].tasks.map((t) => t.text)).toEqual(["write RFC"]);
  });

  it("keeps a page where its first row put it, rather than re-ranking", () => {
    // `zeta` leads because its row came first. Sorting by slug, or by how
    // many rows a page has, would both move it.
    const groups = groupTasksByPage([
      row("zeta", "one"),
      row("alpha", "two"),
      row("alpha", "three"),
    ]);
    expect(groups.map((g) => g.slug)).toEqual(["zeta", "alpha"]);
  });

  it("preserves each page's own row order", () => {
    const groups = groupTasksByPage([
      row("p", "first"),
      row("q", "other page"),
      row("p", "second"),
    ]);
    expect(groups[0].tasks.map((t) => t.text)).toEqual(["first", "second"]);
  });

  it("returns nothing for no tasks", () => {
    expect(groupTasksByPage([])).toEqual([]);
  });

  it("groups one page's rows into a single section", () => {
    const groups = groupTasksByPage([row("p", "a"), row("p", "b")]);
    expect(groups).toHaveLength(1);
    expect(groups[0]).toEqual({
      slug: "p",
      tasks: [row("p", "a"), row("p", "b")],
    });
  });
});
