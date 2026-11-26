/**
 * The OS clipboard, and the only module in this client that touches it.
 *
 * Every read and write goes through `@tauri-apps/plugin-clipboard-manager`
 * (`arboard` on the backend), never `navigator.clipboard`. The web API
 * is gated on a user gesture in WKWebView: a write that follows an IPC
 * round-trip (`copy_markdown`) is no longer inside the keystroke that
 * asked for it, so it was refused, and the refusal was swallowed. A
 * programmatic read pops the native "Paste" button instead of reading.
 *
 * **One queue for both directions.** A yank is `copy_markdown` and then
 * a write; a `p` pressed while the first half is in flight has to paste
 * the new markdown, not what the clipboard held before. So a write is
 * enqueued the moment it is asked for, with its text still a promise,
 * and a read waits for everything queued ahead of it.
 *
 * **Failures go to the caller.** Both functions reject, and the caller
 * owns the error sink (`deps.setError`, the status line). A clipboard
 * that silently kept its old contents reads as a copy that worked.
 */

import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";

/** Shown when the clipboard holds nothing a paste can use. */
export const CLIPBOARD_HAS_NO_TEXT = "Clipboard has no text";

/** Tail of the queue. Never rejects: a failed write is its caller's
 *  error, not a reason to refuse every operation after it. */
let tail: Promise<void> = Promise.resolve();

function messageOf(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/**
 * Write `text` to the OS clipboard, after every write queued before it.
 *
 * `text` may still be computing (a `copy_markdown` reply). It resolves
 * to `undefined` when its producer failed and already reported why,
 * and then nothing is written. Rejects when the plugin refuses.
 */
export function writeOsClipboard(
  text: string | Promise<string | undefined>,
): Promise<void> {
  const done = tail.then(async () => {
    const value = await text;
    if (value == null) return;
    try {
      await writeText(value);
    } catch (e) {
      throw new Error(`Could not copy to the clipboard: ${messageOf(e)}`);
    }
  });
  tail = done.catch(() => {});
  return done;
}

/**
 * Read the OS clipboard as text, once every queued write has landed.
 *
 * The plugin rejects when the clipboard is empty or holds only an
 * image; both read as {@link CLIPBOARD_HAS_NO_TEXT}, since neither is
 * something to paste. The original reason goes to the console.
 */
export async function readOsClipboard(): Promise<string> {
  await tail;
  let text: string;
  try {
    text = await readText();
  } catch (e) {
    console.warn("[os-clipboard] readText refused", e);
    throw new Error(CLIPBOARD_HAS_NO_TEXT);
  }
  if (!text) throw new Error(CLIPBOARD_HAS_NO_TEXT);
  return text;
}
