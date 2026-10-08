import { Show } from "solid-js";

import type { BlockNode } from "@outl/shared/api/types";
import { detectFence } from "@outl/shared/highlight";
import {
  EmbeddedSubtree,
  MarkdownInline,
  MarkdownTable,
  splitQuote,
  stripQuoteFromTokens,
} from "@outl/shared/markdown";
import { embedOnlyHandle } from "@outl/shared/outline";

import { appState, setAppState } from "../lib/store";
import type { BlockCallbacks } from "./block-callbacks";
import { CodeFenceView } from "./CodeFenceView";
import { PropertyEditor } from "./PropertyEditor";

/**
 * A block in read mode — the rendered half of a row.
 *
 * One of four shapes, decided by what the backend says the block *is*,
 * never by anything this file re-parses:
 *
 * 1. a fenced code block → `<CodeFenceView />` (`detectFence`);
 * 2. a markdown table → `<MarkdownTable />` (`block.table`, issue #329);
 * 3. nothing but an embed → the prose line plus the resolved subtree
 *    beneath it (`embedOnlyHandle`);
 * 4. ordinary prose → `<MarkdownInline />` over the backend's tokens.
 *
 * The branches are flat, not nested, because they are mutually
 * exclusive and each one answers "what did the user write here". The
 * blockquote chrome is deliberately *not* here — it wraps this
 * component one level up, in `<BlockRow />`, so the outline bullet
 * stays outside the quote.
 *
 * Every shape routes a click back through the same `onStartEdit`: a
 * table has no cell editor, and a fence has no fence editor. Editing
 * anything means editing the raw markdown.
 */
export function BlockBody(props: {
  block: BlockNode;
  cb: BlockCallbacks;
  /** Enter edit mode on this block. One handler for every "edit this"
   *  affordance — the fence's Edit button, a click on a table, a click
   *  on prose. */
  onStartEdit: () => void;
}) {
  const fence = detectFence(props.block.text);
  if (fence) {
    return (
      <CodeFenceView
        blockId={props.block.id}
        language={fence.language}
        body={fence.body}
        onEdit={props.onStartEdit}
        onRun={() => props.cb.onRunCodeBlock(props.block.id)}
        onOpenPage={props.cb.onOpenPage}
      />
    );
  }
  if (props.block.table) {
    // Fence-branch shape: the backend decided it is a table, and a
    // click edits the raw markdown (no cell editor, by design —
    // issue #329).
    return (
      <MarkdownTable
        table={props.block.table}
        variant="inline"
        onRefClick={props.cb.onRefClick}
        onTagClick={props.cb.onTagClick}
        onLinkClick={props.cb.onLinkClick}
        embeds={appState.embeds}
        onEdit={props.onStartEdit}
      />
    );
  }
  // The chrome lives on the wrapper a level up — here we just strip
  // the `> ` from the tokens so the marker doesn't double-paint.
  const split = splitQuote(props.block.text);
  const renderedTokens = split.quoted
    ? stripQuoteFromTokens(props.block.tokens)
    : props.block.tokens;
  const hasContent = split.quoted
    ? split.body.length > 0
    : Boolean(props.block.text);
  // When the block is *only* an embed (`!((blk-…))` with no surrounding
  // prose), expand the resolved source subtree below the `↳ text` line —
  // read-only, mirroring the TUI's child expansion.
  const embedHandle = embedOnlyHandle(props.block.tokens);
  const embedded = embedHandle ? appState.embeds[embedHandle] : undefined;
  return (
    <>
      <div
        class="cursor-text whitespace-pre-wrap break-words"
        onClick={props.onStartEdit}
      >
        <Show
          when={renderedTokens && renderedTokens.length > 0}
          fallback={
            <span class={!hasContent ? "opacity-30" : ""}>
              {hasContent
                ? split.quoted
                  ? split.body
                  : props.block.text
                : "Click to add text…"}
            </span>
          }
        >
          <MarkdownInline
            tokens={renderedTokens}
            variant="inline"
            blockAssets
            onRefClick={props.cb.onRefClick}
            onTagClick={props.cb.onTagClick}
            onLinkClick={props.cb.onLinkClick}
            embeds={appState.embeds}
          />
        </Show>
      </div>
      {/* `remind::` and friends were invisible here: the chord wrote
          the rule and the block looked untouched. Clicking a chip
          opens the rule for editing. */}
      <PropertyEditor
        properties={props.block.properties}
        onCommit={(key, value) =>
          props.cb.onSetProperty?.(props.block.id, key, value)
        }
        onError={(msg) => setAppState("lastError", msg)}
        addOpen={appState.addPropertyBlockId === props.block.id}
        onAddOpenChange={(open) => {
          if (!open) {
            setAppState("addPropertyBlockId", null);
          }
        }}
        chipClass="rounded bg-(--color-outl-fg)/8 px-1.5 py-0.5 text-xs opacity-70 hover:opacity-100"
        inputClass="rounded border border-(--color-outl-accent)/50 bg-(--color-outl-bg) px-1.5 py-0.5 text-xs outline-none"
      />
      <Show when={embedded?.children?.length}>
        <EmbeddedSubtree
          nodes={embedded!.children}
          embeds={appState.embeds}
        />
      </Show>
    </>
  );
}
