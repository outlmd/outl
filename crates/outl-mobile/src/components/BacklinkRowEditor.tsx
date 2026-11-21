import { JSX, createSignal } from "solid-js";

import { EditableTextarea } from "./BlockTextarea";

/**
 * A backlink row in its editing state: one draft over
 * `EditableTextarea`, the client's own editor, so a cross-page edit
 * types the way every other block on this phone types (the IME guard,
 * the auto-pairs, the double `parkCaret` around a programmatic
 * `value =`).
 *
 * **The draft is seeded once**, like every editor in this client, so a
 * re-render cannot write over what the user is typing.
 *
 * What that does **not** buy is survival across a rows swap: `<For>` is
 * keyed by object reference and every `page_backlinks` reply hands it
 * fresh `Backlink` objects, so a reply landing mid-edit would rebuild
 * this component and re-seed the draft from the new row.
 *
 * **That is latent, not live, and the reason is load-bearing: nothing
 * refetches the backlinks in the background.** `Journal.tsx`'s
 * `createResource` is keyed on `createBacklinksKey` (slug + `title::`),
 * and neither `pullAndReload` nor `applyView` changes either — a peer
 * reload rewrites the page object with the same slug and title, so the
 * key does not refire and no rows are swapped. The only re-ask is the
 * one a commit through this card triggers, and that one carries the
 * text the user just typed.
 *
 * So if you add a `pageBacklinks` call behind a poll, a peer event or a
 * foreground refresh, you have just made this reachable: a sync would
 * then drop an in-flight draft silently. Closing it properly needs a
 * keyed list and a draft owned above the row.
 *
 * **Blur is the one commit point.** The commit runs a projection
 * synchronously on the source page and asks for the rows again
 * afterwards (`pageBacklinks` is an O(blocks-in-workspace) scan), so
 * committing per keystroke would make the expensive read the cheapest
 * gesture there is — the same contract `commitEdit` has in
 * `Journal.tsx`.
 *
 * The host keeps this mounted until those fresh rows are in, so the
 * row never shows its pre-edit text in the gap. During it the textarea
 * is blurred but still on screen, which is why the row around it goes
 * `pointer-events-none` while the write is in flight: a tap back in and
 * out would otherwise fire a second concurrent write
 * (`BacklinksSection.row-ops.ts`).
 *
 * Nothing that walks the outline is wired: a cross-block arrow step,
 * Backspace-deletes-the-block and an outline paste all act on
 * neighbours that are not on this screen. A row's siblings live on
 * another page, so there is nowhere for those gestures to land.
 */
export function BacklinkRowEditor(props: {
  /** Wire-format text, task prefix included (`rawTextWithTodo`). */
  text: string;
  /** Called on blur with the draft. The host decides what to write. */
  onCommit: (text: string) => void;
}): JSX.Element {
  const [draft, setDraft] = createSignal(props.text);

  return (
    <div class="flex-1">
      <EditableTextarea
        value={draft()}
        onInput={setDraft}
        onBlur={() => props.onCommit(draft())}
      />
    </div>
  );
}
