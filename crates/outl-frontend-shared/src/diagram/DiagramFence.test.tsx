import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DiagramFence } from "./DiagramFence";

let dispose: (() => void) | undefined;

function mount(
  source: string,
  extra?: {
    onOpenPage?: (name: string) => void;
    onLinkClick?: (href: string) => void;
  },
) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <DiagramFence
        source={source}
        onOpenPage={extra?.onOpenPage}
        onLinkClick={extra?.onLinkClick}
        fallback={
          <pre data-fallback>
            <code>{source}</code>
          </pre>
        }
      />
    ),
    host,
  );
}

async function waitFor(pred: () => boolean, ms = 8000): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < ms) {
    if (pred()) return;
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error("timed out waiting for the diagram");
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

const FLOW = "flowchart LR\n  A[Start] --> B[End]";

describe("DiagramFence", () => {
  it("renders a mermaid flowchart as SVG", async () => {
    mount(FLOW);
    await waitFor(() => document.querySelector("svg") !== null);
    expect(document.querySelector("[data-fallback]")).toBeNull();
    expect(document.querySelector("[data-diagram-error]")).toBeNull();
  });

  it("shows the source and a line/column error as text, never HTML", async () => {
    mount("flowchart LR\n  A -->\n  <script>alert(1)</script>");
    await waitFor(() => document.querySelector("[data-diagram-error]") !== null);
    const line = document.querySelector("[data-diagram-error]")!;
    expect(line.textContent).toMatch(/^line \d+, col \d+: /);
    expect(line.childElementCount).toBe(0);
    expect(line.querySelector("script")).toBeNull();
    expect(document.querySelector("[data-fallback]")).not.toBeNull();
    expect(document.querySelector(".pele-error")).toBeNull();
  });

  it("falls back to source for a zenuml body with no error line", async () => {
    mount("zenuml\n  Alice -> Bob: hi");
    await waitFor(() => document.querySelector("[data-fallback]") !== null);
    await new Promise((r) => setTimeout(r, 50));
    expect(document.querySelector("svg")).toBeNull();
    expect(document.querySelector("[data-diagram-error]")).toBeNull();
  });

  it("falls back to source when the fence is over the source limit", async () => {
    mount(`flowchart LR\n  A[${"x".repeat(50_001)}]`);
    await waitFor(() => document.querySelector("[data-fallback]") !== null);
    await new Promise((r) => setTimeout(r, 30));
    expect(document.querySelector("svg")).toBeNull();
    expect(document.querySelector("[data-diagram-error]")).toBeNull();
  });

  it("routes an internal link to page navigation", async () => {
    const onOpenPage = vi.fn();
    const onLinkClick = vi.fn();
    mount("flowchart LR\n  A[Journal]:::internal-link --> B[Other]", {
      onOpenPage,
      onLinkClick,
    });
    await waitFor(() => document.querySelector("a.internal-link") !== null);
    const anchor = document.querySelector("a.internal-link") as HTMLAnchorElement;
    anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(onOpenPage).toHaveBeenCalledWith("Journal");
    expect(onLinkClick).not.toHaveBeenCalled();
  });

  it("keeps http, https and mailto and drops javascript and tel", async () => {
    const onLinkClick = vi.fn();
    mount(
      [
        "flowchart LR",
        "  A[Mail]",
        "  B[Site]",
        "  C[Phone]",
        "  D[Bad]",
        '  click A "mailto:a@example.com"',
        '  click B "https://example.com"',
        '  click C "tel:+15551212"',
        '  click D "javascript:alert(1)"',
      ].join("\n"),
      { onLinkClick },
    );
    await waitFor(() => document.querySelector("svg") !== null);
    const hrefs = [...document.querySelectorAll("a")].map((a) => a.getAttribute("href") ?? "");
    expect(hrefs.some((h) => h.startsWith("mailto:"))).toBe(true);
    expect(hrefs.some((h) => h.startsWith("https:"))).toBe(true);
    expect(hrefs.some((h) => h.startsWith("javascript:"))).toBe(false);
    expect(hrefs.some((h) => h.startsWith("tel:"))).toBe(false);

    const site = [...document.querySelectorAll("a")].find((a) =>
      (a.getAttribute("href") ?? "").startsWith("https:"),
    ) as HTMLAnchorElement;
    site.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(onLinkClick).toHaveBeenCalledTimes(1);
    expect(String(onLinkClick.mock.calls[0][0])).toMatch(/^https:/);
  });
});
