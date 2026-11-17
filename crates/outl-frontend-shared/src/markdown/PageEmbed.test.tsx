/**
 * The `![[page]]` render, driven through `<MarkdownInline />` rather
 * than through `<PageEmbedTitle />` directly: what has to hold is that
 * the *token* reaches the right render, and a test that mounts the
 * component alone would still pass with the `pageembed` case deleted
 * from the switch.
 */

import { render } from "solid-js/web";
import { describe, expect, it, vi } from "vitest";

import type { InlineToken, PageEmbedContent } from "../api/types";
import { MarkdownInline } from "./MarkdownInline";
import type { PageEmbedMap } from "./PageEmbed";

// `MarkdownInline`'s `image` branch resolves local assets through this
// wrapper; mocking it keeps `@tauri-apps/*` out of the test run.
vi.mock("../api/commands", () => ({
  readAssetDataUrl: () => Promise.resolve("data:image/png;base64,QUJD"),
}));

function page(slug: string, title: string): PageEmbedContent {
  return { slug, title, blocks: [] };
}

function mount(
  tokens: InlineToken[],
  pageEmbeds?: PageEmbedMap,
  onRefClick?: (target: string) => void,
) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(
    () => (
      <MarkdownInline
        tokens={tokens}
        variant="inline"
        pageEmbeds={pageEmbeds}
        onRefClick={onRefClick}
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

describe("MarkdownInline — page embeds", () => {
  const tokens: InlineToken[] = [{ kind: "pageembed", value: "Projeto X" }];

  it("renders the resolved page's title", () => {
    const m = mount(tokens, { "Projeto X": page("projeto-x", "Projeto X") });
    // `↳ ` is the same marker the TUI's chip and the rows below carry:
    // "this content lives elsewhere".
    expect(m.host.textContent).toBe("Projeto X");
    // The literal is the *unresolved* render; seeing it here would mean
    // the map was keyed by slug instead of by the name in the token.
    expect(m.host.textContent).not.toContain("![[");
    m.dispose();
  });

  it("renders a title that differs from the name written in the token", () => {
    // What proves the title comes from the reply and not from `value`.
    const m = mount([{ kind: "pageembed", value: "projeto-x" }], {
      "projeto-x": page("projeto-x", "Projeto X — Q3"),
    });
    expect(m.host.textContent).toBe("Projeto X — Q3");
    m.dispose();
  });

  it("renders the literal `![[name]]` when the name did not resolve", () => {
    // Covers both "no such page" and "the resolve has not answered
    // yet": a page embed is never blank, because the user wrote that
    // name and it has to stay on screen.
    const m = mount(tokens, {});
    expect(m.host.textContent).toBe("![[Projeto X]]");
    m.dispose();
  });

  it("renders the literal when the client passed no map at all", () => {
    const m = mount(tokens);
    expect(m.host.textContent).toBe("![[Projeto X]]");
    m.dispose();
  });

  it("follows the embed through onRefClick, by name", () => {
    // A page embed *is* a page reference, so it opens through the same
    // handler `[[ref]]` uses, with the name inside the brackets.
    const onRefClick = vi.fn();
    const m = mount(
      tokens,
      { "Projeto X": page("projeto-x", "Projeto X") },
      onRefClick,
    );
    m.host.querySelector<HTMLElement>("[role='button']")?.click();
    expect(onRefClick).toHaveBeenCalledWith("Projeto X");
    m.dispose();
  });

  it("leaves the title inert when no handler is wired", () => {
    // Read-only contexts get text, never a control that does nothing.
    const m = mount(tokens, { "Projeto X": page("projeto-x", "Projeto X") });
    expect(m.host.querySelector("[role='button']")).toBeNull();
    m.dispose();
  });

  it("draws both states as a plain inline span, so the swap cannot reflow", () => {
    // The two states swap after the first paint, inside a block the
    // user may be typing in. An atomic inline box (an `inline-flex`
    // wrapper, an `<svg>` icon, a `block`) can be taller than the
    // line's strut, so the carrying row would change height when a
    // resolve lands and every row below the caret would jump. Nothing
    // in `tsc` or in a render assertion on text can see that, which is
    // why the shape itself is pinned.
    for (const map of [undefined, { "Projeto X": page("projeto-x", "Projeto X") }]) {
      const m = mount(tokens, map);
      expect(m.host.querySelector("svg")).toBeNull();
      const spans = [...m.host.querySelectorAll("span")];
      expect(spans.length).toBeGreaterThan(0);
      for (const el of spans) {
        expect(el.className).not.toContain("flex");
        expect(el.className).not.toContain("block");
      }
      m.dispose();
    }
  });

  it("resolves a page embed nested inside bold", () => {
    // `collectPageEmbedNames` descends into `inner`, so the renderer
    // has to forward the map there — otherwise the walk collects a
    // name whose render can never use it.
    const m = mount(
      [{ kind: "bold", inner: [{ kind: "pageembed", value: "Projeto X" }] }],
      { "Projeto X": page("projeto-x", "Projeto X") },
    );
    expect(m.host.textContent).toBe("Projeto X");
    m.dispose();
  });

  it("keeps surrounding prose and its refs intact", () => {
    const onRefClick = vi.fn();
    const m = mount(
      [
        { kind: "plain", value: "see " },
        { kind: "pageembed", value: "Projeto X" },
        { kind: "plain", value: " and " },
        { kind: "ref", value: "Outro" },
      ],
      { "Projeto X": page("projeto-x", "Projeto X") },
      onRefClick,
    );
    expect(m.host.textContent).toBe("see Projeto X and Outro");
    const buttons = m.host.querySelectorAll<HTMLElement>("[role='button']");
    expect(buttons.length).toBe(2);
    buttons[1].click();
    expect(onRefClick).toHaveBeenCalledWith("Outro");
    m.dispose();
  });
});
