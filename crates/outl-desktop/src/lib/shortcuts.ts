/**
 * Universal keyboard dispatcher for the desktop client.
 *
 * Pulls the binding catalog from the `outl-shortcuts` Rust crate
 * via the `list_shortcut_bindings` Tauri command, translates each
 * `KeyboardEvent` into a portable [`Chord`], buffers prefix
 * sequences (vim-style `g j`, `d d`), and dispatches the resolved
 * [`Action`] to a single map of handler functions.
 *
 * Same chord vocabulary the TUI uses — so a user who learned
 * `[` / `]` in the terminal sees identical navigation here.
 *
 * Mode detection is DOM-driven:
 *
 * - Picker / settings open → `overlay`
 * - Focus in a `<textarea>` (block editor) → `insert`
 * - Otherwise → `normal`
 *
 * Visual mode is engaged explicitly by `EnterVisual` (TUI parity).
 */

import { pluginRun } from "@outl/shared/api/commands";

import {
  type Action,
  type Binding,
  type Chord,
  type Key,
  type PluginKeybinding,
  type ShortcutMode,
  type SupportDto,
  MOD_ALT,
  MOD_CTRL,
  MOD_META,
  MOD_SHIFT,
  listActionSupport,
  listShortcutBindings,
  pluginKeybindings,
} from "./api";

import { playPluginViews } from "./plugin-views";
import { appState, setAppState, setOutline } from "./store";

// ---------------------------------------------------------------------------
// KeyboardEvent → Chord
// ---------------------------------------------------------------------------

function modsOf(e: KeyboardEvent): number {
  let m = 0;
  if (e.ctrlKey) m |= MOD_CTRL;
  if (e.altKey) m |= MOD_ALT;
  if (e.metaKey) m |= MOD_META;
  if (e.shiftKey) {
    // Convention: Shift modifier on a printable single-char `e.key`
    // is only meaningful when the produced character is a letter
    // (`Shift+I` → `I`, lookup as Char('i') + SHIFT). For symbol
    // characters (`?`, `!`, `@`, …) Shift is "how that symbol is
    // typed on a US layout" and `e.key` already reflects the
    // shifted form — adding the SHIFT bit would prevent matches
    // against bindings like `ch('?')` that the TUI / catalog
    // expresses without the modifier.
    const k = e.key;
    const isSymbolChar = k.length === 1 && !/[a-zA-Z]/.test(k);
    if (!isSymbolChar) m |= MOD_SHIFT;
  }
  return m;
}

/**
 * Translate `e.key` (the DOM's localized key name) to a portable
 * [`Key`]. Returns `null` for keys we don't care about (pure
 * modifier presses, dead keys, etc.) so the caller can skip them.
 */
function eventToKey(e: KeyboardEvent): Key | null {
  switch (e.key) {
    case "Enter":
      return { kind: "Enter" };
    case "Escape":
      return { kind: "Esc" };
    case "Tab":
      return { kind: "Tab" };
    case "Backspace":
      return { kind: "Backspace" };
    case "Delete":
      return { kind: "Delete" };
    case "ArrowUp":
      return { kind: "Up" };
    case "ArrowDown":
      return { kind: "Down" };
    case "ArrowLeft":
      return { kind: "Left" };
    case "ArrowRight":
      return { kind: "Right" };
    case "Home":
      return { kind: "Home" };
    case "End":
      return { kind: "End" };
    case "PageUp":
      return { kind: "PageUp" };
    case "PageDown":
      return { kind: "PageDown" };
    case " ":
      return { kind: "Space" };
  }
  if (/^F\d{1,2}$/.test(e.key)) {
    return { kind: "Function", value: Number(e.key.slice(1)) };
  }
  // Plain printable character — lowercase to match Rust's
  // `Key::char` normalisation. Shift is preserved as a modifier
  // bit, so `Shift+I` lands as `Char('i')` + SHIFT.
  if (e.key.length === 1) {
    return { kind: "Char", value: e.key.toLowerCase() };
  }
  return null;
}

function eventToChord(e: KeyboardEvent): Chord | null {
  const key = eventToKey(e);
  if (!key) return null;
  return { mods: modsOf(e), key };
}

// ---------------------------------------------------------------------------
// Chord / ChordSequence equality
// ---------------------------------------------------------------------------

function keyEq(a: Key, b: Key): boolean {
  if (a.kind !== b.kind) return false;
  if (a.kind === "Char" && b.kind === "Char") return a.value === b.value;
  if (a.kind === "Function" && b.kind === "Function")
    return a.value === b.value;
  return true;
}

function chordEq(a: Chord, b: Chord): boolean {
  return a.mods === b.mods && keyEq(a.key, b.key);
}

function seqEq(a: Chord[], b: Chord[]): boolean {
  return a.length === b.length && a.every((c, i) => chordEq(c, b[i]));
}

function isPrefix(prefix: Chord[], full: Chord[]): boolean {
  return (
    prefix.length < full.length && prefix.every((c, i) => chordEq(c, full[i]))
  );
}

// ---------------------------------------------------------------------------
// Mode detection
// ---------------------------------------------------------------------------

function detectMode(): ShortcutMode {
  if (appState.pickerOpen || appState.settingsOpen || appState.helpOpen)
    return "overlay";
  // `appState.mode` already tracks Normal/Insert/Visual when set
  // explicitly by handlers (e.g. EnterVisual / ExitInsert).
  if (appState.mode === "vim-visual") return "visual";
  if (appState.mode === "vim-normal") return "normal";
  if (appState.mode === "vim-insert") return "insert";
  // DOM fallback for the no-vim-mode user: focus in textarea =
  // insert; otherwise normal.
  const el = document.activeElement;
  if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {
    return "insert";
  }
  return "normal";
}

// ---------------------------------------------------------------------------
// Action dispatcher
// ---------------------------------------------------------------------------

/**
 * Handler map. Each property is a function the caller supplies that
 * fires when its corresponding [`Action`] resolves.
 *
 * A missing entry is not a hole to paper over — it is a fact the
 * catalog already knows, and `outl_shortcuts::support` carries the
 * sentence to show the user. Which entries may be absent is pinned
 * by `shortcuts.support.test.ts`: anything the catalog calls `full`
 * or `partial` on the desktop must be here.
 */
export type ActionHandlers = Partial<
  Record<Action["kind"], () => void | Promise<void>>
>;

/** Desktop support column, keyed by action, fetched once per session. */
let cachedSupport: Map<Action["kind"], SupportDto> | null = null;

async function loadSupport(): Promise<Map<Action["kind"], SupportDto>> {
  if (cachedSupport) return cachedSupport;
  const rows = await listActionSupport();
  cachedSupport = new Map(rows.map((r) => [r.action.kind, r.desktop]));
  return cachedSupport;
}

/**
 * Tell the user why a chord they pressed did nothing.
 *
 * The text comes from the catalog, never from here. A client that
 * writes its own wording is a fourth copy of a fact that already had
 * three disagreeing ones — which is the drift
 * `crates/outl-shortcuts/src/support.rs` was added to end.
 */
function nudgeFor(
  kind: Action["kind"],
  support: Map<Action["kind"], SupportDto>,
  onNudge: (msg: string) => void,
) {
  const s = support.get(kind);
  if (!s) {
    // The catalog is exhaustive over `Action`, so a miss here means
    // the frontend and the backend are built from different
    // revisions. Say so in the console: it is a build problem, not
    // something the user can act on.
    console.error(`[shortcuts] ${kind} is absent from the support catalog`);
    return;
  }
  if (s.why) {
    onNudge(s.why);
    return;
  }
  // `full` or `native` with no handler is our bug, not a gap: the
  // catalog promises the desktop does this. `shortcuts.support.test.ts`
  // fails on it, so reaching this at runtime means the test was
  // skipped or the two halves are out of sync.
  console.error(
    `[shortcuts] catalog says the desktop supports ${kind} (${s.kind}), but no handler is wired`,
  );
}

async function dispatch(action: Action, handlers: ActionHandlers) {
  const h = handlers[action.kind];
  if (!h) {
    // The keydown path checks for a handler before calling us (it has
    // to, to decide `preventDefault`), so this is unreachable from
    // there. Kept so a direct caller cannot fail silently.
    console.error(`[shortcuts] dispatch called for unhandled ${action.kind}`);
    return;
  }
  try {
    await h();
  } catch (e) {
    console.error(`[shortcuts] handler ${action.kind} threw`, e);
  }
}

// ---------------------------------------------------------------------------
// Plugin keybinding dispatch
// ---------------------------------------------------------------------------

/**
 * Run a plugin-contributed chord: fire the plugin's command, surface its
 * notifications / errors on the status line, re-render the page if it
 * mutated, and play any `ui-render` overlays — exactly what the plugin
 * palette does for a click. Best-effort: a failure lands on the status
 * line, never throws out of the dispatcher.
 */
async function runPluginChord(kb: PluginKeybinding) {
  try {
    const reply = await pluginRun(
      kb.plugin_id,
      kb.command_id,
      appState.page?.id ?? null,
    );
    for (const note of reply.notifications) setAppState("lastError", note);
    for (const err of reply.errors)
      setAppState("lastError", `plugin: ${err}`);
    // Re-render the on-screen page from the refreshed view — same fields
    // `<AppShell>`'s `applyView` writes. Backlinks are fetched lazily by
    // OutlineView's per-slug effect, not carried on the view.
    if (reply.view) {
      setAppState({ page: reply.view.page });
      setOutline(reply.view.outline);
    }
    playPluginViews(reply.views);
  } catch (e) {
    setAppState("lastError", e instanceof Error ? e.message : String(e));
  }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Install the global keydown dispatcher. Returns a cleanup
 * function — call it in `onCleanup` so the listener is detached
 * on remount.
 *
 * `handlers` is a map from `Action["kind"]` to a function. Only
 * the actions the desktop actively supports today need to be
 * filled in — which ones those are is declared in
 * `outl_shortcuts::support` and pinned by
 * `shortcuts.support.test.ts`.
 *
 * `onNudge` receives the catalog's sentence when the user presses a
 * chord this client cannot honour. Default is a no-op so a test
 * harness can install without one; the app passes its status-line
 * setter.
 */
export async function installShortcuts(
  handlers: ActionHandlers,
  onNudge: (msg: string) => void = () => {},
): Promise<() => void> {
  const bindings = await loadBindings();
  const support = await loadSupport();

  // Plugin-contributed chords, folded in as a Global overlay. They are
  // always single-chord and `global` (the backend stamps the mode), so we
  // match them against the live keystroke regardless of the active mode —
  // BUT only after the native catalog has had its say, and only when no
  // native binding (in any mode) already owns that exact chord. Native
  // always wins; a plugin can never shadow `Cmd+B` / `Cmd+P` / etc.
  const pluginBindings = await loadPluginBindings();
  const nativeOwnsChord = (seq: Chord[]): boolean =>
    bindings.some((b) => seqEq(b.chord, seq));

  /** Buffered chord prefix (e.g. user typed `g`, waiting for `j`). */
  let pending: Chord[] = [];
  let pendingTimer: number | undefined;

  function resetPending() {
    pending = [];
    if (pendingTimer !== undefined) {
      window.clearTimeout(pendingTimer);
      pendingTimer = undefined;
    }
  }

  function armPendingTimeout() {
    if (pendingTimer !== undefined) window.clearTimeout(pendingTimer);
    // 1s window to complete a chord prefix; fall back to plain
    // typing afterwards (matches the TUI's chord buffer).
    pendingTimer = window.setTimeout(resetPending, 1000);
  }

  const onKey = async (e: KeyboardEvent) => {
    const chord = eventToChord(e);
    if (!chord) return;

    const mode = detectMode();

    // Per-keystroke trace at `info` level so it shows up in the
    // default DevTools output (no need to toggle Verbose). One line
    // per key — easy to scan when chasing "why didn't my chord fire?".
    console.info(
      `[shortcuts:trace] mode=${mode} mods=${chord.mods} key=${
        chord.key.kind === "Char" ? chord.key.value : chord.key.kind
      }`,
    );

    // In Insert mode without modifiers, only intercept
    // explicit-key actions (Enter / Esc / Tab / Backspace / arrows
    // / Delete). Plain typing of letters / numbers must reach the
    // textarea — otherwise the user can't write anything.
    const isPlainChar = chord.mods === 0 && chord.key.kind === "Char";
    if (mode === "insert" && isPlainChar && pending.length === 0) {
      return;
    }

    const sequence = [...pending, chord];

    // Mode-specific match wins over Global. Without this the catalog
    // declaration order would silently shadow per-mode overrides —
    // e.g. `Cmd+T` is `OpenToday` (Global) but also `ToggleTodo`
    // (Insert), and inside a textarea the Insert binding has to win
    // even though Global is declared first in `defaults.rs`.
    const hit =
      bindings.find((b) => b.mode === mode && seqEq(b.chord, sequence)) ??
      bindings.find((b) => b.mode === "global" && seqEq(b.chord, sequence));
    if (hit) {
      // Only preventDefault when we actually have a handler ready —
      // otherwise we'd swallow legitimate keystrokes (e.g. `Enter`
      // in Insert mode, which the textarea's own onKeyDown handler
      // is the canonical processor for) just because the chord
      // catalog *names* an action with no JS implementation yet.
      const hasHandler = !!handlers[hit.action.kind];
      if (hasHandler) {
        e.preventDefault();
        resetPending();
        console.info(`[shortcuts] ${mode} → ${hit.action.kind}`);
        await dispatch(hit.action, handlers);
      } else {
        // No `preventDefault` on purpose — the textarea or the OS is
        // the right processor for this key (`Enter` in Insert is the
        // canonical case). But falling through silently is what made
        // a missing feature look like a broken one, so the user is
        // told what the catalog knows.
        nudgeFor(hit.action.kind, support, onNudge);
        resetPending();
      }
      return;
    }

    // Prefix? Buffer and wait for the next keypress.
    const isPrefixOfSomething = bindings.some(
      (b) =>
        (b.mode === "global" || b.mode === mode) && isPrefix(sequence, b.chord),
    );
    if (isPrefixOfSomething) {
      e.preventDefault();
      pending = sequence;
      armPendingTimeout();
      return;
    }

    // No native binding (match or prefix) wanted this chord. Now give a
    // plugin keybinding a shot. Plugin chords are single-chord, so only
    // an unbuffered keystroke (`pending` empty) can match one — a chord
    // mid-sequence belongs to a native prefix that just timed out. Skip
    // any chord a native binding owns in another mode, so plugins can't
    // shadow the OS-standard set.
    if (pending.length === 0) {
      const pluginHit = pluginBindings.find(
        (kb) => seqEq(kb.chord, sequence) && !nativeOwnsChord(kb.chord),
      );
      if (pluginHit) {
        e.preventDefault();
        resetPending();
        console.info(
          `[shortcuts] plugin chord → ${pluginHit.plugin_id}/${pluginHit.command_id}`,
        );
        await runPluginChord(pluginHit);
        return;
      }
    }

    // Not a match, not a prefix — reset and let the event through.
    resetPending();
  };

  window.addEventListener("keydown", onKey);
  return () => {
    window.removeEventListener("keydown", onKey);
    resetPending();
  };
}

// ---------------------------------------------------------------------------
// Catalog caching
// ---------------------------------------------------------------------------

/**
 * Load plugin-contributed keybindings for one dispatcher install.
 * Best-effort: a failure (no host, plugins not loaded yet) yields an
 * empty list so the dispatcher keeps working with only the native
 * catalog. Deliberately **not** module-cached: `installShortcuts` re-runs
 * on a workspace swap (`<AppShell>` remount), and the new workspace can
 * ship a different plugin set — a stale module cache would carry the old
 * workspace's chords. Plugins load lazily on the host's first request, so
 * this can still be empty at install time on a cold boot.
 */
async function loadPluginBindings(): Promise<PluginKeybinding[]> {
  try {
    return await pluginKeybindings();
  } catch {
    return [];
  }
}

let cachedBindings: Binding[] | null = null;

async function loadBindings(): Promise<Binding[]> {
  if (cachedBindings) return cachedBindings;
  cachedBindings = await listShortcutBindings();
  return cachedBindings;
}
