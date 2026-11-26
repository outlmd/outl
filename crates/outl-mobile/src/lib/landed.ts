/**
 * The transient "you landed here" highlight, for a block a backlink's
 * breadcrumb opened the page at. A second instance of the saved-mark
 * primitive (own signal, own hold) rather than a reuse of `saved.ts`:
 * that one means "committed to disk" and fades on its own clock.
 *
 * `BlockRow` reads it; `Journal.landing.ts` writes it. The fade itself
 * is CSS (opacity only, static under `prefers-reduced-motion`).
 */
import { createSavedMarks } from "@outl/shared/trust";

export const landedMarks = createSavedMarks(1200);
