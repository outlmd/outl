/**
 * The desktop's one move gate, plus the held-key flag.
 *
 * `moveOnce` is shared by the textarea's move chord, the Normal and
 * Visual handlers: a move reads edges the previous one has not
 * repainted, so one gate has to cover all three.
 *
 * `editorOwnsHold` closes the hole the gate cannot: a hold inside a
 * backlink editor commits, the textarea unmounts, and the repeats then
 * reach the window dispatcher with nothing focused. The editor sets the
 * flag on the first press, the dispatcher swallows repeats while it is
 * up and clears it on the next non-repeat move chord.
 */
import { createMoveGate } from "@outl/shared/outline";

export const moveOnce = createMoveGate();

let ownsHold = false;

export function editorOwnsHold(): boolean {
  return ownsHold;
}

export function setEditorOwnsHold(v: boolean): void {
  ownsHold = v;
}
