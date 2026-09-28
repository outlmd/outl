import { For, type JSX, Show } from "solid-js";

import type { PageMeta, PluginCommand } from "@outl/shared/api/types";
import type { BlockHit } from "@outl/shared/api/commands";
import type { EmojiHit } from "@outl/shared/api/commands";

/**
 * The floating suggestion lists the desktop shows while the caret sits
 * inside an open trigger — `[[…]]`, `((…))`, `:shortcode`, `/command`.
 *
 * All four are anchored just below the block's textarea and differ only
 * in how one row is drawn, so the shell lives here once and each popup
 * supplies its own row. They were four near-identical copies inside
 * `BlockRow.tsx` until the file-size ratchet forced the split; the
 * duplication was the reason a fix to one (the `onMouseDown` note
 * below) had to be re-applied to the other three by hand.
 *
 * Selection uses `onMouseDown` + `preventDefault` (not `onClick`): a
 * plain click would blur the textarea first, firing its `onBlur`
 * commit and tearing down edit mode before the pick registered.
 * Preventing the default on mousedown keeps focus in the textarea so
 * the accept handler can splice the value and re-park the caret.
 */
function SuggestPopup<T>(props: {
  items: T[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (item: T) => void;
  /** Tailwind width literal. Passed as a literal so the JIT sees it. */
  widthClass: string;
  /** Tailwind gap literal for the row button. */
  gapClass: string;
  row: (item: T) => JSX.Element;
}) {
  return (
    <Show when={props.items.length > 0}>
      <ul
        class={`absolute top-full left-0 z-30 mt-1 max-h-56 overflow-y-auto rounded-md border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 text-[13px] shadow-lg ${props.widthClass}`}
        role="listbox"
      >
        <For each={props.items}>
          {(item, i) => (
            <li role="option" aria-selected={i() === props.activeIndex}>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  props.onPick(item);
                }}
                onMouseEnter={() => props.onHover(i())}
                class={`flex w-full items-center px-2 py-1 text-left ${props.gapClass} ${
                  i() === props.activeIndex
                    ? "bg-(--color-outl-accent) text-(--color-outl-bg)"
                    : "hover:bg-(--color-outl-bg)/50"
                }`}
              >
                {props.row(item)}
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

/**
 * Page suggestions for an open `[[…]]`.
 */
export function RefSuggestPopup(props: {
  items: PageMeta[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (page: PageMeta) => void;
}) {
  return (
    <SuggestPopup
      items={props.items}
      activeIndex={props.activeIndex}
      onHover={props.onHover}
      onPick={props.onPick}
      widthClass="w-72"
      gapClass="gap-1.5"
      row={(page) => (
        <>
          <span aria-hidden="true" class="shrink-0 opacity-70">
            {page.icon || (page.kind === "journal" ? "📅" : "📄")}
          </span>
          <span class="truncate">
            {page.kind === "journal" ? page.slug : page.title}
          </span>
        </>
      )}
    />
  );
}

/**
 * Block suggestions for an open `((…))`. Each row shows the block's
 * text snippet with its hosting page slug dimmed on the right, so the
 * user picks by content — the `blk-XXXXXX` handle it inserts is never
 * shown, it's an internal id, not something the user reasons about.
 */
export function BlockSuggestPopup(props: {
  items: BlockHit[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (hit: BlockHit) => void;
}) {
  return (
    <SuggestPopup
      items={props.items}
      activeIndex={props.activeIndex}
      onHover={props.onHover}
      onPick={props.onPick}
      widthClass="w-96"
      gapClass="gap-2"
      row={(hit) => (
        <>
          <span class="min-w-0 flex-1 truncate">
            {hit.text || "(empty block)"}
          </span>
          <span class="shrink-0 truncate text-[11px] opacity-60">
            {hit.source_slug}
          </span>
        </>
      )}
    />
  );
}

/**
 * Emoji-shortcode suggestions for an open `:shortcode` trigger. The row
 * shows the glyph on the left and the canonical `:shortcode:` form on
 * the right, so the user can scan by glyph but still see the literal
 * that will land on disk.
 */
export function EmojiSuggestPopup(props: {
  items: EmojiHit[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (hit: EmojiHit) => void;
}) {
  return (
    <SuggestPopup
      items={props.items}
      activeIndex={props.activeIndex}
      onHover={props.onHover}
      onPick={props.onPick}
      widthClass="w-72"
      gapClass="gap-2"
      row={(hit) => (
        <>
          <span aria-hidden="true" class="shrink-0 text-base">
            {hit.glyph}
          </span>
          <span class="truncate font-mono text-[12px] opacity-80">
            :{hit.shortcode}:
          </span>
        </>
      )}
    />
  );
}

/**
 * The `/command` slash menu for a block-initial `/` trigger — the
 * desktop's inline equivalent of the TUI slash overlay. Each row shows
 * the command **id** monospaced (what the user types, mirrors `/stats`
 * in the CLI) with the human title dimmed beside it.
 */
export function SlashCommandPopup(props: {
  items: PluginCommand[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (cmd: PluginCommand) => void;
}) {
  return (
    <SuggestPopup
      items={props.items}
      activeIndex={props.activeIndex}
      onHover={props.onHover}
      onPick={props.onPick}
      widthClass="w-72"
      gapClass="gap-2"
      row={(cmd) => (
        <>
          <span class="shrink-0 font-mono text-[12px]">/{cmd.command_id}</span>
          <span class="truncate opacity-70">{cmd.title}</span>
        </>
      )}
    />
  );
}
