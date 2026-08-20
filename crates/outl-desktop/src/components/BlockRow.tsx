import {
  For,
  Match,
  Show,
  Switch,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
} from "solid-js";

import type {
  BlockNode,
  PageMeta,
  PluginCommand,
  PluginTransformResult,
  TodoState,
} from "@outl/shared/api/types";
import type { BlockHit } from "@outl/shared/api/commands";
import {
  EmbeddedSubtree,
  MarkdownInline,
  QuoteWrap,
  isBlockQuoted,
  splitQuote,
  stripQuoteFromTokens,
} from "@outl/shared/markdown";
import {
  applyEmojiSuggestion,
  applySlashContext,
  applySuggestion,
  autoDeletePair,
  autoPairBracket,
  detectEmojiContext,
  detectRefContext,
  detectSlashContext,
  refReplacement,
  withCreateNewPersonCandidate,
} from "@outl/shared/autocomplete";
import {
  type EmojiHit,
  listTemplates,
  openRef,
  pluginList,
  searchBlocks,
  searchEmojis,
  searchPages,
  searchPersons,
} from "@outl/shared/api/commands";
import { HighlightedCode } from "@outl/shared/highlight";
import { embedOnlyHandle, rawTextWithTodo } from "@outl/shared/outline";
import {
  choosePasteRoute,
  utf16OffsetToCharOffset,
} from "@outl/shared/paste";
import {
  runTransform,
  transformerFor,
} from "@outl/shared/plugins/transformer-registry";

import { detectFence } from "@outl/shared/highlight";
import { readText as readClipboardText } from "@tauri-apps/plugin-clipboard-manager";
import { appState, setAppState } from "../lib/store";
import { handlePopupNav } from "../lib/popup-nav";
import { PropertyEditor } from "./PropertyEditor";
import {
  assetSlashCommands,
  rankSlashCommands,
  templateSlashCommands,
} from "../lib/slash-commands";

export interface BlockCallbacks {
  /** A textarea was double-clicked → enter edit mode on `id`. */
  onStartEdit: (id: string) => void;
  /** Commit the current edit. Called on Esc / blur / structural ops. */
  onCommit: (id: string, text: string) => Promise<void>;
  /** Enter pressed → commit + create a sibling below + focus it. */
  onEnter: (id: string, text: string, caretChars: number) => Promise<void>;
  /**
   * `Cmd/Ctrl+Shift+Enter` with the caret at column 0 → commit + create
   * a sibling *before* this one + focus it. The textarea mirror of vim `O`.
   */
  onCreateBefore: (id: string, text: string) => Promise<void>;
  /** Tab pressed inside the textarea. */
  onIndent: (id: string) => Promise<void>;
  /** Shift-Tab pressed inside the textarea. */
  onOutdent: (id: string) => Promise<void>;
  /** Backspace on empty text → delete this block, jump cursor to prev. */
  onDeleteEmpty: (id: string) => Promise<void>;
  /** Checkbox click — flip TODO/DONE/none. */
  onToggleTodo: (id: string) => Promise<void>;
  /** Chevron click — fold / unfold. */
  onToggleCollapsed: (id: string, collapsed: boolean) => Promise<void>;
  /** External-clipboard paste with formatting (Cmd+V) — structured
   *  payload is converted to blocks. `hostText` is the in-flight
   *  textarea value so the parent can flush the draft into the
   *  workspace before splicing (else the caret is measured on the draft
   *  but applied to stale backend text). */
  onPasteMarkdown: (
    id: string,
    caret: number,
    text: string,
    hostText: string,
  ) => Promise<void>;
  /** Paste without formatting (Cmd+Shift+V) — raw text spliced at the
   *  caret, no conversion. `hostText` is flushed like `onPasteMarkdown`. */
  onPastePlain: (
    id: string,
    caret: number,
    text: string,
    hostText: string,
  ) => Promise<void>;
  /** Run a fenced code block through `outl-exec`. */
  onRunCodeBlock: (id: string) => Promise<void>;
  /** Run a plugin command picked from the inline `/` slash menu. The
   *  parent owns the `pluginRun` round-trip + view/overlay application,
   *  same as `PluginPalette` does for the `⧉` palette. */
  onRunPluginCommand: (pluginId: string, commandId: string) => Promise<void>;
  /** Commit a `key:: value` property edit; an empty value clears it.
   *  Returning the promise lets the editor surface a rejected write —
   *  the chip repaints either way, so a swallowed failure reads as a
   *  successful edit. */
  onSetProperty?: (
    blockId: string,
    key: string,
    value: string,
  ) => void | Promise<void>;
  /** Ref / tag click handlers (forwarded to MarkdownInline). */
  onRefClick: (target: string) => void;
  onTagClick: (tag: string) => void;
  /** Navigate to a page by its exact slug. Used by a `call:<name>` code
   *  fence to jump to the template's page. Unlike `onRefClick` (→
   *  `openRef`, which *creates* a page when the target doesn't resolve),
   *  this is an exact `openPageBySlug` — no side effect on a miss. */
  onOpenPage: (slug: string) => void;
  /** External `[label](url)` link click — opens in the system browser.
   *  Optional: contexts that keep links inert simply omit it (the
   *  renderer then draws a plain, non-interactive span). */
  onLinkClick?: (href: string) => void;
  /** Bullet marker click → zoom into this block (Roam/Workflowy focus).
   *  The fold chevron stays the collapse gesture; the `•`/`▢`/`▣` marker
   *  is the zoom gesture. Optional so contexts without zoom omit it. */
  onFocusBlock?: (id: string) => void;
}

/**
 * One outline block. Renders read-only by default; flips to a
 * textarea editor when `editing === true`. Mouse and keyboard
 * interactions route through the shared `BlockCallbacks` so the
 * parent (`OutlineView`) owns the Tauri-side state mutations.
 */
export function BlockRow(props: {
  block: BlockNode;
  depth: number;
  editingId: string | null;
  /** Memoised Visual-range membership set built once in
   *  `<OutlineView />` (`createMemo` over outline + anchor + cursor +
   *  mode). `null` outside Visual mode. We pay one DFS for the whole
   *  outline; every row answers `Set.has(id)` in O(1). The previous
   *  shape called `isInVisualRange(...)` per row, which rebuilt
   *  `flattenVisible` from scratch each call — O(N²) on extension. */
  visualSet: Set<string> | null;
  cb: BlockCallbacks;
}) {
  const isEditing = () => props.editingId === props.block.id;
  // Draft mirrors the wire format (with TODO/DONE prefix). Same
  // shape the TUI buffer and the mobile editor use, so users can
  // type / erase the prefix to flip state.
  const [draft, setDraft] = createSignal<string>(rawTextWithTodo(props.block));

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

  let textareaRef: HTMLTextAreaElement | undefined;

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
    const ta = textareaRef;
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
          const cur = textareaRef
            ? detectSlashContext(textareaRef.value, textareaRef.selectionStart ?? 0)
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
          const cur = textareaRef
            ? detectEmojiContext(
                textareaRef.value,
                textareaRef.selectionStart ?? 0,
              )
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
            const cur = textareaRef
              ? detectRefContext(textareaRef.value, textareaRef.selectionStart ?? 0)
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
        const cur = textareaRef
          ? detectRefContext(textareaRef.value, textareaRef.selectionStart ?? 0)
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
    const ta = textareaRef;
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
    setDraft(completion.value);
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
    const ta = textareaRef;
    if (!ta) return;
    const ctx = detectRefContext(ta.value, ta.selectionStart ?? 0);
    if (!ctx || ctx.kind !== "block") return closeSuggest();
    const completion = applySuggestion(ta.value, ctx, hit.handle);
    setDraft(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    closeSuggest();
    ta.focus();
  }

  /** Accept `hit`: replace the `:shortcode` trigger with the canonical
   *  `:shortcode:` form. The disk stores the shortcode literal; the
   *  renderer translates to the glyph at display time. */
  function acceptEmojiSuggestion(hit: EmojiHit) {
    const ta = textareaRef;
    if (!ta) return;
    const ctx = detectEmojiContext(ta.value, ta.selectionStart ?? 0);
    if (!ctx) return closeSuggest();
    const completion = applyEmojiSuggestion(ta.value, ctx, hit.shortcode);
    setDraft(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    closeSuggest();
    ta.focus();
  }

  /** Accept a slash command: strip the `/query` token from the block,
   *  commit the cleaned text, then hand the run to the parent (which
   *  owns `pluginRun` + view/overlay application, same as the palette). */
  function acceptSlashCommand(cmd: PluginCommand) {
    const ta = textareaRef;
    if (!ta) return;
    const ctx = detectSlashContext(ta.value, ta.selectionStart ?? 0);
    if (!ctx) return closeSuggest();
    const completion = applySlashContext(ta.value, ctx);
    setDraft(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    closeSuggest();
    // Persist the now-cleaned block (drops the `/stats` literal), then
    // run. `commit` is a no-op round-trip when the text is unchanged
    // (the common case: a fresh empty block), so a plain command still
    // just fires.
    void (async () => {
      await commit();
      await props.cb.onRunPluginCommand(cmd.plugin_id, cmd.command_id);
    })();
  }

  function focusTextarea() {
    queueMicrotask(() => {
      textareaRef?.focus();
      autoSize();
    });
  }

  /**
   * Grow the textarea to fit its current content. Without this the
   * `rows={1}` textarea would clip multi-line blocks (code fences,
   * paragraphs typed with plain `Enter`). Cheap enough to call on
   * every draft change.
   */
  function autoSize() {
    const ta = textareaRef;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${ta.scrollHeight}px`;
  }

  // Re-run autoSize when the draft signal changes — Solid's
  // reactivity ties this to keystrokes the textarea fires.
  createEffect(() => {
    draft();
    if (isEditing()) autoSize();
  });

  /*
   * Focus the textarea whenever this block transitions into edit
   * mode — whether the user clicked into it or `<OutlineView />`
   * set `editingId` programmatically after `createBlock` (Cmd+Enter
   * fires the parent to create a sibling, and we want that sibling
   * editable without a second click). The microtask lets Solid
   * commit the `<Show>` swap to the textarea branch so
   * `textareaRef` is populated by the time we call focus().
   */
  createEffect(() => {
    if (isEditing()) {
      queueMicrotask(() => {
        textareaRef?.focus();
        autoSize();
        // Apply pending caret intent (set by `EnterInsertAtEnd` etc.).
        // We do it here, after focus, because the textarea ref is
        // guaranteed populated by this point — `<Show>` mounted it
        // synchronously and the microtask fence drained any pending
        // Solid commits.
        const intent = appState.caretIntent;
        if (intent && textareaRef) {
          const pos = intent === "end" ? textareaRef.value.length : 0;
          textareaRef.setSelectionRange(pos, pos);
          setAppState("caretIntent", null);
        }
      });
    }
  });

  async function commit() {
    // Draft is already wire format (prefix included if any). The
    // backend re-runs `split_todo` on it and re-projects the block
    // with the right `todo` state.
    const raw = draft();
    const wire = rawTextWithTodo(props.block);
    if (raw !== wire) {
      // `onCommit` flips `editingBlockId` to null after the Tauri
      // round-trip, so the row will re-render in read-only mode.
      await props.cb.onCommit(props.block.id, raw);
      return;
    }
    // Unchanged text — still need to leave Insert and flip the row
    // back to render mode. Without this an Esc on an unmodified
    // block leaves the textarea visible with raw markdown showing
    // (`**bold**` instead of the rendered **bold**), which is the
    // "Esc didn't exit edit mode" bug.
    setAppState("editingBlockId", null);
  }

  async function handleKeydown(e: KeyboardEvent) {
    // Cmd/Ctrl+Shift+V — paste WITHOUT formatting: read the clipboard
    // and splice it raw at the caret, no outline / paragraph conversion.
    // (Plain Cmd+V is the native paste event → `handlePaste`, which
    // routes structured content to the backend "with formatting".)
    if (
      (e.metaKey || e.ctrlKey) &&
      e.shiftKey &&
      (e.key === "v" || e.key === "V")
    ) {
      e.preventDefault();
      const ta = textareaRef;
      if (!ta) return;
      let clip = "";
      try {
        // Read via the Tauri clipboard plugin, NOT
        // `navigator.clipboard.readText()`: the macOS WKWebview pops a
        // native "Paste" permission button for a programmatic web-API
        // read (outside a real paste gesture), which showed a "paste"
        // prompt and inserted nothing. The plugin reads on the backend.
        clip = await readClipboardText();
      } catch {
        return; // clipboard read denied / empty — nothing to paste
      }
      if (!clip) return;
      const caretChars = utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? 0);
      await props.cb.onPastePlain(props.block.id, caretChars, clip, ta.value);
      return;
    }
    // The four inline autocomplete popups share one keyboard contract
    // (arrows cycle, Enter/Tab accept, Esc close) via `handlePopupNav`.
    // They never co-exist — one trigger is active at a time — so the
    // first non-empty one consumes the key. Checking slash first is safe
    // (block-initial `/` vs. `:` / `((` / `[[`).
    if (
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
    ) {
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      await commit();
      return;
    }
    // Plain `Enter` (no modifiers) → commit + create a sibling below.
    // TUI parity: `outl-tui/src/input/mod.rs` — "Plain Enter commits the
    // block and creates a sibling below." `Shift+Enter` (no Cmd/Ctrl) is
    // the soft break: it falls through to the textarea default and inserts
    // a literal `\n` for a multi-line block (issue #119).
    // No `stopImmediatePropagation` here (unlike `Cmd/Ctrl+Shift+Enter`
    // below): with a textarea focused the dispatcher is in Insert mode,
    // and the catalog has no Insert binding for a bare `Enter` (its only
    // `Enter` row is Normal → `OpenRefUnderCursor`), so the window
    // dispatcher no-ops — there is no double-fire to guard against.
    if (
      e.key === "Enter" &&
      !e.shiftKey &&
      !e.metaKey &&
      !e.ctrlKey &&
      !e.altKey
    ) {
      e.preventDefault();
      // Pass the caret so the backend splits the block there instead of
      // always appending an empty sibling (issue #184). `selectionStart`
      // is a UTF-16 offset; the backend expects codepoints.
      const caretChars = utf16OffsetToCharOffset(
        textareaRef?.value ?? draft(),
        textareaRef?.selectionStart ?? draft().length,
      );
      await props.cb.onEnter(props.block.id, draft(), caretChars);
      return;
    }
    // `Cmd/Ctrl+Shift+Enter` is caret-position aware, handled here (not
    // via the global catalog) because only the textarea knows the caret:
    //   - caret at column 0  → create a sibling *before* this block
    //     (vim `O`).
    //   - caret anywhere past column 0 → commit + create a sibling
    //     *below* (`onEnter`).
    // `stopImmediatePropagation` is load-bearing: it stops the webview's
    // default Enter (a literal `\n`) *and* keeps the global `window`
    // shortcut dispatcher from also firing the catalog's
    // commit-and-continue on the same keystroke (the old "Cmd+Shift+Enter
    // breaks the line" double-fire bug). `Cmd+Enter` / `Cmd+T` are still
    // owned by the catalog.
    if (e.key === "Enter" && e.shiftKey && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      e.stopImmediatePropagation();
      const atStart =
        (textareaRef?.selectionStart ?? -1) === 0 &&
        (textareaRef?.selectionEnd ?? -1) === 0;
      if (atStart) await props.cb.onCreateBefore(props.block.id, draft());
      // Past column 0: this chord means "sibling below", not "split
      // here" — pass the end offset so `split_block` yields an empty
      // sibling below (the pre-#184 behaviour) regardless of the caret.
      else await props.cb.onEnter(props.block.id, draft(), draft().length);
      return;
    }
    if (e.key === "Tab") {
      e.preventDefault();
      await commit();
      if (e.shiftKey) await props.cb.onOutdent(props.block.id);
      else await props.cb.onIndent(props.block.id);
      return;
    }
    if (e.key === "Backspace" && draft().length === 0) {
      e.preventDefault();
      await props.cb.onDeleteEmpty(props.block.id);
      return;
    }
    if (e.key === "Backspace") {
      const ta = textareaRef;
      if (ta) {
        const collapse = autoDeletePair(ta.value, ta.selectionStart ?? 0);
        if (collapse) {
          e.preventDefault();
          setDraft(collapse.value);
          ta.value = collapse.value;
          ta.setSelectionRange(collapse.caret, collapse.caret);
        }
      }
    }
    // Default: let the keystroke through; the bound input handler
    // updates the draft signal.
  }

  /**
   * Auto-pair `(` / `[` / `{` and step over auto-inserted closers
   * (issue #21) — same Insert-mode behaviour as the TUI. Typing the
   * second `[` lands as `[[|]]`, so the `[[` ref flow keeps working
   * without `autoClosePair` (the closer is never doubled).
   * `beforeinput` (not keydown) so layouts that reach brackets via
   * AltGr / Option dead keys are matched by the character actually
   * produced, never by the physical key.
   */
  function handleBeforeInput(e: InputEvent) {
    if (e.inputType !== "insertText" || e.isComposing) return;
    const ta = textareaRef;
    if (!ta) return;
    if (ta.selectionStart !== ta.selectionEnd) return; // typing over a selection
    const completion = autoPairBracket(
      ta.value,
      ta.selectionStart ?? 0,
      e.data ?? "",
    );
    if (!completion) return;
    e.preventDefault();
    setDraft(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    // Setting the value programmatically doesn't fire `onInput`, and
    // the caret may now sit inside a `[[…]]` — refresh the suggester.
    refreshSuggest();
  }

  async function handlePaste(e: ClipboardEvent) {
    const ta = textareaRef;
    if (!ta) return;
    // Inside a fenced code block the whole block is one raw ```lang…```
    // string. Converting a multi-line / outline clipboard "with
    // formatting" would split the fence into sibling blocks and strand
    // the closing ``` on its own line (the "paste jumps to the last
    // line" bug). Let the browser splice the text in literally (newlines
    // preserved), exactly like typing it — `onInput` keeps the draft in
    // sync. Cmd+Shift+V (paste without formatting) already splices raw.
    if (detectFence(ta.value)) return;
    // "Paste with formatting" (Cmd+V). `choosePasteRoute` (shared with
    // mobile) decides between: rich (text/html converted to markdown so a
    // Slack/Docs/Notion paste keeps its **bold** + lists), structured
    // (plain outline / multi-paragraph the backend splits), or native (a
    // trivial word / URL stays on the browser splice). Cmd+Shift+V is the
    // separate "without formatting" path.
    const decision = choosePasteRoute(
      e.clipboardData?.getData("text/html") ?? "",
      e.clipboardData?.getData("text/plain") ?? "",
    );
    if (decision.route === "native") return;
    e.preventDefault();
    const caretChars = utf16OffsetToCharOffset(
      ta.value,
      ta.selectionStart ?? 0,
    );
    await props.cb.onPasteMarkdown(
      props.block.id,
      caretChars,
      decision.text,
      ta.value,
    );
  }

  /** Task state to render the bullet by. Edit mode is the
   *  **raw markdown view** — the `TODO `/`DOING `/`DONE ` prefix is
   *  literally visible in the textarea, so the bullet collapses to the
   *  neutral `•` to avoid showing the same state twice. Read mode is
   *  where the prefix is replaced by `▢` / `▨` / `▣`. */
  function effectiveTodo(): TodoState | null {
    return isEditing() ? null : props.block.todo;
  }

  /** Bullet glyph + click action — folds the task states and none into
   *  a single visual primitive (TUI parity, `▢` / `▨` / `▣` / `•`). */
  function bulletGlyph(): string {
    const t = effectiveTodo();
    if (t === "DONE") return "▣";
    if (t === "DOING") return "▨";
    if (t === "TODO") return "▢";
    return "•";
  }
  function bulletClass(): string {
    const t = effectiveTodo();
    if (t === "DONE") {
      return "text-(--color-outl-todo-done-fg)";
    }
    // DOING shares the open colour: it is unfinished work, and the
    // half-filled glyph already carries the distinction.
    if (t === "TODO" || t === "DOING") {
      return "text-(--color-outl-todo-open-fg)";
    }
    return "text-(--color-outl-fg-dimmer)";
  }
  /** Body styling — DONE blocks render dim + struck-through (TUI
   *  uses theme.todo_done_body which is fg_dimmer + CROSSED_OUT). */
  function bodyClass(): string {
    return props.block.todo === "DONE" ? "line-through opacity-60" : "";
  }

  const isSelected = () => appState.selectedBlockId === props.block.id;
  /** Vim Visual range covers this block. Mutually exclusive with
   *  `isSelected()` rendering-wise: when both are true we apply the
   *  Visual style so the user sees the contiguous band, not a single
   *  bright row at the cursor. */
  const isInVisual = () => props.visualSet?.has(props.block.id) ?? false;
  /** This block is armed for a cut (`Cmd+X` in view mode), waiting
   *  for the paste that will move it. Dim it so the user sees what's
   *  on the block clipboard until they paste or cancel with `Esc`. */
  const isPendingCut = () =>
    appState.blockClipboard?.kind === "cut" &&
    appState.blockClipboard.nodeId === props.block.id;
  /** An OS file drag is hovering this block — highlight it so the user
   *  sees where the dropped file's link will be inserted. */
  const isDropTarget = () => appState.dropTargetBlockId === props.block.id;
  const isInteractive = () => isEditing() || props.block.todo !== null;

  /** Outer row click — select without entering Insert. Lets the
   *  user mouse onto a block and then keyboard-nav from there
   *  (j/k), instead of clicking forcing an edit. The inner text
   *  `<div onClick>` still calls `onStartEdit` to enter edit mode,
   *  but the buttons (bullet, chevron) `stopPropagation()` so they
   *  don't both fire. */
  function selectRow(e: MouseEvent) {
    // The text-click handler stops propagation; this only fires
    // when the user clicked on row chrome (gutter, indent area).
    e.stopPropagation();
    setAppState("selectedBlockId", props.block.id);
  }

  return (
    <div>
      <div
        class={`outl-row group relative flex items-start rounded-sm py-[3px] pr-2 ${
          isPendingCut() ? "opacity-50 " : ""
        }${
          isDropTarget() ? "ring-2 ring-inset ring-(--color-outl-accent) " : ""
        }${
          isInVisual()
            ? "bg-(--color-outl-accent)/[0.18]"
            : isSelected()
              ? "bg-(--color-outl-accent)/[0.06]"
              : "hover:bg-(--color-outl-bg-elev)/30"
        }`}
        data-block-id={props.block.id}
        data-selected={isSelected() ? "true" : "false"}
        data-visual={isInVisual() ? "true" : "false"}
        data-editing={isEditing() ? "true" : "false"}
        data-drop-target={isDropTarget() ? "true" : "false"}
        onClick={selectRow}
      >
        {/* Vertical accent bar for the selected row — Bear-style
         * "this is where you are" indicator. Sits in the row's
         * gutter so it never reflows text. */}
        <Show when={isSelected()}>
          <span
            aria-hidden="true"
            class="absolute top-[4px] bottom-[4px] -left-[2px] w-[3px] rounded-full bg-(--color-outl-accent)"
          />
        </Show>

        {/*
         * Indent guides — one per ancestor level. Hairline only,
         * fades in when the row chrome is visible (hover / selected /
         * editing). Keeps the reading surface clean when at rest.
         */}
        <For each={Array.from({ length: props.depth })}>
          {() => (
            <span
              aria-hidden="true"
              class="outl-row-chrome ml-[10px] w-3 shrink-0 self-stretch border-l border-(--color-outl-border)/20"
            />
          )}
        </For>

        {/* Fold chevron — opacity 0 at rest (via `.outl-row-chrome`
         * styles), visible on hover / selection. */}
        <button
          type="button"
          class={`outl-row-chrome ml-[6px] mt-[6px] min-w-[16px] select-none whitespace-nowrap text-left text-[9px] font-mono ${
            props.block.children.length === 0 ? "" : "cursor-pointer"
          } opacity-60 hover:opacity-100 disabled:cursor-default`}
          disabled={props.block.children.length === 0}
          onClick={(e) => {
            e.stopPropagation();
            if (props.block.children.length === 0) return;
            void props.cb.onToggleCollapsed(
              props.block.id,
              !props.block.collapsed,
            );
          }}
          aria-label={props.block.collapsed ? "Expand" : "Collapse"}
        >
          {props.block.children.length > 0 ? (
            props.block.collapsed ? (
              <>
                <span>▶</span>
                <span class="ml-1 text-(--color-outl-fg-dimmer)">
                  {props.block.children.length}
                </span>
              </>
            ) : (
              "▼"
            )
          ) : (
            ""
          )}
        </button>

        {/* The list marker stays outside quote chrome so a quoted body
          * remains an ordinary outline block. */}
        {(() => {
          // Bullet gesture split (no collision):
          //   - TODO/DONE blocks render a checkbox marker (▢/▣) → click
          //     toggles the state (checkbox semantics win).
          //   - A neutral `•` bullet has no other job → click zooms into
          //     the block (Roam/Workflowy focus). When zoom isn't wired
          //     (`onFocusBlock` absent) it falls back to the TODO toggle,
          //     preserving the old "click a plain bullet to add TODO".
          const zoomableBullet = props.block.todo === null && !!props.cb.onFocusBlock;
          const bullet = (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                if (zoomableBullet) props.cb.onFocusBlock?.(props.block.id);
                else void props.cb.onToggleTodo(props.block.id);
              }}
              {...(isInteractive() ? { "data-todo": "true" } : {})}
              class={`outl-row-chrome mt-[5px] mr-2 w-3 shrink-0 cursor-pointer select-none text-center text-[13px] leading-none transition-opacity hover:opacity-70 ${bulletClass()}`}
              title={
                props.block.todo === "DONE"
                  ? "Click to uncheck"
                  : props.block.todo === "DOING"
                    ? "Click to mark done"
                    : props.block.todo === "TODO"
                      ? "Click to mark doing"
                      : zoomableBullet
                        ? "Click to zoom in"
                        : "Click to mark as TODO"
              }
              aria-label={
                props.block.todo === "DONE"
                  ? "Mark not done"
                  : props.block.todo === "DOING"
                    ? "Mark done"
                    : props.block.todo === "TODO"
                      ? "Mark doing"
                      : zoomableBullet
                        ? "Zoom in on block"
                        : "Mark as TODO"
              }
            >
              {bulletGlyph()}
            </button>
          );
          const body = (
            <div class={`min-w-0 flex-1 leading-snug ${bodyClass()}`}>
              {(() => {
                const fence = !isEditing()
                  ? detectFence(props.block.text)
                  : null;
                return (
                  <Show
                    when={isEditing()}
                    fallback={
                      fence ? (
                        <CodeFenceView
                          blockId={props.block.id}
                          language={fence.language}
                          body={fence.body}
                          onEdit={() => {
                            setDraft(rawTextWithTodo(props.block));
                            props.cb.onStartEdit(props.block.id);
                            focusTextarea();
                          }}
                          onRun={() => props.cb.onRunCodeBlock(props.block.id)}
                          onOpenPage={props.cb.onOpenPage}
                        />
                      ) : (
                        (() => {
                          // The chrome lives on the wrapper a level
                          // up — here we just strip the `> ` from the
                          // tokens so the marker doesn't double-paint.
                          const split = splitQuote(props.block.text);
                          const renderedTokens = split.quoted
                            ? stripQuoteFromTokens(props.block.tokens)
                            : props.block.tokens;
                          const hasContent = split.quoted
                            ? split.body.length > 0
                            : Boolean(props.block.text);
                          // When the block is *only* an embed (`!((blk-…))`
                          // with no surrounding prose), expand the resolved
                          // source subtree below the `↳ text` line —
                          // read-only, mirroring the TUI's child expansion.
                          const embedHandle = embedOnlyHandle(
                            props.block.tokens,
                          );
                          const embedded = embedHandle
                            ? appState.embeds[embedHandle]
                            : undefined;
                          return (
                            <>
                              <div
                                class="cursor-text whitespace-pre-wrap break-words"
                                onClick={() => {
                                  setDraft(rawTextWithTodo(props.block));
                                  props.cb.onStartEdit(props.block.id);
                                  focusTextarea();
                                }}
                              >
                                <Show
                                  when={
                                    renderedTokens && renderedTokens.length > 0
                                  }
                                  fallback={
                                    <span
                                      class={!hasContent ? "opacity-30" : ""}
                                    >
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
                                    onRefClick={props.cb.onRefClick}
                                    onTagClick={props.cb.onTagClick}
                                    onLinkClick={props.cb.onLinkClick}
                                    embeds={appState.embeds}
                                  />
                                </Show>
                              </div>
                              {/* `remind::` and friends were invisible
                                  here: the chord wrote the rule and the
                                  block looked untouched. Clicking a chip
                                  opens the rule for editing. */}
                              <PropertyEditor
                                properties={props.block.properties}
                                onCommit={(key, value) =>
                                  props.cb.onSetProperty?.(
                                    props.block.id,
                                    key,
                                    value,
                                  )
                                }
                                onError={(msg) =>
                                  setAppState("lastError", msg)
                                }
                                addOpen={
                                  appState.addPropertyBlockId ===
                                  props.block.id
                                }
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
                        })()
                      )
                    }
                  >
                    <div class="relative">
                      <textarea
                        ref={textareaRef}
                        value={draft()}
                        autofocus
                        rows={1}
                        spellcheck={false}
                        data-block-id={props.block.id}
                        class="w-full resize-none overflow-hidden bg-transparent text-current outline-none"
                        onInput={(e) => {
                          setDraft(e.currentTarget.value);
                          refreshSuggest();
                        }}
                        onSelect={() => refreshSuggest()}
                        onBlur={() => void commit()}
                        onKeyDown={handleKeydown}
                        onBeforeInput={handleBeforeInput}
                        onPaste={handlePaste}
                      />
                      <RefSuggestPopup
                        items={suggestions()}
                        activeIndex={suggestIndex()}
                        onHover={setSuggestIndex}
                        onPick={acceptSuggestion}
                      />
                      <BlockSuggestPopup
                        items={blockSuggestions()}
                        activeIndex={blockIndex()}
                        onHover={setBlockIndex}
                        onPick={acceptBlockSuggestion}
                      />
                      <EmojiSuggestPopup
                        items={emojiSuggestions()}
                        activeIndex={emojiIndex()}
                        onHover={setEmojiIndex}
                        onPick={acceptEmojiSuggestion}
                      />
                      <SlashCommandPopup
                        items={slashCommands()}
                        activeIndex={slashIndex()}
                        onHover={setSlashIndex}
                        onPick={acceptSlashCommand}
                      />
                    </div>
                  </Show>
                );
              })()}
            </div>
          );
          // Tailwind classes are passed as **string literals** so the
          // JIT discovers them at build time — the shared
          // `<QuoteWrap />` just composes the conditional `class=`.
          return (
            <>
              {bullet}
              <QuoteWrap
                quoted={isBlockQuoted(props.block.text)}
                baseClass="flex min-w-0 flex-1"
                chromeClass="rounded-r-md border-l-2 border-(--color-outl-fg-dimmer)/50 bg-(--color-outl-fg-dimmer)/[0.06] pl-2"
              >
                {body}
              </QuoteWrap>
            </>
          );
        })()}
      </div>

      <Show when={!props.block.collapsed && props.block.children.length > 0}>
        <For each={props.block.children}>
          {(child) => (
            <BlockRow
              block={child}
              depth={props.depth + 1}
              editingId={props.editingId}
              visualSet={props.visualSet}
              cb={props.cb}
            />
          )}
        </For>
      </Show>
    </div>
  );
}

/**
 * Floating page-suggestion list shown while the caret is inside an
 * open `[[…]]`. Anchored just below the block's textarea.
 *
 * Selection uses `onMouseDown` + `preventDefault` (not `onClick`): a
 * plain click would blur the textarea first, firing its `onBlur`
 * commit and tearing down edit mode before the pick registered.
 * Preventing the default on mousedown keeps focus in the textarea so
 * `acceptSuggestion` can splice the value and re-park the caret.
 */
function RefSuggestPopup(props: {
  items: PageMeta[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (page: PageMeta) => void;
}) {
  return (
    <Show when={props.items.length > 0}>
      <ul
        class="absolute top-full left-0 z-30 mt-1 max-h-56 w-72 overflow-y-auto rounded-md border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 text-[13px] shadow-lg"
        role="listbox"
      >
        <For each={props.items}>
          {(page, i) => (
            <li role="option" aria-selected={i() === props.activeIndex}>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  props.onPick(page);
                }}
                onMouseEnter={() => props.onHover(i())}
                class={`flex w-full items-center gap-1.5 px-2 py-1 text-left ${
                  i() === props.activeIndex
                    ? "bg-(--color-outl-accent) text-(--color-outl-bg)"
                    : "hover:bg-(--color-outl-bg)/50"
                }`}
              >
                <span aria-hidden="true" class="shrink-0 opacity-70">
                  {page.icon || (page.kind === "journal" ? "📅" : "📄")}
                </span>
                <span class="truncate">
                  {page.kind === "journal" ? page.slug : page.title}
                </span>
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

/**
 * Floating block-suggestion list shown while the caret is inside an
 * open `((…))`. Anchored just below the block's textarea — same pattern
 * as `RefSuggestPopup`. Each row shows the block's text snippet with its
 * hosting page slug dimmed on the right, so the user picks by content
 * (the `blk-XXXXXX` handle it inserts is never shown — it's an internal
 * id, not something the user reasons about).
 */
function BlockSuggestPopup(props: {
  items: BlockHit[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (hit: BlockHit) => void;
}) {
  return (
    <Show when={props.items.length > 0}>
      <ul
        class="absolute top-full left-0 z-30 mt-1 max-h-56 w-96 overflow-y-auto rounded-md border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 text-[13px] shadow-lg"
        role="listbox"
      >
        <For each={props.items}>
          {(hit, i) => (
            <li role="option" aria-selected={i() === props.activeIndex}>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  props.onPick(hit);
                }}
                onMouseEnter={() => props.onHover(i())}
                class={`flex w-full items-center gap-2 px-2 py-1 text-left ${
                  i() === props.activeIndex
                    ? "bg-(--color-outl-accent) text-(--color-outl-bg)"
                    : "hover:bg-(--color-outl-bg)/50"
                }`}
              >
                <span class="min-w-0 flex-1 truncate">
                  {hit.text || "(empty block)"}
                </span>
                <span class="shrink-0 truncate text-[11px] opacity-60">
                  {hit.source_slug}
                </span>
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

/**
 * Floating emoji-shortcode suggestion list shown while the caret is
 * inside an open `:shortcode` trigger. Anchored just below the block's
 * textarea — same pattern as `RefSuggestPopup`. The row shows the
 * glyph on the left and the canonical `:shortcode:` form on the right
 * so the user can scan by glyph but still see the literal that will
 * land on disk.
 */
function EmojiSuggestPopup(props: {
  items: EmojiHit[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (hit: EmojiHit) => void;
}) {
  return (
    <Show when={props.items.length > 0}>
      <ul
        class="absolute top-full left-0 z-30 mt-1 max-h-56 w-72 overflow-y-auto rounded-md border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 text-[13px] shadow-lg"
        role="listbox"
      >
        <For each={props.items}>
          {(hit, i) => (
            <li role="option" aria-selected={i() === props.activeIndex}>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  props.onPick(hit);
                }}
                onMouseEnter={() => props.onHover(i())}
                class={`flex w-full items-center gap-2 px-2 py-1 text-left ${
                  i() === props.activeIndex
                    ? "bg-(--color-outl-accent) text-(--color-outl-bg)"
                    : "hover:bg-(--color-outl-bg)/50"
                }`}
              >
                <span aria-hidden="true" class="shrink-0 text-base">
                  {hit.glyph}
                </span>
                <span class="truncate font-mono text-[12px] opacity-80">
                  :{hit.shortcode}:
                </span>
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

/**
 * Floating `/command` slash menu shown while the caret is inside a
 * block-initial `/` trigger — the desktop's inline equivalent of the
 * TUI slash overlay. Same anchoring/keyboard pattern as
 * `RefSuggestPopup`. Each row shows the command **id** monospaced (what
 * the user types, mirrors `/stats` in the CLI) with the human title
 * dimmed beside it.
 */
function SlashCommandPopup(props: {
  items: PluginCommand[];
  activeIndex: number;
  onHover: (i: number) => void;
  onPick: (cmd: PluginCommand) => void;
}) {
  return (
    <Show when={props.items.length > 0}>
      <ul
        class="absolute top-full left-0 z-30 mt-1 max-h-56 w-72 overflow-y-auto rounded-md border border-(--color-outl-border) bg-(--color-outl-bg-elev) py-1 text-[13px] shadow-lg"
        role="listbox"
      >
        <For each={props.items}>
          {(cmd, i) => (
            <li role="option" aria-selected={i() === props.activeIndex}>
              <button
                type="button"
                onMouseDown={(e) => {
                  e.preventDefault();
                  props.onPick(cmd);
                }}
                onMouseEnter={() => props.onHover(i())}
                class={`flex w-full items-center gap-2 px-2 py-1 text-left ${
                  i() === props.activeIndex
                    ? "bg-(--color-outl-accent) text-(--color-outl-bg)"
                    : "hover:bg-(--color-outl-bg)/50"
                }`}
              >
                <span class="shrink-0 font-mono text-[12px]">
                  /{cmd.command_id}
                </span>
                <span class="truncate opacity-70">{cmd.title}</span>
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}

/**
 * Rendered view of a `\`\`\`lang\n…\n\`\`\`` block. Shows the source
 * monospaced, a tiny language chip, and a Run button. Clicking the
 * source body kicks the editor in (so the user can edit the fence
 * just like any other block).
 *
 * When a plugin declares a **content transformer** for `language`, the
 * source is replaced by the transformer's rendered output:
 * - `kind: "text"` — rendered as plain, whitespace-preserving text (no
 *   client-side markdown parse — a transformer wanting rich formatting
 *   emits `kind: "rich"` HTML).
 * - `kind: "rich"` — HTML run in a sandboxed `<iframe>` **inline** in the
 *   block (one per fence, persistent while the block exists). The iframe is
 *   `sandbox="allow-scripts"` **without** `allow-same-origin` — the same
 *   isolation as the `ui-render` overlay; the plugin JS runs in a null
 *   origin with no access to the app DOM/cookies. Clicking the chip's edit
 *   affordance still drops into the raw fence editor.
 *
 * The transform runs the plugin's JS, so it is cached by `(blockId, body)`
 * (`runTransform`, `@outl/shared/plugins/transformer-registry`) and only
 * re-runs when the body changes.
 */
function CodeFenceView(props: {
  blockId: string;
  language: string;
  body: string;
  onEdit: () => void;
  onRun: () => Promise<void>;
  /** Navigate to a page by slug — wired for `call:<name>` fences so the
   *  language chip links to the template's page. */
  onOpenPage?: (slug: string) => void;
}) {
  const [busy, setBusy] = createSignal(false);

  // A `call:<name>` fence references a template page. Resolve its slug so
  // the language chip can double as a link to that page. `null` for any
  // non-`call:` fence (the resource never runs) or an unknown template
  // name (the chip stays a plain label — no dead link).
  const callName = createMemo(() => {
    const lang = props.language.toLowerCase();
    return lang.startsWith("call:") ? props.language.slice(5).trim() : null;
  });
  const [templateSlug] = createResource(callName, async (name) => {
    const templates = await listTemplates().catch(() => []);
    return templates.find((t) => t.name === name)?.slug ?? null;
  });

  async function run() {
    setBusy(true);
    try {
      await props.onRun();
    } finally {
      setBusy(false);
    }
  }

  // Reactive transformer lookup: a fence that mounts before the registry
  // loads picks the transformer up once `loadTransformers` resolves.
  const match = createMemo(() => transformerFor(props.language));

  // Run (or replay) the transformer when one matches, re-keyed by body so a
  // fence edit re-transforms. `undefined` source ⇒ resource stays unset and
  // the plain source view shows.
  const [transformed] = createResource(
    () => {
      const m = match();
      return m ? { m, body: props.body } : undefined;
    },
    (k) => runTransform(props.blockId, k.m, k.body),
  );

  // Whether to show transformed output: a transformer matched AND it
  // produced a non-null descriptor. A declined transform (null) or an
  // in-flight first run falls back to the source view.
  const result = (): PluginTransformResult | null =>
    transformed.state === "ready" ? (transformed() ?? null) : null;

  return (
    <div class="rounded-md border border-(--color-outl-fg)/10 bg-(--color-outl-bg-elev)/60">
      <div class="flex items-center justify-between border-b border-(--color-outl-fg)/10 px-2 py-1">
        <Show
          when={props.onOpenPage ? templateSlug() : null}
          fallback={
            <span class="font-mono text-[10px] uppercase opacity-60">
              {props.language}
            </span>
          }
        >
          {(slug) => (
            <button
              type="button"
              title={`Open template: ${callName()}`}
              onClick={(e) => {
                e.stopPropagation();
                props.onOpenPage?.(slug());
              }}
              class="cursor-pointer font-mono text-[10px] uppercase opacity-60 underline decoration-dotted underline-offset-2 hover:opacity-100"
            >
              {props.language}
            </button>
          )}
        </Show>
        <Show
          when={match()}
          fallback={
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void run();
              }}
              disabled={busy()}
              class="rounded bg-(--color-outl-fg)/10 px-2 py-0.5 text-[11px] hover:bg-(--color-outl-fg)/20 disabled:opacity-50"
            >
              {busy() ? "Running…" : "▶ Run"}
            </button>
          }
        >
          {/* Transformed fences aren't "run" — clicking edits the source. */}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              props.onEdit();
            }}
            class="rounded bg-(--color-outl-fg)/10 px-2 py-0.5 text-[11px] hover:bg-(--color-outl-fg)/20"
          >
            ✎ Edit
          </button>
        </Show>
      </div>
      <Switch
        fallback={
          <div onClick={props.onEdit} class="cursor-text">
            <HighlightedCode
              language={props.language}
              code={props.body || " "}
            />
          </div>
        }
      >
        <Match when={result()?.kind === "text"}>
          {/* `text` output is rendered as plain, whitespace-preserving
              text. We deliberately do NOT run a client-side markdown
              parser here (root CLAUDE.md forbids a parallel
              implementation of `outl_md`); a transformer that wants rich
              formatting emits `kind: "rich"` HTML instead. */}
          <div class="cursor-text whitespace-pre-wrap break-words px-2 py-1 leading-snug">
            {result()?.content ?? ""}
          </div>
        </Match>
        <Match when={result()?.kind === "rich"}>
          <RichFenceFrame html={result()?.content ?? ""} />
        </Match>
      </Switch>
    </div>
  );
}

/**
 * Inline sandboxed iframe for a `rich` content-transformer's HTML output.
 *
 * **Security — do not weaken.** `sandbox="allow-scripts"` with **no**
 * `allow-same-origin`: the plugin's JS runs in a null origin, isolated from
 * the app's DOM, cookies, `localStorage`, and credentialed fetch. HTML
 * enters via `srcdoc`, never `innerHTML` on the host document. This is the
 * same isolation as the `ui-render` overlay (`PluginEffectLayer`); the
 * difference is only placement (inline in the block, not a fullscreen
 * overlay) and lifetime (persistent while the block exists, not ephemeral).
 *
 * The iframe is sized to its content via a postMessage handshake the plugin
 * may opt into (`parent.postMessage({ outlHeight: n }, "*")`); absent that,
 * it falls back to a reasonable default height so the content is visible.
 */
function RichFenceFrame(props: { html: string }) {
  // Default height until the plugin reports its content height. Bounded so a
  // misbehaving plugin can't grow the iframe without limit.
  const DEFAULT_H = 240;
  const MAX_H = 2000;
  const [height, setHeight] = createSignal(DEFAULT_H);

  let frame: HTMLIFrameElement | undefined;

  function onMessage(e: MessageEvent) {
    // Only trust height reports from *this* iframe's null-origin document.
    if (frame && e.source === frame.contentWindow) {
      const h = (e.data as { outlHeight?: unknown } | null)?.outlHeight;
      if (typeof h === "number" && h > 0) {
        setHeight(Math.min(Math.ceil(h), MAX_H));
      }
    }
  }

  // Listen for the plugin's height report for this frame's lifetime;
  // removed on cleanup so a re-render (new html) doesn't leak handlers.
  window.addEventListener("message", onMessage);
  onCleanup(() => window.removeEventListener("message", onMessage));

  return (
    <iframe
      ref={frame}
      // SECURITY: allow-scripts WITHOUT allow-same-origin — the plugin JS
      // runs in a null origin, isolated from the app. Never add
      // allow-same-origin here (mirrors PluginEffectLayer / ui-render).
      sandbox="allow-scripts"
      srcdoc={props.html}
      title="content-transformer"
      style={{
        width: "100%",
        height: `${height()}px`,
        border: "0",
        display: "block",
      }}
    />
  );
}
