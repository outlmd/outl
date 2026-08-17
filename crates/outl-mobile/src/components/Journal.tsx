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
  copyMarkdown,
  createBlock,
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
  pasteMarkdown,
  peerStatus,
  pluginRun,
  pluginSyncHooks,
  pluginToolbar,
  previousDay,
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
  focusSubtree,
  rawTextWithTodo,
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
import { haptic } from "../lib/haptics";
import { BacklinksSection } from "./BacklinksSection";
import { BlockContextMenu, type BlockContextAction } from "./BlockContextMenu";
import { ConfirmDialog } from "./ConfirmDialog";
import { TemplateSheet } from "./TemplateSheet";
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
    if (v.page.slug !== view()?.page.slug) setFocusBlockId(null);
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
   * prompt — deleting a parent destroys the whole subtree and the
   * user can't undo that from the mobile UI yet. Leaf blocks
   * delete immediately (no prompt) to keep the swipe gesture fast.
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
        class="z-30 shrink-0 bg-(--color-ios-bg)/80 px-3 pt-2 pb-3 backdrop-blur-xl dark:bg-(--color-iosd-bg)/80"
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
            <div class="inline-flex rounded-full bg-(--color-ios-card)/85 shadow-[var(--shadow-capsule)] backdrop-blur-xl dark:bg-(--color-iosd-card)/85 dark:shadow-[var(--shadow-capsule-dark)]">
              <button
                type="button"
                aria-label="Back to today's journal"
                onClick={handleJumpToday}
                class="flex h-9 w-9 items-center justify-center rounded-full text-(--color-ios-accent) active:bg-(--color-ios-divider)/40 dark:text-(--color-iosd-accent) dark:active:bg-(--color-iosd-divider)/40"
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
              truncate work in PageHeader. */}
          <div class="min-w-0">
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
          <div class="ios-scroll inline-flex max-w-full items-center justify-self-end overflow-x-auto rounded-full bg-(--color-ios-card)/85 shadow-[var(--shadow-capsule)] backdrop-blur-xl dark:bg-(--color-iosd-card)/85 dark:shadow-[var(--shadow-capsule-dark)]">
            <button
              type="button"
              aria-label="Calendar"
              onClick={() => {
                haptic("light");
                setCalendarOpen(true);
              }}
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
            >
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-ios-accent)"
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
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
            >
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-ios-accent)"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M21 21l-4.3-4.3M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z" />
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
                  class="flex h-9 w-9 items-center justify-center rounded-full text-[17px] leading-none text-(--color-ios-accent) active:bg-(--color-ios-divider)/40 dark:text-(--color-iosd-accent) dark:active:bg-(--color-iosd-divider)/40"
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
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
            >
              {/* Bell glyph — the reminders surface. */}
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-ios-accent)"
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
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
            >
              {/* Stacked-squares "extensions/plugins" glyph, mirrors the
                  desktop's `⧉` toggle. */}
              <svg
                width="20"
                height="20"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-ios-accent)"
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
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
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
              class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-ios-divider)/40 dark:active:bg-(--color-iosd-divider)/40"
            >
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="var(--color-ios-accent)"
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
                        class="mb-3 text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)"
                        aria-hidden="true"
                      >
                        <path d="M12 20h9" />
                        <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4 12.5-12.5z" />
                      </svg>
                      <p class="text-[15px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                        Nothing here yet.
                      </p>
                      <p class="mt-1 text-[13px] text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                        Tap to start writing
                      </p>
                    </button>
                  }
                >
                  <div class="flex flex-col items-center px-5 py-12 text-center">
                    <p class="text-[15px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
                      Couldn't open the workspace.
                    </p>
                    <button
                      type="button"
                      onClick={() => {
                        setLoaded(false);
                        void loadTodayWithRetry();
                      }}
                      class="mt-3 rounded-full bg-(--color-ios-accent) px-5 py-2 text-[14px] font-medium text-white active:opacity-70 dark:bg-(--color-iosd-accent)"
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
                    class="flex shrink-0 items-center gap-1 rounded-full py-0.5 pr-2 pl-1 text-[13px] font-medium text-(--color-ios-accent) active:opacity-50 dark:text-(--color-iosd-accent)"
                  >
                    <ChevronLeft />
                    Back
                  </button>
                  <For each={fv().breadcrumb}>
                    {(crumb) => (
                      <>
                        <span
                          aria-hidden="true"
                          class="shrink-0 text-[12px] text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)"
                        >
                          /
                        </span>
                        <button
                          type="button"
                          onClick={() => setFocusBlockId(crumb.id)}
                          class="max-w-[12rem] shrink-0 truncate text-[13px] text-(--color-ios-text-secondary) active:opacity-50 dark:text-(--color-iosd-text-secondary)"
                        >
                          {crumb.text || "Untitled"}
                        </button>
                      </>
                    )}
                  </For>
                </div>
              )}
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
          <footer class="px-5 pt-3 pb-32 text-center text-[12px] text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
            {stats()!.blocks} blocks · {stats()!.ops} ops · actor{" "}
            {stats()!.actor.slice(0, 6)}
          </footer>
        </Show>
      </main>

      <Show when={!editingId() && view()}>
        <button
          type="button"
          aria-label="Add block"
          onClick={handleAppendBlock}
          class="outl-press fixed right-5 z-30 flex h-14 w-14 items-center justify-center rounded-full bg-(--color-ios-accent) shadow-lg dark:bg-(--color-iosd-accent)"
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
          },
        )}
      />

      <TemplateSheet
        blockId={templateBlockId()}
        onClose={() => setTemplateBlockId(null)}
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
          class="shrink-0 rounded-full p-1 text-(--color-ios-accent) active:opacity-50 dark:text-(--color-iosd-accent)"
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
          class="shrink-0 rounded-full p-1 text-(--color-ios-accent) active:opacity-50 dark:text-(--color-iosd-accent)"
        >
          <ChevronRight />
        </button>
      </div>
      {/* Always rendered (just hidden when not today) so the header
          keeps the same height across day navigation — otherwise the
          whole outline below jumps by ~14px every time the user pages
          past today, which reads as the header "dancing". */}
      <p
        class="mt-0.5 text-center text-[11px] font-medium uppercase tracking-[0.08em] text-(--color-ios-accent) dark:text-(--color-iosd-accent)"
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
      <p class="text-[11px] font-medium uppercase tracking-wider text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
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
 */
function buildContextActions(
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
    remindMe: (id: string) => void;
    copy: (id: string) => void;
    attachFile: (id: string) => void;
  },
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
    {
      id: "copy",
      label: "Copy text",
      iconPath:
        "M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2 M9 2h6a1 1 0 0 1 1 1v2a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V3a1 1 0 0 1 1-1z",
      onSelect: () => handlers.copy(blockId),
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
