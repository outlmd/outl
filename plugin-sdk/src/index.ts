/**
 * @outl/plugin-sdk — the JS-facing contract for outl plugins.
 *
 * This package is **types + one helper** and nothing else. It has zero runtime
 * dependencies and never talks to Tauri, the filesystem, or the network. The
 * real `PluginContext` is injected by the outl runtime (a Boa JS engine living
 * in the Rust `outl-plugins` crate) when it calls the plugin's `activate(ctx)`.
 *
 * Why types-only: a plugin written once must run identically on every client
 * (TUI, desktop, mobile, CLI). Pinning behavior to a host-provided context —
 * instead of importing anything client-specific — is what makes that possible.
 *
 * Mental model for authors: you think in **blocks and ops**, never in pixels,
 * CRDT internals, or `.md` files. Every mutation you trigger (`ctx.blocks.move`,
 * `ctx.blocks.edit`, ...) becomes a host call that routes through `outl-actions`
 * → `Workspace::apply` → the op log, stamped `plugin:<id>@<device>`. The op log
 * stays the single source of truth; the SDK just gives you a typed door to it.
 */

// ---------------------------------------------------------------------------
// Core identifiers and data shapes
// ---------------------------------------------------------------------------

/**
 * A block identifier. Opaque on the JS side — it is a `ULID` string under the
 * hood, but plugins must treat it as a token to pass back to the host, never
 * parse or construct it. IDs live only in the sidecar, never in the `.md`.
 */
export type BlockId = string;

/**
 * A page slug — a page's flat, filename-safe id (`pages/<slug>.md`). Pages are
 * **not** nested directories, so `/` is not a slug character; use a separator
 * like `-` (e.g. `ouraring-2025-11-29`). Daily notes use ISO `YYYY-MM-DD`.
 */
export type PageSlug = string;

/**
 * Task state of a block, when it has one.
 *
 * Mirrors `outl_actions::TodoState`. A plugin that only branches on
 * `"TODO"` and `"DONE"` silently drops every started block, so treat
 * a state you don't handle as "not finished" rather than "not a task".
 */
export type TodoState = "TODO" | "DOING" | "DONE";

/**
 * A single materialized block, as the host hands it to a plugin.
 *
 * This is a read projection, not a live handle: mutating these fields does
 * nothing. To change a block, call the `ctx.blocks.*` methods, which submit ops.
 */
export interface Block {
  /** Stable id; pass it back to `ctx.blocks.*` to act on this block. */
  id: BlockId;
  /** Raw markdown text of the block (clean — no inline IDs). */
  text: string;
  /** Parent block id, or `null` when the block is a top-level child of a page. */
  parent: BlockId | null;
  /** Slug of the page this block currently lives on. */
  page: PageSlug;
  /** TODO state, or `null` when the block is not a task. */
  todo: TodoState | null;
}

/**
 * Filter passed to `ctx.blocks.query`. All present fields are ANDed together;
 * an empty filter matches every block in the workspace.
 *
 * Kept intentionally small for d0 — extend deliberately as real plugins need
 * more selectors, so the contract does not grow speculative surface.
 */
export interface BlockFilter {
  /** Restrict to blocks on this page. */
  page?: PageSlug;
  /** Restrict to blocks in this TODO state. */
  todo?: TodoState;
  /** Substring the block text must contain (case-insensitive on the host). */
  textContains?: string;
}

/**
 * Where a block should move to. Exactly one variant is meaningful per call; the
 * host validates and rejects ambiguous or empty targets.
 *
 * `toPage` appends the block to the end of the target page's outline.
 * `toParent` reparents under a block (optionally inserting at `index`).
 */
export type MoveTarget =
  | { toPage: PageSlug; toParent?: never; index?: number }
  | { toParent: BlockId; toPage?: never; index?: number };

/**
 * A node in a tree passed to `appendTree`: its text plus optional children.
 * Recursive, so you describe a whole nested outline in one value.
 */
export interface TreeNode {
  /** Block text (include a `TODO `/`DONE ` prefix to set that state). */
  text: string;
  /** Child nodes, created under this one. Omit or empty for a leaf. */
  children?: TreeNode[];
}

/**
 * An op as it appears *after* being applied to the log — what `ctx.ops.onOp`
 * receives. This is the JS-facing projection of the Rust `Op`/log entry, not a
 * 1:1 mirror: it carries just what a hook needs to react.
 *
 * `kind` is the op variant. `node` is the block the op acted on. The remaining
 * fields are populated only for the variants that use them (e.g. `text` on a
 * text update, `target`/`parent` on a move), so they are all optional.
 */
export interface LogOp {
  /** Op variant, e.g. `"TextUpdate"`, `"Move"`, `"ToggleTodo"`, `"Insert"`. */
  kind: string;
  /** The block this op acted on. */
  node: BlockId;
  /** New text, for text-bearing ops. */
  text?: string;
  /** New parent, for move/insert ops. */
  parent?: BlockId | null;
  /** Move destination, for move ops. */
  target?: MoveTarget;
  /** TODO state after the op, for todo toggles. */
  todo?: TodoState | null;
  /**
   * Who produced the op. Plugin-originated ops are stamped
   * `plugin:<id>@<device>`, which lets hooks ignore their own writes and avoid
   * re-entrant loops.
   */
  actor?: string;
}

// ---------------------------------------------------------------------------
// Host API namespaces (the typed `ctx`)
// ---------------------------------------------------------------------------

/** Options for `ctx.net.fetch`. `timeoutMs` is **required** — the host refuses
 * unbounded network calls, so the type bakes that in rather than defaulting. */
export interface FetchOptions {
  /** HTTP method. Defaults to `"GET"` on the host when omitted. */
  method?: "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD";
  /** Request headers. */
  headers?: Record<string, string>;
  /** Request body for write methods. */
  body?: string;
  /** Hard timeout in milliseconds. Required: no unbounded fetches. */
  timeoutMs: number;
}

/** A minimal response shape returned by `ctx.net.fetch`. */
export interface FetchResponse {
  status: number;
  ok: boolean;
  headers: Record<string, string>;
  /** Resolve the body as text. */
  text(): Promise<string>;
  /** Resolve and parse the body as JSON. */
  json<T = unknown>(): Promise<T>;
}

/**
 * Op-log hook namespace. Gated by the `read-op-log` permission.
 */
export interface OpsApi {
  /**
   * Register a callback fired for every op applied to the log — local edits and
   * ops arriving from sync alike. Filter on `op.actor` to skip your own writes.
   * Hooks run with a host-enforced timeout and re-entrancy depth limit.
   */
  onOp(cb: (op: LogOp) => void): void;
}

/**
 * Block read/write namespace. Reads need `read-page`; writes need `write-page`
 * and `submit-op`.
 *
 * Execution is **describe → apply**: reads (`query` / `get`) see a snapshot
 * taken at the start of the turn, and writes are buffered and applied by the
 * host *after* your handler returns. So a block you `edit`/`create` this turn is
 * NOT visible to a later `query` in the same turn — collect what you need first,
 * then mutate. The methods are async-typed for forward-compatibility; today they
 * resolve synchronously, so `await` is harmless but not required.
 */
export interface BlocksApi {
  /** Find blocks matching `filter`. */
  query(filter: BlockFilter): Promise<Block[]>;
  /** Fetch one block by id, or `null` if it no longer exists. */
  get(id: BlockId): Promise<Block | null>;
  /** Replace a block's markdown text (include the `TODO `/`DONE ` prefix to set state). */
  edit(id: BlockId, text: string): Promise<void>;
  /**
   * Create a new block as the last child of `parent`. Does not resolve to the
   * new id: under describe→apply the id does not exist until the host applies
   * the intent, after this turn.
   */
  create(parent: BlockId, text: string): Promise<void>;
  /** Create a new block as the sibling right after `after`. */
  createAfter(after: BlockId, text: string): Promise<void>;
  /** Move a block to a new page (`{ toPage }`) or under another block (`{ toParent }`). */
  move(id: BlockId, target: MoveTarget): Promise<void>;
  /** Cycle a block's task state (None → TODO → DOING → DONE → None). */
  toggleTodo(id: BlockId): Promise<void>;
  /** Delete a block (moved to trash; the op stays in the log). */
  delete(id: BlockId): Promise<void>;
  /**
   * Append a nested tree of blocks under `parent`, all in one turn. The host
   * threads the new ids through internally, so — unlike `create` — you don't
   * need any child's id in hand. Use it to build fresh nested content under a
   * block you already have. To seed a page that has no blocks yet, use
   * `ctx.page.appendTree(slug, tree)` instead.
   */
  appendTree(parent: BlockId, tree: TreeNode[]): Promise<void>;
}

/**
 * Page namespace. Reads need `read-page`; `create` needs `write-page`.
 */
export interface PageApi {
  /** List every page in the workspace. */
  list(): Promise<Page[]>;
  /** Create a page (idempotent on slug). */
  create(slug: PageSlug): Promise<void>;
  /**
   * Append a nested tree of blocks to a page (created if missing), all in one
   * turn. This is the way to give a **brand-new page** its first blocks:
   * `create` needs a parent block id that a fresh page has no way to hand you
   * mid-turn (describe→apply), and `appendTree` sidesteps that — the host
   * resolves the page's root and threads child ids through as it builds.
   */
  appendTree(slug: PageSlug, tree: TreeNode[]): Promise<void>;
  /**
   * @roadmap Not wired yet — calling this throws at runtime in the current
   * outl version. Open a page in the active client view.
   */
  open(slug: PageSlug): Promise<void>;
  /**
   * @roadmap Not wired yet — calling this throws at runtime in the current
   * outl version. Slug of today's daily note (ISO `YYYY-MM-DD`).
   */
  today(): Promise<PageSlug>;
}

/**
 * A page as the host hands it to a plugin (read projection).
 */
export interface Page {
  /** Stable slug. */
  slug: PageSlug;
  /** Human title. */
  title: string;
  /** `"page"` or `"journal"`. */
  kind: "page" | "journal";
}

/**
 * Template namespace. `list` needs `read-page`; `instantiate` needs `write-page`.
 * See [Templates](../docs/templates.md) for the full guide.
 */
export interface TemplateApi {
  /** List every template in the workspace. */
  list(): Promise<Template[]>;
  /** Instantiate a structural template under a target block. */
  instantiate(name: string, targetBlockId: BlockId): Promise<void>;
}

/** A template as the host hands it to a plugin (read projection). */
export interface Template {
  /** Invocation name (the value of `template::`). */
  name: string;
  /** Page slug. */
  slug: PageSlug;
  /** Declared parameter names (empty for structural templates). */
  params?: string[];
}

/**
 * Command namespace. No permission gate — commands are declared up front in
 * `plugin.json` under `contributes.commands`, and the id passed here must match
 * one of them. The handler is fired by a slash menu or a keybinding.
 */
export interface CommandsApi {
  register(id: string, handler: () => void | Promise<void>): void;
}

/**
 * Config namespace. No permission gate. Returns the user's config for this
 * plugin, already validated against `configSchema` by the host, so the value is
 * safe to trust as `T`.
 */
export interface ConfigApi {
  get<T>(): T;
}

/**
 * Per-plugin key/value storage. Gated by `storage:local`.
 *
 * Persisted to `.outl/plugins/<id>/storage.json`. Reads see what you wrote in
 * an earlier turn; a write this turn is flushed after your handler returns.
 *
 * **Local-only: this does NOT converge across devices.** It is kept out of the
 * op log on purpose (so it can't inflate the log). If a value ever needs to
 * sync, model it as an Op instead — do not lean on this for shared state.
 */
export interface StorageApi {
  get<T = unknown>(key: string): Promise<T | null>;
  set<T = unknown>(key: string, value: T): Promise<void>;
  delete(key: string): Promise<void>;
}

/**
 * Network namespace. Gated by `network:<domain>` — every request host is
 * checked against the approved domain rules. A host not covered by an approved
 * permission is refused with `{ ok: false, error }` (not thrown), so handle it.
 * The call is blocking under the hood (on the plugin's own thread); keep
 * `timeoutMs` tight.
 */
export interface NetApi {
  fetch(url: string, opts: FetchOptions): Promise<FetchResponse>;
}

/**
 * Read-only access to the plugin's own secrets. Gated by the `secrets`
 * permission.
 *
 * Unlike `ctx.config` (plaintext in the lockfile) and `ctx.storage` (plaintext
 * on disk, local-only), secrets live in the **OS keychain** — macOS Keychain,
 * Windows Credential Manager, Linux Secret Service — and never touch the
 * workspace on disk. They are namespaced per plugin, so a plugin can only ever
 * read its own.
 *
 * The plugin only **reads**. The value is set out-of-band by the user, through
 * `outl plugin secret set <id> <key>` or a client's plugin settings. Use this
 * for API tokens and anything you would not want synced or committed with the
 * workspace.
 */
export interface SecretsApi {
  /**
   * Read a secret by key, resolving to `null` when it was never set (so the
   * plugin can prompt the user to configure it). Async-typed for
   * forward-compatibility; the runtime resolves it synchronously today.
   */
  get(key: string): Promise<string | null>;
}

/** Structured logging — surfaces in the client's plugin log, prefixed by id. */
export interface LogApi {
  info(msg: string): void;
  warn(msg: string): void;
  error(msg: string): void;
}

/** Lightweight user-facing notifications (toast / status line per client). */
export interface UiApi {
  /** Show a short message (toast / status line, per client). */
  notify(msg: string): void;
  /**
   * Render ephemeral author-written HTML/JS in a sandboxed iframe overlay.
   * Needs the `ui-render` capability and only runs on GUI clients (desktop,
   * mobile) — the TUI/CLI ignore it.
   *
   * The host never interprets the markup: it runs your string in an iframe with
   * `sandbox="allow-scripts"` (no same-origin, no access to the app DOM,
   * cookies, or workspace), positioned as a full-screen, click-through overlay,
   * and torn down shortly after. Write whatever you want — a confetti burst, a
   * toast, an SVG. It is YOUR creativity, not a fixed catalog of effects.
   *
   * Keep it self-contained: the iframe has no network and no imports, so inline
   * everything (a `<canvas>` + a little JS is plenty for confetti).
   */
  render(html: string): void;
}

/**
 * The full host API handed to `activate`. Each namespace is gated by the
 * permission noted in its doc; calling into a namespace you did not request (or
 * the user did not approve) rejects at the host boundary, it does not silently
 * no-op.
 */
export interface PluginContext {
  ops: OpsApi;
  blocks: BlocksApi;
  page: PageApi;
  template: TemplateApi;
  commands: CommandsApi;
  config: ConfigApi;
  content: ContentApi;
  sync: SyncApi;
  storage: StorageApi;
  secrets: SecretsApi;
  net: NetApi;
  log: LogApi;
  ui: UiApi;
}

/**
 * A sync transport the plugin provides (capability `sync-transport`). You only
 * **transport bytes** — the host hands you the JSONL of locally-authored ops to
 * ship in `push`, and applies whatever JSONL you return from `pull` through the
 * CRDT itself (it never trusts your bytes into the tree raw). Talk to your
 * backend with `ctx.net`. The client drives the cadence (push after local
 * edits, pull on a timer).
 */
export interface SyncTransport {
  /** Ship this JSONL of local ops to your backend. */
  push(opsJsonl: string): void;
  /** Return JSONL of remote ops to apply, or `null` if none. */
  pull(): string | null;
}

/** Register the plugin's sync transport (capability `sync-transport`). */
export interface SyncApi {
  register(transport: SyncTransport): void;
}

/** Descriptor a content transformer returns for a block. */
export interface TransformResult {
  /**
   * `"text"` — `content` is text/markdown rendered on every client.
   * `"rich"` — `content` is HTML run in a sandboxed iframe (GUI clients only).
   */
  kind: "text" | "rich";
  /** The rendered content (text or HTML, per `kind`). */
  content: string;
}

/**
 * Content transformers. Register a function for a code-fence language; when a
 * client renders a ```<lang> fence it asks the host, which runs your function
 * with the fence body and renders the descriptor you return.
 *
 * Declare the same `lang` (and `kind`) under `contributes.transformers` in
 * `plugin.json` so clients can skip languages no plugin handles. Needs
 * capability `content-transformer:text` (for `kind: "text"`) or
 * `content-transformer:rich` (HTML in a sandboxed iframe, GUI only).
 *
 * The transformer is a pure function — return the descriptor, don't mutate.
 */
export interface ContentApi {
  register(lang: string, fn: (body: string) => TransformResult | null): void;
}

// ---------------------------------------------------------------------------
// Plugin definition
// ---------------------------------------------------------------------------

/**
 * The object an author returns from `definePlugin`.
 *
 * Behavior only — all metadata (id, version, permissions, contributes, ...)
 * lives in `plugin.json`, never here, so there is exactly one source of truth
 * for each fact.
 */
export interface PluginDefinition {
  /**
   * Called once when the plugin is enabled. Wire up `ctx.ops.onOp` hooks and
   * `ctx.commands.register` handlers here. Throwing aborts activation and the
   * host surfaces the error; it never crashes the client.
   */
  activate(ctx: PluginContext): void;
  /**
   * Optional cleanup on disable/update/uninstall. The host already drops your
   * registered hooks and commands, so only release things the host can't see
   * (timers, in-flight work).
   */
  deactivate?(): void;
}

/**
 * Define a plugin. Validates the shape and returns it unchanged.
 *
 * This is deliberately thin: it exists so authoring is typed and so a malformed
 * default export fails loudly at module-eval time (a clearer error than the
 * host hitting `undefined.activate` later). It does no host work — the runtime
 * imports the returned object and calls `activate(ctx)` with the injected
 * context.
 *
 * @example
 * export default definePlugin({
 *   activate(ctx) {
 *     ctx.commands.register("my-command", () => ctx.ui.notify("hi"));
 *   },
 * });
 */
export function definePlugin(def: PluginDefinition): PluginDefinition {
  if (def === null || typeof def !== "object") {
    throw new TypeError("definePlugin: expected a plugin definition object");
  }
  if (typeof def.activate !== "function") {
    throw new TypeError("definePlugin: `activate` must be a function");
  }
  if (def.deactivate !== undefined && typeof def.deactivate !== "function") {
    throw new TypeError(
      "definePlugin: `deactivate` must be a function when provided",
    );
  }
  // Hand the definition to the host runtime. The outl engine injects
  // `globalThis.__outl_register` before evaluating the bundle, then calls
  // `activate(ctx)` with the real context. Absent in tooling/tests, so this is
  // a no-op there.
  const host = globalThis as {
    __outl_register?: (d: PluginDefinition) => void;
  };
  host.__outl_register?.(def);
  return def;
}
