import { createSignal, For, type JSX } from "solid-js";
import {
  orderedMiddleFromStore,
  PINNED_FIRST,
  PINNED_LAST,
  recordToStore,
  type ToolbarAction,
  TOOLBAR_META,
} from "@outl/shared/toolbar";

/**
 * Web-rendered keyboard accessory bar — the cross-platform successor to
 * the iOS-only native `OutlToolbarView`. Renders the same Bear-style pill
 * (pinned `+` / MFU-scrolled middle / pinned "hide keyboard") straight in
 * the webview, so iOS and Android share one implementation.
 *
 * It ships on **Android** first (there is no `inputAccessoryView` to
 * swizzle there); iOS keeps its native bar until this is device-validated,
 * at which point the Swift bar retires and this becomes the only one.
 *
 * Keyboard docking is owned by the parent `<KeyboardAccessory />`; this is
 * just the pill. The catalog + ordering live in `@outl/shared/toolbar`, so
 * this file is pure chrome (icons + capsule + the focus-preserving tap
 * glue).
 */
export function KeyboardToolbar(props: {
  onAction: (action: ToolbarAction) => void;
}): JSX.Element {
  // Re-read the MFU order after every tap so the middle row reflows just
  // like the native bar's `rebuildButtons()`. A bumped signal is cheaper
  // than diffing and the row is tiny.
  const [order, setOrder] = createSignal<ToolbarAction[]>(orderedMiddleFromStore());

  function fire(action: ToolbarAction) {
    recordToStore(action);
    setOrder(orderedMiddleFromStore());
    props.onAction(action);
  }

  return (
    <div class="outl-kb-capsule pointer-events-auto flex max-w-full items-center gap-1 rounded-full bg-(--color-outl-bg-elev) px-2 py-1 shadow-[var(--shadow-capsule)]">
      <ToolbarButton action={PINNED_FIRST} onFire={fire} />
      <div class="flex min-w-0 items-center gap-1 overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        <For each={order()}>
          {(action) => <ToolbarButton action={action} onFire={fire} />}
        </For>
      </div>
      <ToolbarButton action={PINNED_LAST} onFire={fire} />
    </div>
  );
}

function ToolbarButton(props: {
  action: ToolbarAction;
  onFire: (action: ToolbarAction) => void;
}): JSX.Element {
  const meta = TOOLBAR_META[props.action];
  const destructive =
    meta.style.kind === "symbol" && meta.style.destructive === true;
  return (
    <button
      type="button"
      aria-label={meta.label}
      class="flex h-9 w-10 shrink-0 items-center justify-center rounded-full text-(--color-outl-fg) active:bg-(--color-outl-border)/30"
      classList={{
        "text-(--color-outl-destructive)":
          destructive,
      }}
      // preventDefault on pointerdown keeps focus in the textarea — a
      // plain button tap would blur it and dismiss the soft keyboard,
      // which is exactly what a keyboard accessory must never do. The
      // click still fires (default-prevented focus shift ≠ prevented
      // click), so the action runs with the keyboard still up.
      onPointerDown={(e) => e.preventDefault()}
      onClick={() => props.onFire(props.action)}
    >
      <ToolbarGlyph action={props.action} />
    </button>
  );
}

/** Renders a button's face: a literal glyph (`[[`, `((`, `#`) or an SVG
 *  icon. The `symbol` name is the SF Symbol id the iOS bar uses, reused
 *  here as a stable key; the web draws its own equivalent. */
function ToolbarGlyph(props: { action: ToolbarAction }): JSX.Element {
  const style = TOOLBAR_META[props.action].style;
  if (style.kind === "text") {
    return <span class="font-mono text-[16px] font-medium">{style.glyph}</span>;
  }
  switch (style.symbol) {
    case "plus":
      return <Svg>{<path d="M12 5v14M5 12h14" />}</Svg>;
    case "increase.indent":
      return (
        <Svg>
          <path d="M3 6h18" />
          <path d="M3 18h18" />
          <path d="M11 12h10" />
          <path d="M4 9l3 3-3 3" />
        </Svg>
      );
    case "decrease.indent":
      return (
        <Svg>
          <path d="M3 6h18" />
          <path d="M3 18h18" />
          <path d="M11 12h10" />
          <path d="M7 9l-3 3 3 3" />
        </Svg>
      );
    case "arrow.up":
      return <Svg>{<path d="M12 19V5M5 12l7-7 7 7" />}</Svg>;
    case "arrow.down":
      return <Svg>{<path d="M12 5v14M5 12l7 7 7-7" />}</Svg>;
    case "arrow.uturn.left":
      return <Svg>{<path d="M9 7L4 12l5 5M4 12h11a5 5 0 0 0 0-10h-1" />}</Svg>;
    case "arrow.uturn.right":
      return <Svg>{<path d="M15 7l5 5-5 5M20 12H9a5 5 0 0 1 0-10h1" />}</Svg>;
    case "bold":
      return <span class="text-[16px] font-bold">B</span>;
    case "italic":
      return <span class="font-serif text-[16px] italic">I</span>;
    case "code":
      return (
        <Svg>
          <path d="M8 9l-3 3 3 3" />
          <path d="M16 9l3 3-3 3" />
        </Svg>
      );
    case "checkmark.circle":
      return (
        <Svg>
          <circle cx="12" cy="12" r="9" />
          <path d="M8.5 12.5l2.5 2.5 4.5-5" />
        </Svg>
      );
    case "trash":
      return (
        <Svg>
          <path d="M4 7h16" />
          <path d="M9 7V5h6v2" />
          <path d="M6 7l1 13h10l1-13" />
        </Svg>
      );
    case "keyboard.down":
      return (
        <Svg>
          <rect x="3" y="4" width="18" height="10" rx="2" />
          <path d="M8 18l4 3 4-3" />
        </Svg>
      );
    default:
      return <span class="text-[16px]">?</span>;
  }
}

function Svg(props: { children: JSX.Element }): JSX.Element {
  return (
    <svg
      width="21"
      height="21"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.children}
    </svg>
  );
}
