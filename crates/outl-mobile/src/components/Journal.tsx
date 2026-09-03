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
  copyBlockMarkdown,
  copyBlockRef,
  copyMarkdown,
  createBlock,
  cutBlock,
  dateTitle,
  deleteBlock,
  editBlock,
  deliverDueReminders,
  importAssetFile,
  indentBlock,
  listReminders,
  moveBlockDown,
  moveBlockUp,
  nextDay,
  openAsset,
  openJournalFor,
  openPageBySlug,
  openExternalUrl,
  openRef,
  openTodayJournal,
  outdentBlock,
  pageBacklinks,
  pasteBlockAfter,
  pasteMarkdown,
  peerStatus,
  pluginRun,
  pluginSyncHooks,
  pluginToolbar,
  previousDay,
  redoPage,
  reloadWorkspace,
  runCodeBlock,
  searchEmojis,
  searchPages,
  searchPersons,
  setBacklinksOrder,
  setBlockCollapsed,
  setBlockProperty,
  setBlockRemind,
  splitBlock,
  syncNow,
  todaySlug,
  toggleTodo,
  undoPage,
  workspaceStats,
} from "@outl/shared/api/commands";
import { utf16OffsetToCharOffset } from "@outl/shared/paste";
import { isAssetLink } from "@outl/shared/links";
import { installFileDrop } from "@outl/shared/drag-drop";
import { peersOnline } from "@outl/shared/peers";
import { detectFence } from "@outl/shared/highlight";
import {
  countDescendants,
  findBlock,
  flattenAll,
  flattenParents,
  flattenVisible,
  focusSubtree,
  rawTextWithTodo,
  visualRangeIds,
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
import { parkCaret, spliceText } from "../lib/textarea";
import { withTimeout } from "../lib/async";
import {
  type BlockSelection,
  extendSelectionTo,
  growSelectionDown,
  growSelectionUp,
  selectionIsLive,
  startSelection,
} from "../lib/block-selection";

/**
 * Payload shapes emitted by the backend's `deep-link://navigate` event
 * (and buffered for cold start via `take_pending_deep_link`) — issue #98.
 */
type DeepLinkNavigate =
  | { kind: "today" }
  | { kind: "daily"; date: string }
  | { kind: "page"; slug: string };

/** Maximum time we wait for a single Tauri command to settle before
 *  surfacing a timeout error. Keeps the UI from getting stuck in
 *  "syncing…" forever when iCloud coordination stalls. */
const EDIT_TIMEOUT_MS = 8000;
/** Cap on a `syncNow` force-sync pass. With an unreachable peer the connect
 *  waits out a 10–30s timeout; awaiting that in the reload path froze the UI.
 *  6s lets a healthy pass through and bounds a dead one so the local reload
 *  always proceeds. */
const SYNC_TIMEOUT_MS = 6000;
import {
  HIDE_MESSAGE,
  buildEmojiShowMessage,
  buildShowMessage,
  registerPickedCallback,
  setNativeSuggesterState,
} from "../lib/native-suggester";
import { platform } from "@tauri-apps/plugin-os";
import type { ToolbarAction } from "@outl/shared/toolbar";
import { Calendar } from "./Calendar";
import { KeyboardAccessory } from "./KeyboardAccessory";
import { DevicesSheet } from "./DevicesSheet";
import { RemindersSheet } from "./RemindersSheet";
import { PluginSheet } from "./PluginSheet";
import { PluginViewOverlay } from "./PluginViewOverlay";
import { PageSwitcher } from "./PageSwitcher";
import { PullToRefresh } from "./PullToRefresh";
import { SyncDot } from "./SyncDot";
import { BlockRow } from "./BlockRow";
import { SkeletonOutline } from "./Skeleton";
import { loadTransformers } from "@outl/shared/plugins/transformer-registry";
import { createLongPress } from "../lib/long-press";
import { editableProperties } from "../lib/properties";
import { haptic } from "../lib/haptics";
import { BacklinksSection } from "./BacklinksSection";
import { BlockContextMenu, type BlockContextAction } from "./BlockContextMenu";
import { ConfirmDialog } from "./ConfirmDialog";
import { SelectionToolbar } from "./SelectionToolbar";
import { TemplateSheet } from "./TemplateSheet";
import {
  PropertiesSheet,
  type PropertyScope,
} from "./PropertiesSheet";
import { Toast } from "./Toast";

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
  // Backlinks are fetched lazily, off the page-open path — `view().backlinks`
  // is always empty now (the O(blocks-in-workspace) scan blocked the first
  // journal paint). The resource re-fires on every slug change, so every
  // navigation path is covered without touching `applyView`.
  const [backlinks, { mutate: mutateBacklinks }] = createResource(
    () => view()?.page.slug,
    pageBacklinks,
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
  const [stats] = createResource(workspaceStats);
  const [switcherOpen, setSwitcherOpen] = createSignal(false);
  const [calendarOpen, setCalendarOpen] = createSignal(false);
  const [devicesOpen, setDevicesOpen] = createSignal(false);
  const [remindersOpen, setRemindersOpen] = createSignal(false);
  const [pluginsOpen, setPluginsOpen] = createSignal(false);
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
  // the same round-trip, see `handleCutBlock`). "Paste block" (shown
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
  // Today's journal slug. Re-resolved on mount and whenever the app
  // returns to the foreground, so the affordance stays correct across a
  // midnight rollover (the app can sit open past midnight: "today"
  // changes but a value cached once on mount wouldn't). Single source of
  // truth for every "is this today?" decision — `canJumpToday` here and
  // `JournalHeader`'s label both read it, instead of resolving "today"
  // independently and risking disagreement.
  const [todaySlugValue, setTodaySlugValue] = createSignal<string | null>(null);

  // Monotonic reload generation. Every async reload path captures this at
  // start; a reload whose generation is no longer the latest is a stale read
  // that must NOT clobber a newer one (the mobile "flicker" was an unguarded
  // slow reload applying an older op-log state after a fresh one landed).
  let reloadGen = 0;
  // Set when a peer-driven reload was suppressed because the user was editing.
  // A `createEffect` on `editingId` drains it the moment they leave edit mode,
  // so a sync never swaps the workspace out from under an active edit (that
  // swap re-mints the block id → the `block <id> [Retry]` error + the freeze).
  let reloadPendingWhileEditing = false;

  // See `applyView`: kept out of the `PageView` signal so an edit commit
  // (which replies without the flag) can't silently drop it. Keyed by
  // slug so navigating to another page clears it.
  const [aheadOfLog, setAheadOfLog] = createSignal<{
    slug: string;
    info: MdAheadOfLog;
  } | null>(null);

  function applyView(v: PageView) {
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

  // Track connectivity so the SyncDot can show "offline" when iCloud
  // can't reach peers. Both listeners are pure DOM side-effects but
  // they must be registered + torn down within the component's
  // owner; `onCleanup` here, not deep inside `onMount`'s async body.
  if (typeof window !== "undefined") {
    const upOnline = () => setOnline(true);
    const upOffline = () => setOnline(false);
    window.addEventListener("online", upOnline);
    window.addEventListener("offline", upOffline);
    // Probe iroh peer health on mount, then every 5s, so the dot tracks
    // the mesh without a user action. `peer-ops-changed` (ops bridge)
    // and a force-sync also poke `refreshPeerStatus` for a fresher read.
    void refreshPeerStatus();
    const peerPoll = window.setInterval(() => {
      void refreshPeerStatus();
      // Pull from peers AND reload the view every tick so an edit on the
      // desktop OR the TUI shows up without the refresh button. The mobile side
      // initiating the dial is NAT-friendly (waiting for the desktop to reach an
      // iPhone behind carrier NAT is not), which is why desktop/TUI→mobile needs
      // us to pull. We call the full `pullAndReload` (not just `syncNow`):
      // relying on the `workspace-ready` event alone left the ops on disk
      // without re-rendering — the symptom was "only shows after I hit sync".
      //
      // NOT guarded on `editingId()` here. `pullAndReload` already handles the
      // editing case correctly — it pulls the peer's ops to disk and defers
      // only the RE-RENDER, which the `editingId` effect drains the moment the
      // user leaves the field. Testing it out here too made that branch
      // unreachable and turned "don't reset the textarea" into "don't sync at
      // all while a block is open", which is precisely the state a user is in
      // while waiting for a desktop edit to show up. Same symptom the comment
      // above says this poll exists to prevent, one layer up.
      void pullAndReload({ background: true });
    }, 3000);
    // Reminder delivery. The backend decides what is due and remembers
    // what this device already delivered, so a poll that fires twice
    // never double-buzzes and a phone that was asleep owes one banner,
    // not a backlog. It short-circuits when reminders are off, so this
    // ticks unconditionally rather than re-subscribing on a settings
    // change. 30s, not 3s: the schedule has minute granularity.
    const reminderPoll = window.setInterval(() => {
      void deliverDueReminders().catch(() => {
        // Permission not granted yet — the Rust side logged it, and a
        // toast every 30 seconds would be worse than silence.
      });
    }, 30_000);
    onCleanup(() => {
      window.removeEventListener("online", upOnline);
      window.removeEventListener("offline", upOffline);
      window.clearInterval(peerPoll);
      window.clearInterval(reminderPoll);
    });
  }

  // Resolve "today" up front and again every time the app comes back to
  // the foreground (covers the midnight rollover). `disposed` guards the
  // async setter so a resolution that lands after the component unmounts
  // doesn't poke a torn-down signal.
  let disposed = false;
  function refreshTodaySlug() {
    todaySlug()
      .then((t) => {
        if (!disposed) setTodaySlugValue(t);
      })
      .catch((e) => {
        // Best effort; the affordance just stays hidden until we know
        // today's slug. Log so a backend regression is still visible.
        console.warn("failed to resolve today's slug", e);
      });
  }
  refreshTodaySlug();
  if (typeof document !== "undefined") {
    const onVisible = () => {
      if (document.visibilityState === "visible") refreshTodaySlug();
    };
    document.addEventListener("visibilitychange", onVisible);
    onCleanup(() => {
      disposed = true;
      document.removeEventListener("visibilitychange", onVisible);
    });
  } else {
    onCleanup(() => {
      disposed = true;
    });
  }

  // Drain a reload that was deferred because the user was editing. The moment
  // they leave edit mode (`editingId()` → null), apply the peer's changes that
  // arrived meanwhile — in the background so it doesn't flash the spinner.
  // Guarded so it only fires on the edit→idle transition, not on every keypress.
  createEffect(() => {
    if (editingId() === null && reloadPendingWhileEditing) {
      reloadPendingWhileEditing = false;
      void pullAndReload({ background: true });
    }
  });

  onMount(async () => {
    // Kick P2P sync in the very first tick — BEFORE the journal loads — so the
    // connect starts punching the NAT path immediately instead of waiting for
    // the local load to finish. iOS accepts inbound poorly, so the mobile side
    // dialing first is what actually opens the path; starting it here (not
    // after `loadTodayWithRetry`) shaves that wait off. Fully background +
    // capped + silent (no boot toast): it never blocks the boot or first paint,
    // and the ops it pulls arrive via `workspace-ready` / the next reload.
    void withTimeout(syncNow(), SYNC_TIMEOUT_MS, "sync timed out").catch(() => {});
    listenForWorkspaceReady();
    listenForDeepLink();
    listenForFileDrop();
    await loadTodayWithRetry();
    // Cold-start deep link: a URL that *launched* the app was buffered
    // by the backend before the listener above existed. Drain it now
    // that the workspace is open and override today's journal with the
    // target. A normal launch returns null and keeps the journal.
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const pending = await invoke<DeepLinkNavigate | null>(
        "take_pending_deep_link",
      );
      if (pending) await navigateDeepLink(pending);
    } catch {
      // best-effort — a failed drain just leaves the journal showing
    }
    // Opening the app: pull whatever peers produced while it was closed, so the
    // user sees fresh state without hitting refresh. Runs after the local load
    // so the UI is already up; best-effort.
    void pullAndReload();
    // Plugin toolbar buttons load lazily on the host's first request, so
    // pull them once the workspace is open. Best-effort — a host with no
    // toolbar plugins just leaves the header unchanged.
    void loadToolbar();
    // Content transformers (plugin-claimed code-fence languages) load the
    // same way: pull the registry once the workspace is open so a fenced
    // block in a custom language can render its transformed view. Best-
    // effort — failure leaves fences as plain highlighted code.
    void loadTransformers();
    // iOS freezes JS in the background; on return to the foreground, pull again
    // so edits made on another device while we were away land right away.
    const onVisible = () => {
      if (document.visibilityState === "visible") void pullAndReload();
    };
    document.addEventListener("visibilitychange", onVisible);
    onCleanup(() => document.removeEventListener("visibilitychange", onVisible));
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

  /**
   * Navigate in response to an `outl://` deep link (issue #98). The Rust
   * backend parsed + validated the URL through the shared
   * `outl_actions::parse_deep_link`; map each shape onto the same
   * `open*` command the ref-tap path uses. Shared by the warm listener
   * and the cold-start drain so the two can't diverge.
   */
  async function navigateDeepLink(p: DeepLinkNavigate) {
    try {
      const next =
        p.kind === "today"
          ? await openTodayJournal()
          : p.kind === "daily"
            ? await openJournalFor(p.date)
            : await openPageBySlug(p.slug);
      applyView(next);
      setError(null);
    } catch (err) {
      setError(String(err));
    }
  }

  function listenForDeepLink() {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    // Register cleanup synchronously (inside the component owner, before
    // the dynamic import resolves) so the listener is torn down if
    // Journal ever unmounts — matching the desktop's `onCleanup`. Journal
    // is the mobile root today (singleton), so this is defensive, but it
    // keeps the two clients consistent. If we unmount before `listen()`
    // resolves, dispose the late-arriving handle right away.
    onCleanup(() => {
      disposed = true;
      unlisten?.();
    });
    import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<DeepLinkNavigate>("deep-link://navigate", async (e) => {
          // Skip while editing so a warm-path navigation never yanks the
          // textarea out from under the user mid-keystroke.
          if (editingId()) return;
          await navigateDeepLink(e.payload);
        }),
      )
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      });
  }

  /**
   * Wire the Tauri webview drag-and-drop event (iPad: drag a file from the
   * Files app or split-view onto a block). Registered like the deep-link
   * listener above — cleanup armed synchronously inside the component owner,
   * the dynamic import resolves the real handle afterwards. Best-effort: on
   * iPhone the OS rarely delivers a webview drop, and a registration failure
   * just leaves the long-press "Attach file" action as the import path.
   */
  function listenForFileDrop() {
    let unlisten: (() => void) | undefined;
    let disposed = false;
    onCleanup(() => {
      disposed = true;
      unlisten?.();
    });
    // Shared `installFileDrop` resolves the block under the drop point
    // (physical→CSS pixels, `data-block-id` hit-test) identically to the
    // desktop, so the two clients can't drift on the geometry.
    installFileDrop({
      onDrop: (paths, blockId) => handleFileDrop(paths, blockId),
    })
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      })
      .catch((e) => {
        console.warn("failed to register drag-drop listener", e);
      });
  }

  async function loadTodayWithRetry() {
    // Show a generic "Loading…" first, then upgrade the message to
    // The skeleton placeholder takes the place of the old progress
    // message; we keep retrying the workspace open silently and only
    // flip `loadFailed` if we exhaust the budget.
    setLoadFailed(false);
    for (let i = 0; i < 50; i += 1) {
      try {
        const v = await openTodayJournal();
        applyView(v);
        setError(null);
        setLoaded(true);
        return;
      } catch (e) {
        const msg = String(e);
        if (msg.includes("workspace_loading")) {
          // Workspace opener still in flight; back off briefly and
          // try again. Capped at ~10s of retries.
          await new Promise((r) => setTimeout(r, 200));
          continue;
        }
        setError(msg);
        setLoadFailed(true);
        setLoaded(true);
        return;
      }
    }
    setError("Workspace took too long to open.");
    setLoadFailed(true);
    setLoaded(true);
  }

  function listenForWorkspaceReady() {
    // Best-effort: refresh the current view once the background
    // opener finishes, so anything the user did during the brief
    // "loading" window converges on the freshly opened workspace.
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("workspace-ready", async () => {
        // Mid-edit is handled inside `pullAndReload` (pull now, re-render when
        // the field closes), so returning early here only threw the signal
        // away. "The next idle workspace-ready picks them up" was the flaw:
        // this event fires when peer ops LAND, so once they have landed there
        // may not be another one, and the view then waits for the poll — or
        // for the user to press Sync.
        if (!view()) {
          await loadTodayWithRetry();
          return;
        }
        // Route through the guarded reload path (re-materialize the op log +
        // change / empty / generation guards) instead of applying a raw
        // `openJournalFor` on the possibly-stale in-memory workspace. That
        // unguarded apply — firing on every peer-ops write — is what flipped
        // the page back to an older op-log state (the flicker).
        await pullAndReload({ background: true });
      });
    });
  }

  /**
   * Bridge between the native UIKit keyboard accessory view (defined
   * in `gen/apple/Sources/outl-mobile/main.mm`) and the Solid handlers
   * below. The native buttons call `evaluateJavaScript` with
   * `window.__outlToolbar(action)` and we map each action onto the
   * existing handler.
   */
  /**
   * Single dispatch for a toolbar action, shared by the two surfaces that
   * fire them: the iOS native bar (via `window.__outlToolbar`) and the web
   * `<KeyboardAccessory />` (Android). Keeping one switch means the two
   * bars can't drift on what a button does.
   */
  function dispatchToolbarAction(action: string) {
    const id = editingId();
    switch (action) {
      case "indent":
        if (id) handleIndent(id);
        return;
      case "outdent":
        if (id) handleOutdent(id);
        return;
      case "moveUp":
        if (id) handleMoveUp(id);
        return;
      case "moveDown":
        if (id) handleMoveDown(id);
        return;
      case "undo":
        void handleUndo();
        return;
      case "redo":
        void handleRedo();
        return;
      case "todo":
        if (id) handleToggleTodo(id);
        return;
      case "delete":
        if (id) handleDelete(id);
        return;
      case "newLine":
        if (id) {
          handleCreateAfter(id);
        } else {
          handleAppendBlock();
        }
        return;
      case "bold":
        wrapSelection("bold");
        return;
      case "italic":
        wrapSelection("italic");
        return;
      case "code":
        wrapSelection("code");
        return;
      case "insertRef":
        insertAtCursor("pair", "[[", "]]");
        return;
      case "insertBlock":
        insertAtCursor("pair", "((", "))");
        return;
      case "insertHash":
        insertAtCursor("text", "#");
        return;
      case "done":
        if (editingId()) commitEdit();
        return;
    }
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

  async function handleToggleTodo(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("medium");
    const wasEditing = editingId() === id;
    if (wasEditing) {
      // Commit current draft text into the workspace so the cycle
      // operates on what the user typed, without dropping out of
      // edit mode (we want the keyboard to stay up).
      const text = draft();
      const committed = await withError(() => editBlock(pid, id, text));
      if (committed) setView(committed);
    }
    const next = await withError(() => toggleTodo(pid, id));
    if (!next) return;
    applyView(next);
    if (wasEditing) {
      // Keep edit mode on the same block; refresh draft to the
      // backend's view, **with** the TODO/DONE prefix reattached so
      // the editor stays consistent with what the user just toggled.
      const block = findBlock(next.outline, id);
      if (block) setDraft(rawTextWithTodo(block));
    }
  }

  /**
   * Delete a block. When the block has descendants we *always*
   * prompt — deleting a parent destroys the whole subtree, and while
   * the keyboard toolbar's Undo button (RFC 0254 phase 1) can revert
   * it, that is a second, less immediate tap than the confirm dialog
   * already in front of the user. Leaf blocks delete immediately (no
   * prompt) to keep the swipe gesture fast.
   */
  function handleDelete(id: string) {
    const cur = view();
    if (!cur) return;
    const block = findBlock(cur.outline, id);
    const descendants = block ? countDescendants(block) : 0;
    if (descendants > 0) {
      haptic("warning");
      setPendingDelete({ id, descendants });
      return;
    }
    haptic("heavy");
    void performDelete(id);
  }

  async function performDelete(id: string) {
    const pid = pageId();
    if (!pid) return;
    if (editingId() === id) setEditingId(null);
    const next = await withError(() => deleteBlock(pid, id));
    if (next) applyView(next);
  }

  /**
   * "Copy block" (long-press menu, RFC 0254 phase 2) — arm the
   * in-app block clipboard with `id`'s subtree as clean outl
   * markdown, ready for "Paste block" on another row. Distinct from
   * the existing "Copy text" action: that one writes straight to the
   * OS clipboard for pasting outside outl (desktop's `Y` /
   * `YankCurrentBlock`); this one never touches the OS clipboard —
   * it's the desktop's `Cmd/Ctrl+C` (`CopyBlock`) view-mode gesture,
   * just reached by long-press instead of a chord. `copyBlockMarkdown`
   * is read-only, so arming never mutates the workspace.
   */
  async function handleCopyBlock(id: string) {
    const markdown = await withError(() => copyBlockMarkdown(id));
    if (markdown !== undefined) setBlockClipboard(markdown);
  }

  /**
   * "Paste block" (long-press menu) — duplicate the armed clipboard's
   * subtree as a sibling right after `id`, minting fresh ids
   * (`paste_block_after`, same backend the desktop's `Cmd/Ctrl+V`
   * calls for a `kind: "copy"` clipboard). The clipboard persists
   * after a successful paste so it can be pasted again, mirroring the
   * desktop's non-cut branch. Only reachable when `blockClipboard()`
   * is armed — the context menu hides the action otherwise.
   */
  async function handlePasteBlock(id: string) {
    const pid = pageId();
    const markdown = blockClipboard();
    if (!pid || markdown === null) return;
    const next = await withError(() => pasteBlockAfter(pid, id, markdown));
    if (next) applyView(next);
  }

  /**
   * "Cut block" (long-press menu, RFC 0254 phase 4b) — render `id`'s
   * subtree to markdown and delete it in one backend round-trip
   * (`cutBlock`), then arm the same `blockClipboard` "Paste block"
   * reads. Deliberately **not** identity-preserving (the paste mints
   * fresh ids, per `cutBlock`'s doc comment) — the alternative is the
   * desktop's move-based cut, which needs a `{kind, nodeId}` tagged
   * clipboard this client doesn't have and doesn't need for a
   * long-press gesture.
   */
  async function handleCutBlock(id: string) {
    const pid = pageId();
    if (!pid) return;
    const reply = await withError(() => cutBlock(pid, id));
    if (!reply) return;
    if (editingId() === id) setEditingId(null);
    setBlockClipboard(reply.markdown);
    applyView(reply.view);
  }

  /**
   * "Copy block ref" (long-press menu, issue #18) — resolve `id`'s
   * `((blk-XXXXXX))` handle and put it on the OS clipboard, same
   * best-effort posture as "Copy text" above (some webviews refuse
   * `navigator.clipboard` outside a user-gesture chain).
   */
  async function handleCopyBlockRef(id: string) {
    const ref = await withError(() => copyBlockRef(id));
    if (ref === undefined) return;
    try {
      await navigator.clipboard?.writeText(ref);
    } catch {
      // Best-effort, same as "Copy text" / "Copy block" above.
    }
  }

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

  /** "Select blocks" (long-press menu) — start a selection anchored at
   *  `id`. Commits any in-flight edit first: entering selection mid-
   *  edit would leave a textarea open underneath a row now behaving as
   *  a tap target for range extension instead of text input. */
  async function handleSelectBlocks(id: string) {
    if (editingId()) await commitEdit();
    haptic("medium");
    setSelection(startSelection(id));
  }

  /** A tap on any row while a selection is active — grows or shrinks
   *  the range to meet it. Reachable only through `<BlockRow />`'s
   *  `onSelectTap`, which mobile only wires while `selection()` is
   *  non-null, but this stays defensive (no-op) if that ever changes. */
  function handleSelectTap(id: string) {
    const sel = selection();
    if (!sel) return;
    setSelection(extendSelectionTo(sel, id));
  }

  /** Toolbar `▲`/`▼` — grow the range by exactly one visible row, the
   *  discrete equivalent of the desktop's `Shift+↑`/`Shift+↓`
   *  (`SelectRangeUp` / `SelectRangeDown`) for a row that isn't
   *  directly reachable by tap without scrolling. */
  function handleGrowUp() {
    const sel = selection();
    const cur = view();
    if (!sel || !cur) return;
    setSelection(growSelectionUp(sel, cur.outline));
  }
  function handleGrowDown() {
    const sel = selection();
    const cur = view();
    if (!sel || !cur) return;
    setSelection(growSelectionDown(sel, cur.outline));
  }

  /** Leave selection mode. Captures the range as `lastSelection`
   *  first — every exit does (the toolbar's Done, a yank, a delete —
   *  vim's `gv` convention: `y`/`d` also drop out of Visual but leave
   *  the range reselectable). */
  function exitSelection() {
    const sel = selection();
    if (sel) setLastSelection(sel);
    setSelection(null);
  }

  /** Context-menu "Reselect last selection" — vim `gv`. Only offered
   *  (see `buildContextActions`'s `canReselect`) when `lastSelection`
   *  still resolves against the live outline; a peer edit or a fold
   *  can strand an endpoint between sessions. */
  function handleReselectLast() {
    const sel = lastSelection();
    const cur = view();
    if (!sel || !cur || !selectionIsLive(sel, cur.outline)) return;
    haptic("medium");
    setSelection(sel);
  }

  /** Every block id the active selection covers, in DFS visible
   *  order (top of the range first) — the ordering every range op
   *  below needs, in one place so indent/move/yank/delete can't
   *  disagree about it. `null` when there's no selection or an
   *  endpoint has left the outline. */
  function currentRangeIds(): string[] | null {
    const sel = selection();
    const cur = view();
    if (!sel || !cur) return null;
    const range = visualRangeIds(sel.anchorId, sel.cursorId, cur.outline);
    if (!range) return null;
    const ids = flattenVisible(cur.outline);
    const lo = ids.indexOf(range.lo);
    const hi = ids.indexOf(range.hi);
    if (lo === -1 || hi === -1) return null;
    return ids.slice(lo, hi + 1);
  }

  /** Walk every block in the range and fire `op` for each — the
   *  shared body behind Indent/Outdent/Move-range. `reverse` walks
   *  bottom-up: mirrors the desktop's `applyVisualBlockOp` exactly
   *  (a move-down has to clear the block *below* the range before its
   *  neighbours slide into place, or an ascending walk drags each
   *  block over its own not-yet-moved neighbour). The range stays
   *  selected afterward (vim convention — the user can repeat the
   *  op), matching the desktop's Indent/Outdent/Move handlers. */
  async function applyRangeOp(
    op: (pid: string, id: string) => Promise<PageView>,
    reverse = false,
  ) {
    const pid = pageId();
    const ids = currentRangeIds();
    if (!pid || !ids || ids.length === 0) return;
    const targets = reverse ? [...ids].reverse() : ids;
    let lastView: PageView | undefined;
    for (const id of targets) {
      const v = await withError(() => op(pid, id));
      if (v) lastView = v;
    }
    if (lastView) applyView(lastView);
  }

  async function handleIndentRange() {
    haptic("light");
    await applyRangeOp((pid, id) => indentBlock(pid, id));
  }
  async function handleOutdentRange() {
    haptic("light");
    await applyRangeOp((pid, id) => outdentBlock(pid, id));
  }
  async function handleMoveRangeUp() {
    haptic("light");
    await applyRangeOp((pid, id) => moveBlockUp(pid, id));
  }
  /** Bottom-up walk (see `applyRangeOp`'s doc) — the last block in the
   *  range has to clear the block below the range first. */
  async function handleMoveRangeDown() {
    haptic("light");
    await applyRangeOp((pid, id) => moveBlockDown(pid, id), true);
  }

  /**
   * Toolbar "Copy" — serialize the whole range as clean outl markdown
   * to the OS clipboard (the backend drops a block whose ancestor is
   * also in the range, so a parent+child selection doesn't duplicate
   * the child — same guarantee the desktop's `YankRange` documents).
   * Exits selection afterward, matching vim's `y` — the desktop's
   * `YankRange` does the same via `exitVisual()`.
   */
  async function handleYankRange() {
    const ids = currentRangeIds();
    if (!ids || ids.length === 0) return;
    haptic("light");
    try {
      const md = await copyMarkdown(ids);
      await navigator.clipboard?.writeText(md);
    } catch {
      // Best-effort — same posture as the single-block "Copy text"
      // action; some webviews refuse `navigator.clipboard` outside a
      // user gesture chain.
    }
    exitSelection();
  }

  /** Toolbar "Delete" — always confirms (`<ConfirmDialog>` via
   *  `pendingRangeDelete`), unlike the single-block swipe delete
   *  (which only prompts when that one block has descendants): a
   *  range is N blocks, any of which may carry children the user
   *  can't see from the toolbar. */
  function handleDeleteRangeRequest() {
    const ids = currentRangeIds();
    if (!ids || ids.length === 0) return;
    haptic("warning");
    setPendingRangeDelete(ids);
  }

  /** Bottom-up delete (children before parents) — mirrors the
   *  desktop's `DeleteRange`: when the range covers a parent and its
   *  descendants, deleting the parent first moves the whole subtree to
   *  trash and the follow-up delete on a descendant then fails
   *  ("already in trash"). `withError` records that per-id instead of
   *  aborting, so one bad id can't strand the rest of the range. */
  async function performDeleteRange(ids: string[]) {
    const pid = pageId();
    if (!pid) return;
    if (editingId() && ids.includes(editingId()!)) setEditingId(null);
    let lastView: PageView | undefined;
    for (let i = ids.length - 1; i >= 0; i--) {
      const v = await withError(() => deleteBlock(pid, ids[i]));
      if (v) lastView = v;
    }
    if (lastView) applyView(lastView);
    exitSelection();
  }

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

  async function handleIndent(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const next = await withError(() => indentBlock(pid, id));
    if (next) applyView(next);
  }

  async function handleOutdent(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const next = await withError(() => outdentBlock(pid, id));
    if (next) applyView(next);
  }

  async function handleMoveUp(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const next = await withError(() => moveBlockUp(pid, id));
    if (next) applyView(next);
  }

  async function handleMoveDown(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("light");
    const next = await withError(() => moveBlockDown(pid, id));
    if (next) applyView(next);
  }

  /**
   * Import a file (PDF, image, …) via the system document picker and
   * attach its link as a new block right after `id`. On iOS the picker
   * is the native document picker. The backend copies the file into
   * `<root>/assets/` and returns the refreshed view; outl never renders
   * the file — tapping the link opens it in the OS default viewer.
   */
  /**
   * "Remind me…" — ask for a `remind::` rule in the block's own
   * syntax and write it as a block property.
   *
   * A native time picker would be nicer, but the rule language is
   * richer than a clock (`3pm every 1h until DONE`), and a picker
   * that can only express the anchor would quietly hide the repeat.
   * A prompt seeded with a sane default keeps the whole grammar
   * reachable; the picker is the follow-up, not a substitute.
   *
   * An empty answer clears the rule — the "stop reminding me" path.
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

  /**
   * "New block above" (long-press menu, RFC 0254 phase 4b — mirrors
   * the desktop's `O` / `NewBlockAbove`). Uses `beforeId`, the same
   * floor-slot create the desktop uses — never a post-creation
   * `moveBlockUp` walk, which the desktop's own CLAUDE.md flags as the
   * bug this shape replaced.
   */
  async function handleCreateBefore(id: string) {
    const pid = pageId();
    if (!pid) return;
    haptic("medium");
    if (editingId()) await commitEdit();
    const reply = await withError(() =>
      createBlock(pid, { beforeId: id, text: "" }),
    );
    if (reply) {
      batch(() => {
        applyView(reply.view);
        startEdit(reply.new_id, "");
      });
    }
  }

  async function handleAppendBlock() {
    const pid = pageId();
    if (!pid) return;
    haptic("medium");
    if (editingId()) await commitEdit();
    const reply = await withError(() =>
      createBlock(pid, { afterId: null, parentId: null, text: null }),
    );
    if (reply) {
      batch(() => {
        applyView(reply.view);
        startEdit(reply.new_id, "");
      });
    }
  }

  /**
   * Core P2P pull, shared by the manual pull-to-refresh and the automatic
   * open/foreground sync. Force a sync pass against every iroh peer NOW (dial
   * instead of waiting for the catch-up tick), reload the local op log, and
   * reopen the current page so the re-render reflects what peers delivered.
   * Best-effort: `syncNow` is a no-op when iroh isn't wired, and tolerated
   * (toast, don't wedge) on a flaky peer so it never blocks the local reload.
   */
  async function pullAndReload(opts?: { background?: boolean }) {
    // `background` = the silent 4s poll. It still pulls + replays the op log,
    // but it only swaps the rendered view when the content ACTUALLY changed and
    // the user isn't editing — so an unchanged poll never re-renders (no scroll
    // jump, no cursor churn) and a desktop/TUI edit arriving mid-typing never
    // yanks the textarea out from under the user. The foreground paths (button,
    // app open, resume) always apply and show the spinner.
    // Input is sacred: never swap the workspace while the user is editing.
    // Reloading re-materializes the tree (which can re-mint the block id under
    // the cursor → `block <id> [Retry]`) and a slow reload freezes the UI. So
    // if a block is being edited, pull the peer's ops to disk in the background
    // (no `await` that blocks the user, capped so a dead peer can't hang it)
    // and mark the reload pending — the `editingId` effect below drains it the
    // instant they leave edit mode.
    if (editingId()) {
      reloadPendingWhileEditing = true;
      void withError(() => withTimeout(syncNow(), SYNC_TIMEOUT_MS, "Sync timed out"));
      return;
    }
    const bg = opts?.background ?? false;
    const gen = ++reloadGen;
    if (!bg) setSyncing(true);
    // Cap the force-sync: with an unreachable peer, `syncNow` waits out the
    // 10–30s connect timeout, and awaiting it here froze the reload for that
    // whole window. Time it out so the local reload always proceeds promptly.
    await withError(() => withTimeout(syncNow(), SYNC_TIMEOUT_MS, "Sync timed out"));
    await withError(reloadWorkspace);
    const cur = view();
    if (cur) {
      const next =
        cur.page.kind === "journal"
          ? await withError(() => openJournalFor(cur.page.slug))
          : await withError(() => openPageBySlug(cur.page.slug));
      if (next) {
        // A reload that comes back EMPTY while we already have content is
        // a transient partial read — the op log is mid-ingest / being
        // re-indexed by an inbound sync, not a real "everything was
        // deleted". Never clobber real content with it; the next poll
        // re-reads the settled log. This is what produced the "flip to
        // an empty page (0 ops)" flicker on the 3s poll.
        const clobbersContentWithEmpty =
          next.outline.length === 0 && cur.outline.length > 0;
        const changed =
          JSON.stringify(next.outline) !== JSON.stringify(cur.outline);
        // A newer reload started while our (possibly slow `syncNow`) read was
        // in flight — it read a fresher op log, so applying ours now would flip
        // the page back to the older state. That out-of-order apply is the
        // flicker; drop the superseded read.
        const superseded = gen !== reloadGen;
        if (
          !superseded &&
          !clobbersContentWithEmpty &&
          (!bg || changed) &&
          !editingId()
        ) {
          applyView(next);
        }
      }
    }
    // Re-read the dot off the fresh dial outcomes the force-sync produced.
    void refreshPeerStatus();
    if (!bg) setSyncing(false);
  }

  async function handleRefresh() {
    const pid = pageId();
    if (!pid) return;
    setRefreshing(true);
    haptic("light");
    await pullAndReload();
    setRefreshing(false);
  }

  async function handlePrevDay() {
    const cur = view();
    if (!cur || cur.page.kind !== "journal") return;
    haptic("light");
    const slug = await withError(() => previousDay(cur.page.slug));
    if (slug) {
      const next = await withError(() => openJournalFor(slug));
      if (next) applyView(next);
    }
  }

  async function handleNextDay() {
    const cur = view();
    if (!cur || cur.page.kind !== "journal") return;
    haptic("light");
    const slug = await withError(() => nextDay(cur.page.slug));
    if (slug) {
      const next = await withError(() => openJournalFor(slug));
      if (next) applyView(next);
    }
  }

  async function handleJumpToday() {
    haptic("light");
    const next = await withError(openTodayJournal);
    if (next) applyView(next);
  }

  /**
   * Calendar picked a day. The backend's `open_journal_for` opens-or-
   * creates the journal page, so picking a day that has never been
   * visited still lands on a fresh page ready for the user to type
   * into — no "page doesn't exist" error.
   */
  async function handlePickDate(slug: string) {
    setCalendarOpen(false);
    haptic("light");
    const next = await withError(() => openJournalFor(slug));
    if (next) applyView(next);
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
   * Jump from a block-search hit (page switcher's "Blocks" mode,
   * issue #19) to the page hosting it. A `BlockHit` carries only
   * `source_slug` — no `kind` — so this can't branch like
   * `handlePickPage` does; `openRef` already runs the journal-vs-page
   * decision tree (same call `handleRefClick` makes for a tapped
   * `[[ref]]`), so this delegates to it instead of duplicating that
   * logic. There is no per-block scroll/highlight anywhere in this
   * client yet (the backlinks jump doesn't do it either) — "jump"
   * means "open the hosting page", matching that existing bar.
   */
  async function handleJumpToBlock(hit: BlockHit) {
    setSwitcherOpen(false);
    await handleRefClick(hit.source_slug);
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
      <header
        class="z-30 shrink-0 bg-(--color-outl-bg)/80 px-3 pt-2 pb-3 backdrop-blur-xl"
        style="padding-top: max(env(safe-area-inset-top), 12px);"
      >
        <div class="grid grid-cols-[auto_auto_1fr] items-center gap-2">
          {/* Left capsule — visible only when the user has navigated
              away from today's journal. We always reserve a placeholder
              of the same width so the title doesn't jump horizontally
              when the back button appears / disappears. */}
          <Show
            when={view() && view()!.page.kind !== "journal"}
            fallback={<span aria-hidden="true" class="block h-9 w-9" />}
          >
            <div class="inline-flex rounded-full bg-(--color-outl-bg-elev)/85 shadow-[var(--shadow-capsule)] backdrop-blur-xl dark:shadow-[var(--shadow-capsule-dark)]">
              <button
                type="button"
                aria-label="Back to today's journal"
                onClick={handleJumpToday}
                class="flex h-9 w-9 items-center justify-center rounded-full text-(--color-outl-accent) active:bg-(--color-outl-border)/40"
              >
                <svg
                  width="20"
                  height="20"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="2"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <path d="M9 14L4 9l5-5" />
                  <path d="M4 9h11a5 5 0 0 1 5 5v6" />
                </svg>
              </button>
            </div>
          </Show>

          {/* Center — title region. `min-w-0` is what lets the inner
              truncate work in PageHeader. Press-and-hold anywhere in
              here opens the page's properties (see `titleLongPress`);
              the journal arrows below are buttons, so they keep their
              own taps. */}
          <div
            class="min-w-0"
            onPointerDown={titleLongPress.onPointerDown}
            onPointerMove={titleLongPress.onPointerMove}
            onPointerUp={titleLongPress.onPointerUp}
            onPointerCancel={titleLongPress.onPointerUp}
            onClick={(e) => {
              // Swallow the click the completed hold produces, or the
              // journal header would also step a day.
              if (titleLongPress.consumedClick()) {
                e.preventDefault();
                e.stopPropagation();
              }
            }}
          >
            <Show
              when={view()?.page.kind === "journal"}
              fallback={
                <PageHeader
                  title={view()?.page.title ?? ""}
                  kind={view()?.page.kind ?? null}
                />
              }
            >
              <JournalHeader
                slug={view()?.page.slug ?? ""}
                todaySlug={todaySlugValue()}
                onPrev={handlePrevDay}
                onNext={handleNextDay}
                onToday={handleJumpToday}
              />
            </Show>
          </div>

          {/* Right capsule — grouped page actions. SyncDot lives inline
              between pages-search and refresh so the user reads it as
              "status of the data this capsule controls". */}
          <div class="ios-scroll inline-flex max-w-full items-center justify-self-end overflow-x-auto rounded-full bg-(--color-outl-bg-elev)/85 shadow-[var(--shadow-capsule)] backdrop-blur-xl dark:shadow-[var(--shadow-capsule-dark)]">
            <button
              type="button"
              aria-label="Calendar"
              onClick={() => {
                haptic("light");
                setCalendarOpen(true);
              }}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
            >
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-outl-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <rect x="3" y="4" width="18" height="18" rx="3" />
                <path d="M3 10h18M8 2v4m8-4v4" />
              </svg>
            </button>
            <button
              type="button"
              aria-label="Pages"
              onClick={() => {
                haptic("light");
                setSwitcherOpen(true);
              }}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
            >
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-outl-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M21 21l-4.3-4.3M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z" />
              </svg>
            </button>
            {/* Fold all / unfold all (RFC 0254 phase 4b, mirrors the
                desktop's `z M` / `z R`) — walks the whole page, not just
                the zoomed subtree, same as the desktop's `zM`/`zR`. */}
            <button
              type="button"
              aria-label="Fold all"
              onClick={handleFoldAll}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
            >
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-outl-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M7 14l5 5 5-5M7 5l5 5 5-5" />
              </svg>
            </button>
            <button
              type="button"
              aria-label="Unfold all"
              onClick={handleUnfoldAll}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
            >
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-outl-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M7 9l5-5 5 5M7 19l5-5 5 5" />
              </svg>
            </button>
            {/* Plugin-contributed toolbar buttons — one inline glyph per
                entry, sitting among the native header actions. Discreet:
                the plugin's `icon` rendered as text, tap runs its command
                (re-render + toast handled by `runToolbarButton`). */}
            <For each={toolbarButtons()}>
              {(btn) => (
                <button
                  type="button"
                  aria-label={btn.title ?? `Plugin: ${btn.command_id}`}
                  title={btn.title ?? btn.command_id}
                  onClick={() => void runToolbarButton(btn)}
                  class="flex h-9 w-9 items-center justify-center rounded-full text-[17px] leading-none text-(--color-outl-accent) active:bg-(--color-outl-border)/40"
                >
                  {btn.icon}
                </button>
              )}
            </For>
            <button
              type="button"
              aria-label="Reminders"
              onClick={() => {
                haptic("light");
                setRemindersOpen(true);
              }}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
            >
              {/* Bell glyph — the reminders surface. */}
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-outl-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
                <path d="M13.73 21a2 2 0 0 1-3.46 0" />
              </svg>
            </button>
            <button
              type="button"
              aria-label="Plugin commands"
              onClick={() => {
                haptic("light");
                setPluginsOpen(true);
              }}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
            >
              {/* Stacked-squares "extensions/plugins" glyph, mirrors the
                  desktop's `⧉` toggle. */}
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-outl-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <rect x="3" y="3" width="8" height="8" rx="1.5" />
                <rect x="13" y="3" width="8" height="8" rx="1.5" />
                <rect x="3" y="13" width="8" height="8" rx="1.5" />
                <rect x="13" y="13" width="8" height="8" rx="1.5" />
              </svg>
            </button>
            {/* The sync dot IS the devices/pairing affordance: it shows the
                mesh status AND opens the pairing sheet on tap — no separate
                (ugly) devices glyph. Mirrors the desktop's clickable dot. */}
            <button
              type="button"
              aria-label="Devices and sync — tap to pair"
              onClick={() => {
                haptic("light");
                setDevicesOpen(true);
              }}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
            >
              <SyncDot
                status={
                  // PRIMARY signal is iroh peer health, not navigator.onLine.
                  // A force-sync in flight wins (spinner); else a reachable
                  // peer → synced (green); else offline/orange — either the
                  // device has no radio, or peers exist but none answered
                  // (or none are paired, so there's nothing to sync with).
                  syncing()
                    ? "syncing"
                    : online() && peersUp()
                      ? "synced"
                      : "offline"
                }
              />
            </button>
            <button
              type="button"
              aria-label="Sync now"
              onClick={handleRefresh}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
            >
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-outl-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                style={{
                  transform: refreshing() ? "rotate(360deg)" : "rotate(0deg)",
                  transition: "transform 800ms ease-in-out",
                }}
                aria-hidden="true"
              >
                <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
                <path d="M21 3v5h-5" />
              </svg>
            </button>
          </div>
        </div>
      </header>

      <main class="ios-scroll flex-1 pb-32">
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
                    <button
                      type="button"
                      onClick={handleAppendBlock}
                      class="flex w-full flex-col items-center px-5 py-16 text-center active:opacity-50"
                    >
                      <svg
                        width="44"
                        height="44"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.5"
                        stroke-linecap="round"
                        stroke-linejoin="round"
                        class="mb-3 text-(--color-outl-fg-dimmer)"
                        aria-hidden="true"
                      >
                        <path d="M12 20h9" />
                        <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
                      </svg>
                      <p class="text-[15px] text-(--color-outl-fg-dim)">
                        Nothing here yet.
                      </p>
                      <p class="mt-1 text-[13px] text-(--color-outl-accent)">
                        Tap to start writing
                      </p>
                    </button>
                  }
                >
                  <div class="flex flex-col items-center px-5 py-12 text-center">
                    <p class="text-[15px] text-(--color-outl-fg-dim)">
                      Couldn't open the workspace.
                    </p>
                    <button
                      type="button"
                      onClick={() => {
                        setLoaded(false);
                        void loadTodayWithRetry();
                      }}
                      class="mt-3 rounded-full bg-(--color-outl-accent) px-5 py-2 text-[14px] font-medium text-white active:opacity-70"
                    >
                      Retry
                    </button>
                  </div>
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
                <div class="mb-1 flex items-center gap-1 overflow-x-auto px-4 pt-1 pb-2">
                  <button
                    type="button"
                    aria-label="Zoom out"
                    onClick={handleZoomOut}
                    class="flex shrink-0 items-center gap-1 rounded-full py-0.5 pr-2 pl-1 text-[13px] font-medium text-(--color-outl-accent) active:opacity-50"
                  >
                    <ChevronLeft />
                    Back
                  </button>
                  <For each={fv().breadcrumb}>
                    {(crumb) => (
                      <>
                        <span
                          aria-hidden="true"
                          class="shrink-0 text-[12px] text-(--color-outl-fg-dimmer)"
                        >
                          /
                        </span>
                        <button
                          type="button"
                          onClick={() => setFocusBlockId(crumb.id)}
                          class="max-w-[12rem] shrink-0 truncate text-[13px] text-(--color-outl-fg-dim) active:opacity-50"
                        >
                          {crumb.text || "Untitled"}
                        </button>
                      </>
                    )}
                  </For>
                </div>
              )}
            </Show>
            {/* An empty page used to render as nothing at all: no
                text, no affordance, no hint that a tap anywhere would
                help. It is also the one state with no block to
                long-press, so it was the only place page properties
                were unreachable. Both doors live here, and the whole
                block costs nothing on a page that has content. */}
            <Show when={outlineRoots().length === 0 && view()}>
              <div class="flex flex-col items-center gap-4 py-16 text-center">
                <p class="text-[15px] text-(--color-outl-fg-dim)">
                  This page is empty
                </p>
                <div class="flex items-center gap-2">
                  <button
                    type="button"
                    onClick={() => void handleAppendBlock()}
                    class="rounded-full bg-(--color-outl-accent) px-4 py-2 text-[15px] font-medium text-white active:opacity-70"
                  >
                    Add a block
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      if (!pageId()) return;
                      haptic("light");
                      setPropertiesTarget({ blockId: null, scope: "page" });
                    }}
                    class="rounded-full bg-(--color-outl-bg-elev) px-4 py-2 text-[15px] text-(--color-outl-fg) active:opacity-70"
                  >
                    Properties
                  </button>
                </div>
                <p class="max-w-[16rem] text-[13px] text-(--color-outl-fg-dimmer)">
                  Hold the title to edit page properties from anywhere.
                </p>
              </div>
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
                  onToggleTodo={handleToggleTodo}
                  onDelete={handleDelete}
                  onIndent={handleIndent}
                  onOutdent={handleOutdent}
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
                  onSelectTap={handleSelectTap}
                />
              )}
            </For>
          </Show>
        </section>

        {/* Always render the section for non-journal pages so the
            bidirectional-linking concept is discoverable; journals
            stay hidden when empty (the daily flow is already busy
            enough without an empty box every day). */}
        <Show
          when={
            view()?.page.kind === "page" ||
            (backlinks()?.backlinks.length ?? 0) > 0
          }
        >
          <BacklinksSection
            backlinks={backlinks()?.backlinks ?? []}
            order={backlinks()?.backlinks_order ?? "newest"}
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
            onJump={async (link) => {
              if (!link.source_page) return;
              haptic("light");
              const sp = link.source_page;
              const next =
                sp.kind === "journal"
                  ? await withError(() => openJournalFor(sp.slug))
                  : await withError(() => openPageBySlug(sp.slug));
              if (next) applyView(next);
            }}
          />
        </Show>
        </div>
        </PullToRefresh>

        <Show when={stats()}>
          <footer class="px-5 pt-3 pb-32 text-center text-[12px] text-(--color-outl-fg-dimmer)">
            {stats()!.blocks} blocks · {stats()!.ops} ops · actor{" "}
            {stats()!.actor.slice(0, 6)}
          </footer>
        </Show>
      </main>

      {/* Hidden while a range selection is active — the FAB and the
          selection toolbar both dock bottom-right/bottom-full-width
          and "add a block" mid-batch-op is not a gesture the RFC asks
          for; hiding it keeps the two floating surfaces from
          overlapping. */}
      <Show when={!editingId() && view() && selection() === null}>
        <button
          type="button"
          aria-label="Add block"
          onClick={handleAppendBlock}
          class="outl-press fixed right-5 z-30 flex h-14 w-14 items-center justify-center rounded-full bg-(--color-outl-accent) shadow-lg"
          style="bottom: max(env(safe-area-inset-bottom), 20px);"
        >
          <svg
            width="26"
            height="26"
            viewBox="0 0 24 24"
            fill="none"
            stroke="white"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
            aria-hidden="true"
          >
            <path d="M12 5v14M5 12h14" />
          </svg>
        </button>
      </Show>

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
      />

      <Calendar
        open={calendarOpen()}
        selectedSlug={
          view()?.page.kind === "journal" ? (view()?.page.slug ?? null) : null
        }
        todaySlug={todaySlugValue()}
        onClose={() => setCalendarOpen(false)}
        onPick={handlePickDate}
      />

      <DevicesSheet
        open={devicesOpen()}
        onClose={() => setDevicesOpen(false)}
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

      <ConfirmDialog
        open={pendingDelete() !== null}
        title="Delete block?"
        message={
          pendingDelete()
            ? `This block has ${pendingDelete()!.descendants} ${
                pendingDelete()!.descendants === 1 ? "child" : "children"
              } that will also be deleted. This can't be undone.`
            : ""
        }
        onCancel={() => setPendingDelete(null)}
        onConfirm={() => {
          const p = pendingDelete();
          setPendingDelete(null);
          if (p) void performDelete(p.id);
        }}
      />

      <BlockContextMenu
        open={contextMenuBlockId() !== null}
        onClose={() => setContextMenuBlockId(null)}
        actions={buildContextActions(
          contextMenuBlockId(),
          view(),
          {
            indent: handleIndent,
            outdent: handleOutdent,
            moveUp: handleMoveUp,
            moveDown: handleMoveDown,
            toggleTodo: handleToggleTodo,
            delete: handleDelete,
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
            copyBlock: (id) => void handleCopyBlock(id),
            pasteBlock: (id) => void handlePasteBlock(id),
            cutBlock: (id) => void handleCutBlock(id),
            copyBlockRef: (id) => void handleCopyBlockRef(id),
            newBlockAbove: (id) => void handleCreateBefore(id),
            selectBlocks: (id) => void handleSelectBlocks(id),
            reselectSelection: () => handleReselectLast(),
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
        count={currentRangeIds()?.length ?? 0}
        onGrowUp={handleGrowUp}
        onGrowDown={handleGrowDown}
        onIndent={() => void handleIndentRange()}
        onOutdent={() => void handleOutdentRange()}
        onMoveUp={() => void handleMoveRangeUp()}
        onMoveDown={() => void handleMoveRangeDown()}
        onCopy={() => void handleYankRange()}
        onDelete={handleDeleteRangeRequest}
        onDone={exitSelection}
      />

      <ConfirmDialog
        open={pendingRangeDelete() !== null}
        title="Delete blocks?"
        message={
          pendingRangeDelete()
            ? `This will delete ${pendingRangeDelete()!.length} ${
                pendingRangeDelete()!.length === 1 ? "block" : "blocks"
              }, including any nested children. This can't be undone.`
            : ""
        }
        onCancel={() => setPendingRangeDelete(null)}
        onConfirm={() => {
          const ids = pendingRangeDelete();
          setPendingRangeDelete(null);
          if (ids) void performDeleteRange(ids);
        }}
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

function JournalHeader(props: {
  slug: string;
  /** Today's slug, resolved once by the parent `Journal` so the header
   *  and the "back to today" button share a single source of truth.
   *  `null` while the parent is still resolving it. */
  todaySlug: string | null;
  onPrev: () => void;
  onNext: () => void;
  onToday: () => void;
}) {
  const isToday = () =>
    props.todaySlug !== null && props.todaySlug === props.slug;
  return (
    <div class="min-w-0">
      <div class="flex items-center justify-center gap-1.5">
        <button
          type="button"
          aria-label="Previous day"
          onClick={props.onPrev}
          class="shrink-0 rounded-full p-1 text-(--color-outl-accent) active:opacity-50"
        >
          <ChevronLeft />
        </button>
        <h1
          class="cursor-pointer whitespace-nowrap text-[17px] font-semibold leading-tight tracking-tight tabular-nums active:opacity-60"
          onClick={props.onToday}
        >
          {props.slug}
        </h1>
        <button
          type="button"
          aria-label="Next day"
          onClick={props.onNext}
          class="shrink-0 rounded-full p-1 text-(--color-outl-accent) active:opacity-50"
        >
          <ChevronRight />
        </button>
      </div>
      {/* Always rendered (just hidden when not today) so the header
          keeps the same height across day navigation — otherwise the
          whole outline below jumps by ~14px every time the user pages
          past today, which reads as the header "dancing". */}
      <p
        class="mt-0.5 text-center text-[11px] font-medium uppercase tracking-[0.08em] text-(--color-outl-accent)"
        classList={{ invisible: !isToday() }}
        aria-hidden={!isToday()}
      >
        Today
      </p>
    </div>
  );
}

function PageHeader(props: { title: string; kind: "page" | "journal" | null }) {
  return (
    <div class="min-w-0 text-center">
      <p class="text-[11px] font-medium uppercase tracking-wider text-(--color-outl-fg-dimmer)">
        {props.kind === "journal" ? "Journal" : "Page"}
      </p>
      <h1 class="truncate text-[17px] font-semibold leading-tight tracking-tight">
        {props.title}
      </h1>
    </div>
  );
}

function ChevronLeft() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M15 18l-6-6 6-6" />
    </svg>
  );
}

function ChevronRight() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2.5"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d="M9 18l6-6-6-6" />
    </svg>
  );
}

// Use referenced helper to silence unused-import false-positive.
const _holdTitle = dateTitle;
void _holdTitle;

/**
 * Wire the long-press block id into a typed action list for
 * `<BlockContextMenu>`. Each action carries an SVG path, label, and
 * a guard (`enabled`) so we hide "Move up" on the first sibling and
 * "Move down" on the last — gestures iOS users expect to disappear
 * when they don't apply.
 *
 * The handlers are passed in from `Journal()`'s scope so the menu
 * doesn't have to import every Tauri command directly.
 *
 * Exported (only) so `Journal.buildContextActions.test.ts` can drive
 * it directly — mounting the whole `<Journal>` component just to
 * assert which context-menu rows appear would need a full Tauri
 * command mock surface for no extra coverage.
 */
export function buildContextActions(
  blockId: string | null,
  pageView: import("@outl/shared/api/types").PageView | null,
  handlers: {
    indent: (id: string) => void;
    outdent: (id: string) => void;
    moveUp: (id: string) => void;
    moveDown: (id: string) => void;
    toggleTodo: (id: string) => void;
    delete: (id: string) => void;
    runCode: (id: string) => void;
    insertTemplate: (id: string) => void;
    properties: (id: string) => void;
    remindMe: (id: string) => void;
    copy: (id: string) => void;
    copyBlock: (id: string) => void;
    pasteBlock: (id: string) => void;
    /** RFC 0254 phase 4b — cut `id`'s subtree into the block
     *  clipboard, deleting it from the source. */
    cutBlock: (id: string) => void;
    /** RFC 0254 phase 4b (issue #18) — copy `id`'s `((blk-XXXXXX))`
     *  ref handle to the OS clipboard. */
    copyBlockRef: (id: string) => void;
    /** RFC 0254 phase 4b — create a new sibling immediately above
     *  `id` and start editing it. */
    newBlockAbove: (id: string) => void;
    attachFile: (id: string) => void;
    /** RFC 0254 phase 3 — start a range selection anchored at `id`. */
    selectBlocks: (id: string) => void;
    /** RFC 0254 phase 3 — reselect the range captured on the last
     *  exit (vim `gv`). Takes no id: the range it restores carries
     *  its own anchor/cursor, independent of which block's menu the
     *  user opened to reach it. */
    reselectSelection: () => void;
  },
  /** Is the block clipboard armed (`blockClipboard() !== null`)? Passed
   *  in rather than read from a `handlers` closure so the caller's own
   *  signal read stays inside its `actions=` prop-getter evaluation —
   *  see the call site's comment for why that's what makes this
   *  reactive. */
  canPasteBlock = false,
  /** Does `lastSelection` still resolve against the live outline? Same
   *  "read it at the call site" reactivity reasoning as
   *  `canPasteBlock`. */
  canReselect = false,
): BlockContextAction[] {
  if (!blockId || !pageView) return [];
  // Resolve sibling position so we can hide move-up/down at the
  // ends. Walking the outline is cheap (the user just long-pressed,
  // there's no per-frame budget here).
  const siblings = locateSiblings(pageView.outline, blockId);
  const index = siblings
    ? siblings.findIndex((b) => b.id === blockId)
    : -1;
  const canMoveUp = index > 0;
  const canMoveDown = siblings ? index < siblings.length - 1 : false;
  // `Run code` only shows up when the long-pressed block is a fenced
  // `` ```lang …``` `` AND the fence language is one we actually ship
  // a runtime for. The backend re-validates via `run_block_at_index`
  // (`UnknownLanguage` error path), so this is a UX guard — a long
  // press on a `swift`/`shell`/`ruby` fence shouldn't offer a "Run"
  // button that then errors out, and the narrower set is also
  // cleaner to defend against App Review 2.5.2 if the reviewer
  // browses the contextual menu.
  // Stays in sync with the `outl-exec` features enabled for the
  // mobile IPA (`crates/outl-mobile/src-tauri/Cargo.toml`).
  const block = findBlock(pageView.outline, blockId);
  const fence = block ? detectFence(block.text) : null;
  const fenceLang = fence?.language.toLowerCase() ?? "";
  const canRun =
    fence &&
    (fenceLang === "lisp" ||
      fenceLang === "js" ||
      fenceLang === "javascript" ||
      fenceLang === "node" ||
      fenceLang === "py" ||
      fenceLang === "python" ||
      fenceLang === "lua");
  return [
    ...(canRun && fence
      ? [
          {
            id: "runCode",
            label: `Run ${fence.language}`,
            // SF-Symbols-equivalent "play.fill" — filled right
            // triangle, matches the desktop's `▶ Run` chip.
            iconPath: "M8 5v14l11-7z",
            onSelect: () => handlers.runCode(blockId),
          } satisfies BlockContextAction,
        ]
      : []),
    {
      id: "toggleTodo",
      label: "Toggle TODO",
      iconPath: "M5 12l4 4 10-10",
      onSelect: () => handlers.toggleTodo(blockId),
    },
    // Touch-native range selection (RFC 0254 phase 3) — the one
    // interaction this phase invents. Long-press already opens this
    // menu for every other single-block action, so "start selecting
    // here" is a row in the same sheet rather than a second gesture
    // competing with long-press-for-menu and swipe-for-delete.
    {
      id: "selectBlocks",
      label: "Select blocks",
      // Checklist glyph — three ticked rows, reads as "act on more
      // than one block".
      iconPath:
        "M9 6h11M9 12h11M9 18h11M4 6l1.5 1.5L8 5M4 12l1.5 1.5L8 10M4 18l1.5 1.5L8 16",
      onSelect: () => handlers.selectBlocks(blockId),
    },
    ...(canReselect
      ? [
          {
            id: "reselectSelection",
            label: "Reselect last selection",
            // Circular-arrow "restore" glyph.
            iconPath:
              "M3 12a9 9 0 1 1 3 6.7 M3 12v5 M3 17h5",
            onSelect: () => handlers.reselectSelection(),
          } satisfies BlockContextAction,
        ]
      : []),
    {
      id: "copy",
      label: "Copy text",
      iconPath:
        "M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2 M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z",
      onSelect: () => handlers.copy(blockId),
    },
    // Block clipboard (RFC 0254 phase 2, cut added phase 4b), distinct
    // from "Copy text" above: that one writes to the OS clipboard for
    // pasting outside outl; this trio arms an in-app buffer for
    // duplicating (or, for cut, relocating) the block + subtree
    // elsewhere in this workspace, fresh ids on paste — mirrors the
    // desktop's `Cmd/Ctrl+X` / `Cmd/Ctrl+C` / `Cmd/Ctrl+V` (`CutBlock` /
    // `CopyBlock` / `PasteBlock`).
    {
      id: "cutBlock",
      label: "Cut block",
      // Scissors glyph.
      iconPath:
        "M6 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M20 4L8.5 15.5 M14.5 14.5L20 20 M8.5 8.5L10 10",
      onSelect: () => handlers.cutBlock(blockId),
    },
    {
      id: "copyBlock",
      label: "Copy block",
      // Two overlapping rectangles — the "duplicate" glyph, distinct
      // from "Copy text"'s single-document icon above.
      iconPath:
        "M9 9h10v10H9z M5 15V5a2 2 0 0 1 2-2h10",
      onSelect: () => handlers.copyBlock(blockId),
    },
    ...(canPasteBlock
      ? [
          {
            id: "pasteBlock",
            label: "Paste block",
            // Clipboard glyph.
            iconPath:
              "M9 5h6a1 1 0 0 1 1 1v1H8V6a1 1 0 0 1 1-1z M8 4h8a2 2 0 0 1 2 2v13a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z",
            onSelect: () => handlers.pasteBlock(blockId),
          } satisfies BlockContextAction,
        ]
      : []),
    {
      id: "copyBlockRef",
      label: "Copy block ref",
      // Link/chain glyph — reads as "copy a reference", distinct from
      // both the document (Copy text) and duplicate (Copy block) icons.
      iconPath:
        "M9 12a3 3 0 0 0 4.24 0l3-3a3 3 0 0 0-4.24-4.24l-1 1 M15 12a3 3 0 0 0-4.24 0l-3 3a3 3 0 0 0 4.24 4.24l1-1",
      onSelect: () => handlers.copyBlockRef(blockId),
    },
    {
      id: "newBlockAbove",
      label: "New block above",
      // Plus above a horizontal rule — reads as "insert before".
      iconPath: "M12 4v8 M8 8h8 M4 20h16",
      onSelect: () => handlers.newBlockAbove(blockId),
    },
    {
      id: "remindMe",
      label: "Remind me…",
      // "bell" — the reminders affordance, same glyph family as the
      // header button that opens the list.
      iconPath:
        "M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9 M13.73 21a2 2 0 0 1-3.46 0",
      onSelect: () => handlers.remindMe(blockId),
    },
    {
      id: "properties",
      label: "Properties…",
      // "tag" glyph — a `key:: value` is the block's metadata, and the
      // sheet behind it is the only GUI place to create one.
      iconPath:
        "M20.6 13.4l-7.2 7.2a2 2 0 0 1-2.8 0l-7.2-7.2a2 2 0 0 1-.6-1.4V4a1 1 0 0 1 1-1h8a2 2 0 0 1 1.4.6l7.4 7.4a2 2 0 0 1 0 2.8z M7.5 7.5h.01",
      onSelect: () => handlers.properties(blockId),
    },
    {
      id: "insertTemplate",
      label: "Insert template",
      // "doc.on.doc"-style stacked pages — reads as "stamp a template".
      iconPath:
        "M9 3H5a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h4 M15 7h4a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2h-8a2 2 0 0 1-2-2V9a2 2 0 0 1 2-2z",
      onSelect: () => handlers.insertTemplate(blockId),
    },
    {
      id: "attachFile",
      label: "Attach file",
      // "paperclip" — reads as "attach an uploaded file".
      iconPath:
        "M21 11.5l-9 9a5 5 0 0 1-7-7l9-9a3.5 3.5 0 0 1 5 5l-9 9a2 2 0 0 1-3-3l8-8",
      onSelect: () => handlers.attachFile(blockId),
    },
    {
      id: "indent",
      label: "Indent",
      iconPath: "M3 5h12M3 12h8M3 19h12M15 9l3 3-3 3",
      onSelect: () => handlers.indent(blockId),
    },
    {
      id: "outdent",
      label: "Outdent",
      iconPath: "M3 5h12M3 12h8M3 19h12M21 9l-3 3 3 3",
      onSelect: () => handlers.outdent(blockId),
    },
    {
      id: "moveUp",
      label: "Move up",
      iconPath: "M12 19V5M5 12l7-7 7 7",
      enabled: () => canMoveUp,
      onSelect: () => handlers.moveUp(blockId),
    },
    {
      id: "moveDown",
      label: "Move down",
      iconPath: "M12 5v14M19 12l-7 7-7-7",
      enabled: () => canMoveDown,
      onSelect: () => handlers.moveDown(blockId),
    },
    {
      id: "delete",
      label: "Delete",
      iconPath:
        "M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2m-9 0v14a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2V6",
      destructive: true,
      onSelect: () => handlers.delete(blockId),
    },
  ];
}

/** DFS for the sibling list containing `targetId`. Returns the
 *  block array (not the parent) so the caller can use `findIndex`
 *  without an extra walk. */
function locateSiblings(
  forest: import("@outl/shared/api/types").BlockNode[],
  targetId: string,
): import("@outl/shared/api/types").BlockNode[] | null {
  for (const node of forest) {
    if (node.id === targetId) return forest;
    const inner = locateSiblings(node.children ?? [], targetId);
    if (inner) return inner;
  }
  return null;
}
