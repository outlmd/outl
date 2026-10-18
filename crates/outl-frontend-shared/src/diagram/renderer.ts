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
 * mailto, no icon resolver, zoom left at its default (`"auto"`).
 */

import type { LinkInfo, Mounted, PeleError } from "pele/lazy";

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

export interface DiagramMount {
  update(source: string): Promise<void>;
  destroy(): void;
}

export interface DiagramRenderer {
  supports(source: string): Promise<boolean>;
  mount(
    element: HTMLElement,
    source: string,
    hooks: {
      onLinks?: (links: DiagramLink[]) => void;
      onError?: (failure: DiagramFailure) => void;
    },
  ): Promise<DiagramMount>;
}

type PeleLazy = typeof import("pele/lazy");

let loading: Promise<PeleLazy> | undefined;

function loadPele(): Promise<PeleLazy> {
  loading ??= import("pele/lazy");
  return loading;
}

/** Shared across mount and update so a re-render cannot widen the policy. */
function mountOptions(
  hooks: {
    onLinks?: (links: DiagramLink[]) => void;
    onError?: (failure: DiagramFailure) => void;
  },
) {
  return {
    images: false as const,
    linkSchemes: ["http", "https", "mailto"],
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
      const api = await loadPele();
      const type = api.detectType(source);
      if (type) await api.load(type);
      const options = mountOptions(hooks);
      let current: Mounted = await api.mountAsync(element, source, options);
      return {
        async update(next) {
          const nextType = api.detectType(next);
          if (nextType) await api.load(nextType);
          current.update(next, options);
        },
        destroy() {
          current.destroy();
          element.textContent = "";
        },
      };
    },
  };
}
