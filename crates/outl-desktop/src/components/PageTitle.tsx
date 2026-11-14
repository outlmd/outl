import { Show } from "solid-js";

import type { PageMeta } from "@outl/shared/api/types";
import { journalHeading } from "@outl/shared/journal";

import { appState } from "../lib/store";

/**
 * The words inside the page header's `<h1>`. A journal reads as a day
 * (`Today`, `Yesterday`, `Thursday`) with the full date beside it; its
 * ISO slug is the file name and lives in the eyebrow instead. A page
 * shows its title. Today takes the accent, the one place the header
 * uses it.
 *
 * The day anchor is `appState.todaySlug`, written by `<AppShell />`'s
 * day clock. It used to be a `createResource` keyed on the page slug,
 * which re-asked on navigation and never otherwise — so a window left
 * open past midnight kept calling yesterday "Today" until the user
 * navigated somewhere. Never re-derive the day from `new Date()` here:
 * the slug is resolved in the configured `[calendar] timezone`, which
 * the browser clock does not know about.
 */
export function PageTitle(props: { page: PageMeta }) {
  const heading = () =>
    props.page.kind === "journal"
      ? journalHeading(props.page.slug, appState.todaySlug)
      : null;
  return (
    <Show when={heading()} fallback={props.page.title || props.page.slug}>
      {(h) => (
        <>
          <span
            class={
              h().title === "Today" ? "text-(--color-outl-accent)" : undefined
            }
          >
            {h().title}
          </span>
          <span class="ml-2.5 text-[15px] font-normal tracking-normal text-(--color-outl-fg-dim)">
            {h().date}
          </span>
        </>
      )}
    </Show>
  );
}
