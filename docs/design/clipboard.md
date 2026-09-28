# Design: unified copy/paste across clients

> **Superseded by [RFC 0044](../rfcs/0044-clipboard-and-paste.md), which is the canonical version of this decision.**
> The RFC carries what this page does not: the alternatives that were rejected, what the change makes worse, and the tests that stop it being undone.
> This page stays because it is published at outl.app; read it as the original design note, not as current guidance.

Status: **implemented**
Tracking issue: [#114 — Pasting from TUI into another app is a complete mess](https://github.com/outlmd/outl/issues/114)
Owner: Avelino

> Shipped: `outl_actions::copy_markdown` / `copy_markdown_nodes` (the serializer), TUI app-yank to clipboard (arboard + OSC 52) plus opt-in `[tui] mouse_capture`, and desktop + mobile copy via the shared `copyMarkdown` command.
> The `copy-format` plugin capability for non-markdown output stays a future RFC (see Follow-up).

## Problem

Copying content **out of** the TUI and pasting it into another app produces garbage:
the pasted text carries the tree guides (`│ `), bullets (`- `), and fold markers (`▼`/`▶`) that only make sense on screen.

![the reported mess](https://github.com/user-attachments/assets/287b8a2c-d574-40d8-a734-77e4274a27e7)

The complementary goal, raised alongside the issue, is that pasting **into** any client should normalize external content into our markdown dialect.
That half already works (see below); the copy-out half is the gap.

## Current state

### Paste (outside → blocks) — already solved, already shared

The inbound pipeline is complete and identical across clients:

```
frontend onPaste / TUI bracketed paste
  → @outl/shared looksLikeOutline            (crates/outl-frontend-shared/src/.../paste)
  → paste_markdown_at (Tauri command)        (outl-{desktop,mobile}/src-tauri/.../block.rs)
  → outl_actions::paste_markdown             (crates/outl-actions/src/paste/mod.rs:156)
  → normalize_external_syntax                (crates/outl-actions/src/paste/normalize.rs)
  → outl_md::parse::parse                    (crates/outl-md/src/parse.rs:97)
```

`normalize_external_syntax` already converts Roam `{{[[TODO]]}}`, GitHub `- [ ]`, Roam embeds/queries,
strips unknown `{{…}}`, drops Logseq `id::` lines, and reflows 4-space indent to 2-space.
`looks_like_outline` (`paste/detect.rs:26`) is the canonical gate; the JS mirror is `looksLikeOutline` in `@outl/shared`.

**Conclusion: the "paste converts to our markdown" requirement is met today.**
The only follow-up here is consistency review, not new capability.

### Copy (blocks → outside) — broken or incomplete everywhere

| Client | What "copy" does today | Writes OS clipboard? |
|--------|------------------------|----------------------|
| TUI | `yy` / visual `y` clone AST nodes into an **in-memory** `yank_register` (`actions/yank.rs:102,115`). `y r` writes the clipboard but only the **ref token** `((blk-…))` (`actions/yank.rs:46`). | only `y r`, and only a token |
| Desktop | `Y` / `YankRange` copy `block.text` into an in-memory `yankRegister` (`lib/action-handlers.ts:370,598`); `p`/`P` are **TBD** (`lib/store.ts:100`). | no |
| Mobile | Context-menu "Copy text" writes **one block's raw text** via `navigator.clipboard` (`components/Journal.tsx:1555`), no children, no `- `. | yes, single block only |

There is **no serializer** anywhere (Rust or TS) that turns a selected subtree into clean outline markdown.
The closest primitive, `render_page_md` (`outl-actions/src/journal/render.rs`), is page-scoped:
it skips the root node's own text and **drops block properties** (`build_outline` passes `properties: Vec::new()`).

### Root cause of the issue #114 mess

In the screenshot the reporter did **not** use an app shortcut.
On Crostini the terminal's `Ctrl+C` quits (`runtime/event_loop.rs`), so — as the report says —
they **select with the mouse and the terminal auto-copies**.
A terminal mouse selection copies the *rendered cells*: the `│ ` guides, bullets, and fold markers.
The app has no hook into that path — the emulator is copying the screen, not our data.

So fixing copy-out has two independent fronts:

1. **App-driven copy** (a shortcut serializes clean markdown to the clipboard) — fixes the data path.
2. **Mouse selection** (the habit the reporter actually uses) — only fixable if the app captures the mouse and owns the selection.

The decision (2026-06-30) is to pursue **both**.

## Proposal

### Core piece: one canonical subtree → markdown serializer (the inverse of `parse`)

Add a single owner in `outl-actions` that turns `Workspace + [NodeId]` into a clean outline-markdown string,
mirroring how `paste_markdown` is the single owner of the inbound direction.
Every client wraps it; nobody re-implements outline serialization in client code.

Reuse, do not reinvent — the line-emitting half already exists:

- `outl_md::render::render_blocks` (`crates/outl-md/src/render.rs:29`) already serializes a `&[OutlineNode]`
  at an arbitrary base indent: `- ` prefix, 2-space indent (`INDENT_UNIT`), `key:: value` props, recursive children.
  It is **private** and consumes `outl_md::parse::OutlineNode` (the minimal AST), not the rich `outl_actions::OutlineNode`.
- `project_outline_node` (`crates/outl-actions/src/outline.rs:104`) already projects `Workspace + NodeId`
  into a rich node **including the node itself, its block properties, TODO split, and full child subtree**.

The serializer is the bridge between those two:

```rust
// crates/outl-actions/src/clipboard.rs  (new module, sketch)
pub fn copy_markdown(workspace: &Workspace, roots: &[NodeId]) -> String
```

Two implementation routes, to be chosen during build:

- **(a) Promote + convert.**
  Make `render_blocks`/a thin `render_nodes(&[OutlineNode], indent)` public in `outl-md`,
  add a `rich → minimal OutlineNode` conversion (rich carries `todo`/`collapsed`/`tokens` the minimal one folds back into `text`),
  and feed it.
  Maximizes reuse of the tested renderer.
- **(b) Dedicated walker.**
  Write a small recursive emitter in `outl-actions` straight from the workspace (text via `block_text`, props via `properties_of`),
  bypassing the AST structs entirely.

Route (a) is preferred (one renderer, one indent constant, one bullet rule) unless the type conversion proves heavier than the walk.
Whichever wins, **block properties must survive** (the `render_page_md` path drops them — this serializer must not).

Open question for the serializer: should `TODO`/`DONE` prefixes and `> ` quotes be emitted verbatim (they live inside `block_text`, so they round-trip for free) — yes, keep them; that is what makes a re-paste into outl reconstruct state.

### Wiring per client

- **TUI** — `yy` / `Y` / visual `y` keep filling the in-memory register (for internal `p`/`P`),
  **and** additionally write `copy_markdown` output to the OS clipboard via `arboard` (already a dep, `Cargo.toml:50`)
  plus an **OSC 52** escape as a second channel (arboard is unreliable over SSH / inside Crostini; OSC 52 reaches the outer clipboard).
- **Desktop** — `Y` / `YankRange` call `copy_markdown` (new Tauri command) and `navigator.clipboard.writeText` the result;
  this also unblocks the TBD `p`/`P` since the register now has a defined serialization.
- **Mobile** — context-menu "Copy" switches from single-block `rawTextWithTodo` to `copy_markdown` over the block + its subtree.

A shared TS wrapper (`copyMarkdown` in `@outl/shared/api/commands`) keeps desktop and mobile on one invoke.

### TUI mouse capture (the reporter's actual habit)

Behind a config flag (default off initially), enable terminal mouse capture so the app owns selection:

- drag selects a block range (reuse the existing visual-mode range model),
- release runs `copy_markdown` → clipboard (arboard + OSC 52),
- so a mouse copy yields clean markdown instead of rendered cells.

Trade-off, called out loud: capturing the mouse **disables the terminal's native selection** for everything else (URLs, copying a single word, selecting across panes).
That is why it is opt-in, not default, and why app-yank ships first.

## Trade-offs & risks

- **Mouse capture is genuinely invasive.**
  Native terminal selection is muscle memory for many users.
  Gate it (`[tui] mouse_capture = false` by default), document the toggle, and let users who live in the mouse opt in.
- **OSC 52 size limits.**
  Some terminals cap OSC 52 payloads (~limited KB) and some strip it entirely.
  Keep `arboard` as the primary channel and OSC 52 as the best-effort second; never block on either.
- **Properties vs. the page-render path.**
  `render_page_md` deliberately drops block props; this serializer deliberately keeps them.
  Two different jobs sharing one renderer — make sure promoting `render_blocks` does not accidentally change the page-render output.
  It currently feeds `properties: Vec::new()`, so behavior is preserved as long as the caller controls what it passes.
- **Round-trip guarantee.**
  Copy-out then paste-in (into outl) should reconstruct the same subtree.
  This is the `markdown-roundtrip-tester` agent's domain — the serializer is exactly `parse`'s inverse and must be tested as a pair.
- **Type duplication.**
  The rich vs. minimal `OutlineNode` split forces a conversion.
  Acceptable, but it is the one new bit of glue; keep it in one place.

## Incremental plan (once approved)

1. `outl-actions::clipboard::copy_markdown` + roundtrip tests against `paste_markdown` (pure, no client).
2. TUI app-yank: write clean markdown to clipboard (arboard + OSC 52) on `yy`/`Y`/visual `y`.
3. Desktop + mobile wiring via a shared `copyMarkdown` command; unblock desktop `p`/`P`.
4. TUI mouse capture behind `[tui] mouse_capture` (opt-in), with docs.
5. Docs: `docs/tui.md` (copy behavior + mouse toggle), `docs/shortcuts.md` if a key changes, `docs/primitives-markdown.md` (new serializer entry, indexed from `docs/shared-primitives.md`), changelog.

## Decisions taken

- Scope: **unified copy across all clients**, not TUI-only (the serializer is shared by construction).
- TUI copy-out: **both** app-yank (now) **and** opt-in mouse capture (after).
- **No new key in the TUI.**
  `yy` / `Y` / visual `y` keep filling the internal register (for `p`/`P`) **and** additionally write clean markdown to the OS clipboard, in the same gesture.
  This is the Neovim `clipboard=unnamedplus` model — yank already means "copy", so it copies.
  Serialization runs only on yank (rare), so there is no hot-path cost, and `defaults.rs` gains nothing to drift.
  `y r` stays the one special case (it writes the `((blk-…))` ref token, not the subtree).
- **Core serializes only the canonical outl markdown.**
  Any other output format (plain text, org-mode, HTML, …) ships as an **official plugin**, modelled on [`roamresearch-export-block`](https://github.com/avelino/roamresearch-export-block) — install it if you want it.
  This keeps the core surface to exactly one format (the inverse of `parse`) and lets formats evolve out of band.
- **Block properties are emitted inline per block** (as the markdown dialect already does), preserving round-trip.
  The selected subtree's own root props ride along with its block; there is no separate "props header".

## Follow-up (plugin system)

A format plugin needs a way to register "given a selected subtree, produce text for the clipboard".
The current capabilities (`content-transformer`, `toolbar-button`, `slash-command`) cover rendering and commands but not "claim a copy/export format".
Likely a new `copy-format` / `export-format` capability: the host hands the plugin the `ReadModel` for the selection, the plugin returns a string, the host writes it to the clipboard.
This is its own RFC — out of scope for the copy-out fix, which only ships the canonical markdown path.

## Open questions

- None blocking.
  Re-open the `copy-format` capability as a separate RFC when the first non-markdown format is actually wanted.
