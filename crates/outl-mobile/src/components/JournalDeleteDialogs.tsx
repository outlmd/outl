import type { JSX } from "solid-js";

import { ConfirmDialog } from "./ConfirmDialog";

/** A pending single-block delete: the target plus how many children go
 *  with it, which is the only reason this prompt appears at all. */
export interface PendingBlockDelete {
  id: string;
  descendants: number;
}

/**
 * The two "are you sure" prompts a delete can raise, and the copy that
 * goes in them.
 *
 * Presentational, in the same sense as `JournalChrome`: it owns no
 * state and fires no command. `Journal` keeps the signals and the
 * `performDelete` / `performDeleteRange` calls, and passes what the
 * dialogs need.
 *
 * They live together because the *wording* is the shared part —
 * pluralising a count, warning about children, saying the thing can't
 * be undone — and two copies of that drift into saying different
 * things about the same action. The markup is one `<ConfirmDialog />`
 * each and would not have been worth extracting on its own.
 *
 * Props are read lazily (`props.x`, never destructured): Solid's
 * reactivity rides the getter, so a destructured prop would freeze
 * these at their first render, which for a dialog means it never
 * opens.
 */
export function JournalDeleteDialogs(props: {
  /** Single-block delete awaiting confirmation, `null` when none. */
  pendingBlock: PendingBlockDelete | null;
  /** Range delete awaiting confirmation (snapshotted ids), `null` when none. */
  pendingRange: string[] | null;
  onCancelBlock: () => void;
  onConfirmBlock: (id: string) => void;
  onCancelRange: () => void;
  onConfirmRange: (ids: string[]) => void;
}): JSX.Element {
  return (
    <>
      <ConfirmDialog
        open={props.pendingBlock !== null}
        title="Delete block?"
        message={blockDeleteMessage(props.pendingBlock)}
        onCancel={props.onCancelBlock}
        onConfirm={() => {
          const pending = props.pendingBlock;
          props.onCancelBlock();
          if (pending) props.onConfirmBlock(pending.id);
        }}
      />

      <ConfirmDialog
        open={props.pendingRange !== null}
        title="Delete blocks?"
        message={rangeDeleteMessage(props.pendingRange)}
        onCancel={props.onCancelRange}
        onConfirm={() => {
          const ids = props.pendingRange;
          props.onCancelRange();
          if (ids) props.onConfirmRange(ids);
        }}
      />
    </>
  );
}

/** Copy for the single-block prompt. Only ever shown when the block has
 *  children — a childless delete doesn't ask. */
function blockDeleteMessage(pending: PendingBlockDelete | null): string {
  if (!pending) return "";
  const noun = pending.descendants === 1 ? "child" : "children";
  return `This block has ${pending.descendants} ${noun} that will also be deleted. This can't be undone.`;
}

/** Copy for the range prompt. A range delete always confirms, so this
 *  one can be reached with a single block selected. */
function rangeDeleteMessage(ids: string[] | null): string {
  if (!ids) return "";
  const noun = ids.length === 1 ? "block" : "blocks";
  return `This will delete ${ids.length} ${noun}, including any nested children. This can't be undone.`;
}
