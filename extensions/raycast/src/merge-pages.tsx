/**
 * Merge Pages: fold one page into another.
 *
 * Lists pages with `outl --workspace <ws> page list --json`, asks for a
 * source and a target, confirms with a destructive alert, then runs
 * `outl --workspace <ws> page merge <source> <target> --confirm --json`.
 * What a merge does (move blocks, merge properties, rewrite references,
 * keep the source's names as aliases, trash the source) is owned by the
 * CLI; this file only collects the two slugs and reports the result.
 * The merge is safe to run again, so a retry after a failure is fine.
 */

import React, { useEffect, useState } from "react";
import {
  Action,
  ActionPanel,
  Alert,
  confirmAlert,
  Form,
  Icon,
  open,
  popToRoot,
  showToast,
  Toast,
} from "@raycast/api";
import { runOutl } from "./lib/cli";
import { pageLink } from "./lib/deeplink";
import { showErrorToast } from "./lib/errors";

/** One entry of `page list`, and the `source` / `target` of `page merge`. */
interface PageMeta {
  id: string;
  slug: string;
  title: string;
  kind: string;
}

/** The `data` payload of `outl page list`. */
interface PageListData {
  pages?: PageMeta[];
}

/** A property both pages held with different values (the target's wins). */
interface PropertyConflict {
  key: string;
  kept: string;
  dropped: string;
}

/** The `data` payload of `outl page merge`. */
interface PageMergeData {
  outcome: string;
  source: PageMeta;
  target: PageMeta;
  moved_blocks: number;
  rewritten_refs: string[];
  property_conflicts: PropertyConflict[];
  aliases_added: string[];
  skipped_namespace_refs: unknown[];
  already_merged: boolean;
  nothing_changed: boolean;
}

/** `Title (slug)` when the two differ, so same-titled pages stay apart. */
function labelFor(page: PageMeta): string {
  const title = page.title || page.slug;
  return title === page.slug ? title : `${title} (${page.slug})`;
}

/** `1 block`, `2 blocks`. */
function plural(count: number, noun: string): string {
  return `${count} ${noun}${count === 1 ? "" : "s"}`;
}

/** One-line summary of what a merge changed. */
function summarize(data: PageMergeData): string {
  const parts = [
    `${plural(data.moved_blocks, "block")} moved`,
    `${plural(data.rewritten_refs.length, "reference")} rewritten`,
  ];
  const skipped = data.skipped_namespace_refs.length;
  if (skipped > 0) {
    parts.push(`${plural(skipped, "namespace reference")} left alone`);
  }
  return parts.join(", ");
}

/** Success toast for a finished merge, naming any property conflicts. */
async function showMergedToast(data: PageMergeData): Promise<void> {
  const targetSlug = data.target.slug;
  const primaryAction = {
    title: "Open Target in Outl",
    onAction: async () => {
      await open(pageLink(targetSlug));
    },
  };

  if (data.nothing_changed) {
    await showToast({
      style: Toast.Style.Success,
      title: "Already merged",
      message: `${data.source.slug} is already folded into ${targetSlug}`,
      primaryAction,
    });
    return;
  }

  const conflictKeys = data.property_conflicts.map((c) => c.key);
  if (conflictKeys.length > 0) {
    // Raycast has no warning toast style: keep Success (the merge did
    // happen) and put the keys whose source value was dropped in the title.
    await showToast({
      style: Toast.Style.Success,
      title: `Merged, kept target values for: ${conflictKeys.join(", ")}`,
      message: summarize(data),
      primaryAction,
    });
    return;
  }

  await showToast({
    style: Toast.Style.Success,
    title: "Pages merged",
    message: summarize(data),
    primaryAction,
  });
}

export default function MergePages(): React.JSX.Element {
  const [pages, setPages] = useState<PageMeta[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [submitting, setSubmitting] = useState<boolean>(false);
  const [source, setSource] = useState<string>("");
  const [target, setTarget] = useState<string>("");
  const [sourceError, setSourceError] = useState<string | undefined>();
  const [targetError, setTargetError] = useState<string | undefined>();

  useEffect(() => {
    let cancelled = false;
    runOutl<PageListData>(["page", "list"])
      .then((result) => {
        if (!cancelled) {
          const sorted = [...(result.pages ?? [])].sort((a, b) =>
            labelFor(a).localeCompare(labelFor(b)),
          );
          setPages(sorted);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          void showErrorToast(err);
        }
      })
      .finally(() => {
        if (!cancelled) {
          setLoading(false);
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function changeSource(value: string) {
    setSource(value);
    setSourceError(undefined);
    // The target list drops the source, so a target that just became the
    // source would no longer be one of its options.
    if (value === target) {
      setTarget("");
    }
  }

  function changeTarget(value: string) {
    setTarget(value);
    setTargetError(undefined);
  }

  async function handleSubmit() {
    let valid = true;
    if (source === "") {
      setSourceError("Pick the page to merge away");
      valid = false;
    }
    if (target === "") {
      setTargetError("Pick the page that keeps the content");
      valid = false;
    } else if (target === source) {
      setTargetError("The target must be a different page");
      valid = false;
    }
    if (!valid) {
      return;
    }

    const sourcePage = pages.find((p) => p.slug === source);
    const targetPage = pages.find((p) => p.slug === target);
    const sourceName = sourcePage ? labelFor(sourcePage) : source;
    const targetName = targetPage ? labelFor(targetPage) : target;

    const confirmed = await confirmAlert({
      title: "Merge pages?",
      message:
        `Merging "${sourceName}" into "${targetName}":\n` +
        `- every block of "${sourceName}" moves under "${targetName}"\n` +
        `- every [[${source}]] and #${source} reference is rewritten to "${target}"\n` +
        `- "${sourceName}" is moved to the trash`,
      icon: Icon.Warning,
      primaryAction: {
        title: "Merge Pages",
        style: Alert.ActionStyle.Destructive,
      },
    });
    if (!confirmed) {
      return;
    }

    setSubmitting(true);
    try {
      const data = await runOutl<PageMergeData>([
        "page",
        "merge",
        source,
        target,
        "--confirm",
      ]);
      await showMergedToast(data);
      await popToRoot();
    } catch (err) {
      await showErrorToast(err);
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <Form
      isLoading={loading || submitting}
      actions={
        <ActionPanel>
          <Action.SubmitForm
            title="Merge Pages"
            icon={Icon.ArrowRight}
            onSubmit={handleSubmit}
          />
        </ActionPanel>
      }
    >
      <Form.Dropdown
        id="source"
        title="Source"
        info="The page to fold away. Its blocks move to the target, then it is moved to the trash."
        value={source}
        onChange={changeSource}
        error={sourceError}
      >
        <Form.Dropdown.Item value="" title="Select a page" />
        {pages.map((p) => (
          <Form.Dropdown.Item
            key={p.slug}
            value={p.slug}
            title={labelFor(p)}
            keywords={[p.slug, p.title]}
          />
        ))}
      </Form.Dropdown>
      <Form.Dropdown
        id="target"
        title="Target"
        info="The page that keeps the content. References to the source are rewritten to point here."
        value={target}
        onChange={changeTarget}
        error={targetError}
      >
        <Form.Dropdown.Item value="" title="Select a page" />
        {pages
          .filter((p) => p.slug !== source)
          .map((p) => (
            <Form.Dropdown.Item
              key={p.slug}
              value={p.slug}
              title={labelFor(p)}
              keywords={[p.slug, p.title]}
            />
          ))}
      </Form.Dropdown>
    </Form>
  );
}
