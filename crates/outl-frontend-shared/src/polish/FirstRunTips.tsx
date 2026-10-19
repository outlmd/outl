import { For, Show, createSignal } from "solid-js";

import { TIPS_KEY, dismissOnce, dismissedOnce, firstRunTips, type TipClient } from "./firstRun";

/**
 * The empty-state tour. Renders nothing once dismissed; `onStart` is the
 * client's own "write the first block" action.
 */
export function FirstRunTips(props: { client: TipClient; onStart?: () => void }) {
  const [hidden, setHidden] = createSignal(dismissedOnce(TIPS_KEY));
  const close = () => {
    dismissOnce(TIPS_KEY);
    setHidden(true);
  };
  return (
    <Show when={!hidden()}>
      <section class="outl-tips" aria-label="Getting started">
        <h2>outl in 30 seconds</h2>
        <For each={firstRunTips(props.client)}>
          {(tip) => (
            <div class="outl-tip" data-tip={tip.id}>
              <span class="outl-tip-glyph" aria-hidden="true">
                {tip.glyph}
              </span>
              <div>
                <strong>{tip.title}</strong>
                <div>{tip.body}</div>
              </div>
            </div>
          )}
        </For>
        <div class="outl-tips-actions">
          <button type="button" class="outl-tips-secondary" onClick={close}>
            Got it
          </button>
          <Show when={props.onStart}>
            <button
              type="button"
              class="outl-tips-primary"
              onClick={() => {
                close();
                props.onStart?.();
              }}
            >
              Start writing
            </button>
          </Show>
        </div>
      </section>
    </Show>
  );
}
