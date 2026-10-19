import { type JSX, Show, createEffect, createSignal } from "solid-js";
import {
  clearCountsInStore,
  isOrderLocked,
  lockOrderInStore,
  orderedMiddleActions,
  orderedMiddleFromStore,
  unlockOrderInStore,
} from "@outl/shared/toolbar";
import {
  getTelemetryEnabled,
  setTelemetryEnabled,
} from "@outl/shared/api/telemetry";
import { CRASH_REPORTS_SETTING } from "@outl/shared/onboarding";

import { diagramsMode, setDiagramsMode, type DiagramsMode } from "@outl/shared/diagram";

import { createSheetDrag } from "../lib/sheet-drag";
import { haptic } from "../lib/haptics";
import { getEditorDiagrams, setEditorDiagrams } from "../lib/api";

/**
 * `<SettingsSheet />` — mobile's preferences surface.
 *
 * The first one: every other sheet in this client (`DevicesSheet`,
 * `TemplateSheet`, `RemindersSheet`, `PropertiesSheet`) shows workspace
 * *content*, so a preference had nowhere to live and the toolbar lock
 * (#271) could not be offered at all. Chrome mirrors `TemplateSheet`
 * (same drag-dismiss hook, blurred card, safe-area padding) so it reads
 * as one of the family rather than a new kind of screen.
 *
 * One row is navigation (devices and pairing, which the desktop keeps
 * in Settings too). The toolbar rows are **per-device UI state in
 * `localStorage`**, not workspace data: none of it goes through the op
 * log, because two devices disagreeing about their own toolbar layout is
 * not a conflict to reconcile (root `CLAUDE.md` invariant 7).
 * Diagrams are also per-device, but they live in `config.toml`
 * (`[editor] diagrams`) so the desktop on this machine sees the same
 * choice.
 *
 * **Crash reports is the exception, and it is the first Tauri command
 * this sheet calls.** This file used to say the sheet called none, and
 * that was a property of what had landed here rather than a rule. The
 * reason the flag cannot be `localStorage` is that nothing in the
 * webview reads it: `outl-telemetry` reads `[telemetry] enabled` out of
 * the global config at **process start**, before a workspace — or a
 * webview — exists, so the consent has to be where the process can see
 * it (RFC 0377 section 4). It is also consent rather than a layout
 * preference: a property of the person, which is why it belongs in a
 * config file that follows them to their next device, and why the phone
 * has to offer it at all — there is no `outl` binary on iOS, so this
 * toggle is the only opt-out a mobile user has.
 *
 * The rule that did *not* change: the setting's logic is not in here.
 * The flag's one writer is
 * `outl_tauri_shared::commands::telemetry::set_telemetry_enabled`, and
 * the words beside the toggle are `@outl/shared/onboarding`'s.
 */
export function SettingsSheet(props: {
  open: boolean;
  onClose: () => void;
  /** Hand over to `<DevicesSheet />`. This sheet closes first: it sits at
   *  `z-[55]` and that one at `z-50`, so both open paints pairing
   *  underneath this card. */
  onOpenDevices: () => void;
}): JSX.Element {
  const drag = createSheetDrag(() => props.onClose());
  const [locked, setLocked] = createSignal(false);
  const [resetDone, setResetDone] = createSignal(false);
  const [diagrams, setDiagrams] = createSignal<DiagramsMode>(diagramsMode());
  /** `null` until the first read lands: the default is ON, so painting a
   *  switch before we know would show "off" to nobody and "on" to
   *  everyone, including someone who turned it off. */
  const [reports, setReports] = createSignal<boolean | null>(null);
  const [savingReports, setSavingReports] = createSignal(false);
  const [reportsError, setReportsError] = createSignal(false);

  // Re-read on every open. `<Show>` gates the markup, not the
  // component: `Journal` mounts this once and toggles `open`, so a
  // plain `createSignal(isOrderLocked())` in the body would read the
  // store a single time, at app start. `TemplateSheet` and
  // `RemindersSheet` carry the same effect for the same reason.
  createEffect(() => {
    if (!props.open) return;
    setLocked(isOrderLocked());
    setResetDone(false);
    setDiagrams(diagramsMode());
    try {
      void getEditorDiagrams()
        .then((value) => {
          const mode: DiagramsMode = value === "off" ? "off" : "pele";
          setDiagrams(mode);
          setDiagramsMode(mode);
        })
        .catch(() => {
          // The sheet still renders. The fence keeps the current mode.
          // Re-read on every open for the same reason the lock does, plus one:
    // this value lives in a file another process writes (`outl
    // telemetry off` on a paired desktop, a hand-edited `config.toml`),
    // so a cached answer here can be wrong without anything in this
    // client having changed it.
    void loadReports();
  });
    } catch {
      // No host in a test. Leave the signal as it is.
    }
  });

  function onDiagramsChange(value: string) {
    const mode: DiagramsMode = value === "off" ? "off" : "pele";
    setDiagrams(mode);
    setDiagramsMode(mode);
    try {
      void setEditorDiagrams(mode).catch(() => {});
    } catch {
      // Same as the read: a missing host must not take the sheet down.
    }
  }

  async function loadReports() {
    try {
      setReports(await getTelemetryEnabled());
      setReportsError(false);
    } catch {
      // Say "we could not read it" in the row rather than falling back
      // to the default: the default is on, so a guess here tells
      // someone who opted out that they did not.
      setReports(null);
      setReportsError(true);
    }
  }

  /**
   * Flip crash reporting, moving the switch only once the write returns.
   *
   * Same shape as `RemindersSheet`'s `saveSettings`, and the reason is
   * sharper here: an optimistic flip that fails to save leaves a switch
   * reading "off" over a reporter that is still on.
   */
  async function toggleReports() {
    if (savingReports() || reports() === null) return;
    setSavingReports(true);
    haptic("light");
    try {
      setReports(await setTelemetryEnabled(!reports()));
      setReportsError(false);
    } catch {
      setReportsError(true);
    } finally {
      setSavingReports(false);
    }
  }

  function toggleLock() {
    haptic("light");
    if (locked()) {
      unlockOrderInStore();
      setLocked(false);
      return;
    }
    // Freeze what the user is looking at today, not the cold-start
    // order — "lock" means "keep this", and the row they have learned
    // is the MFU one their taps produced.
    lockOrderInStore(orderedMiddleFromStore());
    setLocked(true);
  }

  function resetOrder() {
    haptic("light");
    clearCountsInStore();
    // A locked bar renders its frozen snapshot, so wiping the counts
    // alone would change nothing on screen. Re-freeze on the cold-start
    // order instead: the user asked for the default back, and they did
    // not ask to be unlocked.
    if (locked()) lockOrderInStore(orderedMiddleActions({}));
    // The bar isn't on screen while this sheet is, and it only
    // re-reads at the next keyboard, so nothing visible confirms the
    // tap. Say so in the row itself rather than leaving the user
    // wondering whether the button did anything.
    setResetDone(true);
  }

  return (
    <Show when={props.open}>
      <div
        class="outl-fade-in fixed inset-0 z-[55] bg-black/40 backdrop-blur-md"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[55] flex flex-col"
        style={{
          "padding-bottom": "max(env(safe-area-inset-bottom), 16px)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="mx-3 mb-2 overflow-hidden rounded-2xl bg-(--color-outl-bg-elev)/95 shadow-[var(--shadow-capsule)] backdrop-blur-2xl">
          <span
            class="block py-2"
            style={{ "touch-action": "none" }}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onPointerCancel={drag.onPointerCancel}
            aria-label="Drag to close"
            role="button"
          >
            <span
              aria-hidden="true"
              class="mx-auto block h-1 w-10 rounded-full bg-(--color-outl-border)"
            />
          </span>

          <div class="px-4 pb-1 pt-1">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-outl-fg-dim)">
              Sync
            </span>
          </div>

          {/* Pairing had no named entry point on the phone: tapping the
              sync dot refreshes and only a press-and-hold opened this
              (`64fe9c53`), so the feature read as missing to anyone who
              looked where the desktop keeps it. The gesture stays, as a
              shortcut rather than as the only way in. */}
          <button
            type="button"
            onClick={() => {
              haptic("light");
              props.onOpenDevices();
              props.onClose();
            }}
            class="flex w-full flex-col items-start gap-0.5 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left active:bg-(--color-outl-border)/30"
          >
            <span class="text-[16px] font-medium text-(--color-outl-fg)">
              Devices and sync
            </span>
            <span class="text-[13px] leading-snug text-(--color-outl-fg-dim)">
              Pair this phone with your other devices, see which ones are
              reachable, and sync now.
            </span>
          </button>

          <div class="px-4 pb-1 pt-3">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-outl-fg-dim)">
              Editor
            </span>
          </div>

          <label class="flex w-full items-center gap-3 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left">
            <span class="min-w-0 flex-1">
              <span class="block text-[16px] font-medium text-(--color-outl-fg)">
                Diagrams
              </span>
              <span class="mt-0.5 block text-[13px] leading-snug text-(--color-outl-fg-dim)">
                Mermaid fences. Off leaves the highlighted source.
              </span>
            </span>
            <select
              aria-label="Diagrams"
              value={diagrams()}
              onChange={(e) => onDiagramsChange(e.currentTarget.value)}
              class="rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg) px-2 py-1 text-[15px] text-(--color-outl-fg)"
            >
              <option value="pele">Pele</option>
              <option value="off">Off</option>
            </select>
          </label>

          <div class="px-4 pb-1 pt-3">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-outl-fg-dim)">
              Keyboard toolbar
            </span>
          </div>

          <button
            type="button"
            role="switch"
            aria-checked={locked()}
            onClick={toggleLock}
            class="flex w-full items-center gap-3 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left active:bg-(--color-outl-border)/30"
          >
            <span class="min-w-0 flex-1">
              <span class="block text-[16px] font-medium text-(--color-outl-fg)">
                Lock button order
              </span>
              <span class="mt-0.5 block text-[13px] leading-snug text-(--color-outl-fg-dim)">
                Keep the buttons where they are. outl still counts which
                ones you use, it just stops rearranging them.
              </span>
            </span>
            <IosToggle on={locked()} />
          </button>

          <button
            type="button"
            onClick={resetOrder}
            class="flex w-full flex-col items-start gap-0.5 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left active:bg-(--color-outl-border)/30"
          >
            <span class="text-[16px] font-medium text-(--color-outl-accent)">
              Reset button order
            </span>
            <span class="text-[13px] leading-snug text-(--color-outl-fg-dim)">
              <Show
                when={resetDone()}
                fallback="Forget the usage counts and go back to the original layout."
              >
                Order reset. It applies the next time the keyboard opens.
              </Show>
            </span>
          </button>

          <p class="border-t border-(--color-outl-border)/30 px-4 py-3 text-[12px] leading-snug text-(--color-outl-fg-dim)/80">
            Takes effect the next time the keyboard opens.
          </p>

          <div class="px-4 pb-1 pt-3">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-outl-fg-dim)">
              Privacy
            </span>
          </div>

          {/* Reporting is on by default and there is no `outl` binary on
              this platform, so this row is the only opt-out a phone has
              (RFC 0377 section 7). The copy is `@outl/shared`'s — the
              desktop shows the identical sentences, and the same claim
              goes to Apple and Google. */}
          <button
            type="button"
            role="switch"
            aria-checked={reports() === true}
            aria-label={CRASH_REPORTS_SETTING.title}
            disabled={reports() === null || savingReports()}
            onClick={() => void toggleReports()}
            class="flex w-full items-center gap-3 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left active:bg-(--color-outl-border)/30 disabled:opacity-50"
          >
            <span class="min-w-0 flex-1">
              <span class="block text-[16px] font-medium text-(--color-outl-fg)">
                {CRASH_REPORTS_SETTING.title}
              </span>
              <span class="mt-0.5 block text-[13px] leading-snug text-(--color-outl-fg-dim)">
                {/* One sentence for both directions, because both end
                    the same way: this device does not know the flag's
                    state. Claiming to know it is the failure the row
                    exists to avoid. */}
                <Show
                  when={!reportsError()}
                  fallback={CRASH_REPORTS_SETTING.unavailable}
                >
                  {CRASH_REPORTS_SETTING.body}
                </Show>
              </span>
            </span>
            <IosToggle on={reports() === true} />
          </button>
        </div>

        <button
          type="button"
          onClick={props.onClose}
          class="mx-3 rounded-2xl bg-(--color-outl-bg-elev)/95 py-3.5 text-center text-[16px] font-semibold text-(--color-outl-accent) shadow-[var(--shadow-capsule)] backdrop-blur-2xl active:bg-(--color-outl-border)/30"
        >
          Done
        </button>
      </div>
    </Show>
  );
}

/** iOS-style switch. Presentational: the enclosing row owns the tap and
 *  the `role="switch"` semantics, so this is `aria-hidden` and never a
 *  second focus stop for the same control.
 *
 *  Named for its shape rather than its first caller: it was `LockToggle`
 *  while the toolbar lock was the only switch in the sheet, and crash
 *  reports is the second.
 *
 *  **Not named `Switch`**, which is a Solid control-flow component:
 *  `babel-preset-solid` auto-imports the built-in for that tag name even
 *  when the module declares its own, so `<Switch on={…} />` compiled to
 *  Solid's `<Switch>`, which throws on having no `<Match>` children and
 *  took down the whole sheet render — the gear opened nothing at all.
 *  Same goes for `For`, `Show`, `Match`, `Index`, `Dynamic`, `Portal`,
 *  `Suspense` and `ErrorBoundary`. */
function IosToggle(props: { on: boolean }): JSX.Element {
  return (
    <span
      aria-hidden="true"
      class="relative block h-[31px] w-[51px] shrink-0 rounded-full transition-colors duration-200"
      classList={{
        "bg-(--color-outl-accent)": props.on,
        "bg-(--color-outl-border)": !props.on,
      }}
    >
      <span
        class="absolute top-[2px] block h-[27px] w-[27px] rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.3)] transition-[left] duration-200"
        style={{ left: props.on ? "22px" : "2px" }}
      />
    </span>
  );
}
