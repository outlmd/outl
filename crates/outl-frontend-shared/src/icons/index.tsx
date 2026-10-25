/**
 * Line icons for the GUI chrome, drawn in `currentColor` so they take
 * whatever palette token the surrounding text uses. They replace the
 * 📅/📄/⏱ emoji the chrome used as defaults: an emoji brings its own
 * colours, renders differently per OS, and reads as content rather
 * than as interface.
 *
 * A page's own `icon::` property is still the user's choice and wins
 * over these; see {@link PageGlyph}.
 *
 * Paths are hand-drawn on a 24px grid with a 1.75 stroke, in the
 * spirit of Lucide (ISC), not copied from it.
 */
import type { JSX } from "solid-js";
import { Show, splitProps } from "solid-js";
import type { PageKind } from "../api/types";

export interface IconProps {
  /** Rendered size in px. Defaults to 14, the chrome's text size. */
  size?: number;
  class?: string;
  /** Accessible name. Omit for decorative icons next to a label. */
  title?: string;
}

function Svg(props: IconProps & { children: JSX.Element }) {
  return (
    <svg
      width={props.size ?? 14}
      height={props.size ?? 14}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.75"
      stroke-linecap="round"
      stroke-linejoin="round"
      class={`inline-block shrink-0 align-[-0.125em] ${props.class ?? ""}`}
      role={props.title ? "img" : undefined}
      aria-hidden={props.title ? undefined : "true"}
      aria-label={props.title}
    >
      {props.children}
    </svg>
  );
}

export const CalendarIcon = (p: IconProps) => (
  <Svg {...p}>
    <rect x="3.5" y="5" width="17" height="15.5" rx="2.5" />
    <path d="M3.5 10h17M8 3v4M16 3v4" />
  </Svg>
);

export const PageIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M6.5 3h7l5 5v11.5a1.5 1.5 0 0 1-1.5 1.5h-10A1.5 1.5 0 0 1 5.5 19.5v-15A1.5 1.5 0 0 1 6.5 3z" />
    <path d="M13.5 3v5h5M9 13h6M9 17h4" />
  </Svg>
);

export const HistoryIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M3.5 12a8.5 8.5 0 1 0 2.5-6" />
    <path d="M3.5 4v4h4M12 7.5V12l3 2" />
  </Svg>
);

export const PinIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M9 3.5h6M10 3.5v5l-3 4h10l-3-4v-5M12 12.5V21" />
  </Svg>
);

export const ClockIcon = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="12" cy="12" r="8.5" />
    <path d="M12 7.5V12l3 2" />
  </Svg>
);

export const BellIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
    <path d="M13.73 21a2 2 0 0 1-3.46 0" />
  </Svg>
);

export const SearchIcon = (p: IconProps) => (
  <Svg {...p}>
    <circle cx="11" cy="11" r="6.5" />
    <path d="m20 20-4.4-4.4" />
  </Svg>
);

export const CheckSquareIcon = (p: IconProps) => (
  <Svg {...p}>
    <rect x="3.5" y="3.5" width="17" height="17" rx="3" />
    <path d="m8 12 3 3 5-6" />
  </Svg>
);

export const PlusIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M12 5v14M5 12h14" />
  </Svg>
);

export const ListIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01" />
  </Svg>
);

export const HashIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M5 9h15M4 15h15M10 3 8 21M16 3l-2 18" />
  </Svg>
);

export const IndentIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M10 6h10M10 12h10M10 18h10M3.5 8.5 6.5 12l-3 3.5" />
  </Svg>
);

export const OutdentIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M10 6h10M10 12h10M10 18h10M6.5 8.5 3.5 12l3 3.5" />
  </Svg>
);

export const LinkIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1" />
    <path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1" />
  </Svg>
);

export const PaperclipIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="M20 11.5 12 19.5a5 5 0 0 1-7-7l8-8a3.5 3.5 0 0 1 5 5l-8 8a2 2 0 0 1-3-3l7.5-7.5" />
  </Svg>
);

export const ImageIcon = (p: IconProps) => (
  <Svg {...p}>
    <rect x="3.5" y="4.5" width="17" height="15" rx="2.5" />
    <path d="M3.5 16l4.5-4.5 4 4 3-3 5 5" />
    <circle cx="9" cy="9.5" r="1.4" />
  </Svg>
);

export const ChevronDownIcon = (p: IconProps) => (
  <Svg {...p}>
    <path d="m6 9 6 6 6-6" />
  </Svg>
);

/**
 * The icon a page shows in chrome: its own `icon::` when set, else a
 * line icon for its kind. Every surface that used to spell the
 * `icon || (journal ? "📅" : "📄")` fallback inline goes through here,
 * so the default lives in one place.
 */
export function PageGlyph(props: IconProps & { icon?: string | null; kind: PageKind | undefined }) {
  const [own, rest] = splitProps(props, ["icon", "kind"]);
  return (
    <Show when={own.icon} fallback={own.kind === "journal" ? <CalendarIcon {...rest} /> : <PageIcon {...rest} />}>
      <span aria-hidden="true" class={rest.class}>{own.icon}</span>
    </Show>
  );
}
