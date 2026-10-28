import { For, Show } from "solid-js";
import type { JSX } from "solid-js";
import {
  BellIcon,
  CalendarIcon,
  CheckSquareIcon,
  PlusIcon,
  SearchIcon,
} from "@outl/shared/icons";

export type BottomTab = "journal" | "search" | "tasks" | "reminders";

/**
 * The app's four places plus capture, always one thumb away.
 *
 * Presentational: which tab is lit and what a tap does both arrive as
 * props. The bar floats over the outline rather than taking a row of
 * the layout, so the page keeps its full height and only the bottom
 * padding of `<main>` makes room for it.
 *
 * Hidden while a block is being edited: the keyboard owns the bottom
 * of the screen then, and the keyboard toolbar is what belongs above
 * it. Hidden during a range selection too, where the selection
 * toolbar docks in the same place.
 */
export function BottomBar(props: {
  active: BottomTab | null;
  hidden: boolean;
  onTab: (tab: BottomTab) => void;
  onCapture: () => void;
}) {
  const left: Tab[] = [
    { id: "journal", label: "Journal", icon: () => <CalendarIcon size={22} /> },
    { id: "search", label: "Search", icon: () => <SearchIcon size={22} /> },
  ];
  const right: Tab[] = [
    { id: "tasks", label: "Tasks", icon: () => <CheckSquareIcon size={22} /> },
    { id: "reminders", label: "Reminders", icon: () => <BellIcon size={22} /> },
  ];
  const item = (t: Tab) => (
    <button
      type="button"
      aria-label={t.label}
      aria-current={props.active === t.id ? "page" : undefined}
      onClick={() => props.onTab(t.id)}
      class="outl-press flex h-12 flex-1 flex-col items-center justify-center gap-0.5"
      classList={{
        "text-(--color-outl-accent)": props.active === t.id,
        "text-(--color-outl-fg-dim)": props.active !== t.id,
      }}
    >
      {t.icon()}
      <span class="text-[10px] font-medium leading-none">{t.label}</span>
    </button>
  );
  return (
    <Show when={!props.hidden}>
      <nav
        aria-label="Main"
        class="outl-fade-in outl-focus-hide fixed inset-x-3 z-30 flex items-center rounded-[22px] border border-(--color-outl-border)/60 bg-(--color-outl-bg-elev)/90 px-1 shadow-[var(--shadow-capsule)] backdrop-blur-xl"
        style="bottom: max(env(safe-area-inset-bottom), 12px);"
      >
        <For each={left}>{item}</For>
        <div class="flex flex-1 justify-center">
          <button
            type="button"
            aria-label="Add to today's journal"
            onClick={props.onCapture}
            class="outl-press flex h-11 w-11 items-center justify-center rounded-full bg-(--color-outl-accent) text-(--color-outl-accent-fg) shadow-[var(--shadow-capsule)]"
          >
            <PlusIcon size={24} />
          </button>
        </div>
        <For each={right}>{item}</For>
      </nav>
    </Show>
  );
}

interface Tab {
  id: BottomTab;
  label: string;
  icon: () => JSX.Element;
}
