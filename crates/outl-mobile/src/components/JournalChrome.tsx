import { For, Show, onCleanup } from "solid-js";
import type { PageView, PluginToolbarButton } from "@outl/shared";

import { JournalHeader, PageHeader } from "./JournalHeader";
import { SyncDot } from "./SyncDot";
import { focusMode } from "@outl/shared/polish";
import { journalHeading } from "@outl/shared/journal";
import { haptic } from "../lib/haptics";
import { createLongPress, type LongPressHandlers } from "../lib/long-press";

/**
 * The journal's top chrome: back capsule, title / day stepper, and the
 * right-hand action capsule (fold, plugin toolbar, plugins,
 * sync dot, settings). The sync dot doubles as the refresh: tap syncs
 * now, press-and-hold opens devices. The calendar and the page switcher moved
 * to the week strip and the bottom bar, the places a thumb already is.
 *
 * Presentational. It owns no state — every signal it reads and every
 * action it fires arrives as a prop, which is what let it come out of
 * `Journal.tsx` without touching behaviour. The split follows the same
 * line the frontend policy draws between chrome and content
 * (`crates/outl-frontend-shared/CLAUDE.md`): this is chrome, and it is
 * mobile-specific chrome, so it stays in this crate rather than moving
 * to `@outl/shared`.
 *
 * Props are accessed lazily (`props.x`, never destructured) because
 * Solid's reactivity rides the getter — destructuring here would freeze
 * the header at its first render, which is the one bug this kind of
 * extraction reliably introduces.
 */
const focus = focusMode();

export function JournalChrome(props: {
  /** The page currently on screen, or `null` while loading. */
  view: PageView | null;
  /** `false` puts the sync dot in its offline state. */
  online: boolean;
  /** At least one peer is reachable. */
  peersUp: boolean;
  /** A sync pass is in flight. */
  syncing: boolean;
  /** A manual refresh is in flight (spins the sync dot). */
  refreshing: boolean;
  /** Today's ISO slug, or `null` before the first resolve. */
  todaySlug: string | null;
  /** Plugin-contributed toolbar buttons. */
  toolbarButtons: PluginToolbarButton[];
  /** Press-and-hold on the title opens page properties. */
  titleLongPress: LongPressHandlers;
  onJumpToday: () => void;
  onPrevDay: () => void;
  onNextDay: () => void;
  onFoldAll: () => void;
  onUnfoldAll: () => void;
  /** Sync now. Fired by a tap on the sync dot; never while one runs. */
  onRefresh: () => void;
  onRunToolbarButton: (btn: PluginToolbarButton) => void;
  onOpenPlugins: () => void;
  onOpenDevices: () => void;
  onOpenSettings: () => void;
}) {
  // Either flag means a pass is running: `syncing` covers the automatic
  // open/foreground pulls, `refreshing` the manual ones.
  const syncBusy = () => props.syncing || props.refreshing;
  /**
   * What the title says while the first page is still opening.
   *
   * A cold start always opens today's journal, and the date needs no
   * workspace (`today_slug` is pure date math), so the header can say
   * "Journal / Today" from the first frame. It used to render the `null`
   * fallback — the label "Page" over an empty title — which is not a
   * placeholder but a wrong answer, held for as long as the open took.
   *
   * Deliberately `PageHeader` and not the day stepper: its chevrons
   * cannot step a day that has not loaded, and a dead button is worse
   * than no button (root `CLAUDE.md` invariant 12).
   */
  const bootTitle = () => {
    const today = props.todaySlug;
    return today === null ? "" : (journalHeading(today, today)?.title ?? "");
  };
  const devicesLongPress = createLongPress({
    onLongPress: () => {
      haptic("medium");
      props.onOpenDevices();
    },
  });
  onCleanup(() => devicesLongPress.cancel());

  return (
    <header
      class="outl-focus-hide z-30 shrink-0 bg-(--color-outl-bg)/80 px-3 pt-2 pb-3 backdrop-blur-xl"
      style="padding-top: max(env(safe-area-inset-top), 12px);"
    >
      <div class="grid grid-cols-[auto_auto_1fr] items-center gap-2">
        {/* Left capsule — visible only when the user has navigated
            away from today's journal. We always reserve a placeholder
            of the same width so the title doesn't jump horizontally
            when the back button appears / disappears. */}
        <Show
          when={props.view && props.view!.page.kind !== "journal"}
          fallback={<span aria-hidden="true" class="block h-9 w-9" />}
        >
          <div class="inline-flex rounded-full bg-(--color-outl-bg-elev)/85 shadow-[var(--shadow-capsule)] backdrop-blur-xl">
            <button
              type="button"
              aria-label="Back to today's journal"
              onClick={props.onJumpToday}
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
          onPointerDown={props.titleLongPress.onPointerDown}
          onPointerMove={props.titleLongPress.onPointerMove}
          onPointerUp={props.titleLongPress.onPointerUp}
          onPointerCancel={props.titleLongPress.onPointerUp}
          onClick={(e) => {
            // Swallow the click the completed hold produces, or the
            // journal header would also step a day.
            if (props.titleLongPress.consumedClick()) {
              e.preventDefault();
              e.stopPropagation();
            }
          }}
        >
          <Show
            when={props.view?.page.kind === "journal"}
            fallback={
              <PageHeader
                title={props.view?.page.title ?? bootTitle()}
                kind={
                  props.view?.page.kind ??
                  (props.todaySlug === null ? null : "journal")
                }
              />
            }
          >
            <JournalHeader
              slug={props.view?.page.slug ?? ""}
              todaySlug={props.todaySlug}
              onPrev={props.onPrevDay}
              onNext={props.onNextDay}
              onToday={props.onJumpToday}
            />
          </Show>
        </div>

        {/* Right capsule — grouped page actions. SyncDot lives inline
            between pages-search and refresh so the user reads it as
            "status of the data this capsule controls". */}
        <div class="ios-scroll inline-flex max-w-full items-center justify-self-end overflow-x-auto rounded-full bg-(--color-outl-bg-elev)/85 shadow-[var(--shadow-capsule)] backdrop-blur-xl">
          {/* Fold all / unfold all (RFC 0254 phase 4b, mirrors the
              desktop's `z M` / `z R`) — walks the whole page, not just
              the zoomed subtree, same as the desktop's `zM`/`zR`. */}
          <button
            type="button"
            aria-label="Fold all"
            onClick={props.onFoldAll}
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
            onClick={props.onUnfoldAll}
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
          <For each={props.toolbarButtons}>
            {(btn) => (
              <button
                type="button"
                aria-label={btn.title ?? `Plugin: ${btn.command_id}`}
                title={btn.title ?? btn.command_id}
                onClick={() => props.onRunToolbarButton(btn)}
                class="flex h-9 w-9 items-center justify-center rounded-full text-[17px] leading-none text-(--color-outl-accent) active:bg-(--color-outl-border)/40"
              >
                {btn.icon}
              </button>
            )}
          </For>
          <button
            type="button"
            aria-label="Plugin commands"
            onClick={() => {
              haptic("light");
              props.onOpenPlugins();
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
          {/* The sync dot IS the refresh: tap syncs now (pull-to-refresh
              still works on the page), press-and-hold opens the
              devices/pairing sheet. One affordance for "is it synced"
              and "sync it" instead of a dot plus a refresh glyph.
              Mirrors the desktop's clickable dot. */}
          <button
            type="button"
            aria-label={
              syncBusy()
                ? "Syncing"
                : "Sync now. Press and hold for devices"
            }
            aria-busy={syncBusy()}
            title="Sync now"
            onPointerDown={devicesLongPress.onPointerDown}
            onPointerMove={devicesLongPress.onPointerMove}
            onPointerUp={devicesLongPress.onPointerUp}
            onPointerCancel={devicesLongPress.cancel}
            onPointerLeave={devicesLongPress.cancel}
            onContextMenu={(e) => e.preventDefault()}
            onClick={() => {
              if (devicesLongPress.consumedClick()) return;
              // A pass is already running: ignore the tap instead of
              // stacking another dial behind it.
              if (syncBusy()) return;
              props.onRefresh();
            }}
            class="flex h-9 w-9 items-center justify-center rounded-full active:bg-(--color-outl-border)/40"
          >
            <SyncDot
              status={
                // PRIMARY signal is iroh peer health, not navigator.onLine.
                // A sync in flight wins (spinner); else a reachable
                // peer → synced (green); else offline/orange — either the
                // device has no radio, or peers exist but none answered
                // (or none are paired, so there's nothing to sync with).
                syncBusy()
                  ? "syncing"
                  : props.online && props.peersUp
                    ? "synced"
                    : "offline"
              }
            />
          </button>
          {/* Focus mode: fades the chrome and dims every row but the
              one being written. Same glyph and slot as desktop. */}
          <button
            type="button"
            aria-label={focus.on() ? "Leave focus mode" : "Focus mode"}
            aria-pressed={focus.on()}
            onClick={() => {
              haptic("light");
              focus.toggle();
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
              <path d="M4 9V5h4M20 9V5h-4M4 15v4h4M20 15v4h-4" />
              <circle cx="12" cy="12" r="2" />
            </svg>
          </button>
          {/* Settings — last in the capsule, where iOS puts it. Sits
              after the sync dot on purpose: that one is read constantly,
              preferences are not, and the capsule scrolls from the left. */}
          <button
            type="button"
            aria-label="Settings"
            onClick={() => {
              haptic("light");
              props.onOpenSettings();
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
              <circle cx="12" cy="12" r="3" />
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1.08-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
            </svg>
          </button>
        </div>
      </div>
    </header>
  );
}
