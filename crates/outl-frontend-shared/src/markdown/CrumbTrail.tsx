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
 * ## Inert by default, the client opts in
 *
 * With no handler the trail is context only: a link renders as an inert
 * span (`MarkdownInline`'s rule for a handler-less link), and a ref or a
 * tag keeps its look but goes nowhere. A client that makes the whole
 * line a target (a backlink row's "open the page at this block") passes
 * `onRefClick` / `onTagClick` as well, and the reason is a click, not
 * styling: `MarkdownInline` returns early for a ref or tag with no
 * handler and does **not** stop the propagation, so a `[[ref]]` that
 * looks pressable would fall through to the container's handler and open
 * the wrong thing. With a handler it navigates and stops there.
 *
 * `onLinkClick` stays unwired on purpose: an external URL inside a
 * breadcrumb is a footnote, and opening a browser from the line that
 * means "go to the block" would be a surprise.
 *
 * `variant="inline"` also means `blockAssets` defaults to `false`, so
 * an image in an ancestor is a chip rather than a picture opening
 * above the reference it is meant to be a footnote for.
 */
export function CrumbTrail(props: {
  crumbs: readonly TrailCrumb[];
  /** Opt-in: a `[[ref]]` in the trail navigates instead of falling
   *  through to the container. */
  onRefClick?: (target: string) => void;
  /** Opt-in: the `#tag` twin of {@link onRefClick}. */
  onTagClick?: (tag: string) => void;
}): JSX.Element {
  return (
    <For each={props.crumbs}>
      {(crumb, index) => (
        <>
          {index() > 0 ? CRUMB_SEPARATOR : null}
          <MarkdownInline
            tokens={crumb.tokens}
            variant="inline"
            onRefClick={props.onRefClick}
            onTagClick={props.onTagClick}
          />
        </>
      )}
    </For>
  );
}
