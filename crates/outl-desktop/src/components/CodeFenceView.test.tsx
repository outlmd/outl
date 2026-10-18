import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setDiagramsMode } from "@outl/shared/diagram";

import { CodeFenceView } from "./CodeFenceView";

const registry = vi.hoisted(() => ({
  map: new Map<string, { plugin_id: string; lang: string }>(),
}));

vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  transformerFor: (lang: string) => registry.map.get(lang) ?? null,
  runTransform: async () => ({ kind: "text", content: "from plugin" }),
}));

vi.mock("@outl/shared/api/commands", () => ({
  listTemplates: async () => [],
}));

let dispose: (() => void) | undefined;

const MERMAID = "flowchart LR\n  A[Start] --> B[End]";

function mount(language = "mermaid", body = MERMAID) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <CodeFenceView
        blockId="b1"
        language={language}
        body={body}
        onEdit={() => {}}
        onRun={async () => {}}
      />
    ),
    host,
  );
  return host;
}

function buttonText(): string {
  return [...document.querySelectorAll("button")].map((b) => b.textContent ?? "").join(" ");
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

describe("CodeFenceView diagrams", () => {
  it("renders a mermaid fence with Pele and the edit affordance", async () => {
    mount();
    await waitFor(() => document.querySelector("svg") !== null);
    expect(buttonText()).toContain("Edit");
    expect(buttonText()).not.toContain("Run");
  });

  it("falls back to highlighted source when diagrams are off", async () => {
    setDiagramsMode("off");
    mount();
    await waitFor(() => document.querySelector("code") !== null);
    expect(document.querySelector("svg")).toBeNull();
    expect(buttonText()).toContain("Run");
    expect(buttonText()).not.toContain("Edit");
  });

  it("lets a plugin transformer win over Pele", async () => {
    registry.map.set("mermaid", { plugin_id: "p", lang: "mermaid" });
    mount();
    await waitFor(() => document.body.textContent?.includes("from plugin") === true);
    expect(document.querySelector("svg")).toBeNull();
    expect(buttonText()).toContain("Edit");
  });
});
