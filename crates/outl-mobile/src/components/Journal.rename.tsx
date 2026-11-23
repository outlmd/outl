/**
 * Renaming the page you are looking at, from its own title.
 *
 * **The flow is not here.** `renameOrMerge` in
 * `@outl/shared/pages/rename-merge` owns the order of events (rename;
 * when the name is taken, ask; merge this page into the existing one on
 * accept) and every sentence the user reads. This owns the phone's
 * chrome around it — a name sheet and a confirm dialog — and nothing
 * else. A second opinion about either would be the drift those files
 * exist to prevent.
 *
 * It is therefore a second *caller*, never a second implementation. The
 * page switcher's `createPageActions` was the first, and reusing it here
 * was the obvious move; two things made a thin header-local flow the
 * better one. Its lifecycle belongs to the switcher — `deps.open` going
 * false abandons every sub-flow, and a header has no close — and its
 * failure path is `window.alert`, which is the wrong surface now that
 * the toast has a tone to say "this failed" with. What is shared is what
 * matters: the commands, the order, and the words.
 *
 * ## Two platform traps this shape exists to avoid
 *
 * **There is no inline `<input>` in the header, on purpose.**
 * `lib/document-scroll-lock.ts` stands down while an `INPUT` holds focus
 * — right for the outline's textareas, exactly wrong for a field living
 * in the chrome: WKWebView then scrolls the *document* to keep the caret
 * above the keyboard, and what it scrolls away is the header the field
 * is in. That is the journal-header-riding-up bug, measured at
 * `scrollTop` 384 with `scrollHeight === clientHeight === 874` and fixed
 * in `51353417`. `PageNameSheet` is `position: fixed`, outside the
 * document flow, so there is nothing to scroll off.
 *
 * **{@link PageRename.start} arms the keyboard before the sheet
 * renders.** `PageNameSheet` focuses its input inside a
 * `requestAnimationFrame`, one frame past the tap's gesture window, and
 * WKWebView raises the keyboard only for a focus inside that window
 * (`lib/keyboard-keeper.ts`). From the switcher the sheet gets away with
 * it by accident — the search box is `autofocus`, so the keyboard is
 * already up and iOS sees an input→input move. From the header the
 * keyboard is down, so this is the documented `CaptureSheet` pattern:
 * arm synchronously, let the focus land late.
 */

import { createSignal } from "solid-js";
import type { PageMeta, PageView } from "@outl/shared/api/types";
import { createConfirmQueue } from "@outl/shared/confirm";
import {
  mergeSummary,
  nameTakenMessage,
  renameOrMerge,
  renameSummary,
} from "@outl/shared/pages/rename-merge";

import { armKeyboard } from "../lib/keyboard-keeper";
import { ConfirmDialog } from "./ConfirmDialog";
import { PageNameSheet } from "./PageNameSheet";

export interface PageRenameDeps {
  /** The page on screen. `null`, or a journal, and nothing opens. */
  view: () => PageView | null;
  /** Navigate to whatever the rename or the merge landed on. */
  open: (slug: string, kind: "page" | "journal") => void;
  /** The summary, on the toast's success tone. */
  notify: (message: string) => void;
  /** A failure from the backend, on the toast's destructive tone. */
  onError: (message: string) => void;
}

/**
 * What the merge alert draws. The promise `renameOrMerge` is waiting on
 * is `@outl/shared/confirm`'s, the same queue the page switcher's
 * actions use — the bridge is shared, the chrome is not.
 */
interface MergeQuestion {
  message: string;
}

/** What the sheet was opened on, frozen at that moment (see `commit`). */
interface RenameTarget {
  slug: string;
  title: string;
}

export function createPageRename(deps: PageRenameDeps) {
  const [target, setTarget] = createSignal<RenameTarget | null>(null);
  const merges = createConfirmQueue<MergeQuestion>();
  const question = merges.pending;

  /**
   * Open the name sheet on the page currently on screen.
   *
   * **Call this synchronously from the tap handler.** `armKeyboard()` is
   * only a keyboard while the gesture window is open; after an `await`
   * it is a no-op with extra steps.
   */
  function start() {
    const page = deps.view()?.page;
    // A journal's slug IS its date, so renaming one is not a thing that
    // exists — the backend refuses it too (`CannotRenameJournal`), and a
    // door that leads to a refusal is worse than no door.
    if (!page || page.kind !== "page") return;
    armKeyboard();
    setTarget({ slug: page.slug, title: page.title || page.slug });
  }

  function cancel() {
    setTarget(null);
  }

  const ask = (message: string): Promise<boolean> => merges.ask({ message });

  /** Answer the merge question. `false` on cancel, and on an unmount. */
  const answer = merges.answer;

  /**
   * Commit the typed name through the shared flow.
   *
   * Against `target()`, not against `deps.view()`: a peer's op or a deep
   * link can replace the page on screen while the sheet is open, and the
   * rename has to mean the page the user tapped.
   */
  async function commit(newName: string) {
    const page = target();
    setTarget(null);
    if (!page) return;
    // `PageNameSheet` already disables the button for an unchanged name.
    // This is the case it cannot see — the title moved under an open
    // sheet — and it is the difference between no IPC at all and a
    // round-trip that renames nothing.
    if (newName.trim() === page.title.trim()) return;
    try {
      const result = await renameOrMerge(page.slug, newName, (existing: PageMeta) =>
        ask(nameTakenMessage(existing.title || existing.slug)),
      );
      if (result.kind === "cancelled") return;
      deps.open(result.view.page.slug, result.view.page.kind);
      deps.notify(
        result.kind === "renamed"
          ? renameSummary(result.report)
          : mergeSummary(result.report).join("\n"),
      );
    } catch (e) {
      deps.onError(e instanceof Error ? e.message : String(e));
    }
  }

  return { target, question, start, cancel, commit, answer };
}

export type PageRename = ReturnType<typeof createPageRename>;

/**
 * The name sheet and the merge confirmation.
 *
 * Props are read lazily (`props.rename`, never destructured) because
 * Solid's reactivity rides the getter.
 */
export function PageRenameOverlays(props: { rename: PageRename }) {
  const r = () => props.rename;
  return (
    <>
      <PageNameSheet
        open={r().target() !== null}
        title="Rename page"
        initial={r().target()?.title ?? ""}
        submitLabel="Rename"
        onSubmit={(name) => void r().commit(name)}
        onCancel={() => r().cancel()}
      />
      <ConfirmDialog
        open={r().question() !== null}
        title="Name already in use"
        message={r().question()?.message ?? ""}
        confirmLabel="Merge"
        onConfirm={() => r().answer(true)}
        onCancel={() => r().answer(false)}
      />
    </>
  );
}
