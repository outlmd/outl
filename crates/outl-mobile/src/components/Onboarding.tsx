/**
 * First-run onboarding for the mobile client.
 *
 * Two honest steps, no filler:
 *
 *   1. **Storage** — "Where do your notes live?" On iOS the notes live on
 *      this device (the app's local folder) and iroh syncs them P2P. There
 *      is no iCloud and no account; a single button acknowledges that and
 *      moves on. (An arbitrary-folder native picker is the deferred
 *      `workspace_picker` concern — until it lands, local is the only root.)
 *   2. **Sync (optional)** — a short, account-free explainer plus a button
 *      into the existing `<DevicesSheet />` pairing flow. Fully skippable:
 *      a single device works fine.
 *   3. Hand off to today's journal via `props.onFinish()`.
 *
 * Onboarding is a pure-UI concern, so "has the user seen it" is a frontend
 * flag (`localStorage` in `App.tsx`), not workspace state — it must not go
 * through the op log (per-install, never converges across devices).
 *
 * The copy ({@link STORAGE_STEP}, {@link SYNC_STEP}, {@link FINISH_CTA})
 * lives in `@outl/shared/onboarding` so mobile and desktop say the same
 * thing; the bottom-sheet chrome + haptics stay here.
 */

import { Show, createSignal } from "solid-js";

import { FINISH_CTA, STORAGE_STEP, SYNC_STEP } from "@outl/shared/onboarding";

import { haptic } from "../lib/haptics";
import { DevicesSheet } from "./DevicesSheet";

type Step = "storage" | "sync";

export function Onboarding(props: { onFinish: () => void }) {
  const [step, setStep] = createSignal<Step>("storage");
  const [devicesOpen, setDevicesOpen] = createSignal(false);

  /** Acknowledge the local-storage step and move to sync. The workspace
   *  already opened against the local default, so there's nothing to set. */
  function continueToSync() {
    haptic("light");
    setStep("sync");
  }

  function finish() {
    haptic("success");
    props.onFinish();
  }

  return (
    <div
      class="flex h-full w-full flex-col bg-(--color-outl-bg)"
      style={{ "padding-top": "max(env(safe-area-inset-top), 16px)" }}
    >
      <div
        class="flex flex-1 flex-col px-6 py-8"
        style={{ "padding-bottom": "max(env(safe-area-inset-bottom), 16px)" }}
      >
        {/* ---- Step 1: storage ---- */}
        <Show when={step() === "storage"}>
          <div class="flex flex-1 flex-col">
            <h1 class="mb-2 text-[28px] font-bold leading-tight">
              {STORAGE_STEP.title}
            </h1>
            <p class="mb-8 text-[15px] text-(--color-outl-fg-dim)">
              {STORAGE_STEP.body}
            </p>

            <div class="mt-auto flex flex-col gap-3">
              <button
                type="button"
                onClick={continueToSync}
                class="flex w-full flex-col items-start gap-1 rounded-2xl bg-(--color-outl-accent) px-5 py-4 text-left active:opacity-80"
              >
                <span class="text-[17px] font-semibold text-white">
                  Keep on this device
                </span>
                <span class="text-[13px] text-white/80">
                  Works offline, syncs peer-to-peer — no account, no cloud
                </span>
              </button>
            </div>
          </div>
        </Show>

        {/* ---- Step 2: optional sync ---- */}
        <Show when={step() === "sync"}>
          <div class="flex flex-1 flex-col">
            <h1 class="mb-2 text-[28px] font-bold leading-tight">
              {SYNC_STEP.title}
            </h1>
            <p class="mb-5 text-[15px] text-(--color-outl-fg-dim)">
              {SYNC_STEP.body}
            </p>

            <ul class="mb-6 flex flex-col gap-2.5">
              {SYNC_STEP.bullets.map((line) => (
                <li class="flex gap-2.5 text-[14px] text-(--color-outl-fg-dim)">
                  <span
                    aria-hidden="true"
                    class="text-(--color-outl-accent)"
                  >
                    ✓
                  </span>
                  <span>{line}</span>
                </li>
              ))}
            </ul>

            <div class="mt-auto flex flex-col gap-3">
              {/* Open the existing pairing flow — never reimplemented here. */}
              <button
                type="button"
                onClick={() => {
                  haptic("light");
                  setDevicesOpen(true);
                }}
                class="w-full rounded-xl border border-(--color-outl-border) px-4 py-3 text-[16px] font-medium text-(--color-outl-accent) active:opacity-60"
              >
                {SYNC_STEP.pairCta}
              </button>

              <button
                type="button"
                onClick={finish}
                class="w-full rounded-xl bg-(--color-outl-accent) px-4 py-3 text-[16px] font-semibold text-white active:opacity-80"
              >
                {FINISH_CTA}
              </button>

              <button
                type="button"
                onClick={finish}
                class="py-1 text-center text-[14px] text-(--color-outl-fg-dimmer) active:opacity-60"
              >
                {SYNC_STEP.skipCta}
              </button>
            </div>
          </div>
        </Show>
      </div>

      {/* Reuse the real pairing sheet (scan QR / show my QR). We only
          flip its `open` prop — its internals are owned elsewhere. */}
      <DevicesSheet
        open={devicesOpen()}
        onClose={() => setDevicesOpen(false)}
      />
    </div>
  );
}
