/**
 * Global Solid store for the desktop client.
 *
 * Holds the currently open page view, the workspace state, and the
 * panel collapse flags. The store is intentionally desktop-specific
 * (3-pane layout) — mobile has its own shape with swipe/gesture
 * state. Generic state shapes are not shared between the two clients
 * because the chrome diverges; only pure helpers and DTOs go through
 * `@outl/shared`.
 */
import { createStore, reconcile } from "solid-js/store";

import type {
  Backlink,
  BacklinksOrder,
  BlockNode,
  MdAheadOfLog,
  ParseWarning,
  PageMeta,
  ResolvedBlock,
  WorkspaceSummary,
} from "@outl/shared/api/types";

export type Mode =
  | "normal"
  | "edit"
  | "vim-normal"
  | "vim-insert"
  | "vim-visual";

export interface AppStateShape {
  /** `null` until the user picks a workspace or boot opener finishes. */
  workspace: WorkspaceSummary | null;
  /** Currently displayed page (today's journal, a regular page, …). */
  page: PageMeta | null;
  /** Outline of the current page, projected from the workspace. */
  outline: BlockNode[];
  /** Backlinks targeting the current page. */
  backlinks: Backlink[];
  /**
   * Parser recovery records emitted while reading the current page's
   * `.md`. Drives the `<ParseWarningsBanner />` above the outline so
   * the user knows outl had to preserve lines that don't match the
   * dialect (a leading `# heading`, free paragraph, etc.). Empty on
   * a clean file.
   */
  parseWarnings: ParseWarning[];
  /**
   * Set when the current page stopped syncing because its `.md` holds
   * content the op log never recorded, so outl refuses to overwrite the
   * file. Drives the `<PageAheadOfLogBanner />` above the outline.
   * `undefined` on every healthy page — which is all of them until
   * something upstream produced unlogged content.
   */
  mdAheadOfLog: MdAheadOfLog | undefined;
  /**
   * The current page's own `key:: value` properties (`icon::`,
   * `type::`, …), from `PageView.page_properties`. Rendered under the
   * page title by the same editor the block chips use — before issue
   * #13 the desktop showed them nowhere at all, so `icon::` could only
   * be read in the TUI or the `.md`.
   */
  pageProperties: Array<[string, string]>;
  /**
   * Block whose property editor should open blank, or `null`.
   *
   * The `Cmd/Ctrl+Shift+P` chord has no chip to click, so it asks for
   * an editor by id and `<BlockRow />` opens it. Cleared by the editor
   * when it closes, which is what lets a second press re-open it.
   */
  addPropertyBlockId: string | null;
  /** Resolved source blocks keyed by ref handle, for `((blk-…))` inline
   *  refs and `!((blk-…))` embeds on the current page. Inline refs render
   *  `text`; embeds also expand `children` as a subtree. */
  embeds: Record<string, ResolvedBlock>;
  /** DFS path of the selected block, or `null` for no selection. */
  selectedPath: number[] | null;
  /**
   * Currently selected block id (vim Normal-mode cursor). `null`
   * before the user navigates or after the page changes; the shell
   * auto-selects the first block when the outline loads so `j/k`
   * work from the first keystroke.
   */
  selectedBlockId: string | null;
  /**
   * When the vim cursor crosses into the backlinks section (below
   * the outline), this carries the highlighted backlink's
   * `block_id` — the id of the **source** block on the OTHER page,
   * not anything in `appState.outline`.
   *
   * Mutually exclusive with [`selectedBlockId`]: at most one is
   * non-null at a time so `j`/`k` traverse a single linear cursor.
   * `Enter` on a non-null `selectedBacklinkBlockId` opens the source
   * page (via `openRef(source_page.slug)`) and snaps the cursor to
   * `selectedBlockId = backlink.block_id` so the user lands on the
   * referencing block.
   *
   * Mirrors the TUI's `Focus::Backlinks` state — the desktop now
   * supports the same `j/k`+`Enter` flow over backlinks.
   */
  selectedBacklinkBlockId: string | null;
  /**
   * Block currently in edit mode (textarea mounted). `null` outside
   * Insert mode. Lifted from `<OutlineView />`'s local signal so
   * `outl-shortcuts` action handlers (`EnterInsert`, `NewBlockBelow`,
   * `CommitAndContinue`, …) can flip it without prop-drilling a
   * callback through `buildHandlers`.
   */
  editingBlockId: string | null;
  /** Editor mode. `edit` while a block's textarea is mounted. */
  mode: Mode;
  /**
   * Block id where the current Visual selection was anchored. Set by
   * `EnterVisual`, cleared on exit. The Visual range covers every
   * block from this id to `selectedBlockId` in DFS order — direction
   * doesn't matter, the renderer picks `[lo, hi]` itself.
   */
  visualAnchorId: string | null;
  /**
   * Last Visual range captured the moment the user left Visual mode
   * (Esc, yank, delete). `gv` re-enters Visual with the same range.
   * `null` until the first Visual session of the app instance.
   *
   * Stores both endpoints by id so an outline mutation between the
   * exit and the `gv` re-entry doesn't shift the range (block ids are
   * stable; flat indices aren't).
   */
  lastVisualRange: { lo: string; hi: string } | null;
  /**
   * Yank register — list of block texts copied via `yy` (Y) or `y`
   * in Visual. `p` / `P` paste these (handlers TBD). One register
   * cross-block (vim convention).
   */
  yankRegister: string[];
  /**
   * Block clipboard for the view-mode cut / copy / paste gesture
   * (`Cmd+X` / `Cmd+C` / `Cmd+V` in Normal mode). A *cut* holds the
   * id of the block to move — the paste emits an `Op::Move`, so the
   * block keeps its identity (`((blk-…))` refs and backlinks stay
   * valid). A *copy* holds rendered markdown the paste re-ingests with
   * fresh ids (duplicate, not move). `null` when the clipboard is empty.
   *
   * A cross-page paste re-renders the source page too, but the source
   * page is **not** carried here: `move_block_after` derives it from
   * the moved node via `enclosing_page_id`, so a `pageId` on the cut
   * payload would just be a second, drift-prone copy of that fact.
   *
   * Distinct from [`yankRegister`], which is the vim `y` text
   * register; this one moves / duplicates whole blocks structurally.
   */
  blockClipboard:
    | { kind: "cut"; nodeId: string }
    | { kind: "copy"; markdown: string }
    | null;
  /**
   * Sidebar (left pane) visibility. Toggled with `Cmd/Ctrl+Shift+E`
   * (mirrors VS Code's "Show Explorer" — see `outl-shortcuts`).
   *
   * Defaults to `false`: editor-hero on first launch (Bear / Ulysses
   * convention), matches the TUI's `show_sidebar: false` default.
   * The user opts in with the chord.
   */
  sidebarOpen: boolean;
  /**
   * Backlinks (right pane) visibility. Toggled with
   * `Cmd/Ctrl+Shift+B` (mirrors the TUI's `Ctrl+B`; we picked the
   * shifted variant on the desktop to keep `Cmd+B` reserved for the
   * universal markdown "bold" chord).
   *
   * Defaults to `true`: references stay visible below the outline so
   * a page's incoming links are discoverable without a chord. The
   * inline section only renders when the page actually has backlinks
   * (`<InlineBacklinks />` guards on `appState.backlinks.length`), so
   * an open default costs nothing on pages with no references.
   */
  backlinksOpen: boolean;
  /**
   * Direction of the backlinks list (issue #142): `"newest"` puts the
   * most recently referenced page on top (default), `"oldest"` flips
   * it. Loaded from `config.toml` at boot (via `getSettings`) and
   * flipped by the toggle in `<InlineBacklinks />`, which persists it
   * through the `set_backlinks_order` command and swaps in the
   * re-sorted view. A pure display preference — never an Op.
   */
  backlinksOrder: BacklinksOrder;
  /**
   * Caret intent the next mounting `<BlockRow />` textarea consumes
   * the moment it lands in the DOM. Set by vim-style entry actions
   * that need the caret somewhere other than where the click would
   * leave it — today only `EnterInsertAtEnd` (`A` in vim) uses
   * `"end"`. Cleared by `<BlockRow />` as soon as it applies the
   * intent so a subsequent regular click doesn't get hijacked.
   *
   * Why a signal and not `queueMicrotask` + `document.querySelector`:
   * the textarea is mounted by Solid's `<Show>` swap, which doesn't
   * guarantee the DOM node exists by the next microtask. A signal
   * lets the row itself apply the intent inside its own
   * `createEffect` — same tick the textarea ref is populated, every
   * time.
   */
  caretIntent: "end" | "start" | null;
  /** Picker overlay open state. `Cmd/Ctrl+P` toggles. */
  pickerOpen: boolean;
  /**
   * Optional pre-fill query consumed by `<Picker />` the moment it
   * opens. Set by `*` / `#` (Normal mode "search inside the selected
   * block") before flipping `pickerOpen`. The picker clears this on
   * close so the next manual `Cmd+P` opens blank.
   */
  pickerSeed: string | null;
  /** Settings modal open state. `Cmd/Ctrl+,` toggles. */
  settingsOpen: boolean;
  /**
   * Plugin palette open state. Lists every command contributed by a
   * loaded plugin (`outl_plugins::PluginHost::commands`) and runs the
   * picked one. Opened from the `⧉` button in `<ChromeToggleBar />`;
   * there is no keyboard chord yet (plugin commands are not in the
   * `outl-shortcuts` catalog).
   */
  /** Plugin marketplace (browse + install from plugins.outl.app) open state. */
  marketplaceOpen: boolean;
  /** Help overlay open state. `?` in Normal mode toggles. */
  helpOpen: boolean;
  /**
   * Reminders panel open state. `Cmd/Ctrl+Shift+R` (or `g n` in
   * Normal) toggles. Lists every block with a `remind::` grouped by
   * next fire — read-only apart from snooze / mark-done.
   */
  remindersOpen: boolean;
  /**
   * Zoom / focus root (Roam/Workflowy style). When non-null, the
   * outline renders only this block's subtree with a clickable
   * ancestor breadcrumb above it. Pure **view state, local per
   * device** — zoom never round-trips through the op log (the client
   * already holds the whole `outline`; `focusSubtree` slices it). Reset
   * to `null` on page navigation (the `OutlineView` page-change effect)
   * or when the focused block goes stale (deleted / moved off-page).
   */
  focusBlockId: string | null;
  /**
   * Block currently under an in-flight OS file drag (drag-and-drop
   * upload). Set on `enter` / `over`, cleared on `leave` / `drop`.
   * `<BlockRow />` highlights the matching row so the user sees where
   * the dropped file's link will land. Pure transient UI state — never
   * an Op.
   */
  dropTargetBlockId: string | null;
  /** Last error surfaced to the user (status line). */
  lastError: string | null;
}

const [state, setState] = createStore<AppStateShape>({
  workspace: null,
  page: null,
  outline: [],
  backlinks: [],
  parseWarnings: [],
  mdAheadOfLog: undefined,
  pageProperties: [],
  addPropertyBlockId: null,
  embeds: {},
  selectedPath: null,
  selectedBlockId: null,
  selectedBacklinkBlockId: null,
  editingBlockId: null,
  mode: "normal",
  visualAnchorId: null,
  lastVisualRange: null,
  yankRegister: [],
  blockClipboard: null,
  sidebarOpen: false,
  backlinksOpen: true,
  backlinksOrder: "newest",
  caretIntent: null,
  pickerOpen: false,
  pickerSeed: null,
  settingsOpen: false,
  marketplaceOpen: false,
  helpOpen: false,
  remindersOpen: false,
  focusBlockId: null,
  dropTargetBlockId: null,
  lastError: null,
});

export { state as appState, setState as setAppState };

/**
 * Update the outline by **reconciling** (keyed on block id), not
 * replacing the array. Replacing it with a fresh array of new objects
 * makes the `<For>` re-create every `<BlockRow>` (it keys by reference),
 * so editing one block on a large page re-renders the whole page — the
 * "commit is slow on a big page" cause. `reconcile` keeps unchanged
 * blocks' identity so only what actually changed re-renders.
 *
 * Every path that swaps in a backend outline (commit, batch op, peer
 * reload, navigation) MUST go through here, never `setAppState("outline", …)`
 * with a raw array.
 */
export function setOutline(outline: BlockNode[]): void {
  setState("outline", reconcile(outline, { key: "id" }));
}
