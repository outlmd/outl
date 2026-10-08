import { type Accessor, type Setter, createSignal, onCleanup } from "solid-js";

import type { PageMeta, PluginCommand } from "@outl/shared/api/types";
import {
  type BlockHit,
  type EmojiHit,
  listTemplates,
  openRef,
  pluginList,
  searchBlocks,
  searchEmojis,
  searchPages,
  searchPersons,
} from "@outl/shared/api/commands";
import {
  applyEmojiSuggestion,
  applySlashContext,
  applySuggestion,
  detectEmojiContext,
  detectRefContext,
  detectSlashContext,
  refReplacement,
  withCreateNewPersonCandidate,
} from "@outl/shared/autocomplete";

import { handlePopupNav } from "../lib/popup-nav";
import {
  assetSlashCommands,
  rankSlashCommands,
  templateSlashCommands,
} from "../lib/slash-commands";

/**
 * What the four inline autocomplete popups need from the editor they
 * live inside. Accessors, never values — a destructured prop freezes
 * at first render in Solid, and `textarea` in particular is
 * `undefined` until the editor mounts.
 */
export interface BlockSuggestDeps {
  /** The live textarea, or `undefined` before it mounts. */
  textarea: () => HTMLTextAreaElement | undefined;
  /** Write the draft signal the textarea is bound to. */
  setText: (value: string) => void;
  /** Commit the block. A slash command runs against persisted text, so
   *  the `/stats` literal has to be out of the block first. */
  commit: () => Promise<void>;
  /** Hand a picked plugin command to the parent, which owns the
   *  `pluginRun` round-trip and the view / overlay application. */
  runPluginCommand: (pluginId: string, commandId: string) => Promise<void>;
}

export interface BlockSuggest {
  /** `[[page]]` and `[[@mention]]` hits. */
  suggestions: Accessor<PageMeta[]>;
  suggestIndex: Accessor<number>;
  setSuggestIndex: Setter<number>;
  /** `((block ref))` hits. */
  blockSuggestions: Accessor<BlockHit[]>;
  blockIndex: Accessor<number>;
  setBlockIndex: Setter<number>;
  /** `:shortcode:` hits. */
  emojiSuggestions: Accessor<EmojiHit[]>;
  emojiIndex: Accessor<number>;
  setEmojiIndex: Setter<number>;
  /** Block-initial `/command` hits. */
  slashCommands: Accessor<PluginCommand[]>;
  slashIndex: Accessor<number>;
  setSlashIndex: Setter<number>;
  acceptSuggestion: (page: PageMeta) => void;
  acceptBlockSuggestion: (hit: BlockHit) => void;
  acceptEmojiSuggestion: (hit: EmojiHit) => void;
  acceptSlashCommand: (cmd: PluginCommand) => void;
  /** Recompute the popup from the live textarea state. */
  refresh: () => void;
  /** Close every popup. */
  close: () => void;
  /** The shared popup keyboard contract — `true` when the keystroke
   *  was consumed and the editor's own chords must not also fire. */
  handleKey: (e: KeyboardEvent) => boolean;
}

/**
 * The four inline autocomplete triggers, in one place: `[[page]]`,
 * `((block))`, `:emoji:` and block-initial `/command`.
 *
 * They are one module because they are one question asked four ways —
 * *what is the caret sitting inside, and what replaces that span when
 * the user accepts?* — and because only one of them is ever open at a
 * time, which is the invariant the whole thing rests on: `closeSuggest`
 * empties all four, every fetch clears the others before setting its
 * own, and `handleKey` lets the first non-empty popup consume the key.
 *
 * Detection and span replacement are **not** implemented here. They
 * come from `@outl/shared/autocomplete`, which the TUI and mobile run
 * too; what this file owns is the backend round-trips, their staleness
 * guards, and the popup state.
 *
 * Must be called during component setup — it creates signals and
 * registers `onCleanup`.
 */
export function createBlockSuggest(deps: BlockSuggestDeps): BlockSuggest {
  // ── `[[page]]` ref autocomplete ──────────────────────────────────
  // While the caret sits inside an open `[[…]]`, we offer a popup of
  // matching pages. Detection + span replacement reuse the shared
  // `@outl/shared/autocomplete` helpers (same logic the TUI and mobile
  // run); page lookup reuses the `search_pages` command the Cmd+P
  // picker already calls. The popup is "open" iff `suggestions` is
  // non-empty.
  const [suggestions, setSuggestions] = createSignal<PageMeta[]>([]);
  const [suggestIndex, setSuggestIndex] = createSignal(0);
  // Emoji shortcode popup. Lives alongside `suggestions` instead of
  // being merged into one heterogeneous list because the two have
  // different cell shapes (emoji shows `glyph :shortcode:`, ref shows
  // icon + title) and the keyboard handlers are simpler when only one
  // popup is active at a time.
  const [emojiSuggestions, setEmojiSuggestions] = createSignal<EmojiHit[]>([]);
  const [emojiIndex, setEmojiIndex] = createSignal(0);
  // ── `((block ref))` autocomplete ─────────────────────────────────
  // While the caret sits inside an open `((…))`, we offer a popup of
  // matching blocks. Same detection (`detectRefContext` → `kind:
  // "block"`) and insertion (`applySuggestion` wraps the pick in
  // `((…))`) as the page-ref path; block lookup goes through the
  // `search_blocks` command. Kept in its own signal because a block
  // hit's cell shape (text snippet + page) differs from a page's
  // (icon + title), and only one popup is ever open at a time.
  const [blockSuggestions, setBlockSuggestions] = createSignal<BlockHit[]>([]);
  const [blockIndex, setBlockIndex] = createSignal(0);
  // ── `/command` inline slash menu ─────────────────────────────────
  // Block-initial `/` opens a filterable list of plugin commands —
  // the desktop's inline equivalent of the TUI's `/` slash overlay
  // (the `⧉` palette is the other surface). Trigger detection +
  // token removal reuse the shared `@outl/shared/autocomplete`
  // helpers; the command universe comes from `pluginList()`, loaded
  // once on the first `/` and filtered client-side as the user types.
  const [slashCommands, setSlashCommands] = createSignal<PluginCommand[]>([]);
  const [slashIndex, setSlashIndex] = createSignal(0);
  // Lazily-loaded, cached command list (null until the first `/`).
  // Native `/template <name>` entries (structural templates, no plugin
  // needed) are merged ahead of plugin commands so the core feature is
  // reachable from the same popup — see `templateSlashCommands`.
  let allSlashCommands: PluginCommand[] | null = null;
  async function ensureSlashCommands(): Promise<PluginCommand[]> {
    if (allSlashCommands) return allSlashCommands;
    const [plugins, templates] = await Promise.all([
      pluginList().catch(() => []),
      listTemplates().catch(() => []),
    ]);
    allSlashCommands = [
      ...assetSlashCommands(),
      ...templateSlashCommands(templates),
      ...plugins,
    ];
    return allSlashCommands;
  }
  // `query` last sent to the backend — skip redundant round-trips when
  // the caret moves without changing the in-ref text (mirrors mobile's
  // `lastQuery` guard). `null` means "not in a ref right now".
  let lastQuery: string | null = null;
  let searchToken = 0;
  // Debounce timer for `searchBlocks` only. Unlike page / person / emoji
  // search (in-memory or a static catalog), the block search rebuilds the
  // whole `WorkspaceIndex` from disk per call, so firing it on every
  // keystroke inside `((…))` janks on large workspaces. Waiting for a
  // short pause keeps the rebuild off the hot path.
  const BLOCK_SEARCH_DEBOUNCE_MS = 150;
  let blockSearchTimer: ReturnType<typeof setTimeout> | undefined;

  onCleanup(() => clearTimeout(blockSearchTimer));

  function closeSuggest() {
    lastQuery = null;
    if (suggestions().length > 0) setSuggestions([]);
    setSuggestIndex(0);
    if (emojiSuggestions().length > 0) setEmojiSuggestions([]);
    setEmojiIndex(0);
    if (blockSuggestions().length > 0) setBlockSuggestions([]);
    setBlockIndex(0);
    if (slashCommands().length > 0) setSlashCommands([]);
    setSlashIndex(0);
  }

  /**
   * Recompute the suggestion popup from the live textarea state.
   * Called after every keystroke / caret move while editing. When the
   * caret is inside an open `[[…]]` it (debounce-free, but de-duped on
   * `lastQuery`) fetches matching pages; otherwise it closes the popup.
   * Block refs (`((…))`) are intentionally ignored here — that's a
   * separate feature.
   */
  function refreshSuggest() {
    const ta = deps.textarea();
    if (!ta) return closeSuggest();
    const cursor = ta.selectionStart ?? 0;
    // Block-initial `/` opens the slash menu. Checked first: it only
    // fires when `/` is the very first character (never mid-prose), so
    // it can't shadow the `:`/`[[` triggers below — but when it IS
    // active those are irrelevant.
    const slashCtx = detectSlashContext(ta.value, cursor);
    if (slashCtx) {
      const key = `slash:${slashCtx.query}`;
      if (key === lastQuery) return;
      lastQuery = key;
      const token = ++searchToken;
      void ensureSlashCommands()
        .then((all) => {
          if (token !== searchToken) return;
          // Stale-caret guard: the caret may have left the trigger
          // while the command list was loading.
          const live = deps.textarea();
          const cur = live
            ? detectSlashContext(live.value, live.selectionStart ?? 0)
            : null;
          if (!cur) return;
          // Match on the command id (what the user types, mirrors the
          // TUI / CLI) and the human title. Rank id-prefix first, then
          // id-substring, then title — so `/sta` puts `stats` on top.
          const ranked = rankSlashCommands(all, cur.query);
          if (suggestions().length > 0) setSuggestions([]);
          if (emojiSuggestions().length > 0) setEmojiSuggestions([]);
          setSlashCommands(ranked);
          setSlashIndex(0);
        })
        .catch(() => closeSuggest());
      return;
    }
    // Emoji takes precedence over ref detection: a `:` typed inside a
    // stray `[[…` window must still surface the glyph popup. The two
    // triggers don't overlap on real prose because `:` is rejected on
    // word-internal positions.
    const emojiCtx = detectEmojiContext(ta.value, cursor);
    if (emojiCtx) {
      const key = `emoji:${emojiCtx.query}`;
      if (key === lastQuery) return;
      lastQuery = key;
      const token = ++searchToken;
      void searchEmojis(emojiCtx.query, 8)
        .then((hits) => {
          if (token !== searchToken) return;
          // Stale-response guard: the caret may have left the trigger
          // while we were waiting for the catalog.
          const live = deps.textarea();
          const cur = live
            ? detectEmojiContext(live.value, live.selectionStart ?? 0)
            : null;
          if (!cur || cur.query !== emojiCtx.query) return;
          // Make sure the ref popup isn't lingering from a previous
          // trigger that is no longer active at this caret.
          if (suggestions().length > 0) setSuggestions([]);
          setEmojiSuggestions(hits);
          setEmojiIndex(0);
        })
        .catch(() => closeSuggest());
      return;
    }
    const ctx = detectRefContext(ta.value, cursor);
    // `block` → fuzzy over every block's text, keyed on the `((…))`
    // trigger. Handled on its own path because the hit shape (handle +
    // snippet) and the accept (insert the handle, not the text) differ
    // from the page/mention path below.
    if (ctx && ctx.kind === "block") {
      const key = `block:${ctx.query}`;
      if (key === lastQuery) return;
      lastQuery = key;
      const token = ++searchToken;
      if (suggestions().length > 0) setSuggestions([]);
      if (emojiSuggestions().length > 0) setEmojiSuggestions([]);
      const query = ctx.query;
      // Debounced: the backend rebuilds the workspace index from disk, so
      // only fire after the user pauses typing. `searchToken` still guards
      // staleness if a newer keystroke supersedes this one mid-flight.
      clearTimeout(blockSearchTimer);
      blockSearchTimer = setTimeout(() => {
        if (token !== searchToken) return;
        void searchBlocks(query)
          .then((hits) => {
            if (token !== searchToken) return;
            // Stale-caret guard: the caret may have left the `((…)` while
            // the search was in flight.
            const live = deps.textarea();
            const cur = live
              ? detectRefContext(live.value, live.selectionStart ?? 0)
              : null;
            if (!cur || cur.kind !== "block" || cur.query !== query) return;
            setBlockSuggestions(hits);
            setBlockIndex(0);
          })
          .catch(() => closeSuggest());
      }, BLOCK_SEARCH_DEBOUNCE_MS);
      return;
    }
    // `page` → fuzzy over every page; `mention` → fuzzy over persons.
    if (!ctx || (ctx.kind !== "page" && ctx.kind !== "mention")) {
      return closeSuggest();
    }
    const key = `${ctx.kind}:${ctx.query}`;
    if (key === lastQuery) return;
    lastQuery = key;
    const token = ++searchToken;
    const fetcher = ctx.kind === "mention" ? searchPersons : searchPages;
    const wantedKind = ctx.kind;
    if (emojiSuggestions().length > 0) setEmojiSuggestions([]);
    void fetcher(ctx.query)
      .then((list) => {
        // Drop stale responses: the user kept typing (newer token) or
        // moved the caret out of the ref while we were waiting.
        if (token !== searchToken) return;
        const live = deps.textarea();
        const cur = live
          ? detectRefContext(live.value, live.selectionStart ?? 0)
          : null;
        if (!cur || cur.kind !== wantedKind || cur.query !== ctx.query) return;
        // Create-new affordance for mentions — shared with mobile
        // via `@outl/shared/autocomplete::withCreateNewPersonCandidate`.
        // Skips the helper entirely for non-mention contexts so plain
        // page-ref searches stay free of synthetic rows.
        const finalList =
          wantedKind === "mention"
            ? withCreateNewPersonCandidate(list, ctx.query)
            : list;
        setSuggestions(finalList);
        setSuggestIndex(0);
      })
      .catch(() => closeSuggest());
  }

  /** Accept `page`: replace the `[[…]]` (or `@…`) span with the
   *  chosen target, sync the draft signal + textarea, and park the
   *  caret after the closer. The shared `applySuggestion` decides
   *  whether to wrap the replacement in `[[…]]` (`page`) or `[[@…]]`
   *  (`mention`). */
  function acceptSuggestion(page: PageMeta) {
    const ta = deps.textarea();
    if (!ta) return;
    const ctx = detectRefContext(ta.value, ta.selectionStart ?? 0);
    if (!ctx || (ctx.kind !== "page" && ctx.kind !== "mention")) {
      return closeSuggest();
    }
    // For mentions the page identity carries no `@` — the shared
    // `refReplacement` passes the title verbatim; `applySuggestion`
    // prepends `@` on the link side.
    const replacement = refReplacement(page, {
      mention: ctx.kind === "mention",
    });
    // Mention sugar: materialise the person page in the backend
    // (fire-and-forget) so the inserted `[[@title]]` link resolves
    // on subsequent loads — `open_or_create_by_ref` strips the `@`
    // and sets `type:: person` when the page doesn't exist yet, and
    // is idempotent when it does. Without this, accepting a
    // create-new candidate inserts the link but no page ever lands
    // on disk, and the next `@title` lookup misses it.
    if (ctx.kind === "mention") {
      void openRef(`@${page.title}`).catch((e) => {
        // Non-fatal — the link is already in the buffer; the user
        // can still navigate it later (which would create the page
        // then). Surface to the console so a backend regression
        // (e.g. permission denied on `pages/`) shows up in dev.
        console.warn("openRef for mention failed:", e);
      });
    }
    const completion = applySuggestion(ta.value, ctx, replacement);
    deps.setText(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    closeSuggest();
    ta.focus();
  }

  /** Accept `hit`: replace the open `((…))` span with `((<handle>))`.
   *  The replacement is the block's **ref handle**, never its display
   *  text — block refs resolve by handle. `applySuggestion` wraps a
   *  `block` context in `((…))`, so we pass the bare handle. */
  function acceptBlockSuggestion(hit: BlockHit) {
    const ta = deps.textarea();
    if (!ta) return;
    const ctx = detectRefContext(ta.value, ta.selectionStart ?? 0);
    if (!ctx || ctx.kind !== "block") return closeSuggest();
    const completion = applySuggestion(ta.value, ctx, hit.handle);
    deps.setText(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    closeSuggest();
    ta.focus();
  }

  /** Accept `hit`: replace the `:shortcode` trigger with the canonical
   *  `:shortcode:` form. The disk stores the shortcode literal; the
   *  renderer translates to the glyph at display time. */
  function acceptEmojiSuggestion(hit: EmojiHit) {
    const ta = deps.textarea();
    if (!ta) return;
    const ctx = detectEmojiContext(ta.value, ta.selectionStart ?? 0);
    if (!ctx) return closeSuggest();
    const completion = applyEmojiSuggestion(ta.value, ctx, hit.shortcode);
    deps.setText(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    closeSuggest();
    ta.focus();
  }

  /** Accept a slash command: strip the `/query` token from the block,
   *  commit the cleaned text, then hand the run to the parent (which
   *  owns `pluginRun` + view/overlay application, same as the palette). */
  function acceptSlashCommand(cmd: PluginCommand) {
    const ta = deps.textarea();
    if (!ta) return;
    const ctx = detectSlashContext(ta.value, ta.selectionStart ?? 0);
    if (!ctx) return closeSuggest();
    const completion = applySlashContext(ta.value, ctx);
    deps.setText(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    closeSuggest();
    // Persist the now-cleaned block (drops the `/stats` literal), then
    // run. `commit` is a no-op round-trip when the text is unchanged
    // (the common case: a fresh empty block), so a plain command still
    // just fires.
    void (async () => {
      await deps.commit();
      await deps.runPluginCommand(cmd.plugin_id, cmd.command_id);
    })();
  }

  function handleKey(e: KeyboardEvent): boolean {
    // The four inline autocomplete popups share one keyboard contract
    // (arrows cycle, Enter/Tab accept, Esc close) via `handlePopupNav`.
    // They never co-exist — one trigger is active at a time — so the
    // first non-empty one consumes the key. Checking slash first is safe
    // (block-initial `/` vs. `:` / `((` / `[[`).
    return (
      handlePopupNav(e, {
        items: slashCommands(),
        index: slashIndex(),
        setIndex: setSlashIndex,
        onAccept: acceptSlashCommand,
        onClose: closeSuggest,
      }) ||
      handlePopupNav(e, {
        items: emojiSuggestions(),
        index: emojiIndex(),
        setIndex: setEmojiIndex,
        onAccept: acceptEmojiSuggestion,
        onClose: closeSuggest,
      }) ||
      handlePopupNav(e, {
        items: blockSuggestions(),
        index: blockIndex(),
        setIndex: setBlockIndex,
        onAccept: acceptBlockSuggestion,
        onClose: closeSuggest,
      }) ||
      handlePopupNav(e, {
        items: suggestions(),
        index: suggestIndex(),
        setIndex: setSuggestIndex,
        onAccept: acceptSuggestion,
        onClose: closeSuggest,
      })
    );
  }

  return {
    suggestions,
    suggestIndex,
    setSuggestIndex,
    blockSuggestions,
    blockIndex,
    setBlockIndex,
    emojiSuggestions,
    emojiIndex,
    setEmojiIndex,
    slashCommands,
    slashIndex,
    setSlashIndex,
    acceptSuggestion,
    acceptBlockSuggestion,
    acceptEmojiSuggestion,
    acceptSlashCommand,
    refresh: refreshSuggest,
    close: closeSuggest,
    handleKey,
  };
}
