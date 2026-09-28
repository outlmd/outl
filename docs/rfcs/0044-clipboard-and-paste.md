# RFC 0044 — Copy-out and paste-in are one pair, and the core speaks exactly one format

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#44](https://github.com/outlmd/outl/issues/44), [#114](https://github.com/outlmd/outl/issues/114) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [paste.md](../paste.md), [clients.md § Copy and paste](../clients.md#copy-and-paste) |
| **Invariant** | root `CLAUDE.md` invariant 1 (every pasted block arrives as an `Op`), invariant 7 (block properties converge as `Op::SetProp`) |
| **Guarded by** | `roundtrips_through_paste_with_properties`, `roundtrips_todo_done_and_quote_markers` (`crates/outl-actions/src/clipboard.rs`), `paste_user_prompt_fixture` (`crates/outl-actions/src/paste/tests.rs`) |

> **Supersedes [`docs/design/clipboard.md`](../design/clipboard.md).**
> That file is this RFC, written in `docs/design/` before this process existed.
> It stays where it is on purpose — it is published at `outl.app/docs/design/clipboard.html`, and moving it breaks the link.
> The reasoning below is the same reasoning, restructured to the RFC template; the design doc still needs a pointer back here.

## Why

The clipboard was broken in both directions at once, and each direction hid the other.

**Out (#114).**
The reporter, on Crostini, copied from the TUI and pasted into another app.
The terminal's `Ctrl+C` quits (`crates/outl-tui/src/runtime/event_loop.rs`), so the habit is to select with the mouse and let the emulator auto-copy.
What the emulator copies is the *rendered cells*: the tree guides `│ `, the `- ` bullets, and the fold markers `▼`/`▶`.
None of that means anything outside the screen it was drawn on, and the pasted result is unusable text the user has to clean by hand.

App-driven copy was not a way out, because no client had one.
The TUI's `yy` and visual `y` cloned AST nodes into an **in-memory** register; only `y r` reached the OS clipboard, and only with the ref token `((blk-…))`.
The desktop copied `block.text` into a JS register and left `p`/`P` unimplemented.
Mobile's context menu wrote **one block's raw text** through `navigator.clipboard` — no children, no bullets.
There was no serializer anywhere in the repo, Rust or TypeScript, that turned a selected subtree into clean outline markdown.
The nearest primitive, `render_page_md`, is page-scoped, skips the root's own text, and passes `properties: Vec::new()`, so it drops block properties.

**In (#44).**
The mirror was just as bad, and got reported as a feature request instead of a bug.
The TUI had no bracketed paste, so a clipboard's `\n` characters arrived as `Enter` key events: each newline committed the current block and opened a sibling.
Pasting a six-line Roam outline produced mangled text intercalated with split blocks, and the hierarchy the user had copied was gone.
Mobile's `<textarea>` made the opposite mistake — the whole payload landed in a single block with literal `\n` and literal `- ` in the text.

Net effect: content could not leave outl in a usable shape, and could not enter it in a structured one.
Copying a page from one outl device and pasting it into another produced garbage on both legs of the trip.

## What we chose

Two single owners in `outl-actions`, built and tested as **inverses** of each other.

**In — `outl_actions::paste`** (`crates/outl-actions/src/paste/`).
`paste_markdown(workspace, hlc, anchor, raw)` normalises external syntax, detects outline shape, splits paragraphs, and grafts the result as blocks through the op log.
`normalize_external_syntax` is public and pure, so clients and tests can exercise the conversion without a workspace.
It rewrites Roam `{{[[TODO]]}}` / `{{[[DONE]]}}` and GitHub `- [ ]` / `- [x]` into the `TODO` / `DONE` text prefixes.
It also converts Roam embeds and queries, strips unknown `{{…}}` tokens, drops Logseq `id::` lines, and reflows 4-space indent to 2.
Date refs go through `outl_actions::dates`, so `[[April 22nd, 2026]]` becomes `[[2026-04-22]]`.
`PasteAnchor` gives three insertion points (`AsLastChildOf`, `AfterBlock`, `AtCaret`), and `looks_like_outline` is the gate that decides whether a clipboard becomes a tree at all.
`paste_plain` is the deliberate escape hatch: raw text as one block, no normalisation, no splitting.
Block properties are re-applied as `Op::SetProp`, so a pasted `key:: value` converges across devices like every other property rather than living only in the text.

**Out — `outl_actions::clipboard::copy_markdown`** (`crates/outl-actions/src/clipboard.rs`).
Takes `Workspace + &[NodeId]` and returns clean canonical outline markdown: `- ` bullets, 2-space indent, inline block properties sorted alphabetically, `TODO` / `DONE` / `> ` prefixes verbatim because they live inside the block text.
The design doc left the implementation as an open choice between promoting the `outl-md` renderer and writing a dedicated walker; the shipped code took the first route.
`copy_markdown` builds minimal `outl_md::parse::OutlineNode` values and calls the public `outl_md::render::render`, so there is exactly one renderer, one `INDENT_UNIT`, and one bullet rule in the workspace.
It owns two rules beyond serialization.
A selected id whose ancestor is also selected is dropped, because a visual range spanning a parent and its child must not emit the child twice.
An id no longer in the tree is skipped rather than emitted as a blank `- ` from `block_text(..).unwrap_or_default()`.

**The pair is the test.**
Copy-out then paste-in must reconstruct the same tree, so the round-trip is asserted directly: build a tree with `paste_markdown`, serialize it with `copy_markdown`, and `outl_md::parse::parse` of the copy must equal `parse` of the source.
This is what makes the two functions one decision instead of two features that happen to be adjacent.

**The core emits only the canonical format.**
Any other output — plain text, org-mode, HTML, a Roam-shaped export — ships as an optional format plugin, modelled on [`roamresearch-export-block`](https://github.com/avelino/roamresearch-export-block).
The reason is the round-trip above: the canonical format has an inverse in the same repo and can be pinned by a test, and a second format would have none.

**Client wiring, no new keys.**
The TUI keeps `yy` / `Y` / visual `y` filling the in-memory register for internal `p`/`P`, and **additionally** writes `copy_markdown` output to the OS clipboard in the same gesture.
That is the Neovim `clipboard=unnamedplus` model: yank already means copy, so it copies.
It writes through two channels because neither is reliable alone: `arboard` needs a local display server, and OSC 52 is the only path that reaches the outer clipboard over SSH, inside Crostini, or through tmux.
`copy_markdown_to_clipboard` returns whether *either* channel worked, and the status line reads `(clipboard unavailable)` when neither did.
Desktop and mobile call one shared `copyMarkdown` wrapper in `@outl/shared/api/commands` over one Tauri command.
On the inbound side, `choosePasteRoute(html, plain)` in `@outl/shared/paste` is the shared rich / structured / native decision, and `looksLikeOutline` mirrors the Rust gate so a one-line paste falls through to the browser's native splice.
TUI mouse capture ships behind `[tui] mouse_capture`, default `false`.

## Why not the alternatives

**Strip the guides and bullets from what the TUI draws.**
This would make a terminal mouse selection clean without any new code path.
It also makes the outline unreadable — the guides are the reason a nested tree is legible in a monospace grid — and it does nothing for the desktop and mobile clients, which have no cells to strip.

**Let each client serialize its own selection.**
This is what existed, and it is why the bug report had three different symptoms.
Three implementations of "walk a subtree and emit bullets" drift on indent width, on whether properties survive, and on whether `TODO` is a prefix or a flag, and the user meets the divergence when they move between devices.
The repo has already paid for this shape twice, in keybindings and in backlinks.

**Teach the core several output formats.**
Tempting, because "copy as plain text" is a real request.
Each added format is a second claimed inverse of `parse` with no partner to round-trip against, so its correctness becomes a matter of opinion, and the core surface grows a compatibility obligation for every format forever.
Pushing formats into plugins keeps the core at exactly one format and lets the others evolve out of band.

**Route every paste through the conversion pipeline.**
Simplest possible rule, and it breaks the most common paste.
A single word, a URL, or a copied code snippet would be turned into blocks, so the cheapest gesture in the editor becomes the one most likely to surprise.
`looks_like_outline` plus the trivial-text route exist to keep a routine paste instant and literal.

**Enable TUI mouse capture by default.**
This would fix the reporter's actual habit with no learning required.
It also takes selection away from the terminal for everything else — copying a single word, grabbing a URL, selecting across panes — which is muscle memory for most terminal users.
So app-yank shipped first and capture is opt-in.

**Reuse `render_page_md` for the selection.**
It already renders an outline and is already tested.
It is page-scoped, skips the page root's own text, and feeds `properties: Vec::new()` because a page render deliberately omits block properties.
Reusing it would silently drop every `key:: value` from the clipboard, which is the exact loss the round-trip test exists to catch.

## The opposite direction

**Copy-out is lossless only for what the canonical dialect can express.**
`copy_markdown` keeps textual block properties and drops `PageRef` / `Tag` / `List`-shaped ones silently, per the shared rule in `tree::text_properties_of`, and skips ids that are no longer in the tree.
So a user who selects ten blocks can get nine on the clipboard, with the tenth's properties simplified, and nothing says so.
The dangerous sequence is copy → delete → paste: the clipboard was the only remaining copy, and it is the reduced one.
This is a known gap, not a solved problem — see Scope.

**Paste-in is deliberately destructive, and that direction has no undo outside outl.**
`normalize_external_syntax` strips unknown `{{…}}` tokens and drops Roam block refs, because there is no mapping from a Roam UID to an outl handle.
Copying out of Roam and pasting into outl therefore loses those tokens permanently unless the Roam graph still exists.
The behaviour is pinned (`unknown_tokens_are_stripped`), but the user is not told which tokens went missing.

**The route decision is a guess, and it can be wrong in both directions.**
`choosePasteRoute` reading a code snippet as structured shreds it into sibling blocks and tears the fence apart; reading a real outline as trivial flattens the hierarchy the user came for.
That is precisely why the without-formatting chord exists (`Cmd/Ctrl+Shift+V`, TUI `Shift+P`) and why every paste inside a fenced code block is forced literal on both GUI clients — see [`docs/paste.md`](../paste.md).
The recovery from a bad guess is undo, and the user has to notice first.

**Mouse capture makes the terminal worse on purpose.**
Turning on `[tui] mouse_capture` is the app taking selection away from the emulator, so copying one word, grabbing a URL, or selecting across panes stops working while outl is focused.
That is the whole reason it defaults to off, and it means the reporter of #114 only gets their original gesture fixed if they opt in and accept the trade.

**The one place the user is told.**
Both clipboard channels are best-effort and can both fail — no display server for `arboard`, and a terminal that strips OSC 52 or, like tmux, needs `set-clipboard on`.
Rather than pretending success, `copy_markdown_to_clipboard` reports whether either landed and the status line degrades to `yanked N blocks (clipboard unavailable)`.
The in-memory register is filled either way, so internal `p`/`P` still works.

## How it cannot regress

1. **Invariants.**
   Root `CLAUDE.md` invariant 1 covers the inbound side: `paste_markdown` is listed among the composite actions in `crates/outl-actions/CLAUDE.md`, and every pasted block reaches the tree as an `Op`, never as a direct tree mutation.
   Invariant 7 covers pasted `key:: value` properties, which land as `Op::SetProp` so they converge instead of living in one device's file.
   The `paste` and `clipboard` rows of `crates/outl-actions/CLAUDE.md` carry the rules a future contributor actually reads while editing this crate.
   Those rows state that `clipboard` is the inverse of `paste`, that the two are tested as a pair, and that the core emits only the canonical format.

2. **Tests.**
   The round-trip pair is the load-bearing one, and both halves are named in **Guarded by**.
   `roundtrips_through_paste_with_properties` and `roundtrips_todo_done_and_quote_markers` in `crates/outl-actions/src/clipboard.rs` assert that re-parsing the copy yields the same AST as parsing the source, properties and text prefixes included.
   Alongside them, `range_spanning_parent_and_child_does_not_duplicate` pins the ancestor de-duplication and `properties_are_alphabetically_sorted_and_stable` pins deterministic output.
   On the inbound side, `crates/outl-actions/src/paste/tests.rs` holds `paste_user_prompt_fixture` (the literal example from #44), `paste_preserves_nested_children`, `paste_applies_block_properties`, and `plain_multi_line_pastes_one_block_each`.
   `paste_plain_never_splits_or_converts` sits next to them and fails if someone decides the without-formatting path can normalise "just a little".
   The conversion table itself is pinned per row in `crates/outl-actions/src/paste/normalize.rs`.
   That file holds `roam_todo_becomes_prefix`, `github_checkbox_becomes_todo`, `logseq_id_line_is_dropped`, `indent_4_normalizes_to_2`, `roam_long_date_becomes_iso`, and `unknown_tokens_are_stripped`.
   The shared JS decision is covered by the `choosePasteRoute` and `looksLikeOutline` suites in `crates/outl-frontend-shared/src/paste/paste.test.ts`, so the client-side gate cannot drift from the Rust one unnoticed.
   Any attempt to re-simplify `copy_markdown` into a client-local walker, or to widen the core beyond the canonical format, breaks the round-trip assertions rather than merely changing them.

## Scope

**Not covered — the `copy-format` plugin capability.**
The existing capabilities (`content-transformer`, `toolbar-button`, `slash-command`) cover rendering and commands, not "claim a copy or export format".
A `copy-format` / `export-format` capability — host hands the plugin the read model for the selection, plugin returns a string — is its own RFC, to be written when the first non-markdown format is actually wanted.

**Not covered — telling the user what a copy dropped.**
Non-textual block properties and stale ids are removed silently.
Surfacing a count, the way `outl doctor` surfaces unlogged lines, is unowned work.

**Not covered — mouse capture as the default, or a partial capture.**
A mode where outl owns drag-select for blocks while leaving word and URL selection to the terminal does not exist, and may not be expressible in the escape-sequence model.
Until it is, `[tui] mouse_capture` stays an all-or-nothing opt-in.

**Not covered — a pointer from the superseded design doc.**
[`docs/design/clipboard.md`](../design/clipboard.md) is deliberately left untouched by this RFC because its published URL is live.
It needs a notice at the top pointing here; that edit is a separate decision.
