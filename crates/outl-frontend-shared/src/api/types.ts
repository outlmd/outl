/**
 * DTOs returned by the Rust backend over the Tauri bridge.
 *
 * Every shape here mirrors a `serde`-serialized Rust type. Adding a
 * field on the Rust side means extending the interface here in the
 * same change — backend and frontend share the wire format, never
 * a generator.
 */

/**
 * One entry of the workspace's property-key catalogue: the key as most
 * of its uses spell it, and how many properties use it.
 *
 * Mirrors `outl_tauri_shared::commands::property::PropertyKey`. Lives
 * here rather than per client because both GUI clients register the
 * same `known_property_keys` command and ask it the same question when
 * the user adds a property; two copies of a wire type is how they end
 * up disagreeing about a field.
 */
export interface PropertyKey {
  key: string;
  uses: number;
}

export type TodoState = "TODO" | "DOING" | "DONE";
export type PageKind = "page" | "journal";
/** Direction of the backlinks ("Linked from") list (issue #142).
 * Mirrors `outl_config::BacklinksOrder`. */
export type BacklinksOrder = "newest" | "oldest";

/**
 * Pre-tokenized inline markdown coming from the Rust backend
 * (`outl_md::tokenize_owned`). The renderer at
 * `@outl/shared/markdown::MarkdownInline` maps each variant to JSX.
 * There is no parallel TS tokenizer — `outl_md::inline::tokenize` is
 * the single source of truth for inline syntax across every client.
 * Adding a token in Rust means extending this union and the renderer
 * switch in the same change.
 */
export type InlineToken =
  | { kind: "plain"; value: string }
  // Bold / italic / strike carry their inner span as a re-tokenized
  // list so nested refs, tags, and block-refs render with their own
  // styling. `**[[avelino]]**` arrives as `Bold { inner: [Ref … ] }`
  // — the renderer wraps the inner tokens in the bold style.
  | { kind: "bold"; inner: InlineToken[] }
  | { kind: "italic"; inner: InlineToken[] }
  | { kind: "strike"; inner: InlineToken[] }
  // `==highlight==` — the on-disk form of Roam's `^^highlight^^`.
  | { kind: "highlight"; inner: InlineToken[] }
  | { kind: "code"; value: string }
  | { kind: "link"; value: string; href: string }
  // `![alt](href)` image / embedded asset. `href` is a workspace-relative
  // `assets/<hash>.<ext>` path or a remote URL. The renderer shows an
  // `<img>` for image extensions and a file chip for other kinds (pdf,
  // …). Mirrors `outl_md::InlineToken::Image { alt, href }`.
  | { kind: "image"; alt: string; href: string }
  | { kind: "ref"; value: string }
  | { kind: "tag"; value: string }
  | { kind: "blockref"; value: string }
  | { kind: "embed"; value: string }
  // `:shortcode:` — GitHub gemoji shortcode. `shortcode` is the disk
  // form (`"tada"`); `glyph` is the resolved unicode codepoint
  // (`"🎉"`). The renderer shows the glyph and surfaces the shortcode
  // for hover / `aria-label`. Mirrors `outl_md::InlineToken::Emoji`.
  | { kind: "emoji"; shortcode: string; glyph: string };

export interface BlockNode {
  id: string;
  text: string;
  todo: TodoState | null;
  /**
   * Inline markdown tokens for `text` (no TODO/DONE prefix). Backend
   * pre-tokenizes via `outl_md::tokenize_owned` so the renderer
   * doesn't run a second tokenizer in JS. See {@link InlineToken}.
   */
  tokens: InlineToken[];
  /**
   * UI fold state overlaid from the backend's op log. `true` means
   * the children are hidden in the outline. Mutated via
   * {@link import("./commands").setBlockCollapsed}, which generates
   * `Op::SetCollapsed` and appends it to the device's
   * `ops-<actor>.jsonl`; iCloud / Syncthing propagate the per-actor
   * file and every peer's CRDT replays the op in HLC order. The
   * sidecar is never written for this flag.
   */
  collapsed: boolean;
  /**
   * `(key, value)` block properties — the `key:: value` lines a user
   * authored under the block in markdown. Empty when the block has
   * none. Backend builds this from `Op::SetProp` entries so it
   * survives the same way collapsed does (op log, not sidecar).
   */
  properties: Array<[string, string]>;
  children: BlockNode[];
}

/** A block resolved from its ref handle (`blk-XXXXXX`) — reply shape of
 *  `resolveEmbeds`. Serves both surfaces the handle appears in:
 *  `((…))` inline refs render `text`; `!((…))` embeds render `text` plus
 *  the `children` subtree. Mirrors the Rust `EmbedContent`. */
export interface ResolvedBlock {
  handle: string;
  text: string;
  page_slug: string;
  status: string | null;
  children: BlockNode[];
}

export interface PageMeta {
  id: string;
  slug: string;
  title: string;
  kind: PageKind;
  /**
   * Optional emoji / icon string the user set on the page via the
   * `icon::` page property. Backend omits the field when unset; the
   * frontend can fall back to `📄`/`📅` based on `kind`.
   */
  icon?: string;
  /**
   * `pinned:: true` page-level property. Sidebars surface pinned
   * pages prominently (canonical entry points like "inbox" or
   * "weekly review"). Defaults to `false`; backend serialises the
   * field only when `true` so an older binary reading a newer wire
   * still parses cleanly.
   */
  pinned?: boolean;
  /**
   * `type::` page-level property, lowercased+trimmed. `null`/omitted
   * when unset. The `@` mention autocomplete filters to
   * `page_type === "person"` candidates without re-querying the
   * workspace index. Mirrors `outl_actions::PageMeta.page_type` and
   * `outl_md::index::PageEntry.page_type`.
   */
  page_type?: string | null;
}

/**
 * One ancestor step in a backlink's breadcrumb. Plain text only — the
 * breadcrumb is dimmed context, so clients render a muted trail rather
 * than re-rendering inline markdown the way they do for the citing
 * block. `text` already has the `TODO `/`DONE ` prefix stripped. Mirrors
 * `outl_actions::BacklinkCrumb`. (Same shape as `FocusCrumb` from
 * `@outl/shared/outline`, but a distinct concept — zoom vs. backlink.)
 */
export interface BacklinkCrumb {
  id: string;
  text: string;
}

export interface Backlink {
  block_id: string;
  todo: TodoState | null;
  source_page: PageMeta | null;
  /**
   * Ancestor blocks between the source page root and the citing block,
   * **root-first**: `ancestors[0]` is the direct child of the page
   * root, the last entry is the block's immediate parent. Empty when
   * the block sits at page-root level. The page root itself is not
   * included — clients show it as the group header. Mirrors
   * `outl_actions::Backlink::ancestors`.
   */
  ancestors: BacklinkCrumb[];
  /**
   * Source block as a self-contained outline subtree (text, tokens,
   * children, properties). Mirrors what `read_page_view_with_workspace`
   * would return for the same block. Clients read
   * `source_block.tokens` for the inline markdown; the raw text
   * lives at `source_block.text` if a caller ever needs it.
   *
   * Note: the Rust `Backlink` struct also carries `block_text` for
   * the CLI/MCP JSON envelope. It's intentionally omitted from this
   * TS interface because no client reads it — Tauri still ships the
   * field over the wire; JS just ignores it.
   */
  source_block: BlockNode;
  /**
   * DFS path of the source block inside `source_page`. Empty array
   * means the block is a direct child of the page root. Mirrors
   * `outl_actions::Backlink::source_block_path` — used by clients
   * that navigate inside a backlink's subtree.
   */
  source_block_path: number[];
  /**
   * On-disk path of `source_page`'s `.md` (inside the workspace
   * storage root — iCloud container on mobile, user-picked path on
   * desktop). Backend omits the field when the source block has no
   * enclosing page (legacy data).
   */
  source_path?: string;
}

export interface PageView {
  page: PageMeta;
  outline: BlockNode[];
  /**
   * **Always empty from the open commands now.** Backlinks moved off the
   * page-open path (`backlinks_for_page` is an O(blocks-in-workspace)
   * scan that used to block the first journal paint on a large
   * workspace). Fetch them lazily with {@link PageBacklinks} via
   * `pageBacklinks(slug)` after the outline renders — the same lazy
   * policy the TUI uses. Kept in the wire shape for back-compat.
   */
  backlinks: Backlink[];
  /** Direction `backlinks` was sorted in (issue #142) — `"newest"` or
   * `"oldest"`. Lets a client's direction toggle show the right arrow
   * without a separate settings read. Mirrors `PageView.backlinks_order`. */
  backlinks_order: BacklinksOrder;
  /**
   * The page's own `key:: value` properties (`icon::`, `type::`, …),
   * alpha-sorted, with the structural keys (`page-slug`, `page-kind`)
   * already filtered out by the backend. Same shape as
   * {@link BlockNode.properties}, so one editor serves both.
   *
   * Optional on the wire only for back-compat with an older binary;
   * a current backend always sends it (empty array when the page has
   * no properties). Mirrors `PageView.page_properties`.
   */
  page_properties?: Array<[string, string]>;
  /**
   * Parser recovery records for the page's `.md`. Empty (or
   * absent) when the file is fully in the outl dialect. Drives
   * the `<ParseWarningsBanner />` above the outline so the user
   * knows outl had to keep lines that don't match the dialect
   * (e.g. a leading `# heading`, a stray paragraph, imported
   * markdown). Mirrors `outl_md::ParseWarning` exactly.
   */
  warnings?: ParseWarning[];
  /**
   * Present when this page stopped syncing because its `.md` holds
   * content that exists in no op, so outl refuses to overwrite the file
   * (root `CLAUDE.md` invariant 8). Absent on every healthy page.
   *
   * Drives the `<PageAheadOfLogBanner />` above the outline. Rendering
   * nothing here is what the state used to do — the refusal died in a
   * backend log line and the page just quietly stopped updating.
   * Mirrors `outl_tauri_shared::state::MdAheadOfLog`.
   */
  md_ahead_of_log?: MdAheadOfLog;
  /**
   * `true` when this reply actually ran the check, so
   * {@link PageView.md_ahead_of_log} is authoritative **in both
   * directions**: absent then means the page is healthy again, and the
   * banner must clear.
   *
   * Only the open commands run it. A mutation reply is built from the
   * tree and can never carry the notice, which is why a client keeps the
   * banner across those (clearing it there would drop the warning on the
   * user's first edit — the very action it warns against). This bit is
   * what tells the client when clearing *is* correct, e.g. right after
   * `outl reconcile --ahead-of-log` fixed the page.
   *
   * Mirrors `outl_tauri_shared::state::PageView.md_ahead_of_log_checked`.
   */
  md_ahead_of_log_checked?: boolean;
}

/**
 * Why a page stopped syncing.
 *
 * outl will not re-project a `.md` that holds lines the op log never
 * recorded, because that write deletes them for good. The cost of
 * refusing is that the page is frozen in both directions until
 * `outl reconcile --ahead-of-log` runs: those lines never reach another
 * device, and a peer's edits never reach this file.
 *
 * Mirrors `outl_tauri_shared::state::MdAheadOfLog`.
 */
export interface MdAheadOfLog {
  /** Absolute path of the `.md` that was left alone. */
  path: string;
  /** How many content lines exist only on this device. */
  lines: number;
  /** One of those lines, already quoted by the backend. */
  sample: string;
}

/**
 * Reply from `pageBacklinks(slug)` and `setBacklinksOrder(...)`.
 *
 * Backlinks are fetched lazily, decoupled from {@link PageView}, so the
 * O(blocks-in-workspace) `backlinks_for_page` scan never blocks the
 * journal appearing. Mirrors the Rust `BacklinksReply`.
 */
export interface PageBacklinks {
  backlinks: Backlink[];
  backlinks_order: BacklinksOrder;
}

/**
 * Reason a parser warning was emitted.
 *
 * Mirrors `outl_md::ParseWarningKind`. The enum is serialised as
 * snake_case strings (`"unrecognized_block_marker"`), so the union
 * stays narrow and future variants land here in lockstep.
 */
export type ParseWarningKind = "unrecognized_block_marker";

/**
 * One non-fatal parser recovery. Carries the **1-based** source
 * line number and the raw line text, so a UI can highlight the
 * offending row without re-scanning the file.
 *
 * Mirrors `outl_md::ParseWarning` (`crates/outl-md/src/parse.rs`).
 * See `docs/markdown-format.md` § "Permissive parsing & warnings"
 * for the user-facing contract.
 */
export interface ParseWarning {
  line: number;
  raw: string;
  kind: ParseWarningKind;
}

/**
 * Reply of {@link import("./commands").createBlock}. Pairs the
 * refreshed {@link PageView} with the id of the freshly-inserted
 * block so the client can focus / put the new row into edit mode
 * without re-discovering the id by diffing the outline.
 *
 * Why the new id is on the wire: the previous design returned only
 * the `PageView` and let the frontend find the new block via a DFS
 * walk (`flat[idx+1]` after the anchor). That walk lands on the
 * anchor's *first child* whenever the anchor has children, not on
 * the new sibling. The eventual `editBlock(stale_id, ...)` then
 * targeted an id that may have been moved or already replaced, and
 * the backend surfaced `block <ULID> is not in the tree` as a toast.
 */
export interface CreateBlockReply {
  view: PageView;
  new_id: string;
}

/**
 * Successful execution payload of `runCodeBlock`. Mirrors the Rust
 * `ExecOutputDto` (which itself is a serialisable view of
 * `outl_exec::ExecOutput`). `duration_ms` is the wall-clock runtime
 * across the runtime call; `exit` is the stringified Rust
 * `ExitStatus` (`"Ok"`, `"NonZero(1)"`, `"Trap(\"…\")"`).
 */
export interface ExecOutputDto {
  stdout: string;
  stderr: string;
  duration_ms: number;
  exit: string;
}

/**
 * Reply of {@link import("./commands").runCodeBlock}. `outl-exec`
 * writes the `> **result:**` sibling subblock and reconciles with
 * the op log before the command returns, so `view` is the refreshed
 * outline the caller should swap straight in — no follow-up
 * `openPage…` round-trip needed.
 *
 * - `result_ok` is populated when the runtime ran (stdout / stderr
 *   captured, exit available).
 * - `error` is populated on infrastructure failure (unknown
 *   language, timeout, sandbox crash). Mutually exclusive with
 *   `result_ok`.
 */
export interface RunCodeBlockReply {
  language: string;
  result_ok: ExecOutputDto | null;
  error: string | null;
  view: PageView;
}

/**
 * A paired device, as returned by `outl_peer_list` and
 * `outl_peer_pair_join`. Mirrors the Rust `PeerDto` in both clients'
 * `src-tauri/src/commands/peers.rs` (and `outl_sync_iroh::PeerEntry`).
 *
 * - `node_id` is the iroh node id (hex), the stable identity of the
 *   remote device. Clients show a short prefix; `outl_peer_remove`
 *   matches on a prefix of this.
 * - `alias` is the optional human label set during pairing (`null`
 *   when the user didn't name the device).
 * - `added_at` is an RFC3339 timestamp string of when the pairing was
 *   persisted to `peers.json`.
 */
export interface PeerDto {
  node_id: string;
  alias: string | null;
  added_at: string;
}

/**
 * A paired peer's live reachability, as returned by `outl_peer_status`.
 * Mirrors the Rust `PeerStatusDto` in both clients' `commands/peers.rs`.
 * On GUI clients this reads the running transport's `peer_health()` snapshot
 * (populated by the transport's own boot / catch-up / gossip dials), NOT a
 * transient probe endpoint — a second endpoint with the device identity would
 * hijack the relay route (see outl-sync-iroh "One endpoint per identity,
 * elected not assigned").
 * Only the CLI `outl peer status`, which has no running transport, probes.
 *
 * - `online` is `true` when the peer is currently reachable.
 * - `rtt_ms` is the last round-trip time in milliseconds when reachable,
 *   `null` when the peer is offline.
 */
export interface PeerStatusDto {
  node_id: string;
  alias: string | null;
  online: boolean;
  rtt_ms: number | null;
}

/**
 * A sync-progress update pushed on the `sync-progress` Tauri event while a
 * pass runs. Mirrors the Rust `outl_actions::SyncProgress` enum (serialized
 * with an internal `phase` tag, kebab-case). Purely informational — it drives
 * the pairing-screen progress feed; the load-bearing reload signal is separate.
 *
 * `peer` is the peer's short node id; the UI resolves it to a friendly alias
 * against the peer list it already holds. `snapshot` and `asset` are the phases
 * with an honest percentage (`received` / `total` in bytes, total known up
 * front — for `asset`, the bytes of the file currently transferring); op counts
 * surface as a live number, not a bar. `received-ops.nodes` carries the (capped)
 * block ids touched, resolved to page slugs via `resolvePageLabels`; empty on a
 * bulk pass (the initial pair).
 */
export type SyncProgress =
  | { phase: "connecting"; peer: string }
  | { phase: "snapshot"; peer: string; received: number; total: number }
  | { phase: "asset"; peer: string; received: number; total: number }
  | { phase: "received-ops"; peer: string; count: number; nodes: string[] }
  | { phase: "pushed-ops"; peer: string; count: number }
  | { phase: "synced"; peer: string }
  /**
   * The exchange was cut off before the peer confirmed, and this device will
   * retry on its next pass. A phone that locked its screen, a laptop that
   * slept, a dropped carrier-NAT flow — expected, transient, not a defect.
   *
   * Kept apart from `failed` because a responder only confirms durable ingest
   * by closing cleanly, so a suspended peer produces an unconfirmed pass every
   * single time; rendering that red made locking your phone look like breakage.
   * Styled amber and deliberately NOT pushed into the feed — it updates
   * `current` only, the way `connecting` does.
   */
  | { phase: "interrupted"; peer: string; reason: string }
  | { phase: "failed"; peer: string; error: string };

export interface WorkspaceSummary {
  blocks: number;
  ops: number;
  actor: string;
  storage_root: string;
  /**
   * `true` when a workspace is currently loaded (mobile: iCloud
   * container available; desktop: user picked a directory). `false`
   * while the picker is still up (desktop) or the background opener
   * is in flight (both). Older mobile builds always sent `true`; the
   * field is `boolean | undefined` semantically — treat missing as
   * `true` when targeting an old binary, or use `?? true` if you
   * need a safe default.
   */
  ready: boolean;
}

// ── Plugins ─────────────────────────────────────────────────────────
// Wire shapes of the plugin host commands (`plugin_list` / `plugin_run`
// / `plugin_sync_hooks` / `plugin_toolbar` / `plugin_transformers` /
// `plugin_transform`). Both GUI clients register the identical Rust
// commands (thin shims over `PluginService`), so the DTOs live here
// once. The desktop-only `PluginKeybinding` (chord surface) stays in
// `outl-desktop/src/lib/api.ts`.

/** A command a loaded plugin contributes — surfaced in the plugin
 *  palette (desktop) / plugin sheet (mobile). */
export interface PluginCommand {
  plugin_id: string;
  command_id: string;
  title: string;
}

/**
 * A toolbar button a loaded plugin contributes to the client chrome.
 * `icon` is the glyph painted inline; clicking / tapping it runs
 * `command_id` via `pluginRun`. `title` is the accessible label /
 * tooltip.
 */
export interface PluginToolbarButton {
  plugin_id: string;
  command_id: string;
  icon: string;
  title?: string;
}

/**
 * Outcome of running a plugin command. `view` is the refreshed
 * {@link PageView} of the page that was on screen when the command
 * fired (so the caller re-renders in one trip); absent when no page id
 * was supplied or the page no longer resolves.
 *
 * `views` carries HTML documents the plugin emitted via
 * `ctx.ui.render` (gated by the `ui-render` capability). Each is
 * played as an ephemeral sandboxed iframe overlay — untrusted plugin
 * output, never injected into the app DOM.
 */
export interface PluginRunReply {
  applied: number;
  notifications: string[];
  errors: string[];
  view?: PageView;
  views: string[];
}

/**
 * Outcome of the plugins' `onOp` hook sweep: a refreshed
 * {@link PageView} **only** when a hook actually mutated the on-screen
 * page (`view` absent otherwise, so the caller skips a needless
 * render), plus any `ui-render` payloads the hooks emitted (`views` —
 * the confetti path, present even when no page re-render is needed).
 */
export interface PluginSyncHooksReply {
  view?: PageView;
  views: string[];
}

/**
 * A content transformer a loaded plugin declared for a code-fence
 * language. Clients load the list once per workspace open and, when a
 * fence's language matches a `lang` here, call `pluginTransform` to
 * render it.
 *
 * `kind` decides how the result renders inline in the block:
 * - `"text"` → the `content` is markdown/plain text, rendered inline.
 * - `"rich"` → the `content` is HTML, run in a sandboxed `<iframe>`.
 */
export interface PluginTransformer {
  plugin_id: string;
  lang: string;
  kind: "text" | "rich";
}

/**
 * The descriptor a content transformer produced for a fence body.
 * `kind` mirrors the matching {@link PluginTransformer.kind};
 * `content` is the rendered text (for `"text"`) or HTML run in a
 * sandboxed iframe (for `"rich"` — untrusted plugin output, never
 * injected into the app DOM).
 */
export interface PluginTransformResult {
  kind: "text" | "rich";
  content: string;
}

/** The value type of a plugin settings field (from its config schema). */
export type PluginFieldKind = "string" | "integer" | "number" | "boolean" | "json";

/**
 * One configurable field of a plugin, from `plugin_settings_describe`. Wire
 * shape of `outl_plugins::settings::SettingsField`. Config fields carry their
 * current `value`; secret fields carry only `isSet` (the value stays in the OS
 * keychain and never crosses the wire).
 */
export interface PluginSettingsField {
  /** Property key (`ctx.config.get()[key]` / `ctx.secrets.get(key)`). */
  key: string;
  /** Human label (schema `title`, falling back to the key). */
  title: string;
  /** Help text (schema `description`), when present. */
  description?: string;
  /** Value type. */
  kind: PluginFieldKind;
  /** Whether the field is keychain-backed (schema `x-outl-secret`). */
  secret: boolean;
  /** Schema default, when declared. */
  default?: unknown;
  /** Current config value (config fields only; absent when unset). */
  value?: unknown;
  /** For secret fields: whether a value is stored in the keychain. */
  isSet: boolean;
}

/**
 * One plugin marketplace row: a registry entry (plugins.outl.app) plus the
 * workspace's local state. Wire shape of `outl_plugins::MarketplaceItem`,
 * returned by `plugin_registry_list` on both clients. `installed` / `enabled`
 * drive the install vs. manage affordances.
 */
export interface RegistryItem {
  id: string;
  name: string;
  description: string;
  author: string | null;
  category: string | null;
  capabilities: string[];
  permissions: string[];
  latest: string | null;
  installed: boolean;
  enabled: boolean;
}

/**
 * One structural template surfaced by {@link import("./commands").listTemplates}.
 * Mirrors `outl_tauri_shared::state::TemplateDto`. A template is any page
 * with a non-empty `template::` property; its outline is the body
 * deep-copied under a target block by
 * {@link import("./commands").instantiateTemplateAt}.
 */
export interface TemplateDto {
  /** Invocation name (the page's `template::` property value). */
  name: string;
  /** Slug of the page that defines the template. */
  slug: string;
  /**
   * `true` when another page shares this `template:: <name>`; the picker
   * can flag it since resolution silently picks the first in tree order.
   * Omitted from the wire (defaults to `false`) when not a duplicate.
   */
  duplicate?: boolean;
}

/**
 * One scheduled `remind::` rule. Mirrors
 * `outl_tauri_shared::commands::reminders::ReminderDto`.
 *
 * `nextFire` / `snoozedUntil` are ISO-8601 **local** strings
 * (`2026-12-12T15:00:00`) with no zone suffix — deliberately. The
 * backend already resolved them in the user's configured timezone
 * (`outl_actions::clock`); re-deriving a local time from an epoch in
 * JS would reintroduce the timezone bug that module exists to fix.
 * Render them as-is, or parse with `new Date(iso)` which treats a
 * suffix-less string as local.
 */
/**
 * How late a reminder is. Mirrors `outl_actions::reminders::Urgency`
 * and is computed in Rust, so the TUI, the desktop panel and the
 * mobile sheet paint the same row the same way.
 */
export type ReminderUrgency = "overdue" | "soon" | "later" | "finished";

/** One snooze option. Mirrors `SnoozePresetDto`. */
export interface SnoozePreset {
  /** Wire id sent back to `snoozeReminder`. */
  id: string;
  /** What the button reads. */
  label: string;
}

export interface Reminder {
  block_id: string;
  page_slug: string;
  page_title: string;
  /** Block body, TODO/DONE prefix already stripped. */
  text: string;
  /** The rule verbatim, e.g. `"3pm every 1h until DONE"`. */
  rule: string;
  /** `YYYY-MM-DD` the rule is anchored to. */
  anchor_date: string;
  /** The block is DONE — listed for context, never fires again. */
  done: boolean;
  /** Local ISO datetime of the next fire, `null` when finished. */
  next_fire: string | null;
  /** Local ISO datetime the snooze runs until, `null` when not snoozed. */
  snoozed_until: string | null;
  /** Row styling class, decided by the backend. */
  urgency: ReminderUrgency;
}

/**
 * This device's reminder delivery preferences. Mirrors
 * `outl_tauri_shared::commands::reminders::ReminderSettingsDto`.
 *
 * Device-local by design: quiet hours are a property of *this* phone
 * or laptop, not of the workspace, so they never travel through the op
 * log (unlike the rule itself and a snooze, which do).
 */
export interface ReminderSettings {
  enabled: boolean;
  /** `"22:00-07:00"`, or `""` when the user set no quiet hours. */
  quiet_hours: string;
}

/**
 * Property key carrying a reminder rule on a block. Mirrors
 * `outl_md::REMIND_KEY` — don't hardcode `"remind"` at a call site.
 */
export const REMIND_KEY = "remind";
