import { Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";

import {
  openJournalFor,
  openPageBySlug,
  openTodayJournal,
  pageBacklinks,
  reloadWorkspace,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";
import { flattenAll } from "@outl/shared/outline";

import { appState, setAppState, setOutline } from "../lib/store";
import { takePendingDeepLink, workspaceStats } from "../lib/api";
import {
  onDeepLinkNavigate,
  onPeerOpsChanged,
  onWorkspaceReady,
} from "../lib/events";
import type { DeepLinkNavigate } from "../lib/events";
import { installShortcuts, type ActionHandlers } from "../lib/shortcuts";
import { loadTransformers } from "@outl/shared/plugins/transformer-registry";
import { buildHandlers } from "../lib/action-handlers";
import { BatchToolbar } from "./BatchToolbar";
import { Sidebar } from "./Sidebar";
import { OutlineView } from "./OutlineView";
import { Picker } from "./Picker";
import { PluginMarketplace } from "./PluginMarketplace";
import { PluginEffectLayer } from "./PluginEffectLayer";
import { SettingsModal } from "./SettingsModal";
import { HelpOverlay } from "./HelpOverlay";
import { RemindersPanel } from "./RemindersPanel";
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

  async function loadToday() {
    try {
      const view = await openTodayJournal();
      applyView(view);
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
      // page's backlinks. Refetch them here explicitly.
      pageBacklinks(page.slug)
        .then((r) =>
          setAppState({
            backlinks: r.backlinks,
            backlinksOrder: r.backlinks_order,
          }),
        )
        .catch(() => {});
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
    try {
      const view =
        payload.kind === "today"
          ? await openTodayJournal()
          : payload.kind === "daily"
            ? await openJournalFor(payload.date)
            : await openPageBySlug(payload.slug);
      applyView(view);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  onMount(async () => {
    // A cold-start deep link wins over today's journal: if an `outl://`
    // URL launched the app, navigate there instead of loading the
    // journal (which would otherwise race and overwrite the target).
    const pending = await takePendingDeepLink();
    if (pending) {
      void handleDeepLink(pending);
    } else {
      void loadToday();
    }

    const handlers = buildHandlers({ applyView, setError });
    setHandlers(handlers);
    const unbindShortcuts = await installShortcuts(handlers);
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

    const unlisten = await onPeerOpsChanged(() => {
      void onPeerChange();
    });
    onCleanup(() => unlisten());

    // `outl://` deep links opened while the app is running (issue #98).
    const unlistenDeepLink = await onDeepLinkNavigate((payload) => {
      void handleDeepLink(payload);
    });
    onCleanup(() => unlistenDeepLink());

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
          <Sidebar onToday={loadToday} onPickPage={applyView} />
        </Show>

        <OutlineView />
      </div>

      <Show when={handlers()}>
        {(h) => <BatchToolbar handlers={h()} />}
      </Show>
      <ChromeToggleBar />
      <Picker onPicked={applyView} />
      <PluginMarketplace />
      <SettingsModal />
      <HelpOverlay />
      <RemindersPanel applyView={applyView} setError={setError} />
      <TimelinePanel setError={setError} />
      <PluginEffectLayer />
      {/* Mounted last so the notification toast sits above every chrome
       *  element (ChromeToggleBar, overlays) in the stacking order. */}
      <ErrorToast />
    </>
  );
}
