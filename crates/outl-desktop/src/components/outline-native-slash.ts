/**
 * The inline `/` menu rows that are **this client's own**, not a
 * plugin's: `/template <name>`, `/upload` and `/quote`.
 *
 * They arrive through `onRunPluginCommand` because they reuse the
 * plugin popup, and each is tagged with a reserved sentinel
 * `plugin_id` (`../lib/slash-commands`) that no loadable plugin
 * directory can claim. `run` takes the ones it owns and answers
 * `false` for everything else, so the caller falls through to
 * `pluginRun` rather than swallowing a pick.
 *
 * Its own module for the reason `./outline-structure` is one: the
 * branching is worth a test, and `<OutlineView />` mounts the whole
 * 3-pane shell, which is not where that test can live. Extracting it
 * is also what paid for `/quote` — `OutlineView.tsx` sits on the
 * 780-line ratchet in `.github/file-size-baseline.txt`, and a row
 * whose only cost was two lines is not a reason to raise a ceiling.
 *
 * `/quote` is the row with no chord and no `Action` behind it.
 * `outl_shortcuts`' `support()` match and `defaults.rs` are both at
 * their own baseline, so a `ToggleQuote` action cannot be declared
 * yet; `Capability::BlockquoteToggle` carries the per-client verdict
 * instead, and `docs/client-parity.md` is where it reads. The
 * `toggle_quote` command it calls has been registered on both GUI
 * clients for releases with nothing calling it.
 *
 * Deps are accessors, never values: Solid's reactivity rides the
 * getter, and a snapshot taken here would freeze at first render.
 */

import type { PageView } from "@outl/shared/api/types";
import { instantiateTemplateAt, toggleQuote } from "@outl/shared/api/commands";

import {
  NATIVE_ASSET_PLUGIN_ID,
  NATIVE_BLOCK_PLUGIN_ID,
  NATIVE_TEMPLATE_PLUGIN_ID,
} from "../lib/slash-commands";

export interface NativeSlashOpsDeps {
  /** The open page's id, or `undefined` before the first view lands. */
  pageId: () => string | undefined;
  /** The block the slash was typed in — `onStartEdit` keeps the
   *  selection cursor on it, so this is the editing block. */
  selectedBlockId: () => string | null;
  /** Fold a fresh view into the store. */
  applyView: (view: PageView) => void;
  /** Await a command, recording a failure instead of throwing. */
  handleError: <T>(promise: Promise<T>) => Promise<T | undefined>;
  /** Put a sentence on the status line. */
  setLastError: (message: string) => void;
  /** Open the OS file picker and attach what comes back — the same
   *  flow the 📎 button runs. */
  attachFile: () => Promise<void>;
}

export function createNativeSlashOps(deps: NativeSlashOpsDeps) {
  /** Run `command` against the block the slash was typed in. */
  async function onSelectedBlock(
    noTarget: string,
    command: (pageId: string, id: string) => Promise<PageView>,
  ) {
    const target = deps.selectedBlockId();
    if (!target) {
      deps.setLastError(noTarget);
      return;
    }
    const pageId = deps.pageId();
    if (!pageId) return;
    const view = await deps.handleError(command(pageId, target));
    if (view) deps.applyView(view);
  }

  return {
    /** `true` when this module owned the row and ran it. */
    async run(pluginId: string, commandId: string): Promise<boolean> {
      // `/template <name>`: instantiate the structural template under
      // the block the slash was typed in. `commandId` is the template
      // name (see `templateSlashCommands`), so this one does not go
      // through `onSelectedBlock` — the command takes the name first
      // and no page id at all.
      if (pluginId === NATIVE_TEMPLATE_PLUGIN_ID) {
        const target = deps.selectedBlockId();
        if (!target) {
          deps.setLastError("select a block to insert a template");
          return true;
        }
        const view = await deps.handleError(
          instantiateTemplateAt(commandId, target),
        );
        if (view) deps.applyView(view);
        return true;
      }
      // `/upload`: the OS file picker, reachable without leaving the
      // block.
      if (pluginId === NATIVE_ASSET_PLUGIN_ID) {
        await deps.attachFile();
        return true;
      }
      // `/quote`: flip the CommonMark `"> "` prefix on this block.
      // `outl_actions::quote::toggle_quote` owns both directions and
      // the canonical ordering against a `TODO` marker, so nothing
      // here reads or writes the prefix itself.
      if (pluginId === NATIVE_BLOCK_PLUGIN_ID && commandId === "quote") {
        await onSelectedBlock("select a block to quote", toggleQuote);
        return true;
      }
      return false;
    },
  };
}
