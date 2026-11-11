/**
 * The slice of the Excalidraw file format and the Excalidraw+ API this plugin
 * reads.
 *
 * Deliberately partial and deliberately loose: every field is optional because
 * the scene on the other side was written by a newer Excalidraw than the one
 * these types were pinned against, and an element it does not understand must
 * render as "skipped", never as a crash. Defaults live next to the reader
 * (`src/render/*`), not here.
 */

/** Roundness descriptor on a shape. `type` picks how the radius is derived. */
export interface Roundness {
  /** 1 = legacy proportional, 2 = adaptive radius, 3 = proportional radius. */
  type: number;
  value?: number;
}

/**
 * One element of a scene. Excalidraw keeps every element kind in one array with
 * a `type` discriminator, so this is the union flattened into optional fields —
 * the shape the JSON actually has.
 */
export interface ExcalidrawElement {
  id?: string;
  /** `rectangle`, `ellipse`, `diamond`, `arrow`, `line`, `freedraw`, `text`, `image`, `frame`, … */
  type?: string;
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  /** Rotation in **radians**, clockwise, about the element's centre. */
  angle?: number;
  strokeColor?: string;
  backgroundColor?: string;
  /** `hachure` | `cross-hatch` | `solid` | `zigzag`. */
  fillStyle?: string;
  strokeWidth?: number;
  /** `solid` | `dashed` | `dotted`. */
  strokeStyle?: string;
  /** 0 (architect) · 1 (artist) · 2 (cartoonist). Drives the jitter amount. */
  roughness?: number;
  /** 0–100, not 0–1. */
  opacity?: number;
  /** Seeds the per-element PRNG, so a scene redraws identically every time. */
  seed?: number;
  roundness?: Roundness | null;
  isDeleted?: boolean;

  /** Linear + freedraw: vertices **relative** to `x`/`y`. */
  points?: number[][];
  startArrowhead?: string | null;
  endArrowhead?: string | null;

  /** Text: already wrapped by Excalidraw, so `\n` is a real line break. */
  text?: string;
  fontSize?: number;
  /** 1/5/6 hand-drawn · 2 sans · 3 mono. */
  fontFamily?: number;
  /** `left` | `center` | `right`. */
  textAlign?: string;
  /** `top` | `middle` | `bottom`. */
  verticalAlign?: string;
  /** Multiplier on `fontSize`; Excalidraw's default is 1.25. */
  lineHeight?: number;
  containerId?: string | null;

  /** Image: key into the scene's `files` map. */
  fileId?: string | null;

  /** Frame: the label drawn above the outline. */
  name?: string | null;
}

/** An embedded file. The API hands back `dataURL`, so images need no network. */
export interface BinaryFile {
  id?: string;
  mimeType?: string;
  dataURL?: string;
}

/** `GET /api/v1/scenes/{id}/content`, and the shape of a `.excalidraw` file. */
export interface SceneContent {
  elements?: ExcalidrawElement[];
  files?: Record<string, BinaryFile> | null;
  appState?: { viewBackgroundColor?: string } | null;
}

/** The subset of `GET /api/v1/scenes` metadata this plugin shows. */
export interface SceneMetadata {
  id?: string;
  name?: string;
  isUntitled?: boolean;
  updated?: string;
  sceneVersion?: string;
  totalElements?: number;
  isDeleted?: boolean;
}

/** One row of `GET /api/v1/scenes`. */
export interface SceneListRow {
  metadata?: SceneMetadata;
}

/** `GET /api/v1/scenes` envelope. */
export interface SceneListResponse {
  limit?: number;
  offset?: number;
  hasNextPage?: boolean;
  data?: SceneListRow[];
}

/**
 * What `excalidraw-sync` leaves in `ctx.storage` for the transformer to draw.
 *
 * The cache holds the scene's **elements**, not a rendered SVG, because theme
 * and height are per-fence: two fences can point at one scene and ask for
 * different renderings, and re-rendering is cheap next to a round trip.
 */
export interface CachedScene {
  id: string;
  name: string;
  sceneVersion: string;
  /** `appState.viewBackgroundColor`, or `""` when the scene set none. */
  background: string;
  elements: ExcalidrawElement[];
  /** `fileId` → `dataURL`. Empty when the scene's files were over the budget. */
  files: Record<string, string>;
  /** True when files were dropped to keep `storage.json` small. */
  filesDropped: boolean;
  /** ISO timestamp of the sync that wrote this entry. */
  at: string;
}

/** Theme a fence renders in. `auto` follows the client's colour scheme. */
export type Theme = "auto" | "light" | "dark";

/** User config from `config.schema.json`. Every field has a default. */
export interface PluginConfig {
  theme?: Theme;
  maxHeight?: number;
  indexPage?: string;
}
