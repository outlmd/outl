import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

import { DiagramFence } from "./DiagramFence";
import { createPeleRenderer } from "./renderer";

let dispose: (() => void) | undefined;

beforeAll(() => {
  // Pele skips a source update when the host's width is 0. happy-dom
  // never lays the element out, so give every host a width.
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get() {
      return 640;
    },
  });
});

function mount(
  source: string,
  extra?: {
    onOpenPage?: (name: string) => void;
    onLinkClick?: (href: string) => void;
    onRowClick?: () => void;
  },
) {
  return mountLive(source, extra);
}

function mountLive(
  source: string,
  extra?: {
    onOpenPage?: (name: string) => void;
    onLinkClick?: (href: string) => void;
    onRowClick?: () => void;
  },
) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const [text, setText] = createSignal(source);
  dispose = render(
    () => (
      <div onClick={() => extra?.onRowClick?.()}>
        <DiagramFence
          source={text()}
          onOpenPage={extra?.onOpenPage}
          onLinkClick={extra?.onLinkClick}
          fallback={
            <pre data-fallback>
              <code>{text()}</code>
            </pre>
          }
        />
      </div>
    ),
    host,
  );
  return setText;
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
    expect(line.textContent).not.toMatch(/\n/);
    expect(line.textContent).not.toMatch(/Parse error on line/i);
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
    const onRowClick = vi.fn();
    mount("flowchart LR\n  A[Journal]:::internal-link --> B[Other]", {
      onOpenPage,
      onLinkClick,
      onRowClick,
    });
    await waitFor(() => document.querySelector("a.internal-link") !== null);
    const anchor = document.querySelector("a.internal-link") as HTMLAnchorElement;
    anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(onOpenPage).toHaveBeenCalledWith("Journal");
    expect(onLinkClick).not.toHaveBeenCalled();
    expect(onRowClick).not.toHaveBeenCalled();
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

  it("reports a syntax error when the source changes without remounting", async () => {
    const setSource = mountLive(FLOW);
    await waitFor(() => document.querySelector("svg") !== null);
    setSource("flowchart LR\n  A --> ");
    await waitFor(() => document.querySelector("[data-diagram-error]") !== null);
    const line = document.querySelector("[data-diagram-error]")!;
    expect(line.textContent).toMatch(/^line \d+, col \d+: /);
    expect(line.textContent).not.toMatch(/\n/);
    expect(line.textContent).not.toMatch(/line \d+.*line \d+/);
    expect(document.querySelector("[data-fallback]")).not.toBeNull();
    expect(document.querySelector(".pele-error")).toBeNull();
  });

  it("refreshes links when the source changes to another valid diagram", async () => {
    const onOpenPage = vi.fn();
    const setSource = mountLive(FLOW, { onOpenPage });
    await waitFor(() => document.querySelector("svg") !== null);
    expect(document.querySelector("a.internal-link")).toBeNull();
    setSource("flowchart LR\n  A[Notes]:::internal-link --> B[Other]");
    await waitFor(() => document.querySelector("a.internal-link") !== null);
    const anchor = document.querySelector("a.internal-link") as HTMLAnchorElement;
    anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(onOpenPage).toHaveBeenCalledWith("Notes");
  });

  it("keeps the latest diagram when the source changes twice before the first mount resolves", async () => {
    const setSource = mountLive("flowchart LR\n  A[One] --> B[Two]");
    setSource("flowchart LR\n  C[Three] --> D[Four]");
    setSource("flowchart LR\n  E[Last] --> F[Six]");
    await waitFor(() => document.querySelector("svg")?.textContent?.includes("Last") ?? false);
    await new Promise((r) => setTimeout(r, 200));
    const svg = document.querySelector("svg");
    expect(svg).not.toBeNull();
    expect(svg?.textContent).toContain("Last");
    expect(document.querySelector("[data-diagram-error]")).toBeNull();
  });

  it("lets a link click reach the row when no navigation handler is set", async () => {
    const onRowClick = vi.fn();
    mount("flowchart LR\n  A[Journal]:::internal-link --> B[Other]", { onRowClick });
    await waitFor(() => document.querySelector("a.internal-link") !== null);
    const anchor = document.querySelector("a.internal-link") as HTMLAnchorElement;
    anchor.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
    expect(onRowClick).toHaveBeenCalled();
  });

  it("paints diagram colours through the CSSOM", async () => {
    mount(FLOW);
    await waitFor(() => document.querySelector("svg") !== null);
    const host = document.querySelector(".outl-diagram")!;
    const svg = host.querySelector("svg") as SVGElement;
    // `cssText` is what a style-src nonce allows, and the DOM reflects
    // that assignment back into the style attribute. The colour is
    // readable here, through the CSSOM.
    expect(svg.style.getPropertyValue("--_fg").trim()).not.toBe("");
    for (const el of host.querySelectorAll("*")) {
      if (!(el instanceof HTMLElement) && !(el instanceof SVGElement)) continue;
      const raw = el.getAttribute("style");
      if (raw !== null) expect(el.style.cssText.length).toBeGreaterThan(0);
    }
    expect(host.querySelector("script, foreignObject")).toBeNull();
    for (const el of host.querySelectorAll("*")) {
      for (const name of el.getAttributeNames()) {
        expect(name.toLowerCase().startsWith("on")).toBe(false);
      }
    }
  });
});

describe("a stale mount", () => {
  it("does not clear a diagram a newer mount already painted", async () => {
    const renderer = createPeleRenderer();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const older = renderer.mount(host, "flowchart LR\n  A[One] --> B[Two]", {});
    const newer = renderer.mount(host, "flowchart LR\n  E[Last] --> F[Six]", {});
    const [first, second] = await Promise.all([older, newer]);
    first.destroy();
    expect(host.textContent).toContain("Last");
    expect(host.querySelector("svg")).not.toBeNull();
    second.destroy();
    expect(host.querySelector("svg")).toBeNull();
  });
});
