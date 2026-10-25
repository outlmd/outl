import {
  For,
  Show,
  batch,
  createEffect,
  createMemo,
  createResource,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import type {
  BlockNode,
  MdAheadOfLog,
  PageView,
  PluginToolbarButton,
} from "@outl/shared/api/types";
import { open } from "@tauri-apps/plugin-dialog";
import {
  attachAsset,
  type BlockHit,
  copyMarkdown,
  createBlock,
  editBlock,
  importAssetFile,
  listReminders,
  openAsset,
  openJournalFor,
  openPageBySlug,
  openExternalUrl,
  openRef,
  pageBacklinks,
  pasteMarkdown,
  peerStatus,
  pluginRun,
  pluginSyncHooks,
  pluginToolbar,
  redoPage,
  runCodeBlock,
  searchEmojis,
  searchPages,
  searchPersons,
  setBacklinksOrder,
  setBlockCollapsed,
  setBlockProperty,
  setBlockRemind,
  splitBlock,
  undoPage,
} from "@outl/shared/api/commands";
import { utf16OffsetToCharOffset } from "@outl/shared/paste";
import { isAssetLink } from "@outl/shared/links";
import { peersOnline } from "@outl/shared/peers";
import {
  findBlock,
  flattenAll,
  flattenParents,
  focusSubtree,
  rawTextWithTodo,
  visualRangeSet,
} from "@outl/shared/outline";
import {
  applyEmojiSuggestion,
  applySuggestion,
  detectEmojiContext,
  detectRefContext,
  withCreateNewPersonCandidate,
} from "@outl/shared/autocomplete";
import { PageAheadOfLogBanner, ParseWarningsBanner } from "@outl/shared/warnings";
import { createBacklinksKey } from "@outl/shared/namespace";
import { parkCaret, spliceText } from "../lib/textarea";
import { withTimeout } from "../lib/async";
import { savedMarks } from "../lib/saved";
import {
  type BlockSelection,
  selectionIsLive,
} from "../lib/block-selection";

/** Maximum time we wait for a single Tauri command to settle before
 *  surfacing a timeout error. Keeps the UI from getting stuck in
 *  "syncing…" forever when iCloud coordination stalls. */
const EDIT_TIMEOUT_MS = 8000;
import {
  HIDE_MESSAGE,
  buildEmojiShowMessage,
  buildShowMessage,
  registerPickedCallback,
  setNativeSuggesterState,
} from "../lib/native-suggester";
import { platform } from "@tauri-apps/plugin-os";
import type { ToolbarAction } from "@outl/shared/toolbar";
import { dispatchToolbarAction as dispatch } from "./Journal.toolbar-dispatch";
import { Calendar } from "./Calendar";
import { KeyboardAccessory } from "./KeyboardAccessory";
import { DevicesSheet } from "./DevicesSheet";
import { RemindersSheet } from "./RemindersSheet";
import { SettingsSheet } from "./SettingsSheet";
import { JournalDeleteDialogs } from "./JournalDeleteDialogs";
import { PluginSheet } from "./PluginSheet";
import { PluginViewOverlay } from "./PluginViewOverlay";
import { PageSwitcher } from "./PageSwitcher";
import type { MobileAction } from "../lib/palette-actions";
import { PullToRefresh } from "./PullToRefresh";
import { JournalChrome } from "./JournalChrome";
import { BlockRow } from "./BlockRow";
import { SkeletonOutline } from "./Skeleton";
import { loadTransformers } from "@outl/shared/plugins/transformer-registry";
import { createLongPress } from "../lib/long-press";
import { editableProperties } from "../lib/properties";
import { haptic } from "../lib/haptics";
import { armKeyboard, installKeyboardKeeper } from "../lib/keyboard-keeper";
import { createBlockOps, type JournalBlockDeps } from "./Journal.block-ops";
import { createNewBlockOps } from "./Journal.new-block";
import { createSelectionOps } from "./Journal.selection-ops";
import {
  type DeepLinkNavigate,
  type JournalListenerDeps,
  listenForDeepLink,
  listenForFileDrop,
  listenForWorkspaceReady,
  navigateDeepLink,
} from "./Journal.listeners";
import { installForegroundRefresh } from "@outl/shared/capture";
import {
  OPEN_TIMEOUT_MS,
  createJournalBoot,
  nextPaint,
  runBootSequence,
} from "./Journal.boot";
import { installJournalPolls } from "./Journal.polls";
import { PageSections } from "./PageSections";
import { BlockContextMenu } from "./BlockContextMenu";
import { SelectionToolbar } from "./SelectionToolbar";
import { TemplateSheet } from "./TemplateSheet";
import {
  PropertiesSheet,
  type PropertyScope,
} from "./PropertiesSheet";
import { Toast } from "./Toast";
import { BottomBar, type BottomTab } from "./BottomBar";
import { WeekStrip } from "./WeekStrip";
import { TasksSheet } from "./TasksSheet";
import { CaptureSheet } from "./CaptureSheet";
import { JournalEmptyPage, JournalLoadFailed, JournalStartWriting } from "./JournalEmpty";
import { isBlankOutline } from "@outl/shared/polish";
import { JournalZoomBar } from "./JournalZoomBar";
import { createDayNav } from "./Journal.day-nav";
import { createCapture } from "./Journal.capture";
import type { TaskHit } from "@outl/shared/api/overview";
import { journalHeading } from "@outl/shared/journal";
import { buildContextActions } from "./Journal.context-actions";

/** Whether this build runs on Android. The web keyboard accessory bar
 *  mounts only here; iOS keeps its native `OutlToolbarView` until the web
 *  bar is device-validated. `platform()` throws in a plain-browser dev
 *  server (no Tauri), so default to false there. */
function detectAndroid(): boolean {
  try {
    return platform() === "android";
  } catch {
    return false;
  }
}

export function Journal() {
  const isAndroid = detectAndroid();
  const [view, setView] = createSignal<PageView | null>(null);
  // Fetched lazily, off the page-open path (`view().backlinks` is always empty:
  // the O(blocks-in-workspace) scan blocked the first paint). The key refires
  // on a slug or `title::` change, so navigation and namespace renames refetch.
  const [backlinks, { mutate: mutateBacklinks }] = createResource(
    createBacklinksKey(() => view()?.page),
    (key) => pageBacklinks(key.slug),
  );
  const [loaded, setLoaded] = createSignal(false);
  const [refreshing, setRefreshing] = createSignal(false);
  // Loading message + failure flag drive the initial-load placeholder.
  // The `SkeletonOutline` placeholder is the user-facing signal that
  // we're still loading; `loadFailed` flips only when we give up so
  // the retry button has a clean condition to render against.
  const [loadFailed, setLoadFailed] = createSignal(false);
  const [editingId, setEditingId] = createSignal<string | null>(null);
  // Zoom/focus view-state — local per device, never round-trips to the
  // backend (we already hold the whole outline). When non-null, only the
  // focused block's subtree renders as the outline root. Reset to null on
  // page change (see `applyView`).
  const [focusBlockId, setFocusBlockId] = createSignal<string | null>(null);
  const [draft, setDraft] = createSignal("");
  const [error, setError] = createSignal<string | null>(null);
  // Optional retry handler tied to the most recent error. When set,
  // the toast pins (no auto-dismiss) and shows a "Retry" button. We
  // store it alongside `error` so callers can offer the affordance
  // without plumbing it through every async helper.
  const [errorRetry, setErrorRetry] = createSignal<(() => void) | null>(null);
  const [switcherOpen, setSwitcherOpen] = createSignal(false);
  const [calendarOpen, setCalendarOpen] = createSignal(false);
  const [devicesOpen, setDevicesOpen] = createSignal(false);
  const [remindersOpen, setRemindersOpen] = createSignal(false);
  const [pluginsOpen, setPluginsOpen] = createSignal(false);
  const [settingsOpen, setSettingsOpen] = createSignal(false);
  // Plugin-contributed toolbar buttons — one inline glyph each in the
  // header. Loaded after the workspace opens (plugins load lazily on the
  // host's first request), refreshed alongside the plugin-command list.
  const [toolbarButtons, setToolbarButtons] = createSignal<
    PluginToolbarButton[]
  >([]);
  // When set, the delete-confirmation dialog is open. Holds the
  // block id we're about to delete + a descendant count for the
  // copy. Cleared on confirm or cancel.
  const [pendingDelete, setPendingDelete] = createSignal<
    { id: string; descendants: number } | null
  >(null);
  // Block id whose contextual menu is currently open (long-press
  // gesture target). `null` when no menu is showing.
  const [contextMenuBlockId, setContextMenuBlockId] = createSignal<
    string | null
  >(null);
  // Block the template picker instantiates under. Set from the block
  // long-press menu ("Insert template"); `null` keeps the sheet closed.
  const [templateBlockId, setTemplateBlockId] = createSignal<string | null>(
    null,
  );
  // Properties sheet target. `blockId` is the long-pressed block (null
  // when the sheet was opened from the page's own chips); `scope` picks
  // which side it lands on. `null` keeps the sheet closed.
  const [propertiesTarget, setPropertiesTarget] = createSignal<{
    blockId: string | null;
    scope: PropertyScope;
  } | null>(null);
  // Block clipboard (RFC 0254 phase 2, cut added phase 4b) — "Copy
  // block" arms this with the copied subtree's markdown; "Cut block"
  // arms it with the same shape (the backend deletes the source in
  // the same round-trip, see `blockOps.cutBlock`). "Paste block" (shown
  // only while armed) duplicates it after the long-pressed block via
  // `paste_block_after`, minting fresh ids either way — unlike the
  // desktop's `appState.blockClipboard`, which tags a cut with
  // `{ kind: "cut", nodeId }` and pastes it as an identity-preserving
  // move. Mobile's `cut_block` mints fresh ids on paste instead (see
  // its doc comment), so one plain markdown string covers both here.
  const [blockClipboard, setBlockClipboard] = createSignal<string | null>(
    null,
  );
  // Touch-native multi-block selection (RFC 0254 phase 3). Entered
  // from a block's long-press menu ("Select blocks"); `null` means no
  // selection is active. `lastSelection` is the vim-`gv` equivalent
  // ("Reselect last selection") — captured on every exit, live or
  // not (`selectionIsLive` gates whether the menu offers it back).
  const [selection, setSelection] = createSignal<BlockSelection | null>(null);
  const [lastSelection, setLastSelection] = createSignal<BlockSelection | null>(
    null,
  );
  // A range delete is destructive across N blocks (any of which may
  // carry children), so — unlike the single-block swipe-to-delete,
  // which only prompts when that one block has descendants — a range
  // delete always confirms. Holds the snapshotted target ids.
  const [pendingRangeDelete, setPendingRangeDelete] = createSignal<
    string[] | null
  >(null);
  // Membership set for the active range, memoised once per (selection,
  // outline) change — every `<BlockRow />` answers "am I selected?"
  // with `.has(id)` in O(1) instead of re-walking the outline per row.
  // Mirrors the desktop's `visualSet` (`outl-desktop/CLAUDE.md` → vim
  // parity).
  const selectionSet = createMemo(() => {
    const sel = selection();
    const cur = view();
    if (!sel || !cur) return null;
    return visualRangeSet(sel.anchorId, sel.cursorId, cur.outline);
  });
  /** Press-and-hold on the page title opens the sheet on the page's
   *  own properties. It is the only door that does not need a block:
   *  a page with no blocks has nothing to long-press, and `icon::` /
   *  `type::` are page metadata anyway, so routing them through a
   *  block was always the indirect path. */
  const titleLongPress = createLongPress({
    onLongPress: () => {
      if (!pageId()) return;
      haptic("medium");
      setPropertiesTarget({ blockId: null, scope: "page" });
    },
  });

  const [syncing, setSyncing] = createSignal(false);
  // PRIMARY sync signal: is at least one iroh peer reachable right now?
  // Polled from the transport's own dial outcomes (`peerStatus()` →
  // `peer_health()`), NOT from `navigator.onLine`. The phone having WiFi
  // says nothing about whether a P2P peer answered — iroh is outl's
  // default transport, so the dot must reflect the mesh, not the radio.
  // `false` means nothing to sync with (no peers paired, or all down).
  const [peersUp, setPeersUp] = createSignal(false);
  // SECONDARY signal — drives the `<SyncDot>` "offline" pill when the
  // device itself is offline (truly no radio → no peer can be up
  // anyway). `navigator.onLine` is not perfectly accurate (it lies when
  // a captive portal eats requests) but it's a cheap floor.
  const [online, setOnline] = createSignal(
    typeof navigator !== "undefined" ? navigator.onLine : true,
  );

  // Poll the iroh transport's per-peer health so the dot tracks the live
  // mesh. Best-effort: a failed probe leaves the last value rather than
  // flapping the dot to offline on a transient error.
  async function refreshPeerStatus() {
    try {
      setPeersUp(peersOnline(await peerStatus()));
    } catch {
      // keep the previous value; the next tick retries
    }
  }
  // Single in-flight `editBlock` lock. Two concurrent edits to the
  // same block can land in arbitrary order at the backend (e.g.
  // toggle-todo's optimistic commit racing with a delayed onBlur
  // commit), and the loser overwrites the winner. We serialize so
  // the user's last keystroke always wins.
  let commitInFlight: Promise<unknown> | null = null;
  const [activeTextareaSignal, setActiveTextareaSignal] = createSignal<
    HTMLTextAreaElement | null
  >(null);
  let activeTextarea: HTMLTextAreaElement | undefined;

  // See `applyView`: kept out of the `PageView` signal so an edit commit
  // (which replies without the flag) can't silently drop it. Keyed by
  // slug so navigating to another page clears it.
  const [aheadOfLog, setAheadOfLog] = createSignal<{
    slug: string;
    info: MdAheadOfLog;
  } | null>(null);

  function applyView(v: PageView) {
    if (v.projection_error) setError(v.projection_error);
    // Dropping the zoom on a page switch keeps focus scoped to the page
    // it was set on. A same-page refresh (background poll, edit commit)
    // keeps it — `focusSubtree` re-resolves the id against the fresh
    // outline every render, and falls back to the full page if the block
    // vanished.
    if (v.page.slug !== view()?.page.slug) {
      setFocusBlockId(null);
      // A range selection is scoped to the page it was started on —
      // block ids from another page would resolve to nothing (or,
      // worse, to an unrelated same-id-shaped block after a future
      // cross-page id collision that can't happen today but shouldn't
      // be assumed). Drop it rather than carry stale anchor/cursor
      // ids across a navigation the user didn't ask the selection to
      // survive.
      setSelection(null);
    }
    // "This page isn't syncing" is sticky per page across the replies
    // that cannot answer: only the open commands attempt the
    // re-projection that discovers it, so a mutation reply never carries
    // the flag. Reading it off the current view would clear the banner
    // on the user's first edit — the exact action it warns against,
    // since a local edit re-projects the page and overwrites the
    // unlogged lines.
    //
    // `md_ahead_of_log_checked` marks a reply that *did* run the check,
    // and that one is authoritative in both directions: no notice means
    // the page is healthy again (`outl reconcile --ahead-of-log` ran on
    // a computer), so the banner has to go. Sticking past it would leave
    // a page that syncs wearing a permanent "not syncing" warning.
    if (v.md_ahead_of_log) {
      setAheadOfLog({ slug: v.page.slug, info: v.md_ahead_of_log });
    } else if (v.md_ahead_of_log_checked || aheadOfLog()?.slug !== v.page.slug) {
      setAheadOfLog(null);
    }
    setView(v);
  }

  /** Open the page a reminder is on and show its block, for the
   *  notification tap. Scrolling is preferred over zooming: zooming
   *  narrows a page the user did not ask to narrow. */
  async function navigateToBlock(slug: string, blockId: string) {
    const v = await openPageBySlug(slug);
    applyView(v);
    // One frame, so the outline has rendered. A block under a collapsed
    // ancestor has no node, and `?.` made that a silent no-op: the page
    // opened with the reminder's block nowhere on it. Zoom always renders.
    requestAnimationFrame(() => {
      const sel = `[data-block-id="${CSS.escape(blockId)}"]`;
      const el = document.querySelector(sel);
      if (el) el.scrollIntoView({ block: "center", behavior: "smooth" });
      else setFocusBlockId(blockId);
    });
  }

  // Imperative bridge to `<PluginViewOverlay />`: it hands us its `push`
  // fn on mount so any path that receives plugin `ctx.ui.render` payloads
  // (the sheet's `run`, the `commitEdit` hook sweep) can paint a sandboxed
  // iframe overlay without threading state through the tree.
  let pushPluginView: ((html: string) => void) | undefined;
  function showPluginViews(views: string[] | undefined) {
    if (!views || !pushPluginView) return;
    for (const html of views) pushPluginView(html);
  }

  // Refresh the plugin-contributed toolbar buttons. Best-effort: plugins
  // load lazily on the host's first request, so this is called after the
  // workspace opens (a host with no toolbar plugins returns an empty list).
  async function loadToolbar() {
    try {
      setToolbarButtons(await pluginToolbar());
    } catch {
      setToolbarButtons([]); // never let a plugin failure break the header
    }
  }

  // Run a plugin's toolbar command. Mirrors `<PluginSheet />`'s `run`:
  // surface `notify` / error output as a toast, paint any `ctx.ui.render`
  // overlays, and re-render the on-screen page from the refreshed
  // `PageView` (the host re-projects every page before returning, since a
  // plugin can move blocks across pages). Guarded by `!editingId()` so it
  // never resets a textarea mid-edit.
  async function runToolbarButton(btn: PluginToolbarButton) {
    haptic("light");
    try {
      const reply = await pluginRun(btn.plugin_id, btn.command_id, pageId());
      for (const note of reply.notifications) setError(note);
      for (const err of reply.errors) setError(`plugin: ${err}`);
      showPluginViews(reply.views);
      if (reply.view && !editingId()) applyView(reply.view);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  // Native bridges + reactive effects MUST register synchronously,
  // before any `await`. Solid loses the owner context across an
  // `await` boundary, so `createEffect` / `onCleanup` called after
  // an awaited call become orphans — the effect never tracks
  // signals, the cleanup never fires. Specifically: putting
  // `registerNativeSuggesterBridge()` after `await loadTodayWithRetry()`
  // is what made the ref autocomplete look broken on iOS: state was
  // published once and then never updated as the user typed inside
  // `[[…]]`.
  registerNativeToolbarBridge();
  registerNativeSuggesterBridge();

  // The two workspace-open paths and their user-visible gates
  // (`Journal.boot.ts`): every call capped, every gate cleared in a
  // `finally`.
  const boot = createJournalBoot({
    view,
    editingId,
    applyView,
    setError,
    setLoaded,
    setLoadFailed,
    setSyncing,
    withError,
    refreshPeerStatus,
  });
  const { loadTodayWithRetry, pullAndReload } = boot;
  // Connectivity listeners + the background tickers (`Journal.polls.ts`).
  // Registered here (the owner `onCleanup` needs), STARTED from `onMount`
  // after the boot load: a poll started during render queues `peer_status`
  // — a full workspace load — on the iOS main thread ahead of the
  // journal's own first paint, which is the watchdog kill.
  const polls = installJournalPolls({
    setOnline,
    refreshPeerStatus,
    pullAndReload,
    navigateToBlock,
    setError: (m) => setError(m),
  });

  // Day navigation, today's slug and wake-to-today (`Journal.day-nav.ts`).
  const dayNav = createDayNav({ view, editingId, applyView, withError });
  /**
   * Which day the week strip highlights, and `null` to hide it.
   *
   * Before the first reply lands this is today's slug, because that is
   * knowable without the workspace: `today_slug` is pure date math, it
   * takes no lock, and a cold start always opens today. Holding the strip
   * back until the outline arrived only bought a layout shift under the
   * user's thumb — the rows moved down the moment the page landed
   * (`UX.md` → Never block a keystroke: a load state belongs to the
   * section whose own content is missing, and the week's dates are not
   * missing).
   */
  const weekStripSlug = createMemo(() => {
    const v = view();
    if (!v) return dayNav.today();
    return v.page.kind === "journal" ? v.page.slug : null;
  });
  const capture = createCapture({
    view,
    today: dayNav.today,
    applyView,
    withError,
    notify: (text) => setError(text),
  });
  const [tasksOpen, setTasksOpen] = createSignal(false);
  const [captureOpen, setCaptureOpen] = createSignal(false);

  // Drain a reload that was deferred because the user was editing. The moment
  // they leave edit mode (`editingId()` → null), apply the peer's changes that
  // arrived meanwhile — in the background so it doesn't flash the spinner.
  // Guarded so it only fires on the edit→idle transition, not on every keypress.
  createEffect(() => {
    if (editingId() === null && boot.takePendingReload()) {
      void pullAndReload({ background: true });
    }
  });

  /**
   * Cold-start deep link: a URL that *launched* the app was buffered by
   * the backend before `listenForDeepLink` existed. Drain it once the
   * workspace is open and let the target override today's journal. A
   * normal launch returns null and keeps the journal.
   *
   * Capped like every other open in the boot path: this is a main-thread
   * command too, and the boot sequence waits on it (so a deep link can't
   * be undone by the first sync reopening today) — which means a stall
   * here would also hold back the tickers.
   */
  async function drainColdStartDeepLink() {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const pending = await withTimeout(
        invoke<DeepLinkNavigate | null>("take_pending_deep_link"),
        OPEN_TIMEOUT_MS,
        "Opening the link",
      );
      if (pending) await navigateDeepLink(listenerDeps, pending);
    } catch {
      // best-effort — a failed drain just leaves the journal showing
    }
  }

  onMount(() => {
    // The off-screen input that raises the iOS keyboard inside a tap's
    // gesture window (`lib/keyboard-keeper.ts`). Installed here, at
    // first paint, so it is already laid out when the first "+" of the
    // session is pressed — not created inside the gesture that needs it.
    installKeyboardKeeper();
    // Listeners first: registering them costs nothing (no command, no
    // workspace), and `workspace-ready` can fire while the load below is
    // still in flight.
    listenForWorkspaceReady(listenerDeps);
    listenForDeepLink(listenerDeps);
    listenForFileDrop(listenerDeps);
    // Order lives in `Journal.boot.ts` — including why the force-sync that
    // used to run here, ahead of the load, now runs as `firstSync` one
    // painted frame later.
    void runBootSequence({
      load: loadTodayWithRetry,
      painted: nextPaint,
      deepLink: drainColdStartDeepLink,
      // Opening the app: pull whatever peers produced while it was closed,
      // so the user sees fresh state without hitting refresh. This is the
      // boot's only dial — one pass, started after the journal is up.
      firstSync: () => void pullAndReload(),
      catchUp: () => {
        // Plugin toolbar buttons and content transformers (plugin-claimed
        // code-fence languages) both load lazily on the host's first
        // request. Best-effort: a host with no plugins leaves the header
        // unchanged and fences as plain highlighted code.
        void loadToolbar();
        void loadTransformers();
        // Back in the foreground: pull peers' edits and drain the
        // capture inbox.
        installForegroundRefresh({
          pull: () => pullAndReload(),
          notify: (text) => setError(text),
        });
        // Last: arm the recurring tickers. Every one of them calls a
        // Tauri command, so they go behind everything the user is waiting
        // for rather than into the render pass.
        polls.startPolling();
      },
    });
  });

  /**
   * Drive the native ref suggester (UIKit chip strip above the
   * toolbar — see `main.mm` → `OutlSuggestView` /
   * `OutlAccessoryContainer`). UIKit polls
   * `window.__outlSuggesterState` every 150ms while the keyboard is
   * up; tap → `window.__outlSuggesterPicked(slug, kind)` calls back
   * into here.
   */
  function registerNativeSuggesterBridge() {
    const cleanup = registerPickedCallback((slug, kind) => {
      const el = activeTextareaSignal();
      if (!el) return;
      // Emoji branch: the chip strip published `:shortcode:` candidates,
      // tap returns the shortcode. Use `detectEmojiContext` (the same
      // trigger detector the effect below ran) + `applyEmojiSuggestion`
      // so the disk form stays the canonical `:shortcode:` literal.
      if (kind === "emoji") {
        const ctx = detectEmojiContext(el.value, el.selectionStart ?? 0);
        if (!ctx) return;
        const result = applyEmojiSuggestion(el.value, ctx, slug);
        const insert = result.value.slice(ctx.openIndex, result.caret);
        spliceText(el, ctx.openIndex, ctx.replaceEnd, insert);
        parkCaret(el, result.caret);
        setDraft(el.value);
        parkCaret(el, result.caret);
        setNativeSuggesterState(null);
        return;
      }
      const ctx = detectRefContext(el.value, el.selectionStart ?? 0);
      if (!ctx) return;
      // Mention sugar: materialise the person page in the backend
      // (fire-and-forget) so the inserted `[[@title]]` link resolves
      // on subsequent loads. Idempotent — `open_or_create_by_ref`
      // strips the `@`, sets `type:: person` on a fresh page, and
      // returns the existing node otherwise. Same policy desktop +
      // TUI apply on the same gesture.
      if (ctx.kind === "mention") {
        void openRef(`@${slug}`).catch((e) => {
          console.warn("openRef for mention failed:", e);
        });
      }
      // Build the result through the pure helper so its semantics
      // (e.g. choosing `[[` vs `((` delimiters) stay one place, but
      // apply it via `spliceText` + `parkCaret` to dodge the
      // Solid-binding caret-reset trap that bit `el.value = …`.
      const result = applySuggestion(el.value, ctx, slug);
      const insert = result.value.slice(ctx.openIndex, result.caret);
      spliceText(el, ctx.openIndex, ctx.replaceEnd, insert);
      parkCaret(el, result.caret);
      setDraft(el.value);
      parkCaret(el, result.caret);
      setNativeSuggesterState(null);
    });
    onCleanup(cleanup);

    let queryToken = 0;
    let lastQuery: string | null = null;
    createEffect(() => {
      const el = activeTextareaSignal();
      const text = draft();
      if (!el || !editingId()) {
        if (lastQuery !== null) {
          setNativeSuggesterState(null);
          lastQuery = null;
        }
        return;
      }
      const cursor = el.selectionStart ?? text.length;
      // Emoji takes precedence over ref detection because both can be
      // active at the same caret position (a `:` typed inside a stray
      // `[[…` would otherwise stay invisible). Bail to the ref branch
      // only when no `:shortcode` trigger is open.
      const emojiCtx = detectEmojiContext(el.value, cursor);
      if (emojiCtx) {
        const key = `emoji:${emojiCtx.query}`;
        if (key === lastQuery) return;
        lastQuery = key;
        const token = ++queryToken;
        // `limit: 8` mirrors every other client's autocomplete cap so
        // the chip strip doesn't overflow on long substring queries.
        void searchEmojis(emojiCtx.query, 8).then((hits) => {
          if (token !== queryToken) return;
          if (hits.length === 0) {
            setNativeSuggesterState(HIDE_MESSAGE);
            return;
          }
          setNativeSuggesterState(buildEmojiShowMessage(hits));
        });
        return;
      }
      const ctx = detectRefContext(el.value, cursor);
      // `page` → fuzzy over every page; `mention` → fuzzy over
      // persons only. Block-ref autocompletion stays out of this path.
      if (!ctx || (ctx.kind !== "page" && ctx.kind !== "mention")) {
        if (lastQuery !== null) {
          setNativeSuggesterState(null);
          lastQuery = null;
        }
        return;
      }
      const key = `${ctx.kind}:${ctx.query}`;
      if (key === lastQuery) return;
      lastQuery = key;
      const token = ++queryToken;
      const fetcher = ctx.kind === "mention" ? searchPersons : searchPages;
      const mention = ctx.kind === "mention";
      fetcher(ctx.query).then((items) => {
        if (token !== queryToken) return;
        // Create-new affordance for mentions — shared with desktop
        // via `@outl/shared/autocomplete::withCreateNewPersonCandidate`.
        const finalItems = mention
          ? withCreateNewPersonCandidate(items, ctx.query)
          : items;
        if (finalItems.length === 0) {
          setNativeSuggesterState(HIDE_MESSAGE);
          return;
        }
        setNativeSuggesterState(buildShowMessage(finalItems, { mention }));
      });
    });
  }

  // ── Platform listeners (issue #265 phase 3) ─────────────────────
  //
  // Bodies live in `Journal.listeners.ts`; this is only the dependency
  // wiring. They must be *called* from here — `onCleanup` needs the
  // component owner.
  const listenerDeps: JournalListenerDeps = {
    view,
    editingId,
    applyView,
    setError,
    setAheadOfLog,
    loadTodayWithRetry,
    pullAndReload,
    onFileDrop: (paths, blockId) => handleFileDrop(paths, blockId),
    openCapture: () => setCaptureOpen(true),
  };

  /**
   * Bridge between the native UIKit keyboard accessory view (defined
   * in `gen/apple/Sources/outl-mobile/main.mm`) and the Solid handlers
   * below. The native buttons call `evaluateJavaScript` with
   * `window.__outlToolbar(action)`; the switch (and the tap counting)
   * lives in `Journal.toolbar-dispatch.ts`, shared with the Android bar.
   */
  function dispatchToolbarAction(action: string) {
    dispatch(action, {
      editingId,
      indent: blockOps.indent,
      outdent: blockOps.outdent,
      moveUp: blockOps.moveUp,
      moveDown: blockOps.moveDown,
      undo: () => void handleUndo(),
      redo: () => void handleRedo(),
      toggleTodo: blockOps.toggleTodo,
      delete: blockOps.requestDelete,
      createAfter: handleCreateAfter,
      appendBlock: () => void newBlock.appendBlock(),
      wrapSelection,
      insertPair: (open, close) => insertAtCursor("pair", open, close),
      insertText: (text) => insertAtCursor("text", text),
      commitEdit,
    });
  }

  function registerNativeToolbarBridge() {
    (window as unknown as {
      __outlToolbar?: (action: string) => void;
    }).__outlToolbar = dispatchToolbarAction;
  }

  async function withError<T>(fn: () => Promise<T>): Promise<T | undefined> {
    try {
      setError(null);
      return await fn();
    } catch (e) {
      setError(String(e));
      haptic("warning");
      return undefined;
    }
  }

  function pageId(): string | null {
    return view()?.page.id ?? null;
  }

  /**
   * The active zoom, resolved against the live outline. `null` when not
   * zoomed OR when the focused block vanished (stale target) — both cases
   * fall back to rendering the full page. A `createMemo` (not a plain
   * function) so the `focusSubtree` tree walk runs once per relevant
   * state change instead of on every read: it's read multiple times per
   * render (`<Show when={focusView()}>`, `outlineRoots()`), and on a
   * large page the O(N) walk per read is noticeable. Still resolves
   * against the live outline, so an edit / collapse inside the zoom stays
   * reflected — the memo re-runs whenever `focusBlockId` or `view` moves.
   */
  const focusView = createMemo(() => {
    const id = focusBlockId();
    const cur = view();
    if (!id || !cur) return null;
    return focusSubtree(cur.outline, id);
  });

  /** Blocks to render as the outline root: the focused subtree when
   *  zoomed, else the whole page. */
  function outlineRoots(): BlockNode[] {
    const fv = focusView();
    return fv ? [fv.root] : (view()?.outline ?? []);
  }

  function startEdit(id: string, initial: string) {
    batch(() => {
      setEditingId(id);
      setDraft(initial);
    });
    haptic("light");
  }

  async function commitEdit() {
    const id = editingId();
    const pid = pageId();
    if (!id || !pid) return;
    const text = draft();

    // Nothing typed — leave without writing. The draft was seeded
    // from `rawTextWithTodo`, which rebuilds the text from the DTO's
    // split `todo` + `text`, so it comes back in the canonical word
    // form even when the block on disk is written as a checkbox
    // (`[ ] buy milk`). Committing unconditionally therefore rewrote
    // that block to `TODO buy milk` on a tap-in / tap-out with no
    // keystroke, which is a real `Op::Edit` and a silent loss of the
    // user's spelling. The desktop has had this guard all along
    // (`BlockRow.tsx` → `commit`).
    const current = findBlock(view()?.outline ?? [], id);
    if (current && text === rawTextWithTodo(current)) {
      setEditingId(null);
      return;
    }
    // Serialize: if an earlier edit is still in flight, wait for it
    // to land before we send this one. Without this, a quick
    // sequence like (type → toggle TODO → blur) can hit the
    // backend out of order and the older edit overwrites the newer.
    if (commitInFlight) {
      try {
        await commitInFlight;
      } catch {
        // ignore — we still want our own commit to try
      }
    }
    setSyncing(true);
    const op: Promise<PageView> = withTimeout(
      editBlock(pid, id, text),
      EDIT_TIMEOUT_MS,
      "Save is taking too long",
    );
    commitInFlight = op;
    const next = await withError(() => op);
    if (commitInFlight === op) commitInFlight = null;
    setSyncing(false);
    if (next) {
      savedMarks.mark(id);
      // Only drop out of edit mode once the backend confirmed the
      // save. If it failed, `withError` already surfaced the
      // message and we leave the editor open with the draft intact
      // so the user can retry instead of silently losing the text.
      setEditingId(null);
      applyView(next);
      // Fire the plugins' `onOp` sweep once, after the commit lands.
      // `sync_hooks` dispatches EVERY op since the host's last sweep
      // (not just this edit), so one call here also catches up the
      // structural ops (indent / move / delete) that don't route
      // through `commitEdit` — mirrors the desktop's single
      // `OutlineView.onCommit` hook + the TUI's once-per-tick sweep.
      // Best-effort: a host with no op-hook plugins is a cheap no-op,
      // and any failure stays out of the edit path entirely.
      void (async () => {
        try {
          const reply = await pluginSyncHooks(pid);
          // Paint any `ctx.ui.render` payloads the hooks emitted — this is
          // the confetti path: marking a block DONE → commit → this sweep
          // → a confetti plugin emits HTML → sandboxed iframe overlay.
          // Independent of the mutation guard below: a view can fire even
          // when the workspace didn't change.
          showPluginViews(reply.views);
          for (const err of reply.errors) setError(err);
          // Re-render only if a hook actually mutated the workspace AND
          // the user hasn't started editing again in the meantime (so
          // we never reset a fresh textarea mid-edit).
          if (reply.view && !editingId()) applyView(reply.view);
        } catch {
          // Plugins must never break editing.
        }
      })();
    } else if (error()) {
      // Save failed (timeout, backend error, etc). Offer a retry
      // affordance — the draft is still in the editor, so the
      // user's text is not lost.
      setErrorRetry(() => () => {
        void commitEdit();
      });
    }
  }

  /**
   * Apply an external-clipboard markdown paste to the workspace.
   *
   * `BlockRow`'s textarea has already detected via `looksLikeOutline`
   * that the payload deserves the outline → blocks conversion and
   * called `preventDefault` on the original paste event. We commit
   * any in-flight draft first (the host block's text would otherwise
   * race with the paste's `AtCaret` splice), hand the raw text to
   * the backend, then re-apply the resulting `PageView`.
   */
  async function handlePasteMarkdown(blockId: string, caret: number, text: string) {
    const pid = pageId();
    if (!pid) return;
    if (editingId() === blockId) {
      // Flush whatever the user was typing so the splice operates on
      // the workspace state the textarea is showing, not on stale
      // backend text.
      const draftText = draft();
      const committed = await withError(() => editBlock(pid, blockId, draftText));
      if (committed) setView(committed);
    }
    const next = await withError(() => pasteMarkdown(pid, blockId, caret, text));
    if (next) applyView(next);
  }

  // ── Block + range operations (issue #265 phases 1-2) ────────────
  //
  // The single-block handlers live in `Journal.block-ops.ts` and the
  // selection state machine plus its range ops in
  // `Journal.selection-ops.ts`. Both are handed accessors, never
  // values: a destructured prop freezes at first render in Solid, and
  // the same rule governs a dependency object.
  const blockDeps: JournalBlockDeps = {
    pageId,
    view,
    withError,
    applyView,
    setView,
    editingId,
    setEditingId,
    draft,
    setDraft,
    blockClipboard,
    setBlockClipboard,
    setPendingDelete,
  };
  const blockOps = createBlockOps(blockDeps);
  // The two creates reached from a tap with the keyboard down. They are
  // their own module because arming the keyboard before the first await
  // is the rule they exist to hold — see `Journal.new-block.ts`.
  const newBlock = createNewBlockOps({
    pageId,
    editingId,
    commitEdit,
    applyView,
    startEdit,
    withError,
  });
  const selectionOps = createSelectionOps({
    ...blockDeps,
    selection,
    setSelection,
    lastSelection,
    setLastSelection,
    setPendingRangeDelete,
    commitEdit,
  });








  // ── Touch-native block range selection (RFC 0254 phase 3) ────────
  //
  // Mobile has no keyboard and deliberately no modal vim Visual state
  // (the RFC rejects one explicitly — a hidden mode on a touch surface
  // is worse than a gesture the user can see). The anchor + cursor
  // model is the desktop's Visual mode unchanged (`visualRangeIds` /
  // `visualRangeSet` from `@outl/shared/outline`); only how it's
  // *driven* differs — a long-press menu item starts it, a tap on any
  // other block extends it, a floating toolbar (`<SelectionToolbar />`)
  // fires the same range ops the desktop's `>` / `<` / `⌘⇧↑↓` / `y` /
  // `d` chords do.












  /**
   * Revert the last committed block mutation on this page
   * (`outl_tauri_shared::commands::history::undo_page`, RFC 0254 phase
   * 1 — the same shared body the desktop's `Cmd+Z` calls). Commits any
   * in-flight draft first: undo walks *committed* mutations, so an
   * uncommitted keystroke would otherwise sit invisibly ahead of
   * whatever `undo_page` restores. `withError` surfaces "nothing to
   * undo" as a toast rather than a silent no-op — the fired keyboard
   * button and the console line the desktop had before this UI existed
   * would read identically to a broken tap.
   */
  async function handleUndo() {
    const pid = pageId();
    if (!pid) return;
    if (editingId()) await commitEdit();
    const next = await withError(() => undoPage(pid));
    if (next) applyView(next);
  }

  /** Re-apply the mutation the last {@link handleUndo} reverted. */
  async function handleRedo() {
    const pid = pageId();
    if (!pid) return;
    if (editingId()) await commitEdit();
    const next = await withError(() => redoPage(pid));
    if (next) applyView(next);
  }

  /**
   * Flip the collapsed flag on a block. The backend generates
   * `Op::SetCollapsed`, applies it through the op log (same path as
   * every other mutation), and returns a fresh page view so the
   * renderer picks up the new flag in the same frame the user tapped
   * the triangle. The sidecar is not touched — fold state syncs
   * device-to-device via the per-actor jsonl, not the `.outl` file.
   */
  async function handleToggleCollapse(id: string, next: boolean) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const updated = await withError(() => setBlockCollapsed(pid, id, next));
    if (updated) applyView(updated);
  }

  /**
   * Walk the whole page and set every block's `collapsed` flag to
   * `value` — mirrors the desktop's `applyCollapsedToAll` exactly
   * (RFC 0254 phase 4b: `FoldAll` / `UnfoldAll` have no bulk backend
   * op, each flip is its own `Op::SetCollapsed` so concurrent flips
   * converge via HLC). **Never `flattenVisible`** — the point of
   * "unfold all" is to expand subtrees hidden under an already-
   * collapsed parent, and a visible-only walk would no-op on every
   * descendant of a folded node.
   *
   * `value=true` (fold) uses `flattenParents` so leaves are skipped:
   * folding a leaf is invisible today, but `set_block_collapsed`
   * always writes the op (a CRDT contract — every flip must land so
   * concurrent flips converge), so a leaf folded now would surprise
   * the user the next time they add a child under it. `value=false`
   * (unfold) uses `flattenAll`: unfolding a leaf has no future effect
   * and keeps the op count symmetric with the TUI's `collect_collapse_candidates`.
   */
  async function applyCollapsedToAll(value: boolean) {
    const pid = pageId();
    const cur = view();
    if (!pid || !cur) return;
    haptic("light");
    const ids = value ? flattenParents(cur.outline) : flattenAll(cur.outline);
    let lastView: PageView | undefined;
    for (const id of ids) {
      const updated = await withError(() => setBlockCollapsed(pid, id, value));
      if (updated) lastView = updated;
    }
    if (lastView) applyView(lastView);
  }

  function handleFoldAll() {
    void applyCollapsedToAll(true);
  }

  function handleUnfoldAll() {
    void applyCollapsedToAll(false);
  }

  /**
   * Zoom in on a block: tapping its bullet makes that block the outline
   * root (Roam/Workflowy style). Pure view-state — no backend call, the
   * client already holds the whole outline.
   */
  function handleFocusBlock(id: string) {
    haptic("light");
    setFocusBlockId(id);
  }

  /**
   * Zoom out one level. Derived (no stack): re-resolve the current focus
   * against the live outline; go to its parent when there's a breadcrumb,
   * else leave zoom entirely. A stale target (block gone) also exits.
   */
  function handleZoomOut() {
    const id = focusBlockId();
    const cur = view();
    if (!id || !cur) return;
    haptic("light");
    const fv = focusSubtree(cur.outline, id);
    if (fv && fv.breadcrumb.length > 0) {
      setFocusBlockId(fv.breadcrumb[fv.breadcrumb.length - 1].id);
    } else {
      setFocusBlockId(null);
    }
  }





  /**
   * "Remind me…": prompt for a `remind::` rule in the block's own syntax
   * (a time picker could not express `3pm every 1h until DONE`) and write
   * it as a block property. An empty answer clears the rule.
   */
  async function handleRemindMe(id: string) {
    const pid = pageId();
    if (!pid) return;
    const current =
      (await listReminders().catch(() => []))
        .find((r) => r.block_id === id)?.rule ?? "";
    const rule = window.prompt(
      "Remind me — e.g. 3pm, 10am every 1h, now every 30min until DONE",
      current || "9am",
    );
    if (rule === null) return;
    try {
      applyView(await setBlockRemind(pid, id, rule.trim()));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  async function handleAttachFile(id: string) {
    const pid = pageId();
    if (!pid) return;
    const selected = await open({ multiple: false, directory: false });
    // A cancelled picker resolves `null`; a single pick is a string.
    if (typeof selected !== "string") return;
    haptic("light");
    const next = await withError(() => attachAsset(selected, pid, id));
    if (next) applyView(next);
  }

  /**
   * A file was dropped onto the outline via iPad drag-and-drop (Files app,
   * split-view). Import each file (content-addressed copy, size-capped) and
   * insert its ready-made markdown link into the block under the drop point.
   *
   * Target resolution: the block row under the drop position, else the block
   * being edited, else the last block on the page (empty page → a fresh
   * block). When the drop lands on the block being edited, the link is
   * spliced into the live textarea at the caret (respecting the in-flight
   * edit) instead of racing a backend mutation; otherwise it's appended to
   * the block's text via `editBlock`.
   *
   * Best-effort throughout: an import failure surfaces a toast and drops
   * that one file without wedging the rest, mirroring the long-press
   * "Attach file" action's error handling.
   */
  async function handleFileDrop(paths: string[], droppedBlockId: string | null) {
    const pid = pageId();
    if (!pid || paths.length === 0) return;
    // The dropped-on block (resolved by the shared hit-test) is the target;
    // fall back to the block being edited, then the last top-level block.
    const targetId = droppedBlockId ?? editingId() ?? lastBlockId();
    const dropInEditor = targetId !== null && editingId() === targetId;
    haptic("light");
    // Import each file and collect the ready-to-insert markdown links.
    const links: string[] = [];
    for (const path of paths) {
      const asset = await withError(() => importAssetFile(path));
      if (asset) links.push(asset.markdown);
    }
    if (links.length === 0) return;
    const markdown = links.join(" ");

    // Dropped on the block being edited: splice into the live textarea at
    // the caret, same pattern as paste / toolbar insert.
    const el = activeTextarea;
    if (dropInEditor && el) {
      const start = el.selectionStart ?? el.value.length;
      const end = el.selectionEnd ?? el.value.length;
      // Space the link off the preceding word so it doesn't glue on.
      const lead =
        start > 0 && !/\s$/.test(el.value.slice(0, start)) ? " " : "";
      const insert = `${lead}${markdown}`;
      const caret = start + insert.length;
      spliceText(el, start, end, insert);
      parkCaret(el, caret);
      setDraft(el.value);
      parkCaret(el, caret);
      return;
    }

    // A different block is mid-edit: commit it first so applying the fresh
    // view below doesn't yank that textarea out (the same guard the reloads
    // honour).
    if (editingId()) await commitEdit();

    // No block to attach to (empty page): create a fresh block carrying the
    // link at the end of the page.
    if (!targetId) {
      const reply = await withError(() =>
        createBlock(pid, { afterId: null, parentId: null, text: markdown }),
      );
      if (reply) applyView(reply.view);
      return;
    }

    // Append the link to the target block's existing text.
    const block = findBlock(view()?.outline ?? [], targetId);
    const base = block ? rawTextWithTodo(block) : "";
    const text = base ? `${base} ${markdown}` : markdown;
    const next = await withError(() => editBlock(pid, targetId, text));
    if (next) applyView(next);
  }

  /** Last top-level block on the current page, or null when it's empty. */
  function lastBlockId(): string | null {
    const roots = view()?.outline ?? [];
    return roots.length > 0 ? roots[roots.length - 1].id : null;
  }

  /**
   * Run a `\`\`\`lang …\`\`\`` block through `outl-exec`. Triggered
   * from the long-press context menu (the only "Run code" surface on
   * mobile — desktop has Cmd+X too). The backend persists the
   * `> **result:**` subblock and returns the refreshed `PageView`,
   * so a single round-trip swaps the outline in. Runtime errors
   * (`unknown language`, `timeout`) surface via the toast so the
   * user knows why nothing visibly happened.
   */
  async function handleRunCodeBlock(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("medium");
    const reply = await withError(() => runCodeBlock(pid, id));
    if (!reply) return;
    applyView(reply.view);
    if (reply.error) {
      setError(`${reply.language}: ${reply.error}`);
    }
  }

  async function handleCreateAfter(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    // Commit the current block, THEN create + focus the new one. The "keep
    // editing across the create" experiment (to avoid the iOS keyboard bounce)
    // was reverted: it kept `editingId` on the OLD block during the async
    // create, so anything typed before the create returned landed on the wrong
    // block and was discarded when focus jumped to the new one — with a slow
    // sync that meant lost text + leftover empty blocks. Correctness wins; the
    // keyboard bounce needs a truly optimistic create (mount+focus the new
    // block synchronously), which is a separate, carefully-validated change.
    // Capture the caret BEFORE committing (commit drops focus). A caret
    // in the middle splits the block there (issue #184); the tail moves
    // into the new sibling and we drop the caret at its start. No active
    // textarea (newLine fired from a selected-but-not-editing block) →
    // split at the end, i.e. an empty sibling below (the old behaviour).
    const ta = activeTextareaSignal();
    const caretChars = ta
      ? utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? ta.value.length)
      : Number.MAX_SAFE_INTEGER;
    const tail = ta ? ta.value.slice(ta.selectionStart ?? ta.value.length) : "";
    if (editingId()) await commitEdit();
    const reply = await withError(() => splitBlock(pid, id, caretChars));
    if (reply) {
      applyView(reply.view);
      startEdit(reply.new_id, tail);
    }
  }

  async function handleRefresh() {
    const pid = pageId();
    if (!pid || refreshing() || syncing()) return; // one sync pass at a time
    setRefreshing(true);
    haptic("light");
    await pullAndReload().finally(() => setRefreshing(false));
  }

  async function handleRefClick(target: string) {
    // One Tauri call — `openRef` runs the journal-vs-page decision
    // tree on the Rust side and creates the page if nothing exists,
    // so this handler has no branching to keep in sync with the
    // backend. Used to be three commands gated by a `^\d{4}-\d{2}-\d{2}$`
    // regex, which surfaced `invalid date slug` toasts on inputs
    // like `[[2026-13-01]]` (regex shape OK, semantic parse fails).
    haptic("light");
    const next = await withError(() => openRef(target));
    if (next) applyView(next);
  }

  async function handleTagClick(tag: string) {
    // `#foo` arrives as `#foo`; strip the leading hash and route
    // through the same `openRef` decision tree as `[[foo]]`.
    const target = tag.startsWith("#") ? tag.slice(1) : tag;
    if (!target) return;
    haptic("light");
    const next = await withError(() => openRef(target));
    if (next) applyView(next);
  }

  function handleLinkClick(href: string) {
    // A `[label](assets/…)` link opens the uploaded file in the OS
    // default app (`open_asset` → iOS document/quick-look viewer);
    // everything else is an external `[label](url)` opened in the system
    // browser (scheme-guarded to http(s)/mailto). Mirrors desktop;
    // errors surface on the same status line instead of throwing into
    // the tap handler.
    haptic("light");
    const opening = isAssetLink(href) ? openAsset(href) : openExternalUrl(href);
    void opening.catch((e) => {
      setError(e instanceof Error ? e.message : String(e));
    });
  }

  async function handlePickPage(slug: string, kind: "page" | "journal") {
    setSwitcherOpen(false);
    haptic("light");
    const next =
      kind === "journal"
        ? await withError(() => openJournalFor(slug))
        : await withError(() => openPageBySlug(slug));
    if (next) applyView(next);
  }

  /**
   * Open the page a block lives on, zoomed into that block, for a
   * block-search hit or a row in Tasks. `openRef` runs the
   * journal-vs-page decision, since neither carries a page kind. The
   * zoom goes on after `applyView`, which drops any zoom on a page
   * switch; a block that has since moved away falls back to the full
   * page like every other stale zoom.
   */
  async function openBlock(slug: string, id: string) {
    haptic("light");
    const next = await withError(() => openRef(slug));
    if (!next) return;
    applyView(next);
    setFocusBlockId(id);
  }

  async function handleJumpToBlock(hit: BlockHit) {
    setSwitcherOpen(false);
    await openBlock(hit.source_slug, hit.id);
  }

  function openTask(task: TaskHit) {
    setTasksOpen(false);
    void openBlock(task.source_slug, task.id);
  }

  /** Which bottom bar tab is lit: the open sheet, else Journal on a day. */
  function activeTab(): BottomTab | null {
    if (tasksOpen()) return "tasks";
    if (remindersOpen()) return "reminders";
    return switcherOpen() ? "search" : view()?.page.kind === "journal" ? "journal" : null;
  }

  function handleTab(tab: BottomTab) {
    haptic("light");
    setTasksOpen(tab === "tasks");
    setRemindersOpen(tab === "reminders");
    setSwitcherOpen(tab === "search");
    if (tab === "journal") void dayNav.jumpToday();
  }

  /** What each palette action opens (`lib/palette-actions.ts`). */
  const paletteHandlers = (): Record<MobileAction, () => void> => ({
    OpenToday: () => void dayNav.jumpToday(),
    QuickCapture: () => setCaptureOpen(true),
    TaskList: () => setTasksOpen(true),
    OpenReminders: () => setRemindersOpen(true),
    OpenCalendar: () => setCalendarOpen(true),
    SyncNow: () => void handleRefresh(),
    OpenDevices: () => setDevicesOpen(true),
    OpenPlugins: () => setPluginsOpen(true),
    OpenSettings: () => setSettingsOpen(true),
    FoldAll: () => void handleFoldAll(),
    UnfoldAll: () => void handleUnfoldAll(),
  });

  function handleCapture() {
    if (capture.onToday()) void newBlock.appendBlock();
    else {
      // `<CaptureSheet />` focuses its field on the next animation
      // frame, which is already outside this tap's gesture window — so
      // the keyboard goes up here, inside it, and the sheet's focus
      // becomes a move between two text inputs. One frame is too short
      // to type into, and `armKeyboard` clears the keeper either way.
      armKeyboard();
      haptic("light");
      setCaptureOpen(true);
    }
  }


  /**
   * Insert a snippet (or open/close pair) into the active textarea
   * synchronously so iOS keeps the keyboard up across the change.
   *
   * Uses the `spliceText` + double `parkCaret` pattern (see
   * `lib/textarea.ts`) so the caret lands at the intended spot
   * even when Solid's `value={draft()}` binding effect fires later
   * and would otherwise jump the caret to the end.
   */
  function insertAtCursor(
    mode: "text" | "pair",
    open: string,
    close: string = "",
  ) {
    const el = activeTextarea;
    if (!el) return;
    const start = el.selectionStart ?? el.value.length;
    const end = el.selectionEnd ?? el.value.length;
    const insert = mode === "pair" ? open + close : open;
    const targetCaret =
      mode === "pair" ? start + open.length : start + insert.length;

    spliceText(el, start, end, insert);
    parkCaret(el, targetCaret);
    setDraft(el.value);
    parkCaret(el, targetCaret);
  }

  function wrapSelection(style: "bold" | "italic" | "code") {
    const el = activeTextarea;
    if (!el) return;
    const start = el.selectionStart ?? el.value.length;
    const end = el.selectionEnd ?? el.value.length;
    const wrap = style === "bold" ? "**" : style === "italic" ? "*" : "`";
    const selected = el.value.slice(start, end);
    const insert = `${wrap}${selected}${wrap}`;
    spliceText(el, start, end, insert);
    const targetCaret = start + insert.length;
    parkCaret(el, targetCaret);
    setDraft(el.value);
    parkCaret(el, targetCaret);
  }

  return (
    <div class="flex h-full flex-col">
      {/* Bear-style chrome: header background stays as a soft blur over
          the canvas, with no divider underneath. Actions sit inside
          two floating capsules (left = back, right = grouped icons)
          so the title can breathe in the middle. */}
      <JournalChrome
        view={view()}
        online={online()}
        peersUp={peersUp()}
        syncing={syncing()}
        refreshing={refreshing()}
        todaySlug={dayNav.today()}
        toolbarButtons={toolbarButtons()}
        titleLongPress={titleLongPress}
        onJumpToday={() => void dayNav.jumpToday()}
        onPrevDay={() => void dayNav.prevDay()}
        onNextDay={() => void dayNav.nextDay()}
        onFoldAll={handleFoldAll}
        onUnfoldAll={handleUnfoldAll}
        onRefresh={handleRefresh}
        onRunToolbarButton={(btn) => void runToolbarButton(btn)}
        onOpenPlugins={() => setPluginsOpen(true)}
        onOpenDevices={() => setDevicesOpen(true)}
        onOpenSettings={() => setSettingsOpen(true)}
      />

      <Show when={weekStripSlug()}>
        {(slug) => (
          <WeekStrip
            selectedSlug={slug()}
            todaySlug={dayNav.today()}
            contentVersion={view()}
            onPick={(d) => void dayNav.pickDay(d)}
            onPrevDay={() => void dayNav.prevDay()}
            onNextDay={() => void dayNav.nextDay()}
            onToday={() => void dayNav.jumpToday()}
            onOpenCalendar={() => setCalendarOpen(true)}
          />
        )}
      </Show>

      <main class="outl-reading ios-scroll flex-1 pb-32">
        <PullToRefresh onRefresh={handleRefresh}>
        <div class="min-h-[60vh]">
        {/* The page's own `key:: value` metadata (`icon::`, `type::`).
            Mobile showed none of it before — it existed only in the
            `.md` and the TUI. Tapping a chip opens the same properties
            sheet the block long-press does, on the Page side. */}
        <Show when={(view()?.page_properties ?? []).length > 0}>
          <div class="ios-scroll flex gap-1.5 overflow-x-auto px-4 pt-2">
            <For each={editableProperties(view()!.page_properties)}>
              {([key, value]) => (
                <button
                  type="button"
                  onClick={() => {
                    haptic("light");
                    setPropertiesTarget({ blockId: null, scope: "page" });
                  }}
                  class="shrink-0 rounded-full bg-(--color-outl-border)/40 px-2.5 py-1 text-[11px] text-(--color-outl-fg-dim) active:opacity-60"
                >
                  <span class="font-mono">{key}</span>: {value}
                </button>
              )}
            </For>
          </div>
        </Show>
        <section class="mt-1 pb-1">
          <Show
            when={loaded() && view() && view()!.outline.length > 0}
            fallback={
              <Show when={loaded()} fallback={<SkeletonOutline />}>
                <Show
                  when={loadFailed()}
                  fallback={
                    <JournalStartWriting
                      onStart={() => void newBlock.appendBlock()}
                    />
                  }
                >
                  <JournalLoadFailed
                    onRetry={() => {
                      // Back to the skeleton only when there is nothing
                      // to lose. `loaded` gates this whole section, so
                      // clearing it with a page up would take the
                      // outline, the caret and the user's keystrokes
                      // with it (`UX.md` → Never block a keystroke).
                      // A failed boot has no outline, which is the one
                      // case where the skeleton is the honest answer.
                      if (!view()) setLoaded(false);
                      void loadTodayWithRetry();
                    }}
                  />
                </Show>
              </Show>
            }
          >
            <PageAheadOfLogBanner info={aheadOfLog()?.info} client="mobile" />
            <ParseWarningsBanner warnings={view()!.warnings ?? []} />
            {/* Zoom header — visible only while focused on a block. The
                "← Back" chevron zooms out one level (or exits); each
                breadcrumb crumb is tappable to jump straight to that
                ancestor. */}
            <Show when={focusView()}>
              {(fv) => (
                <JournalZoomBar
                  breadcrumb={fv().breadcrumb}
                  onZoomOut={handleZoomOut}
                  onZoomTo={(id) => setFocusBlockId(id)}
                />
              )}
            </Show>
            {/* An empty page used to render as nothing at all, with no
                hint that a tap would help. It is also the one state with no block to
                long-press, so it was the only place page properties
                were unreachable. Both doors live here. */}
            <Show when={outlineRoots().length === 0 && view()}>
              <JournalEmptyPage
                journal={view()?.page.kind === "journal"}
                onAddBlock={() => void newBlock.appendBlock()}
                onProperties={() => {
                  if (!pageId()) return;
                  haptic("light");
                  setPropertiesTarget({ blockId: null, scope: "page" });
                }}
              />
            </Show>
            <For each={outlineRoots()}>
              {(block) => (
                <BlockRow
                  block={block}
                  depth={0}
                  editingId={editingId()}
                  draftText={draft}
                  onStartEdit={startEdit}
                  onDraftChange={setDraft}
                  onCommitEdit={commitEdit}
                  onToggleTodo={blockOps.toggleTodo}
                  onDelete={blockOps.requestDelete}
                  onIndent={blockOps.indent}
                  onOutdent={blockOps.outdent}
                  onCreateAfter={handleCreateAfter}
                  onToggleCollapse={handleToggleCollapse}
                  onFocusBlock={handleFocusBlock}
                  onContextMenu={(id) => setContextMenuBlockId(id)}
                  onSetProperty={(blockId, key, value) => {
                    const pid = pageId();
                    if (!pid) return;
                    void setBlockProperty(pid, blockId, key, value)
                      .then(applyView)
                      .catch((e) =>
                        setError(e instanceof Error ? e.message : String(e)),
                      );
                  }}
                  onRefClick={handleRefClick}
                  onTagClick={handleTagClick}
                  onLinkClick={handleLinkClick}
                  onPasteMarkdown={handlePasteMarkdown}
                  onTextareaMount={(el) => {
                    activeTextarea = el;
                    setActiveTextareaSignal(el);
                  }}
                  selectionMode={selection() !== null}
                  selectionSet={selectionSet()}
                  onSelectTap={selectionOps.selectTap}
                />
              )}
            </For>
          </Show>
        </section>

        <PageSections
          backlinks={backlinks()}
          pageKind={view()?.page.kind}
          blank={isBlankOutline(outlineRoots())}
          onToggleOrder={async () => {
            const v = view();
            if (!v) return;
            haptic("light");
            const next =
              (backlinks()?.backlinks_order ?? "newest") === "newest"
                ? "oldest"
                : "newest";
            const r = await withError(() =>
              setBacklinksOrder(next, v.page.slug),
            );
            if (r) mutateBacklinks(r);
          }}
          onOpenPage={async (slug, kind) => {
            haptic("light");
            const next =
              kind === "journal"
                ? await withError(() => openJournalFor(slug))
                : await withError(() => openPageBySlug(slug));
            if (next) applyView(next);
          }}
        />

        </div>
        </PullToRefresh>

      </main>

      {/* Hidden while editing (the keyboard owns the bottom edge) and
          during a range selection, whose toolbar docks in the same place. */}
      <BottomBar
        active={activeTab()}
        hidden={editingId() !== null || selection() !== null || !view()}
        onTab={handleTab}
        onCapture={handleCapture}
      />

      {/* Web keyboard accessory bar (suggester strip + edit toolbar).
          Android only — iOS keeps its native `OutlToolbarView`. Both
          surfaces fire the same `dispatchToolbarAction`. */}
      <KeyboardAccessory
        active={isAndroid && editingId() !== null}
        onAction={(action: ToolbarAction) => dispatchToolbarAction(action)}
      />

      <Toast
        message={error()}
        onRetry={errorRetry() ?? undefined}
        onDismiss={() => {
          setError(null);
          setErrorRetry(null);
        }}
      />

      <PageSwitcher
        open={switcherOpen()}
        currentSlug={view()?.page.slug ?? null}
        onClose={() => setSwitcherOpen(false)}
        onPick={handlePickPage}
        onJumpToBlock={handleJumpToBlock}
        onAction={(a) => paletteHandlers()[a]()}
      />

      <TasksSheet
        open={tasksOpen()}
        todaySlug={dayNav.today()}
        onClose={() => setTasksOpen(false)}
        onOpenTask={openTask}
      />

      <CaptureSheet
        open={captureOpen()}
        target={(() => {
          const t = dayNav.today();
          const h = t ? journalHeading(t, t) : null;
          return h ? `${h.title}, ${h.date}` : "Today";
        })()}
        onClose={() => setCaptureOpen(false)}
        onSubmit={capture.captureToToday}
      />

      <Calendar
        open={calendarOpen()}
        selectedSlug={
          view()?.page.kind === "journal" ? (view()?.page.slug ?? null) : null
        }
        todaySlug={dayNav.today()}
        onClose={() => setCalendarOpen(false)}
        onPick={(slug) => {
          setCalendarOpen(false);
          void dayNav.pickDay(slug);
        }}
      />

      <DevicesSheet open={devicesOpen()} onClose={() => setDevicesOpen(false)} />

      <SettingsSheet
        open={settingsOpen()}
        onClose={() => setSettingsOpen(false)}
        onOpenDevices={() => setDevicesOpen(true)}
      />

      <RemindersSheet
        open={remindersOpen()}
        onClose={() => setRemindersOpen(false)}
        onMessage={(text) => setError(text)}
        onView={(v) => applyView(v)}
        currentSlug={view()?.page.slug ?? null}
      />

      <PluginSheet
        open={pluginsOpen()}
        pageId={pageId()}
        onClose={() => setPluginsOpen(false)}
        onMessage={(text) => setError(text)}
        onView={(v) => applyView(v)}
        onViews={(views) => showPluginViews(views)}
      />

      {/* Sandboxed, ephemeral iframe overlays for plugin `ctx.ui.render`
          payloads (confetti, etc). Binds its `push` fn up to
          `showPluginViews`. */}
      <PluginViewOverlay bind={(push) => (pushPluginView = push)} />

      <JournalDeleteDialogs
        pendingBlock={pendingDelete()}
        pendingRange={pendingRangeDelete()}
        onCancelBlock={() => setPendingDelete(null)}
        onConfirmBlock={(id) => void blockOps.performDelete(id)}
        onCancelRange={() => setPendingRangeDelete(null)}
        onConfirmRange={(ids) => void selectionOps.performDeleteRange(ids)}
      />

      <BlockContextMenu
        open={contextMenuBlockId() !== null}
        onClose={() => setContextMenuBlockId(null)}
        actions={buildContextActions(
          contextMenuBlockId(),
          view(),
          {
            indent: blockOps.indent,
            outdent: blockOps.outdent,
            moveUp: blockOps.moveUp,
            moveDown: blockOps.moveDown,
            toggleTodo: blockOps.toggleTodo,
            delete: blockOps.requestDelete,
            runCode: handleRunCodeBlock,
            insertTemplate: (id) => setTemplateBlockId(id),
            properties: (id) =>
              setPropertiesTarget({ blockId: id, scope: "block" }),
            remindMe: (id) => void handleRemindMe(id),
            attachFile: handleAttachFile,
            copy: async (id) => {
              // Copy the block as clean outl markdown (its subtree
              // included) — the inverse of paste, so it re-pastes into
              // outl as the same tree, and reads as a tidy bullet list
              // anywhere else. The backend serializes; we just write it.
              try {
                const md = await copyMarkdown([id]);
                await navigator.clipboard?.writeText(md);
              } catch {
                // Some webviews refuse navigator.clipboard outside a
                // user gesture chain; failing silently is acceptable.
              }
            },
            copyBlock: (id) => void blockOps.copyBlock(id),
            pasteBlock: (id) => void blockOps.pasteBlock(id),
            cutBlock: (id) => void blockOps.cutBlock(id),
            copyBlockRef: (id) => void blockOps.copyBlockRef(id),
            newBlockAbove: (id) => void newBlock.createBefore(id),
            selectBlocks: (id) => void selectionOps.selectBlocks(id),
            reselectSelection: () => selectionOps.reselectLast(),
          },
          // Reading the signal here (not inside a handler) is what
          // makes this reactive: `actions=` is a Solid prop getter, so
          // a read during its own evaluation registers as a dependency
          // — the same way `contextMenuBlockId()` / `view()` above do.
          // Without this, "Paste block" would only reveal itself the
          // next time some *other* signal it depends on changed.
          blockClipboard() !== null,
          // Same reactivity reasoning for "Reselect last selection":
          // read `lastSelection()` / `view()` here so a fresh
          // selection (or a peer edit that strands the old one)
          // updates the row without needing an unrelated signal to
          // change first.
          (() => {
            const sel = lastSelection();
            const cur = view();
            return sel !== null && cur !== null && selectionIsLive(sel, cur.outline);
          })(),
        )}
      />

      <SelectionToolbar
        open={selection() !== null}
        count={selectionOps.currentRangeIds()?.length ?? 0}
        onGrowUp={selectionOps.growUp}
        onGrowDown={selectionOps.growDown}
        onIndent={() => void selectionOps.indentRange()}
        onOutdent={() => void selectionOps.outdentRange()}
        onMoveUp={() => void selectionOps.moveRangeUp()}
        onMoveDown={() => void selectionOps.moveRangeDown()}
        onCopy={() => void selectionOps.yankRange()}
        onDelete={selectionOps.requestDeleteRange}
        onDone={selectionOps.exit}
      />

      <TemplateSheet
        blockId={templateBlockId()}
        onClose={() => setTemplateBlockId(null)}
        onMessage={(text) => setError(text)}
        onView={(v) => applyView(v)}
      />

      <PropertiesSheet
        blockId={propertiesTarget()?.blockId ?? null}
        scope={propertiesTarget()?.scope ?? null}
        pageId={pageId()}
        view={view() ?? null}
        onClose={() => setPropertiesTarget(null)}
        onMessage={(text) => setError(text)}
        onView={(v) => applyView(v)}
      />
    </div>
  );
}
