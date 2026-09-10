/**
 * `localStorage` access for the toolbar's per-device UI state.
 *
 * Both the MFU counts and the lock order live in `localStorage` and
 * both need the same defensive wrapper, so the wrapper has one home
 * here rather than a copy in each module.
 *
 * `localStorage` may be unavailable (SSR, private-mode quotas) and can
 * throw on access, not just on write. Every helper degrades to the
 * "nothing stored" answer instead of throwing, so a storage failure
 * never breaks the toolbar — this is UI polish, not durable state.
 */

/** Resolve the `Storage` from whichever global exposes it (the bare
 *  `localStorage` identifier resolves against `window` in the webview
 *  and against the happy-dom global in tests; `globalThis.localStorage`
 *  is undefined in some of those environments). Returns `null` when
 *  storage is unavailable. */
function store(): Storage | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null;
  }
}

export function safeGet(key: string): string | null {
  try {
    return store()?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

export function safeSet(key: string, value: string): void {
  try {
    store()?.setItem(key, value);
  } catch {
    // ignore — best-effort UI polish, not durable state
  }
}

export function safeRemove(key: string): void {
  try {
    store()?.removeItem(key);
  } catch {
    // ignore — same reason as `safeSet`
  }
}
