import { Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";

import {
  openExternalFile,
  openJournalFor,
  openPageBySlug,
  openTodayJournal,
  pageBacklinks,
  reloadWorkspace,
  todaySlug,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";
import { bootMark } from "@outl/shared/boot";
import { installForegroundRefresh } from "@outl/shared/capture";
import { createDayClock } from "@outl/shared/journal";
import { flattenAll } from "@outl/shared/outline";
import { CrashReportsNotice } from "@outl/shared/polish";

import {
  appState,
  setAppState,
  setOutline,
  trackBacklinks,
} from "../lib/store";
import {
  takePendingDeepLink,
  takePendingOpenFile,
  workspaceStats,
} from "../lib/api";
import {
  onDeepLinkNavigate,
  onOpenFileImport,
  onQuickCaptureHotkey,
  onMenuSwitchWorkspace,
  onPeerOpsChanged,
  onProjectionWriteFailed,
  onWorkspaceReady,
  onWorkspaceReconciled,
} from "../lib/events";
import type { DeepLinkNavigate } from "../lib/events";
import { installShortcuts, type ActionHandlers } from "../lib/shortcuts";
import { loadTransformers } from "@outl/shared/plugins/transformer-registry";
import { buildHandlers } from "../lib/action-handlers";
import { followSelection, installRegionKeys } from "../lib/keyboard-reach";
import { resetForNavigation } from "../lib/page-change";
import { BatchToolbar } from "./BatchToolbar";
import { Sidebar } from "./Sidebar";
import { OutlineView } from "./OutlineView";
import { Picker } from "./Picker";
import { QuickCapture } from "./QuickCapture";
import { openQuickCapture } from "../lib/quick-capture";
import { switchWorkspace } from "../lib/switch-workspace";
import { PluginMarketplace } from "./PluginMarketplace";
import { PluginEffectLayer } from "./PluginEffectLayer";
import { SettingsModal } from "./SettingsModal";
import { HelpOverlay } from "./HelpOverlay";
import { RemindersPanel } from "./RemindersPanel";
import { TasksPanel } from "./TasksPanel";
import { TrashPanel } from "./TrashPanel";
import { TimelinePanel } from "./TimelinePanel";
import { deliverDueReminders } from "@outl/shared/api/commands";

/**
 * How often the shell asks the backend to deliver anything that came
 * due. 30s bounds how late a fire can be; the schedule itself has
 * minute granularity, so a tighter tick would only add IPC.
 */
const REMINDER_POLL_MS = 30_000;
import { ChromeToggleBar } from "./ChromeToggleBar";
import { ErrorToast } from "./ErrorToast";
import { NoticeToast } from "./NoticeToast";

/**
 * 2-pane shell rendered once a workspace is loaded.
 *
 * ```
 * | Sidebar | OutlineView (+ inline Backlinks at bottom) |
 * ```
 *
 * The TUI renders backlinks **inline below the outline**, not as a
 * side panel — same shape here so a user moving between clients
 * sees the same structure. `Cmd/Ctrl+Shift+B` toggles the inline
 * section (`<InlineBacklinks />` reads `appState.backlinksOpen`).
 * `Cmd/Ctrl+Shift+E` toggles the sidebar.
 */
export function AppShell() {
  // The dispatcher's handler map, lifted into a signal so the
  // `<BatchToolbar />` can fire the very same batch ops the keyboard
  // does (built in `onMount`, so `null` until the shell wires up).
  const [handlers, setHandlers] = createSignal<ActionHandlers | null>(null);

  function applyView(view: PageView) {
    setAppState({ page: view.page });
    setOutline(view.outline);
    // Drop editing / selection cursors that the new outline no longer
    // contains. This path fires on peer-driven reloads (onPeerChange →
    // reloadWorkspace), which replace the outline without going through
    // OutlineView's page-change effect. A stale editingBlockId /
    // selectedBlockId would make the next edit_block / create_after hit
    // "block <id> is not in the tree" — the cursor points at a block the
    // reload re-materialized under a different id (or dropped).
    const ids = new Set(flattenAll(view.outline));
    if (appState.editingBlockId && !ids.has(appState.editingBlockId)) {
      setAppState("editingBlockId", null);
      setAppState("mode", "normal");
    }
    if (appState.selectedBlockId && !ids.has(appState.selectedBlockId)) {
      // Fall back to the first block so `o` / `Enter` still have a valid
      // anchor instead of a dangling id.
      setAppState("selectedBlockId", view.outline[0]?.id ?? null);
    }
  }

  /**
   * Show a page the user **asked** for — the chrome's half of
   * `ctx.navigateTo` (`lib/handler-context.ts`), which owns why that is
   * not the same as applying a reply.
   *
   * Which surfaces take which is the part that is not obvious, and the
   * list is **not** "everything that shows a page": `refreshActivePage`
   * (a peer reload keeps the page *and* the user's zoom), a quick
   * capture — which only refreshes today when today is already on
   * screen — and a timeline restore all keep `applyView`.
   * `<RemindersPanel />` does too, and calls `resetForNavigation` inside
   * its own `jumpTo`, because one prop cannot answer for both that and
   * its `markDone`.
   */
  function openView(view: PageView) {
    resetForNavigation();
    applyView(view);
  }

  function setError(msg: string) {
    setAppState("lastError", msg);
  }

  async function refreshStats() {
    try {
      const stats = await workspaceStats();
      setAppState("workspace", stats);
    } catch {
      // Backend not ready yet — picker stays visible; nothing to do.
    }
  }

  /** Shared by the macOS menu item and the Settings button. */
  function switchWorkspaceHere(): Promise<boolean> {
    return switchWorkspace({ setError, reload: loadToday });
  }

  async function loadToday() {
    try {
      const view = await openTodayJournal();
      openView(view);
      await refreshStats();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * Refresh the active page after a peer-driven workspace reload.
   *
   * Walks the page kind so we use the right command (`open_ref`
   * would create a fresh page if a peer deleted ours mid-flight).
   * If the active page is gone, fall back to today's journal.
   */
  async function refreshActivePage() {
    const page = appState.page;
    if (!page) {
      await loadToday();
      return;
    }
    try {
      const view =
        page.kind === "journal"
          ? await openJournalFor(page.slug)
          : await openPageBySlug(page.slug);
      applyView(view);
      // A peer reload keeps the same slug, so OutlineView's `on(slug)`
      // backlinks effect won't refire — but a peer's edit CAN change this
      // page's backlinks. Refetch them here explicitly, behind the page:
      // the rows already on screen stay up and the section carries its
      // own pending mark (`UX.md` → Never block a keystroke).
      void trackBacklinks(pageBacklinks(page.slug));
    } catch {
      await loadToday();
    }
  }

  // Coalesce peer-driven reloads and never run one mid-edit. A peer streaming
  // its op log fires many `peer-ops-changed` events; firing a full
  // `reloadWorkspace` per event pegged the CPU and queued the workspace lock
  // that `create_block` needs (the `esc+o` delay). And reloading while the user
  // edits re-materializes the tree under the cursor. So: defer while
  // `editingBlockId` is set, run at most one reload at a time, and collapse any
  // events that arrive during a reload into a single follow-up.
  let peerChangeInFlight = false;
  let peerChangePending = false;

  async function onPeerChange() {
    if (appState.editingBlockId !== null || peerChangeInFlight) {
      peerChangePending = true;
      return;
    }
    peerChangeInFlight = true;
    try {
      do {
        peerChangePending = false;
        await reloadWorkspace();
        await refreshActivePage();
        await refreshStats();
      } while (peerChangePending && appState.editingBlockId === null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      peerChangeInFlight = false;
    }
  }

  // The background reconcile pass (orphan `.md` materialised, journal or
  // namespaced `title::` repaired) already mutated the in-memory tree, so
  // no `reloadWorkspace` — a full op-log replay — is owed here. What is
  // stale is what the screen loaded before the pass: the active page's
  // properties and its lazy backlinks reply (nested pages hang off
  // `title::`). Re-read those. Mid-edit, fold it into the deferred peer
  // reload instead of resetting the textarea; the drain below re-reads
  // the page once the user leaves edit mode.
  async function onReconciled() {
    if (appState.editingBlockId !== null || peerChangeInFlight) {
      peerChangePending = true;
      return;
    }
    await refreshActivePage();
    await refreshStats();
  }

  // Drain a peer reload that was deferred because the user was editing, the
  // moment they leave edit mode. Guarded so it only fires on the edit→idle
  // transition, and only when a reload is actually pending.
  createEffect(() => {
    if (
      appState.editingBlockId === null &&
      peerChangePending &&
      !peerChangeInFlight
    ) {
      void onPeerChange();
    }
  });

  /**
   * Navigate in response to an `outl://` deep link (issue #98). The
   * backend already parsed + validated the URL through the shared
   * `outl_actions::parse_deep_link` and focused the window; here we just
   * map the resulting shape onto the same `open*` command the picker /
   * sidebar use, then render it. A failed open lands on the status line.
   */
  async function handleDeepLink(payload: DeepLinkNavigate) {
    if (payload.kind === "capture") return void openQuickCapture();
    try {
      const view =
        payload.kind === "today"
          ? await openTodayJournal()
          : payload.kind === "daily"
            ? await openJournalFor(payload.date)
            : await openPageBySlug(payload.slug);
      openView(view);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * Import a file the OS handed us ("Open With → outl") and show its
   * page. The backend refuses a second import of the same path, so
   * re-opening a file navigates to the page it already produced — the
   * user sees their page either way, which is why there is nothing to
   * say on the happy path. A refusal (wrong type, not text, too big)
   * is worded by `outl_actions::open_with` and lands on the status
   * line.
   */
  async function handleOpenFile(path: string) {
    try {
      openView(await openExternalFile(path));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  // F6 cycles sidebar / outline / chrome, Esc returns to the outline.
  onMount(() => onCleanup(installRegionKeys()));
  followSelection(() => appState.selectedBlockId);

  /**
   * Which day it is, for a window that is open for days.
   *
   * It feeds `appState.todaySlug`, which the header and the sidebar
   * calendar read — both used to answer from their own frozen copy, so
   * a window left open overnight called yesterday "Today" and ringed
   * yesterday on the calendar.
   *
   * The `wake` half is the same rule the phone uses: come back to a
   * window that sat on another day and it opens today, but never on the
   * midnight tick itself (the user may be looking at the page) and
   * never while they are in a block.
   *
   * Created in the component body, not in `onMount`: it registers an
   * `onCleanup`, and that one is `async`, so the owner is gone by the
   * time the body resumes.
   */
  const dayClock = createDayClock({
    resolve: todaySlug,
    wake: {
      currentSlug: () => appState.page?.slug ?? null,
      editing: () => appState.editingBlockId !== null,
      jumpToday: () => void loadToday(),
    },
  });
  createEffect(() => setAppState("todaySlug", dayClock.today()));

  onMount(async () => {
    // Subscribe BEFORE draining. `take_pending_open_file` is what tells
    // the backend a listener exists — after that call it emits instead
    // of buffering — so draining first opens a window where a file
    // opened right then is emitted into nothing.
    const unlistenOpenFile = await onOpenFileImport((path) => {
      void handleOpenFile(path);
    });
    onCleanup(() => unlistenOpenFile());

    // Two cold-start targets beat today's journal, and only one can
    // win: an `outl://` URL that launched the app, or a file the user
    // opened with outl. Loading the journal as well would race the
    // target and overwrite it. A deep link is checked first because it
    // is the more specific gesture — it names a page, where a file
    // import creates one — but both buffers are drained either way, so
    // an unread one cannot replay on the next plain launch.
    const pending = await takePendingDeepLink();
    const pendingFile = await takePendingOpenFile();
    // The drain below waits on this before refreshing, so a capture
    // landing first cannot reach `refreshActivePage`'s no-page branch
    // and load today over the requested target.
    // `first_content` is reported off whichever of the three cold-start
    // targets wins, not off `loadToday` alone — a launch that opened a
    // deep link or an imported file painted real content just the same,
    // and the backend's deferred boot work is waiting on this mark.
    const markFirstContent = () => bootMark("first_content");
    const initialNav: Promise<unknown> = (
      pending
        ? handleDeepLink(pending)
        : pendingFile
          ? handleOpenFile(pendingFile)
          : loadToday()
    ).then(markFirstContent, (e: unknown) => {
      // A failed open still ends the "nothing on screen" phase: the
      // shell is mounted and the error is rendered. Holding the mark
      // back would hold back sync and the orphan sweep behind a state
      // nothing is going to resolve on its own.
      markFirstContent();
      throw e;
    });

    // Land anything `outl capture` left in the inbox (RFC 0363): on
    // mount, and on every return to the foreground. That is what makes a
    // macOS Service, a Shortcut or a hotkey daemon a real capture path on
    // the desktop — none of them can boot a workspace, so they write a
    // file and this drains it.
    //
    // The refresh is skipped while the user is in a block, for the reason
    // `onPeerChange` defers: re-applying a view re-materializes the
    // outline under the cursor. The capture is already in the op log
    // either way, and the next refresh shows it.
    installForegroundRefresh({
      pull: async () => {
        await initialNav.catch(() => undefined);
        if (appState.editingBlockId === null) await refreshActivePage();
      },
      notify: setError,
    });

    const handlers = buildHandlers({ applyView, setError });
    setHandlers(handlers);
    const unbindShortcuts = await installShortcuts(handlers, setError);
    onCleanup(unbindShortcuts);

    // Content transformers (code-fence renderers). Plugins load lazily on
    // the first host request after the workspace opens, so this boot call
    // can come back empty; `workspace-ready` re-loads once they're in (and
    // catches a workspace swap clearing/adding transformers). Best-effort.
    void loadTransformers();
    const unlistenReady = await onWorkspaceReady(() => {
      void loadTransformers();
    });
    onCleanup(() => unlistenReady());

    // File → Switch Workspace… (native menu). `loadToday` runs after the
    // store is reset, so the new workspace opens on its own journal.
    const unlistenSwitch = await onMenuSwitchWorkspace(() => {
      void switchWorkspaceHere();
    });
    onCleanup(() => unlistenSwitch());

    const unlisten = await onPeerOpsChanged(() => {
      void onPeerChange();
    });
    onCleanup(() => unlisten());

    const unlistenReconciled = await onWorkspaceReconciled(() => {
      void onReconciled();
    });
    onCleanup(() => unlistenReconciled());

    const unlistenProjection = await onProjectionWriteFailed((failure) => {
      if (failure.md_ahead_of_log && appState.page?.id === failure.page_id) {
        setAppState("mdAheadOfLog", failure.md_ahead_of_log);
      } else {
        // An off-screen refusal still froze a page. It cannot use the
        // current page's sticky banner, but it must reach the user.
        setError(failure.error);
      }
    });
    onCleanup(() => unlistenProjection());

    // `outl://` deep links opened while the app is running (issue #98).
    const unlistenDeepLink = await onDeepLinkNavigate((payload) => {
      void handleDeepLink(payload);
    });
    onCleanup(() => unlistenDeepLink());

    // The OS-level quick capture chord, for when outl is in the background.
    const unlistenCapture = await onQuickCaptureHotkey(openQuickCapture);
    onCleanup(() => unlistenCapture());

    // Reminder delivery. The backend decides what is due and remembers
    // what this device already delivered, so polling twice is harmless
    // and a device that was asleep owes exactly one banner, not a
    // backlog. It short-circuits when reminders are off, so the tick
    // runs unconditionally rather than re-subscribing on a settings
    // change.
    const remindersTimer = setInterval(() => {
      void deliverDueReminders().catch(() => {
        // A missing notification daemon is not worth a status-line
        // error every 30 seconds; the Rust side already logged it.
      });
    }, REMINDER_POLL_MS);
    onCleanup(() => clearInterval(remindersTimer));

    // Last line of the shell's mount: every listener is armed and every
    // ticker is running. It emits the summary line, so a launch missing
    // it in the log is a launch that did not get here.
    bootMark("interactive");
  });

  function gridTemplate(): string {
    // `minmax(0, …)` on every track is what lets the columns
    // shrink below their content's natural width. With a bare
    // `1fr`, a row in the outline carrying a long unbreakable
    // token (`BUSER-DJANGO-KX9`, `((blk-XXXXXX))`) would push the
    // column past the viewport and the `overflow: hidden` on body
    // would clip it — outl rendered with cut-off text in narrow
    // windows. Same pitfall flex has on `min-width: auto`; the
    // grid analogue is here.
    const left = appState.sidebarOpen ? "minmax(0, 220px)" : "0";
    return `${left} minmax(0, 1fr)`;
  }

  return (
    <>
      <div
        // `minmax(0, 1fr)` instead of `1fr` so the second grid
        // column can shrink below its content's natural width.
        // Without this, a row with a long unbreakable token pushes
        // the column past the viewport and outl renders with a
        // horizontal cut-off in narrow windows. CSS grid's
        // `min-width: auto` default is the same pitfall flex hits
        // (`min-w-0` on flex children); the grid analogue is
        // baking `minmax(0, …)` into the template.
        class="grid h-full overflow-hidden"
        style={{ "grid-template-columns": gridTemplate() }}
      >
        <Show when={appState.sidebarOpen} fallback={<div />}>
          <Sidebar onToday={loadToday} onPickPage={openView} />
        </Show>

        <OutlineView />
      </div>

      <Show when={handlers()}>
        {(h) => <BatchToolbar handlers={h()} />}
      </Show>
      <ChromeToggleBar handlers={handlers()} />
      <Picker onPicked={openView} handlers={handlers()} />
      <QuickCapture applyView={applyView} />
      <PluginMarketplace />
      <SettingsModal onSwitchWorkspace={switchWorkspaceHere} />
      <HelpOverlay />
      <RemindersPanel applyView={applyView} setError={setError} />
      <TasksPanel applyView={openView} setError={setError} />
      <TrashPanel applyView={openView} setError={setError} />
      <TimelinePanel setError={setError} applyView={applyView} />
      <PluginEffectLayer />
      {/* Shell level, not above the outline: it is a statement about the
       *  app, so it must not re-run its dismissed check on every page
       *  navigation, and it does not belong in the stack of banners that
       *  report *this page's* health (RFC 0377 section 6). */}
      <CrashReportsNotice />
      {/* Mounted last so the notification toast sits above every chrome
       *  element (ChromeToggleBar, overlays) in the stacking order. */}
      <NoticeToast />
      <ErrorToast />
    </>
  );
}
