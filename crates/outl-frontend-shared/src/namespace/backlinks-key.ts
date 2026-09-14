/**
 * The source signal for a `pageBacklinks` resource.
 *
 * A `PageBacklinks` reply depends on more than the slug: its
 * `namespace_children` hang off `title::`, so a page-property edit that
 * renames `os-linux` to `os/linux` changes which pages nest under it
 * while the slug (and therefore a slug-keyed resource) stays put.
 * Keying on `{ slug, title }` refetches on either change.
 *
 * The memo compares by value, so an ordinary block edit (a new view
 * object with the same slug and title) does not refire the resource.
 */
import { createMemo, type Accessor } from "solid-js";

import type { PageMeta } from "../api/types";

export interface BacklinksKey {
  slug: string;
  title: string;
}

export function backlinksKeyOf(
  page: Pick<PageMeta, "slug" | "title"> | null | undefined,
): BacklinksKey | undefined {
  return page ? { slug: page.slug, title: page.title } : undefined;
}

export function sameBacklinksKey(
  a: BacklinksKey | undefined,
  b: BacklinksKey | undefined,
): boolean {
  return a?.slug === b?.slug && a?.title === b?.title;
}

/**
 * Memoised `{ slug, title }` of the current page, `undefined` while no
 * page is open (which keeps the resource idle, as Solid treats a falsy
 * source). Pass it straight in as a `createResource` source.
 */
export function createBacklinksKey(
  page: Accessor<Pick<PageMeta, "slug" | "title"> | null | undefined>,
): Accessor<BacklinksKey | undefined> {
  return createMemo(() => backlinksKeyOf(page()), undefined, {
    equals: sameBacklinksKey,
  });
}
