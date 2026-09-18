/**
 * Everything the journal listens to from outside the webview — the
 * platform-wiring half of issue #265 phase 3.
 *
 * Three listeners, one shape, and that shape has exactly one
 * implementation: {@link armCleanup}. It arms `onCleanup` **synchronously**
 * so the handle is torn down even if the component unmounts before the
 * dynamic `import()` resolves, then disposes a late-arriving handle on the
 * spot. Registering cleanup inside the `.then()` instead leaks the listener
 * on a fast unmount, and it is invisible until something unmounts — which
 * on mobile is "never" right up until it isn't.
 *
 * `Journal.listeners.test.ts` drives that window directly: it holds the
 * dynamic import open, unmounts, and then releases it. It also keeps the
 * callback each `listen()` was handed, so the listener **bodies** are
 * driven too — without that, the mid-edit guard below, the
 * `workspace-ready` branch and the refusal routing could all be deleted
 * with the suite still green.
 *
 * Every function here must be called from inside the component body:
 * `onCleanup` needs a Solid owner, and there is deliberately no
 * `runWithOwner` wrapper hiding that requirement.
 */

import { onCleanup } from "solid-js";

import type { PageView, ProjectionWriteFailed } from "@outl/shared/api/types";
import {
  openJournalFor,
  openPageBySlug,
  openTodayJournal,
} from "@outl/shared/api/commands";
import { installFileDrop } from "@outl/shared/drag-drop";

/**
 * Payload shapes emitted by the backend's `deep-link://navigate` event
 * (and buffered for cold start via `take_pending_deep_link`) — issue #98.
 */
export type DeepLinkNavigate =
  | { kind: "today" }
  | { kind: "daily"; date: string }
  | { kind: "page"; slug: string };

/** What the listeners need from `Journal`. Accessors, never values. */
export interface JournalListenerDeps {
  view: () => PageView | null;
  editingId: () => string | null;
  applyView: (v: PageView) => void;
  setError: (message: string | null) => void;
  setAheadOfLog: (notice: { slug: string; info: NonNullable<ProjectionWriteFailed["md_ahead_of_log"]> }) => void;
  /** Open today's journal, retrying while the workspace is still booting. */
  loadTodayWithRetry: () => Promise<void>;
  /** The guarded reload path — pull, then re-render behind its generation guard. */
  pullAndReload: (opts: { background: boolean }) => Promise<void>;
  /** Import dropped files against the block under the drop point. */
  onFileDrop: (paths: string[], blockId: string | null) => void;
}

/**
 * Arm the unmount cleanup **now**, and hand back the one way to register
 * a handle with it.
 *
 * This is the module's whole reason to exist, and it was written out
 * three times — once per listener — which is one copy per chance to get
 * it wrong. `onCleanup` runs synchronously here, before any dynamic
 * `import()` has resolved, so a component that unmounts mid-import is
 * already covered; a handle that lands afterwards is disposed on the
 * spot instead of being pushed onto a list nobody will walk again.
 *
 * Must be called from inside a Solid owner — see the module doc.
 */
function armCleanup(): (unlisten: () => void) => void {
  let disposed = false;
  const handles: Array<() => void> = [];
  onCleanup(() => {
    disposed = true;
    for (const unlisten of handles) unlisten();
  });
  return (unlisten) => {
    if (disposed) unlisten();
    else handles.push(unlisten);
  };
}

/**
 * Navigate to whatever an `outl://` link addressed.
 *
 * Exported on its own because the cold-start path calls it directly
 * with a buffered payload, without a listener in between.
 */
export async function navigateDeepLink(
  deps: Pick<JournalListenerDeps, "applyView" | "setError">,
  p: DeepLinkNavigate,
): Promise<void> {
  try {
    const next =
      p.kind === "today"
        ? await openTodayJournal()
        : p.kind === "daily"
          ? await openJournalFor(p.date)
          : await openPageBySlug(p.slug);
    deps.applyView(next);
    deps.setError(null);
  } catch (err) {
    deps.setError(String(err));
  }
}

/**
 * The warm path for `outl://` links — the app is already running and
 * the backend emits `deep-link://navigate`.
 *
 * Skips while a block is being edited so a navigation never yanks the
 * textarea out from under the user mid-keystroke.
 */
export function listenForDeepLink(deps: JournalListenerDeps): void {
  const keep = armCleanup();
  void import("@tauri-apps/api/event")
    .then(({ listen }) =>
      listen<DeepLinkNavigate>("deep-link://navigate", async (e) => {
        if (deps.editingId()) return;
        await navigateDeepLink(deps, e.payload);
      }),
    )
    .then(keep);
}

/**
 * Wire the Tauri webview drag-and-drop event (iPad: drag a file from the
 * Files app or split-view onto a block).
 *
 * Best-effort: on iPhone the OS rarely delivers a webview drop, and a
 * registration failure just leaves the long-press "Attach file" action
 * as the import path. The shared `installFileDrop` resolves the block
 * under the drop point (physical→CSS pixels, `data-block-id` hit-test)
 * identically to the desktop, so the two clients can't drift on the
 * geometry.
 */
export function listenForFileDrop(deps: JournalListenerDeps): void {
  const keep = armCleanup();
  installFileDrop({
    onDrop: (paths, blockId) => deps.onFileDrop(paths, blockId),
  })
    .then(keep)
    .catch((e) => {
      console.warn("failed to register drag-drop listener", e);
    });
}

/**
 * The two backend events that change what the page should be showing.
 *
 * `projection-write-failed` carries a refusal the user must see. When it
 * names the open page it becomes that page's sticky banner; otherwise it
 * still reaches the user as an error, because an off-screen refusal
 * froze a page just the same (root `CLAUDE.md` invariant 8 — a refusal
 * swallowed into a log line ships a page that silently stopped syncing).
 *
 * `workspace-ready` fires when peer ops **land**, so there may not be
 * another one: returning early mid-edit threw the signal away and left
 * the view waiting for the 5s poll. `pullAndReload` handles the mid-edit
 * case itself (pull now, re-render when the field closes) and carries
 * the generation guard that stops a slow reload flipping the page back
 * to an older op-log state.
 */
export function listenForWorkspaceReady(deps: JournalListenerDeps): void {
  const keep = armCleanup();

  import("@tauri-apps/api/event")
    .then(async ({ listen }) => {
      keep(
        await listen<ProjectionWriteFailed>("projection-write-failed", (event) => {
          const failure = event.payload;
          const current = deps.view();
          if (failure.md_ahead_of_log && current?.page.id === failure.page_id) {
            deps.setAheadOfLog({
              slug: current.page.slug,
              info: failure.md_ahead_of_log,
            });
          } else {
            deps.setError(failure.error);
          }
        }),
      );

      keep(
        await listen("workspace-ready", async () => {
          if (!deps.view()) {
            await deps.loadTodayWithRetry();
            return;
          }
          await deps.pullAndReload({ background: true });
        }),
      );
    })
    .catch((error) => {
      console.warn("failed to register workspace listeners", error);
    });
}
