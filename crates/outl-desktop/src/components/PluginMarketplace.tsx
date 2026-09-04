import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";

import {
  filterRegistryItems,
  pluginInstallOfficial,
  pluginRegistryList,
  pluginSetEnabled,
  pluginUninstall,
} from "@outl/shared/api/commands";
import type { RegistryItem } from "@outl/shared/api/types";
import { PluginSettings } from "@outl/shared/plugins";
import { appState, setAppState } from "../lib/store";

/**
 * Plugin marketplace.
 *
 * Browses the official registry (`plugins.outl.app/registry.json`, fetched +
 * cross-referenced with the lockfile by `plugin_registry_list`) and installs
 * a plugin with one tap (`plugin_install_official` downloads the bundle from
 * `plugins.outl.app/p/<id>/`, freezes the hash, and reloads the host).
 * Installed plugins can be enabled / disabled / removed inline.
 *
 * Only registry-listed (official, human-reviewed) plugins are installable
 * here — an unlisted plugin still installs via the CLI (`outl plugin install
 * github:… | ./dir`). All host work happens on the plugin thread; this modal
 * only talks to it through those Tauri commands.
 */
export function PluginMarketplace() {
  const [items, setItems] = createSignal<RegistryItem[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [busyId, setBusyId] = createSignal<string | null>(null);
  const [query, setQuery] = createSignal("");
  // Which installed plugin's settings panel is expanded (one at a time).
  const [settingsFor, setSettingsFor] = createSignal<string | null>(null);

  function close() {
    setAppState("marketplaceOpen", false);
  }

  const toggleSettings = (id: string) =>
    setSettingsFor((cur) => (cur === id ? null : id));

  onMount(() => {
    const esc = (e: KeyboardEvent) => {
      if (appState.marketplaceOpen && e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", esc);
    onCleanup(() => window.removeEventListener("keydown", esc));
  });

  async function refresh() {
    setLoading(true);
    setError(null);
    try {
      setItems(await pluginRegistryList());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setItems([]);
    } finally {
      setLoading(false);
    }
  }

  // Fetch each time it opens (the registry is remote; the lockfile changes
  // as the user installs/removes).
  createEffect(() => {
    if (appState.marketplaceOpen) void refresh();
  });

  const filtered = () => filterRegistryItems(items(), query());

  async function withBusy(id: string, fn: () => Promise<unknown>) {
    if (busyId()) return;
    setBusyId(id);
    setError(null);
    try {
      await fn();
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
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

  return (
    <Show when={appState.marketplaceOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={(e) => {
          if (e.target === e.currentTarget) close();
        }}
      >
        <div class="mt-16 flex max-h-[80vh] w-[620px] max-w-[92vw] flex-col overflow-hidden rounded-lg border border-(--color-outl-fg)/15 bg-(--color-outl-bg-elev)/95 shadow-2xl">
          <header class="flex items-center gap-3 border-b border-(--color-outl-fg)/10 px-5 py-3">
            <h2 class="text-lg font-semibold">Plugin marketplace</h2>
            <Show when={loading()}>
              <span class="text-xs opacity-50">loading…</span>
            </Show>
            <input
              type="text"
              placeholder="Search…"
              value={query()}
              onInput={(e) => setQuery(e.currentTarget.value)}
              class="ml-auto w-44 rounded border border-(--color-outl-fg)/15 bg-(--color-outl-bg) px-2 py-1 text-sm outline-none"
            />
          </header>

          <Show when={error()}>
            <div class="border-b border-(--color-outl-fg)/10 bg-(--color-outl-destructive)/10 px-5 py-2 text-xs text-(--color-outl-destructive)">
              {error()}
            </div>
          </Show>

          <div class="flex-1 overflow-y-auto">
            <For each={filtered()}>
              {(i) => (
                <div class="border-b border-(--color-outl-fg)/5">
                <div class="flex items-start gap-3 px-5 py-3">
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2">
                      <span class="font-medium">{i.name}</span>
                      <Show when={i.installed}>
                        <span
                          class={`rounded px-1.5 py-0.5 text-[10px] ${
                            i.enabled
                              ? "bg-(--color-outl-accent)/20 text-(--color-outl-accent)"
                              : "bg-(--color-outl-fg)/10 opacity-60"
                          }`}
                        >
                          {i.enabled ? "installed" : "disabled"}
                        </span>
                      </Show>
                    </div>
                    <p class="mt-0.5 text-sm opacity-70">{i.description}</p>
                    <div class="mt-1 flex flex-wrap gap-1">
                      <For each={i.capabilities}>
                        {(c) => (
                          <span class="rounded bg-(--color-outl-fg)/8 px-1.5 py-0.5 font-mono text-[10px] opacity-70">
                            {c}
                          </span>
                        )}
                      </For>
                    </div>
                    <Show when={i.permissions.length > 0}>
                      <div class="mt-1 font-mono text-[10px] opacity-50">
                        permissions: {i.permissions.join(", ")}
                      </div>
                    </Show>
                  </div>

                  <div class="flex shrink-0 flex-col items-end gap-1">
                    <Show
                      when={i.installed}
                      fallback={
                        <button
                          type="button"
                          disabled={busyId() === i.id}
                          onClick={() => void install(i)}
                          class="rounded bg-(--color-outl-accent) px-3 py-1 text-sm font-medium text-(--color-outl-bg) hover:opacity-90 disabled:opacity-50"
                        >
                          {busyId() === i.id ? "installing…" : "Install"}
                        </button>
                      }
                    >
                      <button
                        type="button"
                        onClick={() => toggleSettings(i.id)}
                        class="rounded border border-(--color-outl-fg)/20 px-2 py-1 text-xs hover:bg-(--color-outl-fg)/5"
                      >
                        {settingsFor() === i.id ? "Close" : "Settings"}
                      </button>
                      <button
                        type="button"
                        disabled={busyId() === i.id}
                        onClick={() => void toggle(i)}
                        class="rounded border border-(--color-outl-fg)/20 px-2 py-1 text-xs hover:bg-(--color-outl-fg)/5 disabled:opacity-50"
                      >
                        {i.enabled ? "Disable" : "Enable"}
                      </button>
                      <button
                        type="button"
                        disabled={busyId() === i.id}
                        onClick={() => void remove(i)}
                        class="rounded px-2 py-1 text-xs text-(--color-outl-destructive) hover:bg-(--color-outl-destructive)/10 disabled:opacity-50"
                      >
                        Remove
                      </button>
                    </Show>
                  </div>
                </div>
                <Show when={i.installed && settingsFor() === i.id}>
                  <div class="border-t border-(--color-outl-fg)/10 bg-(--color-outl-bg)/40 px-5 py-4">
                    <PluginSettings pluginId={i.id} />
                  </div>
                </Show>
                </div>
              )}
            </For>

            <Show when={!loading() && filtered().length === 0}>
              <div class="px-5 py-10 text-center text-sm opacity-60">
                <Show when={error()} fallback="No plugins match.">
                  Couldn't reach the registry.
                </Show>
              </div>
            </Show>
          </div>

          <div class="border-t border-(--color-outl-fg)/10 px-5 py-1.5 text-[10px] opacity-50">
            Official plugins from plugins.outl.app · unlisted plugins install
            via the CLI · Esc close
          </div>
        </div>
      </div>
    </Show>
  );
}
