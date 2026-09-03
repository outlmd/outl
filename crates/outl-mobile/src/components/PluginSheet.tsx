import { For, Show, createEffect, createSignal } from "solid-js";
import { Portal } from "solid-js/web";

import type { PageView } from "@outl/shared/api/types";

import {
  filterRegistryItems,
  pluginInstallOfficial,
  pluginList,
  pluginRegistryList,
  pluginRun,
  pluginSetEnabled,
  pluginUninstall,
} from "@outl/shared/api/commands";
import type { PluginCommand, RegistryItem } from "@outl/shared/api/types";
import { PluginSettings } from "@outl/shared/plugins";
import { haptic } from "../lib/haptics";

/**
 * `<PluginSheet />` — the mobile plugin surface, two tabs:
 *
 * - **Browse** — the marketplace: the official registry
 *   (`plugins.outl.app/registry.json`, fetched + cross-referenced with the
 *   lockfile by `plugin_registry_list`), tap-to-install
 *   (`plugin_install_official` downloads the bundle from
 *   `plugins.outl.app/p/<id>/`, freezes the hash, reloads the host), plus
 *   enable / disable / remove of installed plugins. Only registry-listed
 *   (official, human-reviewed) plugins install here; an unlisted plugin
 *   installs via the CLI.
 * - **Commands** — the commands installed plugins contribute, tap-to-run
 *   (`plugin_run`). Mobile has no `/` slash surface, so this is how a
 *   command is invoked.
 *
 * The Boa host lives on a dedicated thread (`!Send`); this sheet only talks
 * to it through those Tauri commands.
 */
export function PluginSheet(props: {
  open: boolean;
  /** Id of the page currently on screen, so a run can re-render it. */
  pageId: string | null;
  onClose: () => void;
  /** Surface a plugin notification / error to the parent (toast). */
  onMessage: (text: string) => void;
  /** Refreshed view after a successful run that mutated the workspace. */
  onView: (view: PageView) => void;
  /**
   * `ctx.ui.render(html)` payloads a run emitted — handed up so the parent
   * paints them as sandboxed iframe overlays (`<PluginViewOverlay />`).
   */
  onViews: (views: string[]) => void;
}) {
  const [tab, setTab] = createSignal<"browse" | "commands">("browse");

  // Marketplace state.
  const [items, setItems] = createSignal<RegistryItem[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [query, setQuery] = createSignal("");
  const [busyId, setBusyId] = createSignal<string | null>(null);
  // Which installed plugin's settings panel is expanded (one at a time).
  const [settingsFor, setSettingsFor] = createSignal<string | null>(null);
  const toggleSettings = (id: string) => {
    haptic("light");
    setSettingsFor((cur) => (cur === id ? null : id));
  };

  // Commands state.
  const [commands, setCommands] = createSignal<PluginCommand[]>([]);
  const [busy, setBusy] = createSignal(false);

  async function refreshMarketplace() {
    setLoading(true);
    try {
      setItems(await pluginRegistryList());
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
      setItems([]);
    } finally {
      setLoading(false);
    }
  }

  async function refreshCommands() {
    try {
      setCommands(await pluginList());
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
      setCommands([]);
    }
  }

  // Refresh whatever tab is showing whenever the sheet opens or the tab
  // flips (plugins + the registry load lazily / change as the user acts).
  createEffect(() => {
    if (!props.open) return;
    if (tab() === "browse") void refreshMarketplace();
    else void refreshCommands();
  });

  const filtered = () => filterRegistryItems(items(), query());

  async function withBusy(id: string, fn: () => Promise<unknown>) {
    if (busyId()) return;
    setBusyId(id);
    haptic("light");
    try {
      await fn();
      await refreshMarketplace();
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  }

  const install = (i: RegistryItem) =>
    withBusy(i.id, () => pluginInstallOfficial(i.id));
  const toggle = (i: RegistryItem) =>
    withBusy(i.id, () => pluginSetEnabled(i.id, !i.enabled));
  const remove = (i: RegistryItem) =>
    withBusy(i.id, () => pluginUninstall(i.id));

  async function run(cmd: PluginCommand) {
    if (busy()) return;
    setBusy(true);
    haptic("light");
    try {
      const reply = await pluginRun(cmd.plugin_id, cmd.command_id, props.pageId);
      for (const note of reply.notifications) props.onMessage(note);
      for (const err of reply.errors) props.onMessage(`plugin: ${err}`);
      if (reply.views.length > 0) props.onViews(reply.views);
      if (reply.view) props.onView(reply.view);
      props.onClose();
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Show when={props.open}>
      <Portal>
        <div
          class="fixed inset-0 z-50 flex flex-col justify-end bg-black/40"
          onClick={(e) => {
            if (e.target === e.currentTarget) props.onClose();
          }}
        >
          <div
            class="flex max-h-[80vh] flex-col overflow-hidden rounded-t-2xl bg-(--color-outl-bg-elev)"
            style="padding-bottom: max(env(safe-area-inset-bottom), 16px);"
          >
            <div class="flex justify-center py-2">
              <div class="h-1 w-9 rounded-full bg-(--color-outl-fg-dim)/30" />
            </div>

            {/* Tab switcher */}
            <div class="flex gap-1 px-5 pb-2">
              <TabButton
                label="Browse"
                active={tab() === "browse"}
                onTap={() => setTab("browse")}
              />
              <TabButton
                label="Commands"
                active={tab() === "commands"}
                onTap={() => setTab("commands")}
              />
              <Show when={loading() && tab() === "browse"}>
                <span class="ml-auto self-center text-[12px] text-(--color-outl-fg-dim)">
                  loading…
                </span>
              </Show>
            </div>

            {/* Browse / marketplace */}
            <Show when={tab() === "browse"}>
              <div class="px-5 pb-2">
                <input
                  type="text"
                  inputmode="search"
                  placeholder="Search plugins…"
                  value={query()}
                  onInput={(e) => setQuery(e.currentTarget.value)}
                  class="w-full rounded-lg bg-(--color-outl-border)/30 px-3 py-2 text-[15px] text-(--color-outl-fg) outline-none"
                />
              </div>
              <div class="ios-scroll max-h-[58vh] overflow-y-auto">
                <For each={filtered()}>
                  {(i) => (
                    <div class="border-t border-(--color-outl-border)/40 px-5 py-3">
                      <div class="flex items-start gap-3">
                        <div class="min-w-0 flex-1">
                          <div class="flex items-center gap-2">
                            <span class="text-[15px] font-medium text-(--color-outl-fg)">
                              {i.name}
                            </span>
                            <Show when={i.installed}>
                              <span
                                class={`rounded px-1.5 py-0.5 text-[10px] ${
                                  i.enabled
                                    ? "bg-(--color-outl-accent)/15 text-(--color-outl-accent)"
                                    : "bg-(--color-outl-border)/40 text-(--color-outl-fg-dim)"
                                }`}
                              >
                                {i.enabled ? "installed" : "disabled"}
                              </span>
                            </Show>
                          </div>
                          <p class="mt-0.5 text-[13px] text-(--color-outl-fg-dim)">
                            {i.description}
                          </p>
                          <Show when={i.permissions.length > 0}>
                            <div class="mt-1 font-mono text-[10px] text-(--color-outl-fg-dim)/70">
                              {i.permissions.join(", ")}
                            </div>
                          </Show>
                        </div>
                        <div class="shrink-0">
                          <Show
                            when={i.installed}
                            fallback={
                              <button
                                type="button"
                                disabled={busyId() === i.id}
                                onClick={() => void install(i)}
                                class="rounded-full bg-(--color-outl-accent) px-3 py-1.5 text-[13px] font-medium text-white active:opacity-80 disabled:opacity-50"
                              >
                                {busyId() === i.id ? "…" : "Install"}
                              </button>
                            }
                          >
                            <div class="flex flex-col items-end gap-1">
                              <button
                                type="button"
                                onClick={() => toggleSettings(i.id)}
                                class="rounded-full border border-(--color-outl-border) px-2.5 py-1 text-[12px] text-(--color-outl-fg) active:opacity-60"
                              >
                                {settingsFor() === i.id ? "Close" : "Settings"}
                              </button>
                              <button
                                type="button"
                                disabled={busyId() === i.id}
                                onClick={() => void toggle(i)}
                                class="rounded-full border border-(--color-outl-border) px-2.5 py-1 text-[12px] text-(--color-outl-fg) active:opacity-60 disabled:opacity-50"
                              >
                                {i.enabled ? "Disable" : "Enable"}
                              </button>
                              <button
                                type="button"
                                disabled={busyId() === i.id}
                                onClick={() => void remove(i)}
                                class="px-2.5 py-1 text-[12px] text-(--color-outl-destructive) active:opacity-60 disabled:opacity-50"
                              >
                                Remove
                              </button>
                            </div>
                          </Show>
                        </div>
                      </div>
                      <Show when={i.installed && settingsFor() === i.id}>
                        <div class="mt-3 border-t border-(--color-outl-border)/40 pt-3">
                          <PluginSettings pluginId={i.id} />
                        </div>
                      </Show>
                    </div>
                  )}
                </For>
                <Show when={!loading() && filtered().length === 0}>
                  <div class="px-5 py-8 text-center text-[14px] text-(--color-outl-fg-dim)">
                    No plugins found.
                  </div>
                </Show>
                <div class="px-5 py-3 text-center text-[11px] text-(--color-outl-fg-dim)/70">
                  Official plugins from plugins.outl.app
                </div>
              </div>
            </Show>

            {/* Commands */}
            <Show when={tab() === "commands"}>
              <div class="ios-scroll max-h-[62vh] overflow-y-auto">
                <For each={commands()}>
                  {(cmd) => (
                    <button
                      type="button"
                      disabled={busy()}
                      onClick={() => void run(cmd)}
                      class="block w-full border-t border-(--color-outl-border)/40 px-5 py-3 text-left active:bg-(--color-outl-border)/30 disabled:opacity-50"
                    >
                      <div class="text-[15px] font-medium text-(--color-outl-fg)">
                        {cmd.title}
                      </div>
                      <div class="font-mono text-[11px] text-(--color-outl-fg-dim)/70">
                        {cmd.plugin_id} · {cmd.command_id}
                      </div>
                    </button>
                  )}
                </For>
                <Show when={commands().length === 0}>
                  <div class="px-5 py-8 text-center text-[14px] text-(--color-outl-fg-dim)">
                    No commands yet. Install a plugin from Browse.
                  </div>
                </Show>
              </div>
            </Show>
          </div>
        </div>
      </Portal>
    </Show>
  );
}

function TabButton(props: {
  label: string;
  active: boolean;
  onTap: () => void;
}) {
  return (
    <button
      type="button"
      onClick={props.onTap}
      class={`rounded-full px-3 py-1 text-[14px] font-medium ${
        props.active
          ? "bg-(--color-outl-accent) text-white"
          : "text-(--color-outl-fg-dim) active:opacity-60"
      }`}
    >
      {props.label}
    </button>
  );
}
