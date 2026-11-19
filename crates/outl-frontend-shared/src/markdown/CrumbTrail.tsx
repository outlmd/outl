import { For, JSX } from "solid-js";

import type { InlineToken } from "../api/types";
import { MarkdownInline } from "./MarkdownInline";

/** What the trail needs off a crumb. Structural rather than
 *  `BacklinkCrumb`, mirroring `sameCrumbTrail` next door, which is
 *  generic over `{ id }` for the same reason.
 *
 *  Module-level, not on the barrel: both clients pass `link.ancestors`
 *  and annotate nothing. */
export interface TrailCrumb {
  text: string;
  tokens: InlineToken[];
}

/** The separator between two crumbs. Both GUI clients and the TUI drew
 *  this character when the trail was a plain `join`; it stays one
 *  owner on this side so the two cannot drift. The TUI keeps its own
 *  literal — a const does not cross the Rust/TS boundary.
 *
 *  Module-level, not on the barrel: no client calls it. */
export const CRUMB_SEPARATOR = " › ";

/**
 * A crumb trail as plain text — what a `title=` tooltip reads, and the
 * fallback for anywhere a tooltip is all there is room for.
 *
 * Deliberately **not** what the trail renders: the rendered form is
 * `<CrumbTrail />` below.
 */
export function crumbTrailText(crumbs: readonly TrailCrumb[]): string {
  return crumbs.map((c) => c.text).join(CRUMB_SEPARATOR);
}

/**
 * The ancestor breadcrumb above a backlink row, rendered.
 *
 * ## Why this is rendered at all
 *
 * The backend called a crumb "dimmed context" and shipped it as raw
 * source, so the one line on the panel describing *where* a reference
 * lives was the one line written in markup: a trail read
 * `#alinhamento [[buser/team/vendas]] internet 0800
 * [mail](https://mail.google.com/mail?authuser=…)` while the citing
 * block right under it read as prose. The page name was buried in
 * brackets and a link swallowed the rest of the line with a URL the
 * reader has no use for.
 *
 * Dimmed is a **styling** rule. The caller owns it (`opacity`, a dim
 * token), and it composes with the token colours underneath — the
 * trail stays quieter than the block below it without being unparsed.
 *
 * ## Why it is inert
 *
 * No `onRefClick`, no `onTagClick`, no `onLinkClick`: a crumb is
 * context for the row, and the row already has one destination (the
 * citing block, via its own handler). Wiring the trail would put two
 * targets in one tap area, and on a phone they would overlap.
 * `MarkdownInline` renders a link with no handler as an inert span
 * rather than a dead button, which is exactly what is wanted here.
 *
 * `variant="inline"` also means `blockAssets` defaults to `false`, so
 * an image in an ancestor is a chip rather than a picture opening
 * above the reference it is meant to be a footnote for.
 */
export function CrumbTrail(props: {
  crumbs: readonly TrailCrumb[];
}): JSX.Element {
  return (
    <For each={props.crumbs}>
      {(crumb, index) => (
        <>
          {index() > 0 ? CRUMB_SEPARATOR : null}
          <MarkdownInline tokens={crumb.tokens} variant="inline" />
        </>
      )}
    </For>
  );
}
