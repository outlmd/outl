/**
 * The single source of the system safe-area insets for every surface.
 *
 * **Why `env(safe-area-inset-*)` is not enough on its own.** The native
 * shell is edge-to-edge (a full-screen `WKWebView` on iOS,
 * `enableEdgeToEdge()` on Android), so the status bar, notch and
 * navigation bar all sit on top of the page. Neither webview reliably
 * publishes them through `env()`: when the value is missing, a bare
 * `max(env(safe-area-inset-top), 12px)` is an invalid comparison and the
 * whole declaration is dropped, and when it is published as 0 the 12px
 * floor becomes the only padding, which is not a safe area.
 *
 * So the native side measures the insets (iOS `safeAreaInsets`, Android
 * `WindowInsets` system bars + display cutout) and hands them over, in
 * CSS pixels, through `window.__outlSafeArea` / `window.__outlApplySafeArea`.
 * This module writes them onto `:root` as `--safe-area-inset-*`. The
 * stylesheet declares those same properties with an `env()` fallback
 * (`styles.css`), so a browser, the dev mock, or a platform that never
 * calls in still resolves to whatever `env()` says. Components read the
 * variable, never `env()` directly.
 */

export type SafeAreaInsets = {
  top: number;
  right: number;
  bottom: number;
  left: number;
};

const SIDES = ["top", "right", "bottom", "left"] as const;

/** The custom property carrying one side, e.g. `--safe-area-inset-top`. */
export function safeAreaProperty(side: (typeof SIDES)[number]): string {
  return `--safe-area-inset-${side}`;
}

/** The native bridge as the page sees it. All members are optional: a
 *  browser has none of them. */
export type SafeAreaGlobals = {
  /** Last value the native side pushed. Survives until JS boots. */
  __outlSafeArea?: unknown;
  /** Installed by {@link installSafeArea}; native calls it on every change. */
  __outlApplySafeArea?: (raw: unknown) => void;
  /** Android: `@JavascriptInterface` returning the current insets as JSON. */
  OutlSafeArea?: { get?: () => string };
  /** iOS: the `WKScriptMessageHandler` that answers with a push. */
  webkit?: {
    messageHandlers?: { outlSafeArea?: { postMessage?: (m: unknown) => void } };
  };
};

/**
 * Reads a native payload (an object, or the JSON string Android returns)
 * into the sides that are usable: finite and non-negative. A side that is
 * missing or garbage is left out, so its property is cleared and the
 * `env()` fallback applies instead of a bogus value.
 */
export function parseSafeAreaInsets(raw: unknown): Partial<SafeAreaInsets> {
  let value = raw;
  if (typeof value === "string") {
    try {
      value = JSON.parse(value);
    } catch {
      return {};
    }
  }
  if (typeof value !== "object" || value === null) return {};
  const out: Partial<SafeAreaInsets> = {};
  for (const side of SIDES) {
    const n = (value as Record<string, unknown>)[side];
    if (typeof n === "number" && Number.isFinite(n) && n >= 0) {
      out[side] = Math.round(n * 100) / 100;
    }
  }
  return out;
}

/** Writes the insets onto `root`; a side not in `insets` is cleared. */
export function applySafeAreaInsets(
  insets: Partial<SafeAreaInsets>,
  root: HTMLElement = document.documentElement,
): void {
  for (const side of SIDES) {
    const n = insets[side];
    if (n === undefined) root.style.removeProperty(safeAreaProperty(side));
    else root.style.setProperty(safeAreaProperty(side), `${n}px`);
  }
}

/**
 * Wires the bridge once, at app boot. Applies whatever the native side
 * already pushed or can answer synchronously, asks iOS to push if it has
 * not, and registers the hook native calls on every later change
 * (rotation, multitasking, display-cutout or system-bar changes).
 */
export function installSafeArea(
  win: Window & SafeAreaGlobals = window as Window & SafeAreaGlobals,
  root: HTMLElement = win.document.documentElement,
): void {
  const apply = (raw: unknown) => applySafeAreaInsets(parseSafeAreaInsets(raw), root);
  win.__outlApplySafeArea = (raw) => {
    win.__outlSafeArea = raw;
    apply(raw);
  };

  let initial: unknown = win.__outlSafeArea;
  if (initial === undefined) {
    try {
      initial = win.OutlSafeArea?.get?.();
    } catch {
      initial = undefined;
    }
  }
  if (initial !== undefined) apply(initial);

  // iOS cannot be read synchronously; the handler replies through
  // `__outlApplySafeArea`, which is already installed above.
  try {
    win.webkit?.messageHandlers?.outlSafeArea?.postMessage?.(null);
  } catch {
    // Handler not registered yet: native pushes when it registers.
  }
}
