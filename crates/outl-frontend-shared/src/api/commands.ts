/**
 * Typed wrappers around the Tauri commands every outl frontend
 * client invokes. The Rust side (`outl-mobile/src-tauri/src/lib.rs`,
 * `outl-desktop/src-tauri/src/lib.rs`) registers commands with the
 * exact names below; this file is the single TS surface so both
 * clients agree on shape, name and return type.
 *
 * Client-specific commands (mobile gestures, `pick_workspace_dir`
 * on desktop, etc) live in the client's own `lib/api.ts` and never
 * end up here.
 *
 * `run_code_block` used to be considered client-specific (desktop
 * only). Mobile picked up the same command as of v0.6.x (long-press
 * → "Run code"), and both clients now share the wrapper below.
 */

import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

import type {
  BacklinksOrder,
  CreateBlockReply,
  PageBacklinks,
  PageMeta,
  PageView,
  PeerDto,
  PeerStatusDto,
  PluginCommand,
  PluginRunReply,
  PluginSettingsField,
  PluginSyncHooksReply,
  PluginToolbarButton,
  PluginTransformer,
  PluginTransformResult,
  PropertyKey,
  RegistryItem,
  Reminder,
  ReminderSettings,
  SnoozePreset,
  ResolvedBlock,
  RunCodeBlockReply,
  TemplateDto,
  WorkspaceSummary,
} from "./types";

/**
 * One emoji match returned by {@link searchEmojis}. Mirrors
 * `outl_md::emoji::EmojiHit` (`outl-md/src/emoji.rs`). `score` is
 * stable enough for the autocomplete popup to sort on — not a public
 * ranking guarantee.
 */
export interface EmojiHit {
  shortcode: string;
  glyph: string;
  score: number;
}

/**
 * One block match returned by {@link searchBlocks}. Mirrors
 * `outl_tauri_shared::state::BlockHit`. `handle` is the ref handle
 * (`blk-XXXXXX`) the caller inserts wrapped in `((…))`; block refs
 * resolve by handle, never by the display `text`. `text` is a
 * single-line snippet for the popup label and `source_slug` the page
 * hosting the block, for context.
 */
export interface BlockHit {
  handle: string;
  text: string;
  source_slug: string;
}

// ---------------------------------------------------------------------------
// Page / journal navigation
// ---------------------------------------------------------------------------

export function listPages(): Promise<PageMeta[]> {
  return invoke<PageMeta[]>("list_all_pages");
}

/**
 * Fuzzy-search known pages by `query`. Empty query returns up to 25
 * pages in the workspace's natural order; non-empty filters by
 * case-insensitive substring on title and slug and ranks exact and
 * prefix matches first.
 *
 * Used by the floating ref suggester that appears while the user
 * types inside `[[…]]`.
 */
export function searchPages(query: string): Promise<PageMeta[]> {
  return invoke<PageMeta[]>("search_pages", { query });
}

/**
 * Search the workspace for pages with `type:: person`, ranked by the
 * query (same fuzzy shape as {@link searchPages} — exact, prefix,
 * contains). Powers the `@` mention autocomplete in every client.
 * Backed by `outl_actions::search_persons` so every surface ranks the
 * same way without re-implementing the filter.
 */
export function searchPersons(query: string): Promise<PageMeta[]> {
  return invoke<PageMeta[]>("search_persons", { query });
}

/**
 * Search the GitHub gemoji catalog for shortcodes matching `query`.
 * Ranks exact → prefix → substring; shorter shortcodes win ties.
 * `limit` caps the result set (defaults to 8 — the size of the
 * autocomplete popup in every client). Backed by
 * `outl_md::emoji::search` so TUI / mobile / desktop rank identically.
 */
export function searchEmojis(query: string, limit = 8): Promise<EmojiHit[]> {
  return invoke<EmojiHit[]>("outl_emoji_search", { query, limit });
}

/**
 * Fuzzy-search block text for the `((…))` block-ref autocomplete.
 * Empty query returns the most recently created blocks; a non-empty
 * query ranks case-insensitive substring matches (prefix first, shorter
 * blocks winning ties). Backed by `outl_md::WorkspaceIndex::search_block_text`
 * so every client ranks the same way. The caller inserts each hit's
 * `handle` wrapped in `((…))` — never the display `text`.
 */
export function searchBlocks(query: string): Promise<BlockHit[]> {
  return invoke<BlockHit[]>("search_blocks", { query });
}

export function openTodayJournal(): Promise<PageView> {
  return invoke<PageView>("open_today_journal");
}

export function openJournalFor(slug: string): Promise<PageView> {
  return invoke<PageView>("open_journal_for", { slug });
}

export function openPageBySlug(slug: string): Promise<PageView> {
  return invoke<PageView>("open_page_by_slug", { slug });
}

/**
 * Resolve and open whatever a user-typed ref / tag / picker entry
 * points at, in one round-trip. The backend runs the canonical
 * decision tree (date → journal, else literal/slugified/title match
 * → existing page, else create a fresh page with the typed string
 * as the title). This is the entry point every ref-click handler
 * on the frontend should use; never branch by shape in TS before
 * calling — the regex-vs-parser drift is exactly what this command
 * exists to remove (`[[2026-13-01]]` used to surface
 * `invalid date slug` because the TS shape regex disagreed with
 * the Rust semantic parser).
 */
export function openRef(target: string): Promise<PageView> {
  return invoke<PageView>("open_ref", { target });
}

export function previousDay(slug: string): Promise<string> {
  return invoke<string>("previous_day", { slug });
}

export function nextDay(slug: string): Promise<string> {
  return invoke<string>("next_day", { slug });
}

export function todaySlug(): Promise<string> {
  return invoke<string>("today_slug_cmd");
}

export function dateTitle(slug: string): Promise<string> {
  return invoke<string>("date_title", { slug });
}

export function resolveRef(target: string): Promise<PageMeta | null> {
  return invoke<PageMeta | null>("resolve_ref", { target });
}

/**
 * Delete a page by slug. The caller MUST confirm before invoking —
 * this call does not re-prompt. The backend moves the page root to
 * `NodeId::trash()` (a single `Op::Move`, so the whole subtree goes
 * with it), drops the on-disk `.md` + `.outl` projection, and
 * returns a fresh {@link PageView} of **today's journal** so the
 * caller navigates away from the (now-gone) page in the same
 * round-trip.
 *
 * The op stays in the log forever — deletion converges across
 * devices through the normal CRDT replay and is theoretically
 * recoverable from the log (no UI for that exists today).
 */
export function deletePage(slug: string): Promise<PageView> {
  return invoke<PageView>("delete_page", { slug });
}

/**
 * Compute a page's backlinks lazily, off the page-open path.
 *
 * `backlinks_for_page` is an O(blocks-in-workspace) scan, so it's no
 * longer bundled into {@link PageView} — computing it there blocked the
 * first journal paint on a large workspace. Call this after the outline
 * renders (keyed on the current page slug) and fill the backlinks panel
 * when it resolves. Mirrors the TUI's lazy/cached backlinks.
 */
export function pageBacklinks(slug: string): Promise<PageBacklinks> {
  return invoke<PageBacklinks>("page_backlinks", { slug });
}

/**
 * Persist the backlinks-list direction (issue #142) and get `slug`'s
 * backlinks back re-sorted under it. A pure display preference — it lives
 * in `config.toml`, never the op log, so it does not converge between
 * devices (same policy as the theme). Returns only the re-sorted
 * backlinks (not a whole `PageView`): the panel already has the outline.
 */
export function setBacklinksOrder(
  order: BacklinksOrder,
  slug: string,
): Promise<PageBacklinks> {
  return invoke<PageBacklinks>("set_backlinks_order", { order, slug });
}

/**
 * Resolve a batch of block ids (from a `sync-progress` `received-ops` event)
 * to the distinct page/journal slugs they belong to — the "page X synced"
 * labels in the pairing-screen feed. Best-effort: ids not yet materialized (a
 * reload may still be in flight) are dropped, so the caller renders whatever
 * resolved. The engine caps the id list, so this stays cheap.
 */
export function resolvePageLabels(nodeIds: string[]): Promise<string[]> {
  return invoke<string[]>("resolve_page_labels", { nodeIds });
}

// ---------------------------------------------------------------------------
// Structural templates
// ---------------------------------------------------------------------------

/**
 * List every structural template defined in the workspace, sorted by
 * invocation name. A template is any page with a non-empty `template::`
 * property. Powers the desktop `/template` slash picker and the mobile
 * template sheet. Wraps `list_templates_cmd` (the `_cmd` suffix on the
 * Rust side avoids a name clash with the `outl_actions::list_templates`
 * re-export in each client's command glob).
 */
export function listTemplates(): Promise<TemplateDto[]> {
  return invoke<TemplateDto[]>("list_templates_cmd");
}

/**
 * Deep-copy the template `name` under `targetBlockId`, applying built-in
 * variable substitution (`{{date}}`, `{{page}}`, …) and stamping
 * `from-template:: <slug>` on each root clone. Returns a refreshed
 * {@link PageView} of the target block's enclosing page so the caller
 * `applyView`s it in one round-trip.
 *
 * Rejects (string error → toast) on an unknown template name or a stale
 * block id — nothing is created on a miss.
 */
export function instantiateTemplateAt(
  name: string,
  targetBlockId: string,
): Promise<PageView> {
  return invoke<PageView>("instantiate_template_at", {
    name,
    targetBlock: targetBlockId,
  });
}

export function workspaceStats(): Promise<WorkspaceSummary> {
  return invoke<WorkspaceSummary>("workspace_stats");
}

// ---------------------------------------------------------------------------
// Block mutations (all scoped to a page)
// ---------------------------------------------------------------------------

/**
 * Insert a new block under `pageId`. Placement precedence:
 * `beforeId` (sibling immediately before — vim `O`) wins over
 * `afterId` (sibling immediately after — vim `o`), falling back to
 * the last child of `parentId` (defaults to the page itself when all
 * are null). Returns the refreshed {@link PageView} paired with
 * `new_id` — the id of the freshly-inserted block — so the caller
 * can put it into edit mode without diffing the outline. See
 * {@link CreateBlockReply} for why the id is on the wire.
 */
export function createBlock(
  pageId: string,
  opts: {
    afterId?: string | null;
    beforeId?: string | null;
    parentId?: string | null;
    text?: string | null;
  },
): Promise<CreateBlockReply> {
  return invoke<CreateBlockReply>("create_block", {
    pageId,
    afterId: opts.afterId ?? null,
    beforeId: opts.beforeId ?? null,
    parentId: opts.parentId ?? null,
    text: opts.text ?? null,
  });
}

export function editBlock(pageId: string, id: string, text: string): Promise<PageView> {
  return invoke<PageView>("edit_block", { pageId, id, text });
}

/**
 * Split a block at the caret: the text up to `charOffset` stays in the
 * block, the rest moves into a new sibling below (returned as `new_id`
 * so the client can drop the caret at its start). Mirrors
 * `outl_actions::split_block`.
 *
 * `charOffset` is a **codepoint** offset — convert the textarea's
 * UTF-16 `selectionStart` with `utf16OffsetToCharOffset` first, same as
 * {@link pastePlain}. `charOffset === 0` opens an empty block above;
 * a caret at/after the end creates an empty sibling below (plain Enter).
 */
export function splitBlock(
  pageId: string,
  id: string,
  charOffset: number,
): Promise<CreateBlockReply> {
  return invoke<CreateBlockReply>("split_block", { pageId, id, charOffset });
}

export function toggleTodo(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("toggle_todo", { pageId, id });
}

/**
 * Flip the block's blockquote marker on or off. Mirrors
 * `outl_actions::block::toggle_quote`. Both mobile and desktop expose
 * the same `toggle_quote` Tauri command so this wrapper works on
 * every client.
 */
export function toggleQuote(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("toggle_quote", { pageId, id });
}

export function deleteBlock(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("delete_block", { pageId, id });
}

export function indentBlock(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("indent_block", { pageId, id });
}

export function outdentBlock(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("outdent_block", { pageId, id });
}

export function moveBlockUp(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("move_block_up", { pageId, id });
}

export function moveBlockDown(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("move_block_down", { pageId, id });
}

/**
 * Move block `id` to sit immediately after `afterId`, re-parenting it
 * under the target's parent. Backs the cut-and-paste-block gesture
 * (`Cmd+X` then `Cmd+V` in view mode): the block keeps its identity,
 * so `((blk-…))` refs and backlinks survive — and `afterId` may live
 * on another page, which moves the block across pages.
 */
export function moveBlockAfter(
  pageId: string,
  id: string,
  afterId: string,
): Promise<PageView> {
  return invoke<PageView>("move_block_after", { pageId, id, afterId });
}

/**
 * Render block `id` and its subtree to clean outl markdown for the
 * block clipboard (`Cmd+C` in view mode). The paste re-ingests it and
 * mints fresh ids, so a copy duplicates rather than moves.
 */
export function copyBlockMarkdown(id: string): Promise<string> {
  return invoke<string>("copy_block_markdown", { id });
}

/**
 * Paste clipboard `text` (clean outl markdown) as the sibling(s)
 * immediately after `afterId` — the `Cmd+V` of a copied block.
 */
export function pasteBlockAfter(
  pageId: string,
  afterId: string,
  text: string,
): Promise<PageView> {
  return invoke<PageView>("paste_block_after", { pageId, afterId, text });
}

export function reloadWorkspace(): Promise<void> {
  return invoke<void>("reload_workspace");
}

/**
 * Hand off an external-clipboard paste to the backend for conversion
 * into a tree of blocks. The Rust side normalises external syntax
 * (Roam `{{[[TODO]]}}`, GitHub checkboxes, etc.), parses the bullet
 * structure, and grafts it under `blockId` at the caret position.
 *
 * Returns the refreshed `PageView` so the caller can re-render.
 * `caret` is a `char` offset into the host block's text — for ASCII
 * content the textarea's `selectionStart` (UTF-16 code units) is
 * equivalent. The frontend should `preventDefault` on the original
 * paste event before calling this so the default browser splice
 * doesn't run alongside the backend conversion.
 */
export function pasteMarkdown(
  pageId: string,
  blockId: string,
  caret: number,
  text: string,
): Promise<PageView> {
  return invoke<PageView>("paste_markdown_at", {
    pageId,
    blockId,
    caret,
    text,
  });
}

/**
 * Paste clipboard text **without formatting** — the raw string is
 * spliced into the host block at `caret`, with no outline detection,
 * syntax normalization, or paragraph splitting. The plain counterpart of
 * {@link pasteMarkdown} (desktop `Cmd+Shift+V`). Returns the refreshed
 * `PageView`.
 */
export function pastePlain(
  pageId: string,
  blockId: string,
  caret: number,
  text: string,
): Promise<PageView> {
  return invoke<PageView>("paste_plain_at", {
    pageId,
    blockId,
    caret,
    text,
  });
}

/**
 * Serialize a block selection — each block plus its full subtree — to
 * clean outl markdown for the OS clipboard. The inverse of
 * {@link pasteMarkdown}: copy out, paste back into outl, same tree.
 *
 * `blockIds` is in document order (a single block, or a Visual range
 * top-to-bottom); the markdown preserves it. A descendant whose ancestor
 * is also selected is dropped (the ancestor's subtree already carries
 * it), so a range spanning a parent and child never duplicates the child.
 *
 * Read-only — the backend produces the string, the caller writes it to
 * `navigator.clipboard.writeText`.
 */
export function copyMarkdown(blockIds: string[]): Promise<string> {
  return invoke<string>("copy_markdown", { blockIds });
}

/**
 * Persist the collapsed flag on a single block. The backend generates
 * `Op::SetCollapsed` and appends it to the device's per-actor
 * `ops-<actor>.jsonl` so the flag converges across peers through the
 * tree CRDT — never written to the sidecar (last-write-wins per file
 * would lose concurrent flips). Returns the refreshed page view so
 * the caller can re-render in one round trip.
 */
export function setBlockCollapsed(
  pageId: string,
  id: string,
  collapsed: boolean,
): Promise<PageView> {
  return invoke<PageView>("set_block_collapsed", { pageId, id, collapsed });
}

/**
 * Run the fenced code block identified by `blockId` inside the page
 * identified by `pageId`. The Rust side resolves the flat-DFS index,
 * runs `outl_exec::run_block_at_index` on a worker thread, and
 * persists the result as a `> **result:**` sibling subblock.
 *
 * The reply bundles the refreshed `PageView` so the caller swaps the
 * outline straight in — no follow-up navigation round-trip needed.
 * `result_ok` is the runtime payload (stdout/stderr/exit/duration);
 * `error` is an infrastructure failure message (unknown language,
 * timeout, sandbox crash). They are mutually exclusive.
 */
export function runCodeBlock(
  pageId: string,
  blockId: string,
): Promise<RunCodeBlockReply> {
  return invoke<RunCodeBlockReply>("run_code_block", { pageId, blockId });
}

/**
 * Sweep the page for query blocks (runtimes with `auto_run() == true`)
 * and execute them. Returns the refreshed `PageView`.
 */
export function runAutoRunBlocks(
  pageId: string,
): Promise<{ ran: number; view: PageView }> {
  return invoke<{ ran: number; view: PageView }>("run_auto_run_blocks", {
    pageId,
  });
}

/**
 * Batch-resolve embed handles (`blk-XXXXXX`) to their source content.
 * Returns a map from handle to {@link ResolvedBlock} — the source
 * block's `text`, `page_slug`, `status`, and its `children` subtree
 * (populated for `!((…))` embeds, empty for a leaf).
 */
export function resolveEmbeds(
  handles: string[],
): Promise<Record<string, ResolvedBlock>> {
  return invoke<Record<string, ResolvedBlock>>("resolve_embeds", { handles });
}

/**
 * Open an external `[label](url)` link in the user's default browser
 * via `tauri-plugin-opener`. Shared by every client's `onLinkClick`
 * handler so the scheme allow-list lives in one place.
 *
 * Only `http(s)` and `mailto` are allowed — anything else (`file:`,
 * `javascript:`, …) is rejected so a crafted link inside a synced
 * `.md` can't trigger an arbitrary local action. The promise rejects
 * on a malformed or disallowed URL; callers surface it on the status
 * line. The host must register the opener plugin and grant
 * `opener:allow-open-url` for the call to succeed.
 */
export async function openExternalUrl(href: string): Promise<void> {
  let scheme: string;
  try {
    scheme = new URL(href).protocol.replace(/:$/, "").toLowerCase();
  } catch {
    // `href` comes from synced / remote markdown — strip control chars
    // and cap length before it reaches the status line so a hostile
    // `.md` can't flood or corrupt the UI with the error string.
    throw new Error(`refusing to open malformed URL: ${describeHref(href)}`);
  }
  if (scheme !== "http" && scheme !== "https" && scheme !== "mailto") {
    throw new Error(`refusing to open non-web URL scheme: ${scheme}:`);
  }
  await openUrl(href);
}

/**
 * Open a workspace asset (`assets/<hash>.<ext>`) in the OS default app
 * (Preview, an image viewer, …). outl does **not** render the file — the
 * backend resolves the link to an absolute path under `<root>/assets/`
 * (rejecting traversal / external schemes) and hands it to the OS.
 *
 * The promise rejects when the asset hasn't synced to this device yet or
 * the link is invalid; callers surface it on the status line. Route a
 * clicked link here only when {@link isAssetLink} is true — external
 * links go through {@link openExternalUrl}.
 */
export function openAsset(url: string): Promise<void> {
  return invoke<void>("open_asset", { url });
}

/**
 * Resolve a workspace asset (`assets/<hash>.<ext>`) to a
 * `data:<mime>;base64,<…>` URL the webview can load directly — an
 * `<img src>` for images, a PDF viewer for pdf.
 *
 * The backend resolves the link under `<root>/assets/` (rejecting
 * traversal / external schemes), reads the bytes (size-capped so a giant
 * file can't wedge the renderer) and base64-encodes them. Use only when
 * {@link isAssetLink} is true; a remote `http(s)` image can be an `<img
 * src>` directly. The promise rejects when the asset hasn't synced to
 * this device yet or is too large — callers show a fallback chip.
 */
export function readAssetDataUrl(url: string): Promise<string> {
  return invoke<string>("read_asset_data_url", { url });
}

/**
 * Import a file from the OS into the workspace and attach its link as a
 * new block, returning the refreshed {@link PageView}.
 *
 * The backend copies the file into `<root>/assets/<hash>.<ext>`
 * (content-addressed, idempotent, size-capped by `[assets] max_bytes`)
 * and inserts a block carrying its markdown link. `afterBlockId` places
 * the new block right after that block (tolerating a stale anchor);
 * omitting it appends at the end of the page.
 */
export function attachAsset(
  sourcePath: string,
  pageId: string,
  afterBlockId?: string,
): Promise<PageView> {
  return invoke<PageView>("attach_asset", {
    sourcePath,
    pageId,
    afterBlockId: afterBlockId ?? null,
  });
}

/**
 * The result of importing a file: where it landed under `assets/` and the
 * ready-to-insert markdown link. Mirror of `outl_actions::ImportedAsset`.
 */
export interface ImportedAsset {
  /** Workspace-relative link target, e.g. `assets/<hash>.pdf`. */
  rel_path: string;
  /** Original file name — the link's display text. */
  display_name: string;
  /** Whether the asset is an image (`![]()` vs `[]()`). */
  is_image: boolean;
  /** Ready-to-insert markdown: `![name](rel)` for images, else `[name](rel)`. */
  markdown: string;
}

/**
 * Import a file into the workspace **without** touching the outline, and
 * return its {@link ImportedAsset} (with `markdown` ready to splice).
 *
 * This is the drag-and-drop path: a file dropped onto the active line is
 * imported here, then the client inserts `markdown` straight into that
 * block's editor text at the caret — so the insert respects an in-flight
 * edit instead of racing it through a backend mutation. The copy is
 * content-addressed / idempotent / size-capped, exactly like
 * {@link attachAsset}, but no block is created here; the client's own
 * commit writes the block text. Use {@link attachAsset} when you want a
 * fresh block instead (the file-picker button).
 */
export function importAssetFile(sourcePath: string): Promise<ImportedAsset> {
  return invoke<ImportedAsset>("import_asset_file", { sourcePath });
}

// ---------------------------------------------------------------------------
// Peer / device pairing (iroh sync transport)
// ---------------------------------------------------------------------------
//
// These wrap the `outl_peer_*` Tauri commands both clients register in
// `src-tauri/src/commands/peers.rs`. They touch the iroh `peers.json`,
// not the workspace lock — peer pairing is sync-transport state, not
// workspace state. See `outl-mobile/CLAUDE.md` / `outl-desktop/CLAUDE.md`
// § "Peers".

/**
 * List every paired device. Mirrors `outl_peer_list`. Reads the iroh
 * `peers.json` (empty list when the file is absent).
 */
export function peerList(): Promise<PeerDto[]> {
  return invoke<PeerDto[]>("outl_peer_list");
}

/**
 * Remove paired peers whose `node_id` starts with `id` (prefix match).
 * Resolves `true` when at least one peer matched and was removed.
 * Mirrors `outl_peer_remove`.
 */
export function peerRemove(id: string): Promise<boolean> {
  return invoke<boolean>("outl_peer_remove", { id });
}

/**
 * Live reachability + RTT for each paired peer. Mirrors
 * `outl_peer_status`. Reads the running iroh transport's own dial
 * outcomes (`peer_health()`) — no fresh probe endpoint — and merges
 * them onto the full `peers.json` list, so a peer the transport hasn't
 * dialed yet (or the file-transport case) comes back `online: false`.
 * Returns one {@link PeerStatusDto} per paired peer.
 */
export function peerStatus(): Promise<PeerStatusDto[]> {
  return invoke<PeerStatusDto[]>("outl_peer_status");
}

/**
 * Force an immediate P2P (iroh) sync pass against every paired peer.
 * Mirrors `outl_sync_now`.
 *
 * Backs the GUI's pull-to-refresh / "sync now" affordance: instead of
 * waiting for the iroh transport's ~8s catch-up tick, this dials every
 * peer right now to pull the freshest state. Callers typically chain it
 * with {@link reloadWorkspace} (sync, then re-render):
 *
 * ```ts
 * await syncNow();
 * await reloadWorkspace();
 * ```
 *
 * Resolves with no value. A no-op on the backend when no iroh transport
 * is wired (the iCloud file transport has no peer to dial) or its runtime
 * is down — it never rejects for "nothing to sync", so a missing peer
 * mesh is silent rather than an error.
 */
export function syncNow(): Promise<void> {
  return invoke<void>("outl_sync_now");
}

/**
 * Host a pairing session and resolve with the **ticket string** the
 * other device scans / types to join. Mirrors `outl_peer_pair_host`.
 *
 * The ticket comes back as soon as the iroh endpoint is bound — long
 * before a peer actually connects — so the caller can render it (e.g.
 * via {@link import("../peers").PairingQR}) while the handshake runs in
 * the background. The completed pairing surfaces through the backend's
 * `peer-paired` Tauri event (payload: {@link PeerDto}); listen for it
 * to refresh the device list. `peer-pair-failed` (payload: error
 * string) fires if the handshake times out or errors.
 *
 * `alias` is this device's own human label. We advertise it to the joining
 * device, which stores it under *our* node id in its `peers.json`. Defaults
 * to the platform name ("desktop" / "mobile") when omitted.
 *
 * Backend note: the desktop's `outl_peer_pair_host` currently resolves
 * with the paired peer object instead of the ticket and emits the
 * ticket early via a `peer-pairing-ticket` event; the mobile command
 * resolves with the ticket directly. This wrapper follows the mobile
 * contract (ticket string) — the desktop Rust command is being aligned
 * to it so both clients share this surface.
 */
export function peerPairHost(alias?: string | null): Promise<string> {
  return invoke<string>("outl_peer_pair_host", { alias: alias ?? null });
}

/**
 * Join a pairing session from a `ticket` produced by a host's
 * {@link peerPairHost}. Connects, completes the handshake, persists the
 * host to `peers.json`, and resolves with the newly paired
 * {@link PeerDto}. Mirrors `outl_peer_pair_join`.
 *
 * `alias` is this device's own human label, advertised to and stored by
 * the host (it persists under *our* node id in the host's `peers.json`).
 * The returned {@link PeerDto} carries the *host's* alias, not this one.
 */
export function peerPairJoin(ticket: string, alias?: string | null): Promise<PeerDto> {
  return invoke<PeerDto>("outl_peer_pair_join", { ticket, alias: alias ?? null });
}

// ---------------------------------------------------------------------------
// External links
// ---------------------------------------------------------------------------

/** Make an untrusted `href` safe to show in an error/status message:
 *  drop control characters and truncate to a sane length. */
function describeHref(href: string): string {
  // Drop control characters (charCode < 0x20 or DEL) and truncate so a
  // hostile `href` can't flood or corrupt the status line.
  const cleaned = Array.from(href)
    .filter((c) => {
      const code = c.charCodeAt(0);
      return code >= 0x20 && code !== 0x7f;
    })
    .join("");
  return cleaned.length > 100 ? `${cleaned.slice(0, 100)}…` : cleaned;
}

// ── Plugin host ─────────────────────────────────────────────────────
// Both GUI clients register identical `plugin_list` / `plugin_run` /
// `plugin_sync_hooks` / `plugin_toolbar` / `plugin_transformers` /
// `plugin_transform` commands (thin shims over `PluginService` — the
// Boa host is `!Send`, so it runs on a dedicated thread), so the
// wrappers live here once. The desktop-only `plugin_keybindings`
// stays in `outl-desktop/src/lib/api.ts` (mobile has no chord surface).

/**
 * List every command contributed by a loaded plugin. Empty until the
 * workspace opens and plugins load (best-effort — never throws on an
 * empty or failed host).
 */
export function pluginList(): Promise<PluginCommand[]> {
  return invoke<PluginCommand[]>("plugin_list");
}

/**
 * Run a plugin command. Pass the currently-open page id so the reply
 * carries its refreshed `PageView` — the plugin thread re-projects
 * every page's `.md` before returning (a plugin can move blocks across
 * pages).
 */
export function pluginRun(
  pluginId: string,
  commandId: string,
  pageId: string | null,
): Promise<PluginRunReply> {
  return invoke<PluginRunReply>("plugin_run", {
    pluginId,
    commandId,
    pageId,
  });
}

/**
 * Fire the plugins' `onOp` hook sweep after a user mutation. The
 * reply's `view` is the refreshed `PageView` of `pageId` **only** when
 * a hook actually mutated the workspace (absent otherwise, so the
 * caller skips a needless render); `views` carries any `ui-render`
 * HTML the hooks emitted (the confetti path — present even when
 * nothing was re-rendered). Best-effort — a host with no op-hook
 * plugins is a cheap no-op.
 */
export function pluginSyncHooks(
  pageId: string | null,
): Promise<PluginSyncHooksReply> {
  return invoke<PluginSyncHooksReply>("plugin_sync_hooks", { pageId });
}

/**
 * List every toolbar button a loaded plugin contributes to the client
 * chrome — one button per entry (glyph = `icon`, tooltip = `title`,
 * click / tap = {@link pluginRun}). Empty until plugins load
 * (best-effort — never throws).
 */
export function pluginToolbar(): Promise<PluginToolbarButton[]> {
  return invoke<PluginToolbarButton[]>("plugin_toolbar");
}

/**
 * List every content transformer a loaded plugin declared. Load once
 * per workspace open and match each code fence's language against the
 * result. Empty until plugins load (best-effort — never throws).
 */
export function pluginTransformers(): Promise<PluginTransformer[]> {
  return invoke<PluginTransformer[]>("plugin_transformers");
}

/**
 * Run a content transformer for `lang` against a fence `input` (its
 * body). Read-only: never mutates the workspace. Resolves to `null`
 * when the transformer declined or no plugin owns `lang`, otherwise
 * the `{ kind, content }` descriptor. Cache the result by
 * `(blockId, body)` — re-run only when the body changes (see
 * `@outl/shared/plugins/transformer-registry`).
 */
export function pluginTransform(
  pluginId: string,
  lang: string,
  input: string,
): Promise<PluginTransformResult | null> {
  return invoke<PluginTransformResult | null>("plugin_transform", {
    pluginId,
    lang,
    input,
  });
}

// ── Plugin marketplace ──────────────────────────────────────────────
// Both GUI clients register identical `plugin_registry_list` /
// `plugin_install_official` / `plugin_set_enabled` / `plugin_uninstall`
// commands on their src-tauri side, so the wrappers live here once.

/** Fetch the marketplace: the official registry crossed with the lockfile. */
export function pluginRegistryList(): Promise<RegistryItem[]> {
  return invoke<RegistryItem[]>("plugin_registry_list");
}

/** Tap-to-install an official plugin by id; resolves to its display name. */
export function pluginInstallOfficial(id: string): Promise<string> {
  return invoke<string>("plugin_install_official", { id });
}

/** Enable / disable an installed plugin. */
export function pluginSetEnabled(id: string, enabled: boolean): Promise<void> {
  return invoke<void>("plugin_set_enabled", { id, enabled });
}

/** Uninstall a plugin; resolves `true` if anything was removed. */
export function pluginUninstall(id: string): Promise<boolean> {
  return invoke<boolean>("plugin_uninstall", { id });
}

/**
 * Describe a plugin's settings form: every config/secret field with its type,
 * current value, and — for secrets — whether it is set (never the value).
 * Empty when the plugin declares no config schema.
 */
export function pluginSettingsDescribe(
  pluginId: string,
): Promise<PluginSettingsField[]> {
  return invoke<PluginSettingsField[]>("plugin_settings_describe", { pluginId });
}

/**
 * Set a plaintext config field. The host coerces the string to the field's
 * schema type and reloads the plugin so the change is live. Rejects secret
 * fields — use {@link pluginSecretSet}.
 */
export function pluginConfigSet(
  pluginId: string,
  key: string,
  value: string,
): Promise<void> {
  return invoke<void>("plugin_config_set", { pluginId, key, value });
}

/** Store a secret field's value in the OS keychain (never on disk). */
export function pluginSecretSet(
  pluginId: string,
  key: string,
  value: string,
): Promise<void> {
  return invoke<void>("plugin_secret_set", { pluginId, key, value });
}

/** Delete a secret field's value from the keychain (idempotent). */
export function pluginSecretRemove(pluginId: string, key: string): Promise<void> {
  return invoke<void>("plugin_secret_remove", { pluginId, key });
}

/**
 * Filter marketplace rows by a query (case-insensitive substring over id,
 * name, description, and capabilities). Empty query returns every item.
 * Pure — both the desktop modal and the mobile sheet derive their list from
 * it, so the match rule stays in one place.
 */
export function filterRegistryItems(
  items: readonly RegistryItem[],
  query: string,
): RegistryItem[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...items];
  return items.filter(
    (i) =>
      i.id.toLowerCase().includes(q) ||
      i.name.toLowerCase().includes(q) ||
      i.description.toLowerCase().includes(q) ||
      i.capabilities.some((c) => c.toLowerCase().includes(q)),
  );
}

// ── Reminders (`remind::`) ──────────────────────────────────────────
//
// The schedule itself is decided in Rust (`outl_actions::reminders`),
// once, for every client. These wrappers only move the answer across
// the bridge — never re-derive "when does this fire" in TS.

/** Every reminder in the workspace, soonest first; finished ones last. */
export function listReminders(): Promise<Reminder[]> {
  return invoke<Reminder[]>("list_reminders");
}

/** This device's reminder delivery preferences. */
export function reminderSettings(): Promise<ReminderSettings> {
  return invoke<ReminderSettings>("reminder_settings");
}

/**
 * Silence a block's reminder for `minutes` from now.
 *
 * Writes `Op::SnoozeRemind`, so the same block goes quiet on every
 * paired device — snoozing on the phone must not leave the laptop
 * buzzing.
 */
export function snoozeReminder(blockId: string, preset: string): Promise<void> {
  return invoke<void>("snooze_reminder", { blockId, preset });
}

/**
 * The snooze options, in render order.
 *
 * Fetched rather than hardcoded because two of the three aren't fixed
 * offsets — "tomorrow 9am" is a wall time — so a client doing its own
 * arithmetic gets them wrong. Render `label`, send back `id`.
 */
export function snoozePresets(): Promise<SnoozePreset[]> {
  return invoke<SnoozePreset[]>("snooze_presets");
}

/** Clear a snooze so the block resumes its normal schedule. */
export function clearReminderSnooze(blockId: string): Promise<void> {
  return invoke<void>("clear_reminder_snooze", { blockId });
}

/**
 * Set — or clear, with an empty `rule` — a block's `remind::` property.
 * Returns the refreshed page. Editing the rule reschedules from
 * scratch, which falls out for free: the schedule is derived on every
 * scan, never cached.
 */
export function setBlockRemind(
  pageId: string,
  blockId: string,
  rule: string,
): Promise<PageView> {
  return invoke<PageView>("set_block_remind", { pageId, blockId, rule });
}

/**
 * Mark a block DONE, cancelling every pending fire of its rule.
 *
 * Not `toggleTodo`. A rule can sit on a block with no TODO marker at
 * all, and toggling that advances it to `TODO` — so the reminders
 * list's "mark done" button used to arm the nag instead of cancelling
 * it. Setting the state outright is also idempotent, which matters
 * for a button that can be double-tapped.
 */
export function markBlockDone(
  pageId: string,
  blockId: string,
): Promise<PageView> {
  return invoke<PageView>("mark_block_done", { pageId, blockId });
}

/**
 * Deliver every reminder that came due as an OS notification, and
 * return what was delivered so an open Reminders panel can refresh
 * without a second round trip.
 *
 * Safe to call on a plain interval: it short-circuits when the device
 * has reminders switched off, and the Rust side keeps the device-local
 * "already fired" log so polling twice never double-buzzes.
 */
export function deliverDueReminders(): Promise<Reminder[]> {
  return invoke<Reminder[]>("deliver_due_reminders");
}

/**
 * How long until `iso` (a local ISO datetime from {@link Reminder}),
 * as a short human label: `"now"`, `"in 20min"`, `"in 3h"`,
 * `"tomorrow 09:00"`, `"Dec 15, 10:00"`.
 *
 * Shared because both the desktop panel and the mobile list show the
 * same column, and two implementations of "in 3h" drift on the edge
 * cases (exactly 60 minutes, midnight rollover) before anyone notices.
 */
export function formatNextFire(iso: string | null, now: Date = new Date()): string {
  if (!iso) return "—";
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "—";
  const minutes = Math.round((at.getTime() - now.getTime()) / 60000);
  if (minutes <= 0) return "now";
  if (minutes < 60) return `in ${minutes}min`;
  const hhmm = `${String(at.getHours()).padStart(2, "0")}:${String(
    at.getMinutes(),
  ).padStart(2, "0")}`;
  // Same calendar day → the relative form reads better than a clock time.
  if (at.toDateString() === now.toDateString()) {
    return `in ${Math.round(minutes / 60)}h`;
  }
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  if (at.toDateString() === tomorrow.toDateString()) return `tomorrow ${hhmm}`;
  return `${at.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${hhmm}`;
}

/**
 * Bucket a reminder list into the groups every client renders:
 * Today / Tomorrow / This week / Later / Done. Order is stable and
 * empty buckets are dropped, so a client can map straight over it.
 */
export function groupReminders(
  reminders: readonly Reminder[],
  now: Date = new Date(),
): { label: string; items: Reminder[] }[] {
  const buckets: Record<string, Reminder[]> = {
    Today: [],
    Tomorrow: [],
    "This week": [],
    Later: [],
    Done: [],
  };
  const startOfDay = (d: Date) =>
    new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const today = startOfDay(now);
  const day = 86_400_000;

  for (const r of reminders) {
    if (!r.next_fire) {
      buckets.Done.push(r);
      continue;
    }
    const at = new Date(r.next_fire);
    const delta = startOfDay(at) - today;
    if (delta <= 0) buckets.Today.push(r);
    else if (delta === day) buckets.Tomorrow.push(r);
    else if (delta < 7 * day) buckets["This week"].push(r);
    else buckets.Later.push(r);
  }
  return Object.entries(buckets)
    .filter(([, items]) => items.length > 0)
    .map(([label, items]) => ({ label, items }));
}

/**
 * Write this device's reminder settings.
 *
 * Mobile has no settings screen, so the Reminders sheet is the only
 * place that can turn delivery on. Without this the sheet could say
 * "notifications are off on this device" and leave the user with no
 * way to change it, since `config.toml` sits inside the iOS sandbox.
 */
export function setReminderSettings(
  enabled: boolean,
  quietHours: string,
): Promise<ReminderSettings> {
  return invoke<ReminderSettings>("set_reminder_settings", {
    enabled,
    quietHours,
  });
}

/**
 * Set — or clear, with an empty `value` — any `key:: value` property
 * on a block. Returns the refreshed page.
 *
 * Generic rather than one command per key: the property chips render
 * whatever the block carries, so the writer has to accept whatever the
 * user edits. {@link setBlockRemind} is the named shortcut for the
 * authoring chords.
 */
export function setBlockProperty(
  pageId: string,
  blockId: string,
  key: string,
  value: string,
): Promise<PageView> {
  return invoke<PageView>("set_block_property", { pageId, blockId, key, value });
}

/**
 * Property keys used anywhere in the workspace, most-used first.
 *
 * Deliberately **not** cached: the backend answer is a scan of the
 * property map (no tree walk, no block text), so it is cheaper to ask
 * every time an editor opens than to hold a list that goes stale the
 * first time the user adds a key.
 */
export function knownPropertyKeys(): Promise<PropertyKey[]> {
  return invoke<PropertyKey[]>("known_property_keys");
}

/**
 * Set — or, with an empty `value`, clear — a property on the **page**
 * itself (`icon::`, `type::`, …). Rejects the structural keys
 * (`page-slug`, `page-kind`): those are the page's identity and
 * renaming is `page_rename`, not a property edit.
 */
export function setPageProperty(
  pageId: string,
  key: string,
  value: string,
): Promise<PageView> {
  return invoke<PageView>("set_page_property", { pageId, key, value });
}
