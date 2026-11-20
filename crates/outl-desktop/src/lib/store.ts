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
import { focusSubtree } from "@outl/shared/outline";
import { createSavedMarks } from "@outl/shared/trust";
import { createStore, reconcile } from "solid-js/store";

import type { FocusView } from "@outl/shared/outline";
import type {
  Backlink,
  BacklinksOrder,
  BlockNode,
  MdAheadOfLog,
  NamespaceChild,
  PageEmbedContent,
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
  /** Pages nested under the current page's namespace (issue #275). */
  namespaceChildren: NamespaceChild[];
  /** Blocks reaching this page only through a descendant, capped
   *  server-side; `namespaceMentionsTotal` carries the real count. */
  namespaceMentions: Backlink[];
  /** Count before the cap, so a truncated list can say so. */
  namespaceMentionsTotal: number;
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
  /** Resolved pages keyed by the name inside `![[page]]`, for the page
   *  embeds on the current page. The inline token renders `title`; a
   *  block that is nothing but the embed also expands `blocks` as a
   *  read-only subtree. Twin of {@link AppStateShape.embeds}, filled by
   *  the same resolve pass in `<OutlineView />`. */
  pageEmbeds: Record<string, PageEmbedContent>;
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
  /**
   * Backlink row currently in edit mode, as the `(page, block)` pair a
   * cross-page mutation is addressed to. `null` outside Insert on a
   * backlink row.
   *
   * A sibling field rather than a wider `editingBlockId`, for the reason
   * `selectedBacklinkBlockId` is a sibling of `selectedBlockId`: the two
   * cursors are declared mutually exclusive and ~20 call sites write the
   * outline one directly. Widening the editing id into a tagged object
   * would make every one of them a type error for a distinction only two
   * of them care about.
   *
   * **Set one, clear the other** — `startBacklinkEdit` /
   * `stopBacklinkEdit` are the mechanism, so the rule is not a sentence
   * each writer has to remember. `pageId` is the **source** page, never
   * `appState.page`.
   */
  editingBacklink: { pageId: string; blockId: string } | null;
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
   * A `page_backlinks` round-trip is in flight.
   *
   * Owned by `trackBacklinks`, read only by `<InlineBacklinks />`, which
   * renders its own placeholder from it. It must stay scoped to that
   * section: the scan is `O(blocks-in-workspace)` and the outline has to
   * keep taking keystrokes through it (`UX.md` → Never block a keystroke).
   */
  backlinksLoading: boolean;
  /**
   * Caret intent the next mounting `<BlockRow />` textarea consumes
   * the moment it lands in the DOM. Set by vim-style entry actions
   * that need the caret somewhere other than where the click would
   * leave it: `EnterInsertAtEnd` (`A` in vim) uses `"end"`. A
   * codepoint offset (`number`) carries the caret Tab / Shift-Tab
   * captured before commit, through the remount `onIndent` /
   * `onOutdent` cause by re-entering edit mode on the moved block
   * (#477); `"end"` / `"start"` can't express a mid-string position.
   * Cleared by `<BlockRow />` as soon as it applies the intent so a
   * subsequent regular click doesn't get hijacked.
   *
   * Why a signal and not `queueMicrotask` + `document.querySelector`:
   * the textarea is mounted by Solid's `<Show>` swap, which doesn't
   * guarantee the DOM node exists by the next microtask. A signal
   * lets the row itself apply the intent inside its own
   * `createEffect` — same tick the textarea ref is populated, every
   * time.
   */
  caretIntent: "end" | "start" | number | null;
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
   * Task list open state. `g t` (Normal) or `Cmd/Ctrl+Shift+L` toggles.
   * The workspace's open `TODO` / `DOING` blocks, by page — the first
   * 200, with the uncapped count shown beside them.
   */
  tasksOpen: boolean;
  /**
   * Trash panel open state. Opened from the `🗑` button in the chrome
   * toggle bar. No chord yet: an `Action` needs an arm in
   * `outl_shortcuts::support`, which is frozen at its size baseline —
   * the same gap `timelineOpen` has. `Capability::Trash` records it.
   *
   * Distinct from the undo stack: that is this session's mutations,
   * this is every deletion from every device, forever.
   */
  trashOpen: boolean;
  /**
   * Page-history panel open state. Opened from the `⏱` button in the
   * page header. Read-only: it lists what the op log says happened to
   * this page, including blocks that were deleted out of it.
   *
   * Distinct from the undo stack — that is this session's mutations,
   * this is every device's, forever.
   */
  timelineOpen: boolean;
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
  /**
   * Today's journal slug, from the backend's `today_slug` (configured
   * timezone, never the browser clock). `null` until it answers.
   *
   * One owner, because a window left open overnight used to carry two
   * stale copies of this fact: the header's own `createResource` keyed
   * on the page slug, which only re-ran on navigation, and the sidebar
   * calendar's `new Date()`, frozen at mount *and* on the wrong clock.
   * `<AppShell />` wires `createDayClock` (`@outl/shared/journal`) to
   * this field; nothing else writes it.
   */
  todaySlug: string | null;
  /** Last error surfaced to the user (status line). */
  lastError: string | null;
  /**
   * What an operation **did**, one line per fact — never a failure, which
   * is `lastError`. Read by `<NoticeToast />` only; written by
   * `page-actions.ts` (every property conflict a merge dropped) and
   * `backlink-handlers.ts` (which page a sibling created from a backlink
   * row landed on). Both report what no surface on screen can show.
   */
  notice: string[] | null;
}

const [state, setState] = createStore<AppStateShape>({
  workspace: null,
  todaySlug: null,
  page: null,
  outline: [],
  backlinks: [],
  namespaceChildren: [],
  namespaceMentions: [],
  namespaceMentionsTotal: 0,
  parseWarnings: [],
  mdAheadOfLog: undefined,
  pageProperties: [],
  addPropertyBlockId: null,
  embeds: {},
  pageEmbeds: {},
  selectedPath: null,
  selectedBlockId: null,
  selectedBacklinkBlockId: null,
  editingBlockId: null,
  editingBacklink: null,
  mode: "normal",
  visualAnchorId: null,
  lastVisualRange: null,
  yankRegister: [],
  blockClipboard: null,
  sidebarOpen: false,
  backlinksOpen: true,
  backlinksOrder: "newest",
  backlinksLoading: false,
  caretIntent: null,
  pickerOpen: false,
  pickerSeed: null,
  settingsOpen: false,
  marketplaceOpen: false,
  helpOpen: false,
  remindersOpen: false,
  tasksOpen: false,
  timelineOpen: false,
  trashOpen: false,
  focusBlockId: null,
  dropTargetBlockId: null,
  lastError: null,
  notice: null,
});

export { state as appState, setState as setAppState };

/**
 * A chrome flag: a panel or overlay that is either on screen or not.
 *
 * Derived from the shape rather than hand-listed. The keyboard path and
 * the click path each carried their own list — a seven-key union in
 * `chrome-handlers.ts`, five inline flips in `<ChromeToggleBar />` — and
 * the two never agreed: `trashOpen` and `marketplaceOpen` are clickable
 * and absent from the union, `backlinksOpen` and `tasksOpen` are the
 * other way round.
 */
export type PanelFlag = Extract<keyof AppStateShape, `${string}Open`>;

/**
 * Flip a chrome flag.
 *
 * The single owner of "the button and the chord do the same thing":
 * both call this, so neither can end up reading one flag and writing
 * another. The `Esc` cascade in `chrome-handlers.ts` keeps its own list
 * because the thing it owns is the *order*, not the flip.
 */
export function togglePanel(flag: PanelFlag): void {
  setState(flag, !state[flag]);
}

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

/**
 * The zoom, resolved against the live outline: the focused block's
 * subtree plus its ancestor breadcrumb, or `null` when there is no zoom
 * **or** the focused block has left the page (a peer deleted it, a move
 * took it elsewhere).
 *
 * The single owner of "is this window zoomed, and into what", because
 * three surfaces have to agree on it and they used to each resolve it
 * themselves: what the outline body renders, where `j` / `k` may go,
 * and whether the page's own sections are on screen. A stale id read as
 * "zoomed" by one of them and as "not zoomed" by another is a cursor
 * walking into a section that is not drawn.
 *
 * Reads the store, so calling it inside a memo / JSX keeps the
 * reactivity — `<OutlineView />` wraps it in a `createMemo` so the walk
 * runs once per render rather than once per reader.
 */
export function zoomView(): FocusView | null {
  const id = state.focusBlockId;
  if (!id) return null;
  return focusSubtree(state.outline, id);
}

/**
 * The blocks on screen: the zoom's children when zoomed, the whole page
 * otherwise.
 *
 * One function because the body renders them and the cursor walks them,
 * and those two must not disagree — `navBlocks()` used to carry a
 * comment saying it matched `<OutlineView />`, which is a parity claim
 * nothing could fail. A stale zoom target falls back to the full page
 * here, so neither caller has to ask twice.
 */
export function visibleRoots(): BlockNode[] {
  const fv = zoomView();
  return fv ? fv.root.children : state.outline;
}

/**
 * Per-block "saved to disk" marks. `OutlineView`'s commit marks a block
 * once `edit_block` resolves (the op is in this device's log and the
 * `.md` is rewritten); `BlockRow` renders the fading dot.
 */
export const savedMarks = createSavedMarks();
