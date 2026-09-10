import { For, Show } from "solid-js";
import type { PageView, PluginToolbarButton } from "@outl/shared";

import { JournalHeader, PageHeader } from "./JournalHeader";
import { SyncDot } from "./SyncDot";
import { haptic } from "../lib/haptics";
import type { LongPressHandlers } from "../lib/long-press";

/**
 * The journal's top chrome: back capsule, title / day stepper, and the
 * right-hand action capsule (calendar, page switcher, fold, plugin
 * toolbar, reminders, plugins, devices, refresh, settings).
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
export function JournalChrome(props: {
  /** The page currently on screen, or `null` while loading. */
  view: PageView | null;
  /** `false` puts the sync dot in its offline state. */
  online: boolean;
  /** At least one peer is reachable. */
  peersUp: boolean;
  /** A sync pass is in flight. */
  syncing: boolean;
  /** A pull-to-refresh is in flight (spins the refresh icon). */
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
  onRefresh: () => void;
  onRunToolbarButton: (btn: PluginToolbarButton) => void;
  onOpenCalendar: () => void;
  onOpenSwitcher: () => void;
  onOpenReminders: () => void;
  onOpenPlugins: () => void;
  onOpenDevices: () => void;
  onOpenSettings: () => void;
}) {
  return (
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
                title={props.view?.page.title ?? ""}
                kind={props.view?.page.kind ?? null}
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
          <button
            type="button"
            aria-label="Calendar"
            onClick={() => {
              haptic("light");
              props.onOpenCalendar();
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
              props.onOpenSwitcher();
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
            aria-label="Reminders"
            onClick={() => {
              haptic("light");
              props.onOpenReminders();
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
          {/* The sync dot IS the devices/pairing affordance: it shows the
              mesh status AND opens the pairing sheet on tap — no separate
              (ugly) devices glyph. Mirrors the desktop's clickable dot. */}
          <button
            type="button"
            aria-label="Devices and sync — tap to pair"
            onClick={() => {
              haptic("light");
              props.onOpenDevices();
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
                props.syncing
                  ? "syncing"
                  : props.online && props.peersUp
                    ? "synced"
                    : "offline"
              }
            />
          </button>
          <button
            type="button"
            aria-label="Sync now"
            onClick={props.onRefresh}
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
                transform: props.refreshing ? "rotate(360deg)" : "rotate(0deg)",
                transition: "transform 800ms ease-in-out",
              }}
              aria-hidden="true"
            >
              <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
              <path d="M21 3v5h-5" />
            </svg>
          </button>
          {/* Settings — last in the capsule, where iOS puts it. Sits
              after the sync/refresh pair on purpose: those two are read
              constantly, preferences are not, and the capsule scrolls
              from the left. */}
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
