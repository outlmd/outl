import { render } from "solid-js/web";
import { describe, expect, it, vi } from "vitest";

import type { TableView } from "../api/table";
import { MarkdownTable } from "./MarkdownTable";

function mount(node: () => unknown) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(node as () => any, host);
  return {
    host,
    dispose: () => {
      dispose();
      host.remove();
    },
  };
}

/** The shape `outl_md::tokenize_table` emits: rectangular, tokenized. */
const table: TableView = {
  aligns: ["none", "right", "center"],
  header: [
    [{ kind: "plain", value: "Route" }],
    [{ kind: "plain", value: "Pax" }],
    [{ kind: "plain", value: "Owner" }],
  ],
  rows: [
    [
      [{ kind: "plain", value: "SP → RJ" }],
      [{ kind: "plain", value: "1203" }],
      [{ kind: "ref", value: "avelino" }],
    ],
    [[{ kind: "plain", value: "BH" }], [{ kind: "plain", value: "12" }], []],
  ],
};

describe("MarkdownTable", () => {
  it("renders one row per body row and one cell per column", () => {
    const m = mount(() => <MarkdownTable table={table} />);
    expect(m.host.querySelectorAll("thead th")).toHaveLength(3);
    expect(m.host.querySelectorAll("tbody tr")).toHaveLength(2);
    expect(m.host.querySelectorAll("tbody tr:first-child td")).toHaveLength(3);
    m.dispose();
  });

  it("applies each column's alignment", () => {
    const m = mount(() => <MarkdownTable table={table} />);
    const cells = m.host.querySelectorAll("tbody tr:first-child td");
    // `none` reads as left — the on-disk delimiter keeps the distinction.
    expect(cells[0].className).toContain("text-left");
    expect(cells[1].className).toContain("text-right");
    expect(cells[2].className).toContain("text-center");
    m.dispose();
  });

  it("renders a cell's inline markdown, not its source text", () => {
    // The whole reason the backend tokenizes cells: a `[[ref]]` inside
    // a table has to render like a ref anywhere else.
    const clicked = vi.fn();
    const m = mount(() => (
      <MarkdownTable table={table} onRefClick={clicked} />
    ));
    const body = m.host.querySelector("tbody") as HTMLElement;
    expect(body.textContent).toContain("avelino");
    expect(body.textContent).not.toContain("[[");
    m.dispose();
  });

  it("scrolls horizontally instead of stretching the outline", () => {
    // What keeps a wide table usable on a phone.
    const m = mount(() => <MarkdownTable table={table} />);
    const wrap = m.host.firstChild as HTMLElement;
    expect(wrap.className).toContain("overflow-x-auto");
    m.dispose();
  });

  it("renders an empty padded cell without collapsing the row", () => {
    const m = mount(() => <MarkdownTable table={table} />);
    const cells = m.host.querySelectorAll("tbody tr:nth-child(2) td");
    expect(cells).toHaveLength(3);
    expect(cells[2].textContent).toBe("");
    m.dispose();
  });

  it("drops into the editor on tap when the host wires onEdit", () => {
    const edit = vi.fn();
    const m = mount(() => <MarkdownTable table={table} onEdit={edit} />);
    const wrap = m.host.firstChild as HTMLElement;
    expect(wrap.className).toContain("cursor-text");
    wrap.click();
    expect(edit).toHaveBeenCalledTimes(1);
    m.dispose();
  });

  it("stays inert with no onEdit", () => {
    // Read-only contexts (a backlink, an embedded subtree) must not
    // look clickable.
    const m = mount(() => <MarkdownTable table={table} />);
    const wrap = m.host.firstChild as HTMLElement;
    expect(wrap.className).not.toContain("cursor-text");
    m.dispose();
  });

  it("following a ref does not also open the editor", () => {
    // `MarkdownInline` stops propagation on a clickable token; without
    // that, tapping a link in a cell would navigate *and* start an edit.
    const edit = vi.fn();
    const ref = vi.fn();
    const m = mount(() => (
      <MarkdownTable table={table} onEdit={edit} onRefClick={ref} />
    ));
    const link = m.host.querySelector('[role="button"]') as HTMLElement;
    link.click();
    expect(ref).toHaveBeenCalledTimes(1);
    expect(edit).not.toHaveBeenCalled();
    m.dispose();
  });
});
