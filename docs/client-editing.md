# Authoring gestures, client by client

What happens on each client when the user writes: running a code fence, instantiating a template, copying and pasting, attaching a file, marking a task, quoting a block, zooming into one, and the mobile keyboard bar.

Moved out of [`docs/clients.md`](clients.md), which had grown to the point where no edit to it was cheap.
That page keeps the cross-client architecture: the stack, who owns which fact, the locks an open takes, and the two conditions every client has to surface.

## Running code blocks

Every client that lets the user execute a `` ```lang ``` `` block (TUI `g x`, desktop `Cmd+Shift+X` / Run button, mobile long-press → "Run code") goes through **one** shared entry point:
`outl_actions::exec::run_code_block(ws, hlc, root, registry, page, block)`.

```text
client gesture (TUI chord / Cmd+Shift+X / long-press)
        │
        ▼
outl_actions::exec::run_code_block
   ├── outl_actions::flat_index_for_block   (DFS-locate the block)
   ├── outl_actions::journal::page_md_path  (resolve .md path)
   └── outl_exec::run_block_at_index        (execute + persist > **result:** sibling)
        │
        ▼
RunCodeBlockOutcome { language, result_ok | error }
        │
        ▼
client wraps with refreshed PageView and ships it down its Tauri/TUI surface
```

The DTO returned is intentionally *narrow* — `language`, `result_ok` (stdout/stderr/duration/exit), `error`.
Clients add the refreshed page projection themselves because each client owns its own `PageView` shape (mobile's iCloud-backed variant differs from desktop's path-picker variant).
The duplication that used to live in `outl-desktop/src-tauri/src/commands/exec.rs` and `outl-mobile/src-tauri/src/exec.rs` was collapsed into this single function.
`flat_index_for_block` and the path lookup were the canonical "two parallel implementations" case the workspace-level Reuse-first policy exists to prevent.

The runtime catalog is selected per-binary via `outl-exec` features:

- `outl-cli`, `outl-tui`, `outl-desktop`: default features (JS + Python + Lua + Rust via wasmtime).
- `outl-mobile` — opts out of `lang-rust` (wasmtime is heavy and trips iOS code-signing restrictions on dynamic code generation).
- `lang-lisp` (Steel) is in no default set: Steel exposes no allowlist to build a host boundary from, so a `lisp` fence can reach a shell however carefully the runtime shadows names.
  A build that only runs the user's own code can turn it on; see the comment in `crates/outl-exec/Cargo.toml`.
- `outl-actions` — `default-features = false` so it never drags `wasmtime` into the mobile IPA via the back door.

## Structural templates

A **template** is any page with a non-empty `template::` property; its outline is a body every client can deep-copy under a target block (see [`docs/templates.md`](templates.md) for the authoring model).
Instantiation is reachable from every surface (TUI/CLI `/template <name>`, MCP), and both GUI clients wrap the **same** two shared command bodies in `outl_tauri_shared::commands::template` — no plugin needed:

- `list_templates` → `Vec<TemplateDto>` (`{ name, slug, duplicate }`) — wraps `outl_actions::list_templates`.
- `instantiate_template_at(name, target_block)` → refreshed `PageView`.
  It resolves the target block's enclosing page (slug + journal date), calls `outl_actions::instantiate_template`, reprojects, and announces the ops.
  An unknown name or stale block id is a typed error the client toasts.

Both clients register the wire command as `list_templates_cmd` plus `instantiate_template_at`.
The `_cmd` suffix dodges a glob-import collision with the `outl_actions::list_templates` re-export.
The TS wrappers are `listTemplates()` / `instantiateTemplateAt(name, targetBlockId)` in `@outl/shared/api/commands`.

Client affordances (chrome per-client, one backend):

- **Desktop** — the block-initial `/` slash menu lists `template: <name>` entries alongside plugin commands.
  They are injected via `templateSlashCommands` under a reserved `@outl/template` sentinel `plugin_id`.
  Picking one instantiates under the selected block; `OutlineView`'s `onRunPluginCommand` intercepts the sentinel and calls `instantiateTemplateAt` instead of `pluginRun`.
- **Mobile** — the block long-press menu has an "Insert template" action that opens `TemplateSheet` (a bottom sheet listing templates).
  Picking a template instantiates it under the long-pressed block and applies the returned `PageView`.

## Copy and paste

> **Why copy-out and paste-in are one pair, and why the core speaks exactly one format:** [RFC 0044](rfcs/0044-clipboard-and-paste.md).

Every client supports copying blocks as clean outl markdown and pasting markdown from external apps.

### Copy out

| Client | How to copy | What is copied |
|---|---|---|
| TUI | `yy` / `Y` (Normal) or `y` (Visual range) | Selected block(s) + full subtrees as canonical outl markdown, written to the OS clipboard via arboard (X11/Wayland/macOS) with an OSC 52 fallback for SSH / tmux / Crostini. Status line confirms: `yanked N block(s) → clipboard` or `(clipboard unavailable)`. |
| Desktop | `Y` (Normal) or `y` (Visual range) | Same serialisation via the `copy_markdown` Tauri command (`outl_actions::copy_markdown`) + `navigator.clipboard.writeText`. |
| Mobile | Long-press → "Copy" in the context menu | Block + full subtree via the `copy_markdown` Tauri command, written to the iOS clipboard. |

The serialisation is handled by `outl_actions::copy_markdown` on the Rust side and `copyMarkdown` (`@outl/shared/api/commands`) on the TS side.
The format is canonical outl markdown: `- ` bullets, 2-space indent, inline block props alpha-sorted, `TODO`/`DONE`/`> ` prefixes verbatim.
Pasting the result back into any outl client reconstructs the same tree.

### Paste in

Every client distinguishes **paste with formatting** from **paste without formatting**.

**With formatting** (`Cmd/Ctrl+V` on desktop, `p` in the TUI, the default paste on mobile) routes the clipboard through `outl_actions::paste_markdown` when the content looks structured.
It applies these conversions:

- Roam `{{[[TODO]]}}` / `{{[[DONE]]}}` → `TODO` / `DONE` prefix.
- GitHub `- [ ]` / `- [x]` → `TODO` / `DONE` prefix.
- Logseq `id::` metadata → stripped.
- 4-space indent → 2-space indent.
- Multi-line plain text (two or more non-blank lines) → one block per non-blank line (blank lines are ignored).
- Single-paragraph plain text → falls through to the browser/terminal default splice.

**Without formatting** (`Cmd/Ctrl+Shift+V` on desktop, `P` in the TUI; not available on mobile) calls `outl_actions::paste_plain`.
The raw clipboard text is inserted as a single block at the anchor with no normalisation, outline parsing, or paragraph splitting.
Use this when the text contains underscores, brackets, or other characters that would be misread as markdown syntax.

The routing decision uses two helpers from `@outl/shared/paste`:
`looksLikeOutline` detects bullet structure;
`hasMultipleParagraphs` is true when the text has two or more non-blank lines (blank lines are ignored).
Either condition sends the paste to the backend (structured path) on `Cmd/Ctrl+V`.

| Client | With formatting | Without formatting |
|---|---|---|
| TUI | `p` — routes to backend when outline or multi-paragraph; else native splice | `P` — raw text, single block |
| Desktop | `Cmd/Ctrl+V` — routes to backend when outline or multi-paragraph; else native splice | `Cmd/Ctrl+Shift+V` — raw text, single block |
| Mobile | paste — routes to backend when outline or multi-paragraph; else native splice | not available |

A paste at the caret rewrites the block the user is typing in, so both GUI clients re-seed that editor from the reply's `host_text` / `host_caret` instead of only refreshing the page.
Skipping it left the editor on the pre-paste string, which the next commit wrote back over the paste ([issue #488](https://github.com/outlmd/outl/issues/488)).
Where the caret lands, and the rest of the paste contract: [`docs/paste.md`](paste.md).

### TUI mouse drag copy (opt-in)

With `[tui] mouse_capture = true` in `~/.config/outl/config.toml`, dragging across blocks in the TUI selects a range and copies it as outl markdown on release.
See [tui.md → Mouse capture](tui.md#mouse-capture-opt-in) and [config.md → `[tui]`](config.md#tui) for details.

## Attach / drag-and-drop file import

Every client can bring an outside file (PDF, image) into a workspace as an asset link, without leaving the outline.
All three routes land on the same backend copy: content-addressed into `<root>/assets/<hash>.<ext>`, idempotent, size-capped by `[assets] max_bytes`.
See [markdown-format.md → Asset links](markdown-format.md#asset-links-nameassetshashext) for the link format and the OS-open behavior.

| Client | File picker | Drag-and-drop |
|---|---|---|
| Desktop | `📎 Attach file` button **or** `/upload` slash command → OS file dialog → new block at page end | Drop a file on a row → link lands in the drop-on block (else selection, else the block being edited via caret splice) |
| Mobile | Block long-press → "Attach file" → new block after it | iPad only: drop a file on a row, same target resolution as desktop; degrades to the long-press action on iPhone |
| TUI | `/upload` slash command (alias `/attach`) → native OS dialog on macOS/Windows, typed path (`upload <path>`) on Linux/SSH → link at the cursor | Insert mode: paste a payload that is exactly an existing file's path (how terminals report a dropped file) → link spliced at the caret |

Desktop and mobile share the drop-handling code (`installFileDrop` + `importAssetFile`, `@outl/shared/drag-drop`), so the drop-target resolution can't drift between the two GUI clients.
See [`docs/frontend-shared-logic.md`](frontend-shared-logic.md) for the pure helpers underneath.
The TUI has no OS drag events over SSH/tmux, so it treats a lone pasted file path as a drop instead (`outl-tui/CLAUDE.md` → "Drag-and-drop file upload").

## Task state convention

A block's task state is **a prefix on its text**, not a property:

```
"foo"             plain block
"TODO foo"        open task
"DOING foo"       started task
"DONE foo"        completed task
```

This is the wire format the TUI already uses and what `.md` files contain when synced to other tools.
`outl-actions::cycle_todo` walks `None → TODO → DOING → DONE → None`; UI surfaces parse the prefix out via `split_todo` to render a checkbox.
Never re-derive the cycle order — [markdown-format.md](markdown-format.md#block-level-prefixes-todo--doing--done--quote) owns it.

## Blockquote convention

Blockquotes follow the same shape as TODO/DONE — a per-block text prefix, no AST field, every client renders its own visual.
The prefix is the CommonMark `"> "` (greater-than + single space), so an `.md` round-trips cleanly when an external tool opens it:

```
"foo"             plain block
"> foo"           quoted block
```

`outl-actions::quote::toggle_quote` flips the prefix on/off; `split_quote` separates the marker from the body for UI rendering.
Composition with TODO/DONE follows a canonical order: `"TODO > body"` (task state before quote marker), so the backend's `split_todo` still surfaces `block.todo` in the DTO when the block is also quoted.
Both `toggle_quote` and `cycle_todo` peel both prefixes off and re-emit in canonical order, so an externally authored `"> TODO foo"` is normalised rather than compounded into a double marker.
A `cycle_todo` reaches canonical order on the first toggle: `"> TODO foo"` becomes `"DOING > foo"`, the task state advanced as usual.
**A todo toggle is the route every client has**, and it is the one most users reach it by: `"> ship it"` toggled becomes `"TODO > ship it"`.
`toggle_quote` has exactly one caller — the desktop's `/quote` slash row.
It is not an `Action`, so no chord reaches it anywhere, and on the TUI and mobile a quote is authored by typing `> `.
The per-client verdict is `Capability::BlockquoteToggle`, and the sentence those two clients show is the catalog's `TYPE_THE_QUOTE_MARKER`, never written in a client.
One thing the two toggles do not share: `toggle_quote` flips the marker, so reaching canonical order from `"> TODO foo"` takes two of them — the first strips the quote and leaves `"TODO foo"`.
Multi-line quote bodies keep the `> ` on every continuation line so the `.md` stays a valid CommonMark blockquote.
Children of a quoted block are **not** implicitly quoted — the marker lives on the block, not on its subtree.
GUI clients render the outline bullet outside the quote chrome, so the quote is visually the body's content rather than a nested list item.
Inline tokens (`**bold**`, `[[ref]]`, `#tag`, `((blk-…))`) continue to tokenize **inside** the body — the wrapper is transparent.

## Zoom / focus on a block (Roam/Workflowy)

Zooming makes one block the outline root so only its subtree renders — the Roam/Workflowy "focus" gesture.
It is **local view state**, never an `Op` and never a Tauri round-trip: the client already holds the whole `outline`, so zoom is a pure decision about what to render, and it does not converge across devices (each device focuses independently).
On the GUI clients (desktop + mobile) the subtree lookup + ancestor breadcrumb is the shared `focusSubtree(blocks, blockId)` in `@outl/shared/outline`, returning `{ root, breadcrumb }` or `null` when the id is gone.
This is pure view state with no Rust mirror — the id never leaves the frontend; the TUI reaches the same behaviour through its own path-based zoom stack in `outl-tui/src/actions/zoom.rs`.
Every client resolves the focused id against the **live** outline on each render, so an edit / collapse inside the zoom stays reflected, and a stale target (block deleted or moved to another page → `null`) transparently falls back to the full page.
Switching pages drops the zoom, so focus is scoped to the page it was set on.

**Zooming is a request for one subtree, so the page's own surfaces go with it.**
Backlinks, nested pages, namespace mentions and the page's `key:: value` row answer questions about the *page*; drawn under a block title they read as the block's, which is a wrong answer rather than a noisy one.
All three clients hide them, and each keeps its cursor out of them too.
The desktop does it through `backlinkBlockIds()`, the TUI through `page_sections_visible()` (which `z i` also uses to bring a cursor back out of the backlinks panel), mobile through having no cursor to move.
The owner per client: `<PageSections zoomed>` (desktop and mobile) and `App::page_sections_visible` (TUI).

**Asking for a page means the whole page, including when it is the page already open.**
A client that drops the zoom only when the page *id* or slug changes leaves `Cmd+J` (or a tap on Today) doing nothing visible while zoomed into a block of today's journal — the one gesture that means "take me back" appears broken.
So "the user asked for a page" is a separate question from "the page changed", and only the first is a navigation:

| Client | What a navigation runs | What catches the rest |
|---|---|---|
| desktop | `resetForNavigation()` via `navigateTo` / `openView` | `installPageChangeEffect` on a page-id change |
| mobile | `resetForNavigation()` via `openView` | `applyView` on a slug change |
| TUI | `load_current()`, which clears `zoom_stack` outright | — (every view load goes through it) |

The TUI never had the bug: `go_today` loads unconditionally, so the reset is keyed on the load rather than on a comparison.
A mutation reply, a peer refresh and a background capture are **not** navigations and keep the zoom on every client.

On mobile the gesture is touch, not a chord: a tap on a block's **plain bullet dot** zooms in (`Journal.tsx` holds one `focusBlockId` signal; `BlockRow` → `BulletOrCheckbox` fires `onFocusBlock` when wired).
That repurposes the plain dot's old mark-as-TODO tap — TODO stays reachable in the long-press context menu — while the TODO/DONE checkbox and the collapse triangle keep their taps, so no gesture collides.
Zoom-out is stackless: a `← Back` header button plus a tappable ancestor breadcrumb, derived by re-resolving `focusSubtree` and stepping to `breadcrumb.at(-1)` (the parent) or exiting when already top-level.

## Keyboard accessory bar (mobile)

The edit toolbar docked above the soft keyboard — a Bear-style pill of `+` / indent / bold / `[[` / TODO / delete / hide-keyboard — plus the ref/emoji chip strip, exist in **two renderings**.
iOS is native: `OutlToolbarView` supplied as the private `WKContentView`'s `inputAccessoryView` via the `OutlSwizzle` swizzle, suggester in `OutlSuggestOverlay` / `OutlSuggestView`.
Android (and iOS later) is web: `KeyboardAccessory.tsx` renders `<SuggesterStrip />` + `<KeyboardToolbar />` in the webview, bottom-anchored at `useKeyboardInset()`, gated on `isAndroid && editingId() !== null`.
There is no web equivalent to iOS's docked accessory view, so the native bar stays until the web one is proven on a device.

The logic is shared, never triplicated.
The action catalog + most-frequently-used ordering live in `@outl/shared/toolbar`, a port of `crates/outl-mobile/swift/OutlKit/Toolbar/{ToolbarAction,ToolbarMFU}.swift`.
The action string ids (`newLine`, `indent`, `todo`, …) are the wire contract the iOS native bar ships to JS via `window.__outlToolbar(action)`.
So the Swift `ToolbarAction` copy and the TS catalog stay byte-identical — rename on both sides in one commit — until the native bar retires.
Both bars dispatch through the same `Journal.tsx` `dispatchToolbarAction` (the native bridge just assigns it to `window.__outlToolbar`).
Both accept a chip through the same `window.__outlSuggesterPicked` callback.
The web strip reads the reactive `nativeSuggesterState` signal that `setNativeSuggesterState` feeds alongside the `window.__outlSuggesterState` global the native strip polls.

Two invariants the web bar must keep.
Every button calls `e.preventDefault()` on `onPointerDown` so the tap can't blur the textarea and dismiss the keyboard.
`AndroidManifest.xml`'s activity sets `android:windowSoftInputMode="adjustResize"`.
With the `interactive-widget=resizes-content` meta the visual viewport then shrinks on keyboard show and `useKeyboardInset()` collapses to ~0, so `bottom: inset` rests the bar on the keys; drop `adjustResize` and it floats behind the keyboard.
Validation needs an Android device/emulator build — docking and `visualViewport` don't exercise under host `cargo check` or `bun run dev`.
