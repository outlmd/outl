import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  BlockSuggestPopup,
  EmojiSuggestPopup,
  RefSuggestPopup,
  SlashCommandPopup,
} from "./SuggestPopups";
import type { PageMeta } from "@outl/shared/api/types";
import type { BlockHit, EmojiHit } from "@outl/shared/api/commands";
import type { PluginCommand } from "@outl/shared/api/plugins";

/**
 * The four popups were four near-identical copies inside `BlockRow.tsx`
 * and had no test at all. They share one shell now, so the parts a
 * caller depends on are pinned here: the popup disappears when there is
 * nothing to suggest, each keeps its own width, the active row is the
 * highlighted one, and a pick arrives via `mousedown` with the default
 * prevented — the whole reason the pattern isn't `onClick` is that a
 * click blurs the textarea and commits the edit before the pick lands.
 */

let dispose: (() => void) | undefined;

function mount(el: () => ReturnType<typeof RefSuggestPopup>): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(el, host);
  return host;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

const page = (slug: string, over: Partial<PageMeta> = {}): PageMeta => ({
  id: `pg-${slug}`,
  slug,
  title: slug,
  kind: "page",
  ...over,
});

describe("SuggestPopups — the shared shell", () => {
  it("renders nothing while there is nothing to suggest", () => {
    const host = mount(() => (
      <RefSuggestPopup
        items={[]}
        activeIndex={0}
        onHover={() => {}}
        onPick={() => {}}
      />
    ));
    expect(host.querySelector('[role="listbox"]')).toBeNull();
  });

  it("marks only the active row selected and highlighted", () => {
    const host = mount(() => (
      <RefSuggestPopup
        items={[page("alpha"), page("beta"), page("gamma")]}
        activeIndex={1}
        onHover={() => {}}
        onPick={() => {}}
      />
    ));
    const rows = Array.from(host.querySelectorAll('[role="option"]'));
    expect(rows.map((r) => r.getAttribute("aria-selected"))).toEqual([
      "false",
      "true",
      "false",
    ]);
    const active = rows[1].querySelector("button");
    expect(active?.className).toContain("bg-(--color-outl-accent)");
    expect(rows[0].querySelector("button")?.className).not.toContain(
      "bg-(--color-outl-accent)",
    );
  });

  it("picks on mousedown with the default prevented, not on click", () => {
    const onPick = vi.fn();
    const host = mount(() => (
      <RefSuggestPopup
        items={[page("alpha")]}
        activeIndex={0}
        onHover={() => {}}
        onPick={onPick}
      />
    ));
    const btn = host.querySelector("button") as HTMLButtonElement;

    btn.click();
    expect(onPick).not.toHaveBeenCalled();

    const ev = new MouseEvent("mousedown", {
      bubbles: true,
      cancelable: true,
    });
    btn.dispatchEvent(ev);
    expect(onPick).toHaveBeenCalledWith(page("alpha"));
    expect(ev.defaultPrevented).toBe(true);
  });

  it("reports the hovered index", () => {
    const onHover = vi.fn();
    const host = mount(() => (
      <RefSuggestPopup
        items={[page("alpha"), page("beta")]}
        activeIndex={0}
        onHover={onHover}
        onPick={() => {}}
      />
    ));
    const rows = host.querySelectorAll('[role="option"] button');
    rows[1].dispatchEvent(new MouseEvent("mouseenter", { bubbles: false }));
    expect(onHover).toHaveBeenCalledWith(1);
  });
});

describe("SuggestPopups — per-popup rows", () => {
  it("RefSuggestPopup shows the journal slug and falls back on the icon", () => {
    const host = mount(() => (
      <RefSuggestPopup
        items={[
          page("2026-09-28", { kind: "journal", title: "September 28th" }),
          page("notes", { icon: "🧠" }),
        ]}
        activeIndex={0}
        onHover={() => {}}
        onPick={() => {}}
      />
    ));
    expect(host.querySelector('[role="listbox"]')?.className).toContain("w-72");
    const rows = host.querySelectorAll('[role="option"]');
    expect(rows[0].textContent).toContain("2026-09-28");
    expect(rows[0].textContent).toContain("📅");
    expect(rows[1].textContent).toContain("🧠");
    expect(rows[1].textContent).toContain("notes");
  });

  it("BlockSuggestPopup shows the snippet and page, never the handle", () => {
    const hit: BlockHit = {
      handle: "blk-abc123",
      text: "ship the ratchet",
      source_slug: "work/plan",
    };
    const host = mount(() => (
      <BlockSuggestPopup
        items={[hit]}
        activeIndex={0}
        onHover={() => {}}
        onPick={() => {}}
      />
    ));
    expect(host.querySelector('[role="listbox"]')?.className).toContain("w-96");
    const row = host.querySelector('[role="option"]');
    expect(row?.textContent).toContain("ship the ratchet");
    expect(row?.textContent).toContain("work/plan");
    expect(row?.textContent).not.toContain("blk-abc123");
  });

  it("BlockSuggestPopup labels an empty block rather than an empty row", () => {
    const hit: BlockHit = { handle: "blk-0", text: "", source_slug: "inbox" };
    const host = mount(() => (
      <BlockSuggestPopup
        items={[hit]}
        activeIndex={0}
        onHover={() => {}}
        onPick={() => {}}
      />
    ));
    expect(host.querySelector('[role="option"]')?.textContent).toContain(
      "(empty block)",
    );
  });

  it("EmojiSuggestPopup shows the glyph and the literal shortcode", () => {
    const hit: EmojiHit = { shortcode: "rocket", glyph: "🚀", score: 1 };
    const host = mount(() => (
      <EmojiSuggestPopup
        items={[hit]}
        activeIndex={0}
        onHover={() => {}}
        onPick={() => {}}
      />
    ));
    const row = host.querySelector('[role="option"]');
    expect(row?.textContent).toContain("🚀");
    expect(row?.textContent).toContain(":rocket:");
  });

  it("SlashCommandPopup shows the typed id and the human title", () => {
    const cmd: PluginCommand = {
      plugin_id: "outl-workspace-stats",
      command_id: "stats",
      title: "Workspace stats",
    };
    const host = mount(() => (
      <SlashCommandPopup
        items={[cmd]}
        activeIndex={0}
        onHover={() => {}}
        onPick={() => {}}
      />
    ));
    const row = host.querySelector('[role="option"]');
    expect(row?.textContent).toContain("/stats");
    expect(row?.textContent).toContain("Workspace stats");
  });
});
