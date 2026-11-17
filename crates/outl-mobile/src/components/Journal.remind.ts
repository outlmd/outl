/**
 * Authoring a block's `remind::` rule: which block the sheet is open
 * on, the rule it opened with, and writing the answer back.
 *
 * A sibling module rather than three more closures in `Journal.tsx`,
 * for the reason `Journal.embeds.ts` is one: **it owns the signal
 * too**, so the component holds no state for this feature and the
 * sequence (read the current rule → open → write or clear) can be
 * driven by a test without a webview.
 *
 * No grammar and no schedule live here. `<RemindSheet />` turns the
 * controls into rule text through `outl_md::remind`, and *when* the
 * rule fires is `outl_actions::reminders`'.
 */

import { createSignal } from "solid-js";

import { listReminders, setBlockRemind } from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";

/** The block the sheet is authoring for, and the rule it opened on. */
export interface RemindTarget {
  blockId: string;
  rule: string;
}

export interface RemindDeps {
  /** Accessor, never a value: a destructured prop freezes in Solid. */
  pageId: () => string | null;
  applyView: (view: PageView) => void;
  onError: (message: string) => void;
}

export function createRemindAuthoring(deps: RemindDeps) {
  const [target, setTarget] = createSignal<RemindTarget | null>(null);

  /**
   * Open the sheet on a block, carrying whatever rule it already has.
   *
   * The lookup happens **before** the sheet opens, and the value is
   * carried as text rather than as parsed fields, because a rule that
   * does not parse still has to reach the user — opening a picker on
   * a default would show them a time that has nothing to do with what
   * their block says. `listReminders` is the only reader of that value
   * this client has; a failure there opens the sheet empty rather than
   * refusing to open it, since an empty sheet still authors a rule.
   */
  async function open(blockId: string): Promise<void> {
    if (!deps.pageId()) return;
    const rule =
      (await listReminders().catch(() => [])).find((r) => r.block_id === blockId)?.rule ?? "";
    setTarget({ blockId, rule });
  }

  /**
   * Write what the sheet decided. An empty rule clears the property,
   * which is how a block stops nagging without being deleted.
   *
   * The sheet closes first: the write is a round-trip, and leaving it
   * up until the reply lands reads as a Save that did nothing.
   */
  async function submit(rule: string): Promise<void> {
    const pid = deps.pageId();
    const blockId = target()?.blockId;
    setTarget(null);
    if (!pid || !blockId) return;
    try {
      deps.applyView(await setBlockRemind(pid, blockId, rule));
    } catch (e) {
      deps.onError(e instanceof Error ? e.message : String(e));
    }
  }

  return {
    target,
    open,
    submit,
    close: () => setTarget(null),
  };
}
