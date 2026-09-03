import { JSX, Show } from "solid-js";

export type SyncStatus = "synced" | "syncing" | "offline";

interface SyncDotProps {
  status: SyncStatus;
}

const STATUS_LABEL: Record<SyncStatus, string> = {
  synced: "Synced",
  syncing: "Syncing",
  offline: "Offline",
};

/**
 * Small status indicator next to the refresh button. Green dot when
 * the workspace is in sync, blue spinner while a sync is in flight,
 * orange dot when offline.
 *
 * The status is also announced via `aria-label` because `title=`
 * tooltips don't render on iOS WKWebView — without the label,
 * colour-blind users get a grey-ish dot they can't interpret.
 */
export function SyncDot(props: SyncDotProps): JSX.Element {
  return (
    <span
      role="status"
      aria-live="polite"
      aria-label={`Sync status: ${STATUS_LABEL[props.status]}`}
      title={STATUS_LABEL[props.status]}
      class="inline-flex h-2.5 w-2.5 items-center justify-center"
    >
      <Show
        when={props.status === "syncing"}
        fallback={
          <span
            aria-hidden="true"
            class="h-2 w-2 rounded-full"
            style={{
              background:
                props.status === "synced"
                  ? "#34c759"
                  : "#ff9500",
            }}
          />
        }
      >
        <span
          aria-hidden="true"
          class="h-2.5 w-2.5 animate-spin rounded-full border-2 border-(--color-outl-accent) border-t-transparent"
        />
      </Show>
    </span>
  );
}
