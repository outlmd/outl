/**
 * Pele adapter.
 *
 * The import is a literal `import("pele/lazy")` so Vite puts the
 * library in its own chunk (https://vite.dev/guide/features.html#dynamic-import,
 * https://vite.dev/guide/build.html#chunking-strategy). Callers never
 * import `pele` statically.
 *
 * Mount options follow https://pele.run/api and https://pele.run/security:
 * images off (no remote fetch), link schemes limited to http / https /
 * mailto, no icon resolver. `sanitize` inserts the SVG through the
 * CSSOM so a style-src nonce still paints strokes. Zoom stays at
 * Pele's default (`"auto"`); its controls are restyled the same way
 * after each paint.
 */

import type { LinkInfo, PeleError } from "pele/lazy";

import { adoptDiagramSvg, liftStyleAttributes, watchStyleAttributes } from "./sanitize";

export interface DiagramLink {
  id: string;
  href: string;
  internal: boolean;
}

export interface DiagramFailure {
  message: string;
  line: number;
  column: number;
  code: string;
}

export interface DiagramHooks {
  onLinks?: (links: DiagramLink[]) => void;
  onError?: (failure: DiagramFailure) => void;
}

export interface DiagramMount {
  update(source: string, hooks: DiagramHooks): Promise<void>;
  destroy(): void;
}

export interface DiagramRenderer {
  supports(source: string): Promise<boolean>;
  mount(element: HTMLElement, source: string, hooks: DiagramHooks): Promise<DiagramMount>;
}

type PeleLazy = typeof import("pele/lazy");

let loading: Promise<PeleLazy> | undefined;
const epochByElement = new WeakMap<HTMLElement, number>();

function loadPele(): Promise<PeleLazy> {
  loading ??= import("pele/lazy");
  return loading;
}

function bumpEpoch(element: HTMLElement): number {
  const next = (epochByElement.get(element) ?? 0) + 1;
  epochByElement.set(element, next);
  return next;
}

/** Shared shape for mount and update. Each call gets its own hooks. */
function mountOptions(hooks: DiagramHooks) {
  return {
    images: false as const,
    linkSchemes: ["http", "https", "mailto"],
    sanitize: adoptDiagramSvg,
    onRender: (result: { links?: LinkInfo[] }) => {
      hooks.onLinks?.(
        (result.links ?? []).map((link) => ({
          id: link.id,
          href: link.href,
          internal: link.internal,
        })),
      );
    },
    onError: (error: unknown) => {
      hooks.onError?.(toFailure(error));
    },
  };
}

function toFailure(error: unknown): DiagramFailure {
  if (isPeleError(error)) {
    return {
      message: error.message,
      line: error.line,
      column: error.column,
      code: error.code,
    };
  }
  if (error instanceof Error) {
    return { message: error.message, line: 0, column: 0, code: "unknown" };
  }
  return { message: String(error), line: 0, column: 0, code: "unknown" };
}

function isPeleError(error: unknown): error is PeleError {
  if (!error || typeof error !== "object") return false;
  const candidate = error as Partial<PeleError>;
  return (
    typeof candidate.message === "string" &&
    typeof candidate.line === "number" &&
    typeof candidate.column === "number" &&
    typeof candidate.code === "string"
  );
}

export function createPeleRenderer(): DiagramRenderer {
  return {
    async supports(source) {
      const api = await loadPele();
      const type = api.detectType(source);
      if (!type) return false;
      const loaded = await api.load(type);
      if (!loaded) return false;
      return api.supports(source);
    },

    async mount(element, source, hooks) {
      const mine = bumpEpoch(element);
      const api = await loadPele();
      const type = api.detectType(source);
      if (type) await api.load(type);
      const painted = await api.mountAsync(element, source, mountOptions(hooks));
      const stopWatch = watchStyleAttributes(element);
      return {
        async update(next, nextHooks) {
          const nextType = api.detectType(next);
          if (nextType) await api.load(nextType);
          painted.update(next, mountOptions(nextHooks));
          liftStyleAttributes(element);
        },
        destroy() {
          stopWatch();
          painted.destroy();
          if (epochByElement.get(element) === mine) {
            element.textContent = "";
          }
        },
      };
    },
  };
}
