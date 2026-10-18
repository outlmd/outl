import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";
import { setDiagramsMode } from "@outl/shared/diagram";

import { BlockBody } from "./BlockBody";

const registry = vi.hoisted(() => ({
  map: new Map<string, { plugin_id: string; lang: string }>(),
}));

vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  transformerFor: (lang: string) => registry.map.get(lang) ?? null,
  runTransform: async () => ({ kind: "text", content: "from plugin" }),
}));

let dispose: (() => void) | undefined;

function block(text: string): BlockNode {
  return {
    id: "b1",
    text,
    todo: null,
    tokens: [{ kind: "plain", value: text }],
    collapsed: false,
    properties: [],
    children: [],
  };
}

function mount(text: string) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BlockBody
        block={block(text)}
        editing={false}
        draftText={() => text}
        depth={0}
        hasChildren={false}
        onToggleCollapse={() => {}}
        onStartEdit={() => {}}
        onDraftChange={() => {}}
        onCommitEdit={() => {}}
        onToggleTodo={() => {}}
        onLongPress={() => {}}
      />
    ),
    host,
  );
  return host;
}

async function waitFor(pred: () => boolean) {
  const start = Date.now();
  while (Date.now() - start < 8000) {
    if (pred()) return;
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error(`timed out. body=${document.body.innerHTML.slice(0, 400)}`);
}

afterEach(() => {
  registry.map.clear();
  setDiagramsMode("pele");
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

const FENCE = "```mermaid\nflowchart LR\n  A[Start] --> B[End]\n```";

describe("mobile fence priority", () => {
  it("renders a mermaid fence with Pele", async () => {
    mount(FENCE);
    await waitFor(() => document.querySelector("svg") !== null);
  });

  it("falls back to highlighted source when diagrams are off", async () => {
    setDiagramsMode("off");
    mount(FENCE);
    await waitFor(() => document.querySelector("code") !== null);
    expect(document.querySelector("svg")).toBeNull();
  });

  it("lets a plugin transformer win over Pele", async () => {
    registry.map.set("mermaid", { plugin_id: "p", lang: "mermaid" });
    mount(FENCE);
    await waitFor(() => document.body.textContent?.includes("from plugin") === true);
    expect(document.querySelector("svg")).toBeNull();
  });

  it("keeps an unsupported language as source", async () => {
    mount("```zenuml\nAlice -> Bob: hi\n```");
    await waitFor(() => document.querySelector("code") !== null);
    expect(document.querySelector("svg")).toBeNull();
  });
});
