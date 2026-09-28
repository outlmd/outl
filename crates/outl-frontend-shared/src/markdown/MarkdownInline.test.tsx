import { render } from "solid-js/web";
import { describe, expect, it, vi } from "vitest";

import type { InlineToken } from "../api/types";
import { MarkdownInline, type EmbedMap } from "./MarkdownInline";

// The `image` render branch resolves a local `assets/…` asset to a
// `data:` URL through this command. Mock the wrapper so the test never
// touches a real Tauri backend (and the module's `@tauri-apps/*` imports
// are never evaluated).
const readAssetDataUrlMock = vi.fn((_url: string) =>
  Promise.resolve("data:image/png;base64,QUJD"),
);
vi.mock("../api/commands", () => ({
  readAssetDataUrl: (url: string) => readAssetDataUrlMock(url),
}));

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

function mount(
  tokens: InlineToken[],
  variant?: "inline",
  onLinkClick?: (href: string) => void,
  embeds?: EmbedMap,
  blockAssets?: boolean,
) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => (
      <MarkdownInline
        tokens={tokens}
        variant={variant}
        blockAssets={blockAssets}
        onLinkClick={onLinkClick}
        embeds={embeds}
      />
    ),
    host,
  );
  return {
    host,
    dispose: () => {
      dispose();
      host.remove();
    },
  };
}

describe("MarkdownInline — tag rendering", () => {
  /**
   * Regression: the Rust serializer stores the leading `#` in `value`
   * already (see `outl_md::inline::InlineToken::from_borrowed` and the
   * `round_trips_every_variant_into_serializable_form` test in
   * `crates/outl-md/src/inline.rs`). Both renderer variants must
   * therefore emit the `value` verbatim — adding a `#` prefix produces
   * the `##avelino` bug seen on desktop's pretty render.
   */
  it("inline variant does not double-prefix the `#`", () => {
    const tokens: InlineToken[] = [{ kind: "tag", value: "#avelino" }];
    const m = mount(tokens, "inline");
    expect(m.host.textContent).toBe("#avelino");
    expect(m.host.textContent).not.toBe("##avelino");
    m.dispose();
  });

  it("default (mobile) variant does not double-prefix the `#`", () => {
    const tokens: InlineToken[] = [{ kind: "tag", value: "#avelino" }];
    const m = mount(tokens);
    expect(m.host.textContent).toBe("#avelino");
    expect(m.host.textContent).not.toBe("##avelino");
    m.dispose();
  });

  it("renders tags inline with surrounding plain text intact", () => {
    const tokens: InlineToken[] = [
      { kind: "plain", value: "see " },
      { kind: "tag", value: "#code-review" },
      { kind: "plain", value: " for context" },
    ];
    const m = mount(tokens, "inline");
    expect(m.host.textContent).toBe("see #code-review for context");
    m.dispose();
  });
});

describe("MarkdownInline — external link rendering", () => {
  const linkTok: InlineToken[] = [
    { kind: "link", value: "site", href: "https://example.com" },
  ];

  it("fires onLinkClick with the href when a link is clicked", () => {
    const opened: string[] = [];
    const m = mount(linkTok, "inline", (href) => opened.push(href));
    const span = m.host.querySelector('[role="button"]') as HTMLElement;
    expect(span).not.toBeNull();
    expect(span.textContent).toBe("site");
    expect(span.getAttribute("tabindex")).toBe("0");
    span.click();
    expect(opened).toEqual(["https://example.com"]);
    m.dispose();
  });

  it("fires onLinkClick on Enter / Space for keyboard users", () => {
    const opened: string[] = [];
    const m = mount(linkTok, "inline", (href) => opened.push(href));
    const span = m.host.querySelector('[role="button"]') as HTMLElement;
    span.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );
    span.dispatchEvent(
      new KeyboardEvent("keydown", { key: " ", bubbles: true }),
    );
    expect(opened).toEqual([
      "https://example.com",
      "https://example.com",
    ]);
    m.dispose();
  });

  it("renders the label as inert text (no fake button) when no onLinkClick", () => {
    const m = mount(linkTok, "inline");
    expect(m.host.textContent).toBe("site");
    // a11y: an inert link must NOT masquerade as a button or be a tab stop.
    expect(m.host.querySelector('[role="button"]')).toBeNull();
    const span = m.host.querySelector("span") as HTMLElement;
    expect(span.getAttribute("tabindex")).toBeNull();
    expect(() => span.click()).not.toThrow();
    expect(span.className).not.toContain("cursor-pointer");
    m.dispose();
  });
});

describe("MarkdownInline — image / asset rendering (issue #203)", () => {
  it("resolves a local image asset to an <img> via readAssetDataUrl", async () => {
    readAssetDataUrlMock.mockClear();
    const tokens: InlineToken[] = [
      { kind: "image", alt: "diagram", href: "assets/abc.png" },
    ];
    const m = mount(tokens);
    // A local asset has no web origin — its bytes come back as a data URL.
    expect(readAssetDataUrlMock).toHaveBeenCalledWith("assets/abc.png");
    await tick();
    const img = m.host.querySelector("img") as HTMLImageElement;
    expect(img).not.toBeNull();
    expect(img.getAttribute("src")).toBe("data:image/png;base64,QUJD");
    expect(img.getAttribute("alt")).toBe("diagram");
    m.dispose();
  });

  it("uses a remote image href directly (no backend round-trip)", () => {
    readAssetDataUrlMock.mockClear();
    const tokens: InlineToken[] = [
      { kind: "image", alt: "logo", href: "https://example.com/logo.png" },
    ];
    const m = mount(tokens);
    expect(readAssetDataUrlMock).not.toHaveBeenCalled();
    const img = m.host.querySelector("img") as HTMLImageElement;
    expect(img.getAttribute("src")).toBe("https://example.com/logo.png");
    m.dispose();
  });

  it("renders a clickable file chip for a .pdf (no <img>)", () => {
    const opened: string[] = [];
    const tokens: InlineToken[] = [
      { kind: "image", alt: "spec", href: "assets/spec.pdf" },
    ];
    const m = mount(tokens, undefined, (href) => opened.push(href));
    expect(m.host.querySelector("img")).toBeNull();
    const chip = m.host.querySelector('[role="button"]') as HTMLElement;
    expect(chip).not.toBeNull();
    expect(chip.textContent).toContain("spec");
    chip.click();
    expect(opened).toEqual(["assets/spec.pdf"]);
    m.dispose();
  });

  it("refuses a non-http(s) remote image scheme and shows a chip", () => {
    // A `file:`/`data:`/`javascript:` href from synced or imported
    // markdown must never be hit as an <img src>. It degrades to a chip.
    readAssetDataUrlMock.mockClear();
    const tokens: InlineToken[] = [
      { kind: "image", alt: "trap", href: "file:///etc/passwd.png" },
    ];
    const m = mount(tokens);
    expect(m.host.querySelector("img")).toBeNull();
    expect(readAssetDataUrlMock).not.toHaveBeenCalled();
    expect(m.host.textContent).toContain("trap");
    m.dispose();
  });

  it("renders an inline-variant image as a compact chip, not a block <img>", () => {
    // The compact `inline` variant can't hold a block image, so even a
    // valid local asset renders as a chip and skips the byte fetch.
    readAssetDataUrlMock.mockClear();
    const tokens: InlineToken[] = [
      { kind: "image", alt: "diagram", href: "assets/abc.png" },
    ];
    const m = mount(tokens, "inline");
    expect(m.host.querySelector("img")).toBeNull();
    expect(readAssetDataUrlMock).not.toHaveBeenCalled();
    expect(m.host.textContent).toContain("diagram");
    m.dispose();
  });

  it("renders a block <img> when variant='inline' and blockAssets is true", async () => {
    readAssetDataUrlMock.mockClear();
    const tokens: InlineToken[] = [
      { kind: "image", alt: "diagram", href: "assets/abc.png" },
    ];
    const m = mount(tokens, "inline", undefined, undefined, true);
    expect(readAssetDataUrlMock).toHaveBeenCalledWith("assets/abc.png");
    await tick();
    const img = m.host.querySelector("img") as HTMLImageElement;
    expect(img).not.toBeNull();
    expect(img.getAttribute("src")).toBe("data:image/png;base64,QUJD");
    expect(img.getAttribute("alt")).toBe("diagram");
    m.dispose();
  });

  it("preserves inline-variant text styling for refs and tags when blockAssets is true", () => {
    const tokens: InlineToken[] = [
      { kind: "ref", value: "project" },
      { kind: "tag", value: "#status" },
    ];
    const m = mount(tokens, "inline", undefined, undefined, true);
    const buttons = m.host.querySelectorAll('[role="button"]');
    const refSpan = Array.from(buttons).find((b) => b.textContent === "project");
    expect(refSpan).toBeDefined();
    expect(refSpan?.className).toContain("text-(--color-outl-ref-link-fg)");
    expect(refSpan?.className).not.toContain("bg-(--color-outl-accent)/12");

    const tagSpan = Array.from(buttons).find((b) => b.textContent === "#status");
    expect(tagSpan).toBeDefined();
    expect(tagSpan?.className).toContain("text-(--color-outl-tag-link-fg)");
    m.dispose();
  });

  it("falls back to a file chip when a local asset fails to load", async () => {
    readAssetDataUrlMock.mockClear();
    readAssetDataUrlMock.mockRejectedValueOnce(new Error("not synced"));
    const opened: string[] = [];
    const tokens: InlineToken[] = [
      { kind: "image", alt: "diagram", href: "assets/abc.png" },
    ];
    const m = mount(tokens, undefined, (href) => opened.push(href));
    await tick();
    expect(m.host.querySelector("img")).toBeNull();
    const chip = m.host.querySelector('[role="button"]') as HTMLElement;
    expect(chip).not.toBeNull();
    expect(chip.textContent).toContain("diagram");
    m.dispose();
  });
});

describe("MarkdownInline — block-ref rendering (issue #147)", () => {
  const handle = "blk-r6s4a1";
  const refTok: InlineToken[] = [{ kind: "blockref", value: handle }];

  function embedMap(status: string | null = null): EmbedMap {
    return {
      [handle]: {
        handle,
        text: "source block text",
        page_slug: "notes",
        status,
        children: [],
      },
    };
  }

  it("renders the resolved text, not the raw handle (inline)", () => {
    const m = mount(refTok, "inline", undefined, embedMap());
    expect(m.host.textContent).toBe("source block text");
    expect(m.host.textContent).not.toContain(handle);
    m.dispose();
  });

  it("renders the resolved text, not the raw handle (pill)", () => {
    const m = mount(refTok, undefined, undefined, embedMap());
    expect(m.host.textContent).toBe("source block text");
    expect(m.host.textContent).not.toContain(handle);
    m.dispose();
  });

  it("prefixes the TODO/DONE marker from the resolved status", () => {
    const todo = mount(refTok, "inline", undefined, embedMap("todo"));
    expect(todo.host.textContent).toBe("☐ source block text");
    todo.dispose();
    const done = mount(refTok, "inline", undefined, embedMap("done"));
    expect(done.host.textContent).toBe("✓ source block text");
    done.dispose();
  });

  it("renders the raw handle chip when the ref is orphan (no embeds)", () => {
    const m = mount(refTok, "inline");
    expect(m.host.textContent).toBe(handle);
    m.dispose();
  });

  it("orphan chip does not regress on the mobile (pill) variant", () => {
    const m = mount(refTok);
    expect(m.host.textContent).toBe(handle);
    m.dispose();
  });
});
