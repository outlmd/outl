/**
 * The context every desktop action handler runs in.
 *
 * Three questions the handler groups all have to answer the same way,
 * and would each answer slightly differently if they owned it:
 * **which block is this keystroke aimed at**, **how does a `PageView`
 * reply reach the screen**, and **how does a failure reach the status
 * bar**. One answer each, here.
 *
 * `safeCall` / `runOn` close over the shell's callbacks, so they come
 * out of `createHandlerCtx`. `targetBlockId` / `lookupBlock` read the
 * global store and nothing else, so they are plain exports — wrapping
 * them in the factory would imply a dependency they do not have.
 */

import type { BlockNode, PageView } from "@outl/shared/api/types";

import { appState, setAppState } from "./store";

export interface DesktopHandlerDeps {
  applyView: (view: PageView) => void;
  setError: (msg: string) => void;
}

/** What a handler group receives. Plain record, built once per
 *  `buildHandlers` — not Solid props, so destructuring it in a group
 *  module is safe (no getter to freeze). */
export interface HandlerCtx {
  deps: DesktopHandlerDeps;
  safeCall: <T>(p: Promise<T>) => Promise<T | undefined>;
  runOn: (cmd: Promise<PageView>, nextSelectedId?: string | null) => Promise<void>;
  runEach: (
    ids: readonly string[],
    op: (id: string) => Promise<PageView>,
  ) => Promise<void>;
}

/** Resolve the block the user means right now — the focused
 *  textarea's id (Insert) takes priority over the selection
 *  cursor (Normal). Returns `null` when neither is set. */
export function targetBlockId(): string | null {
  const el = document.activeElement;
  if (el instanceof HTMLTextAreaElement && el.dataset.blockId) {
    return el.dataset.blockId;
  }
  return appState.selectedBlockId;
}

/** Find a block in the outline by id; deep search. */
export function lookupBlock(id: string): BlockNode | null {
  const walk = (bs: BlockNode[]): BlockNode | null => {
    for (const b of bs) {
      if (b.id === id) return b;
      if (b.children.length > 0) {
        const hit = walk(b.children);
        if (hit) return hit;
      }
    }
    return null;
  };
  return walk(appState.outline);
}

export function createHandlerCtx(deps: DesktopHandlerDeps): HandlerCtx {
  const safeCall = async <T>(p: Promise<T>): Promise<T | undefined> => {
    try {
      return await p;
    } catch (e) {
      deps.setError(e instanceof Error ? e.message : String(e));
      return undefined;
    }
  };

  /** Common pattern: await a Tauri command that returns a
   *  `PageView`, apply it, and leave selection on `nextSelectedId`
   *  (or keep current when `undefined`). */
  async function runOn(cmd: Promise<PageView>, nextSelectedId?: string | null) {
    const view = await safeCall(cmd);
    if (!view) return;
    deps.applyView(view);
    if (nextSelectedId !== undefined) {
      setAppState("selectedBlockId", nextSelectedId);
    }
  }

  /** Fire `op` once per id, in the order given, and paint only the
   *  last view that came back. Every batch action is this shape — fold
   *  all, indent a Visual range, delete one — and they agree on two
   *  decisions that are easy to take differently: **the intermediate
   *  views are dropped** (painting each one makes the page flicker
   *  through states nobody asked to see), and **a failing id does not
   *  stop the walk**. The second is load-bearing for `DeleteRange`: a
   *  range covering a parent and its children sees the descendants
   *  refuse with "already in trash" once the parent's subtree moved,
   *  and stranding the rest of the range on that is worse than the
   *  status-line error `safeCall` already showed. */
  async function runEach(
    ids: readonly string[],
    op: (id: string) => Promise<PageView>,
  ) {
    let lastView: PageView | undefined;
    for (const id of ids) {
      const view = await safeCall(op(id));
      if (view) lastView = view;
    }
    if (lastView) deps.applyView(lastView);
  }

  return { deps, safeCall, runOn, runEach };
}
