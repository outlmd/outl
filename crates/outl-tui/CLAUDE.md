# CLAUDE.md — outl-tui

The TUI.
**Journal-first** — when you open `outl-tui`, you land on today's journal.
That's the spec; don't change it.

## Scope

- **Read + write** outline (text editing inside blocks, block create / indent / outdent / delete).
- Two modes: `Normal` (navigate, block ops) and `Insert` (edit a single block's text).
- Quick switcher (`Ctrl+P`) for fuzzy page/journal jumping.
- Outline panel for current page with inline visible cursor.
- Inline backlinks rendered below the outline (`B` toggles, `j/k` crosses the separator, `Ctrl+O` flips the sort direction).
- Block references and embeds: `((blk-XXXXXX))` resolves to the source block's text + page icon.
  `!((blk-XXXXXX))` (when the block contains a single embed token) expands the source block **and its children** read-only below the carrying block.
  `Enter` on either form opens the source page and lands the cursor on the referenced block.
  The `y r` chord plus `/refer` and `/refer-embed` slash commands copy the current block's handle to the **OS clipboard** (via `arboard`) and stash it in `App::last_yanked_ref` for in-app paste;
  the `((` autocomplete fuzzy-matches block text in Insert.
- Help popup.

## Modes

| State | What it does |
|-------|--------------|
| `Normal` | navigation + block-structure ops (no text typing) |
| `Insert { block_path, buffer, original_text }` | text edits go to the buffer; commit writes back to AST + disk |

`Esc` from Insert commits.
The buffer carries the working text; on commit we replace the AST node's `.text` and call `save()`, which writes the `.md` and runs `outl_md::reconcile_md` to update the op log + sidecar.

## Navigation (Normal)

The full chord catalog (TUI + desktop side-by-side) lives in [`docs/shortcuts.md`](../../docs/shortcuts.md).
This section captures only the **architectural / TUI-specific behaviour** a contributor needs before touching `input/normal.rs`:

- **`c` is fold, not vim's "change".**
  Bullet row shows `▼ `/`▶ ` for parents (two-space gap on leaves so columns stay flush).
  Hidden subtrees are skipped by `j`/`k`.
  Persisted as `Op::SetCollapsed` in the op log — converges across devices through the CRDT, no per-file last-write-wins.
- **`z i` / `z o` zoom (Roam/Workflowy) is a render + nav window, not op-log state.**
  Lives in `actions/zoom.rs` (an `impl App` block) and hangs off `App::zoom_stack: Vec<Vec<usize>>` — a stack of **DFS paths**, top = current render root, empty = whole page.
  Path-based (not id-based) because the TUI already navigates the in-flight AST by path via `outl_md::outline_ops`, so the breadcrumb is just a walk down the root path's ancestors (`zoom_breadcrumb`, rendered in `view::chrome::breadcrumb`).
  `selected` / `id_by_flat` stay **whole-page** flat indices; `render_outline` draws only the root subtree but keeps `cursor` counting from the root's whole-page index (`zoom_root_node`), so nothing re-indexes.
  Navigation is confined to `zoom_root_window()` — `step_forward`/`step_backward` in `actions/nav.rs` clamp to `[start, end)` and don't cross into backlinks while zoomed.
  Zooming a leaf is allowed (Workflowy shows just that block); `z o` at the page root is a silent no-op.
  **Pure local view state** — never an `Op`, per-device, cleared on every view switch (`load_current_no_autorun` empties `zoom_stack` alongside `focus`).
  A stale root path (block moved/deleted) degrades to the whole page instead of panicking.
- **`z M` / `z R` skip-leaves contract.**
  `z M` (fold-all) walks the AST in DFS and only emits `Op::SetCollapsed(true)` for blocks with `!children.is_empty()`.
  Foldar leaf é invisível hoje, mas o op fica no log: adicionar children embaixo depois faria eles aparecerem colapsados (future-surprise).
  `z R` (unfold-all) caminha todos os ids porque descolapsar leaf não tem efeito futuro.
  `outl_actions::set_block_collapsed` **sempre** escreve op no log (mesmo quando o valor já bate) — o `Ok(false)` retornado é só telemetria pra UI, não "log untouched".
  A CRDT precisa de cada flip pra convergir flips concorrentes via HLC.
- **`y r` clipboard fallback.**
  `y r` (chord) copies `((blk-XXXXXX))` to the OS clipboard via `arboard` **and** stashes it in `last_yanked_ref`.
  Status flips to `yanked … (clipboard unavailable)` on headless / no-display environments — the in-app yank register still works.
- **`yy` / `Y` / Visual `y` write to the OS clipboard.**
  Every yank writes clean canonical outl markdown to the OS clipboard via two paths in `actions/yank.rs`:
  1. `arboard` — direct X11 / Wayland / macOS clipboard API (requires a display server).
  2. OSC 52 escape sequence (`\x1b]52;c;<base64>\x07`) — works over SSH, inside tmux, and in Chrome OS Crostini where `arboard` has no display server to talk to.
  The status line reads `yanked N block(s) → clipboard` on success and `yanked N block(s) (clipboard unavailable)` when both paths fail.
  The yank register is always populated regardless.
- **`p` / `P` are OS clipboard paste, not yank-register paste.**
  `p` = paste **with formatting**: reads OS clipboard via `arboard` (`actions/paste.rs`) and routes to `outl_actions::paste_markdown` when `looks_like_outline` or multi-paragraph; else native splice.
  `P` = paste **without formatting**: reads OS clipboard and calls `outl_actions::paste_plain` — raw text as one block, no normalisation.
  The old in-app yank-register paste (`p`/`P` → paste after/before) was removed; the yank register is now mirrored to the OS clipboard on every `yy`/`Y`/`y`, so `p` picks it up via the clipboard path.
- **Visual range captures top-level roots only.**
  When yank (`y`), delete (`d`), or `Esc` exits Visual, `remember_visual_range` walks the selected ids and drops any id whose ancestor is also in the selection (it already comes inside the ancestor's subtree via `copy_markdown`).
  This prevents the same block appearing twice in the copied markdown when a parent and child are both in the Visual range.
- **`Ctrl+O` toggles the backlinks sort direction.**
  Flips `App::backlinks_newest_first` and persists the choice to `[display] backlinks_order` in `~/.config/outl/config.toml`; no index rebuild, since `sort_backlinks` runs on every read.
  Same pure-display-preference policy as `theme.preset` — it never converges between devices.
  The panel header shows the current direction (`↓ newest (^O)` / `↑ oldest (^O)`).
  Read once at boot in `runtime.rs` and set post-construction on `App` (mirrors `mouse_capture`'s wiring).
  Ordering itself runs through `outl_actions::sort_backlinks` in `App::backlinks_for_slug`, the same function the desktop and mobile clients call.
- **Mouse capture (opt-in).**
  Set `[tui] mouse_capture = true` in `~/.config/outl/config.toml` to enable `Event::Mouse` handling (`actions/mouse.rs`).
  When active: the scroll wheel moves the outline selection, a click selects the block under the pointer, and a drag selects a range — on button release the range is yanked as markdown to the OS clipboard (same arboard + OSC 52 dual path as `y`).
  Default is `false` because capturing the mouse **disables the terminal's own text-selection** (Shift-drag in most terminals).
  The keyboard yank paths work regardless of this flag.
- **`Enter` is overloaded.**
  Open `[[ref]]` / `#tag` / journal / block ref (`((blk-X))` / `!((blk-X))`) under cursor, else enter Insert.
  On a block ref it jumps to the source page and positions the cursor on the referenced block; orphan handles surface a status message and stay put.
  It deliberately does **not** follow markdown links `[text](url)` — that's `g x` (below).
- **`g x` is overloaded (code exec first, then link).**
  The decision is the pure `decide_gx(text, cursor_col)` in `actions/exec.rs`.
  A fenced code block always runs — `extract_fence` wins, so `call:` / `query` / any ` ```lang ` still executes.
  Only when the block is **not** code does it consult `link_at_cursor` and open the markdown link under the cursor (anchor or URL) in the system browser via `App::open_external_url` (the `open` crate).
  Scheme-guarded to `http` / `https` / `mailto`, mirroring the desktop frontend's `openExternalUrl`.
  Neither present → a status-line message, not the old `run failed` modal (issue #183).
- **Quit is chord-only.**
  `q q` arms + confirms (single `q` is too easy to hit by accident).
  `Z Z` is the vim "save and quit" alias — outl auto-commits on every Normal boundary, so it reduces to `q q`.
  `Ctrl+C` commits any in-flight Insert before quitting.
- **`r` / `f` / `F` use `state.pending_input_op`.**
  A one-shot enum (mutually exclusive with `pending_chord`) consumed by the next `Char(c)` keystroke.
  Any other keystroke cancels.
- **`*` / `#` reuse the workspace search.** `search_word_under_cursor` extracts the word the cursor sits on (whitespace-bounded), opens the `/` overlay machinery, accepts the first hit, persists the rest into `last_search` so `n` / `N` walk them.
- **Sidebar intercept.** With the sidebar open (`\` / `Ctrl+E`), `j` / `k` move the row selection, `Tab` cycles the section (Today / Pinned / Recent / Calendar), and `Enter` opens the focused page. `d` on a **regular page** arms a `delete page '<title>'? y/n` confirmation in the status line (`App::pending_sidebar_delete: Option<PendingSidebarDelete>`); `y` / `Y` confirms and runs `outl_actions::delete_page` + `remove_page_projection` + `spawn_index_rebuild`, navigates to today if the deleted page was current, and announces to peers. Any other key cancels (and is swallowed, matching the `pending_input_op` contract). Calendar rows are a no-op, and journals pinned/recent are excluded — only regular pages can be deleted from the sidebar. The `g d` chord (Normal mode) routes through the same confirmation flow via `App::delete_page_from_chord`: with the sidebar focused it delegates to `sidebar_delete_current`, otherwise it arms the confirmation against the current page (refusing journals).
- **Visual range capture.**
  Every Visual exit (`Esc`, `y`, `d`) routes through `remember_visual_range` so `g v` can restore the last range.
- **Visual range reorder (`Alt+↑` / `Alt+↓`).**
  `move_{up,down}_visual_range` drag the whole selection among its siblings — mirror of the single-block `Alt`+arrows in Normal (the plain arrows extend the range, so `Alt` is what separates reorder from grow).
  They loop `move_{up,down}_at_path` (`lo→hi` for up, `hi→lo` for down) and follow the selection one row; if the leading block can't move (already first/last sibling) the op aborts before the rest of the range scrambles against itself.

### Properties (`g p`)

`g p` opens the property editor for the selected block (`actions/properties.rs` + `view/properties.rs`, keys routed by `input/overlay.rs::handle_properties_key`).
`j` / `k` move, `Enter` edits the highlighted **value**, `o` adds a pair (key first, `Enter` advances to the value, `Enter` again saves), `d d` deletes, `p` flips between block and page scope, `Esc` / `q` closes.

Three contracts worth knowing before touching it:

- **Rows come from the parsed AST, not the workspace tree.**
  `set_property_on_current_block` / `set_property_on_page` write the AST, and the two only meet at a save boundary — reading the tree would list a value the user cannot see in the outline and would drop the one they just typed.
  This is the same trap `actions/reminders.rs` documents for `g r`.
  `outl_actions::property::page_properties` is therefore **not** the reader here; the structural-key predicate it uses (`outl_actions::tree::is_page_model_key`) is, so `page-slug` / `page-kind` are filtered out of the list and refused on create.
- **Key completion is `outl_actions::known_keys`, snapshotted on open.**
  Frequency-ranked, one owner shared with the GUI clients.
  Snapshotted because the overlay owns the keyboard while it is up, so no key can appear mid-session; `open_properties` calls `flush_pending_save()` first so the catalogue isn't one coalesced edit behind.
  `key_completions` (pure, tested) is the prefix-then-substring ranking `Tab` walks.
- **The value field reuses Insert mode's autocomplete wholesale.**
  Same `detect_trigger`, same `candidates_for_pageref` / `candidates_for_tag`, same `App::autocomplete` slot, same popup renderer — only the accept differs (`apply_value_completion` writes a `String`, `accept_autocomplete` writes an `EditBuffer`).
  Only `[[` and `#` are honoured: `((`, `/`, `@` and `:` are ordinary characters in a property value.
  Because the popup renders off `App::autocomplete` independently of `App::overlay`, **anything that closes the overlay must clear it** (`close_properties`), or a dangling box paints over the outline with no keystream to dismiss it.

**`g p` used to be the `pinned::` toggle; that moved to `g P`.**
Both spellings now live in the shared catalog (`Action::OpenProperties` / `Action::TogglePin`) with drift-guard tests at the bottom of `input/normal.rs`, because a chord outside the catalog is a chord the duplicate-detection test cannot see — which is how the collision happened in the first place.

### Reminders (`remind::`)

`g r` attaches a starter rule (`remind:: 9am`) to the selected block; it **reports** an existing rule instead of overwriting, so the chord can't discard a carefully typed schedule.
`g R` writes the "nag me" preset (`now every 1h until DONE`) and **does** overwrite — escalating is an explicit act.
`g n` opens the reminders overlay (`Enter` opens the page, `s` snoozes 1h, `Esc` / `q` closes).

The issue's original sketch put the overlay on `Ctrl+R`; that is already Redo in Normal mode, and a terminal can't distinguish `Ctrl+R` from `Ctrl+Shift+R` — hence the `g` family here and `Cmd/Ctrl+Shift+R` on the desktop only.

**The TUI delivers too, while it's open.** `deliver_due_reminders` runs on the event-loop tick (after the peer-ops and orphan polls, so a rule or snooze that just synced is honoured on the same tick) and fires an OSC 9 desktop notification plus a 10s toast. OSC 9 is the sibling of the OSC 52 in `actions/yank.rs`: emitted only when stdout is a terminal, best-effort, and the toast covers the emulators that ignore it. Delivery is skipped in Insert mode (a repaint mid-keystroke is how you get the feature turned off) and the fire isn't lost, the fired log only records what actually went out. A reminder due with the TUI closed *is* lost to this client, which is the real limit of a terminal session.
Every schedule question routes to `outl_actions::reminders` (`actions/reminders.rs` is pure wiring) — see [`docs/reminders.md`](../../docs/reminders.md).

## Insert mode

Full Insert key list is in [`docs/shortcuts.md`](../../docs/shortcuts.md).
TUI-specific contracts worth remembering:

- **`Esc` commits** through `commit_insert` (writes buffer → AST → disk via `outl_md::reconcile_md`).
  Aborting without commit is `abort_insert` — wired to nothing today; we never lose user keystrokes silently.
- **`Enter` splits the block at the cursor (issue #184).**
  The text before the cursor stays in the current block (the head), the text from the cursor onward moves into a new sibling created right below (the tail), and Insert re-enters on the new block with the cursor parked at its start.
  Cursor at the end of the text is the degenerate case: an empty tail, i.e. the old "commit + new block below" behaviour.
  Cursor at the start empties the head and pushes the whole line down into the sibling — "open an empty block above".
  The Insert-mode commit path also drains `pending_reload` (peer-ops poller held it back during the edit).
- **`Backspace` on an empty block deletes the block** and moves selection to the previous one — the only structural mutation that can happen from Insert.
- **Autocomplete triggers** are pure trigger-detection inside the buffer (`[[`, `#`, `((`, `/`); they own the keystream while their popup is open.
  See `actions/autocomplete.rs` for the trigger detector contract.
- **Drag-and-drop file upload (`actions/paste.rs`).**
  Terminals report a file dragged into the window as its path pasted as text (bracketed `Event::Paste`).
  In Insert mode, `paste_external` first runs the pure `looks_like_dropped_files` decision *before* the normal paste flow.
  If the payload is exactly the path(s) of existing file(s), it imports each via `outl_actions::import_asset` (into `<root>/assets/<hash>.<ext>`, capped by `[assets] max_bytes`).
  The returned markdown link(s) splice at the caret through the same `EditBuffer::insert_str` the normal Insert-mode paste uses, so the edit commits on the next Esc like any keystroke (AST-first model intact).
  **Anti-hijack heuristic**: the pure `looks_like_dropped_file_path(&str) -> Option<PathBuf>` returns `Some` only when the trimmed payload is a **single line** AND names an **existing file** (`Path::is_file()`).
  Pasting a real file's path into an outliner is rare, so both guards together keep a legitimate text paste from being stolen.
  Multi-file (`looks_like_dropped_files`) requires **every** newline-separated line to be an existing file, else the whole payload falls back to normal paste.
  macOS backslash-escaped spaces (`/dir/my\ file.pdf`) are unescaped as a fallback only when the raw token isn't already a file.
  On an import error (too large, IO) the status line shows the error and **nothing** is inserted — a raw path in the block is worse than a retryable no-op.
  The pure decision lives apart from the import effect so it's unit-tested without touching the workspace.

## Visual conventions

- Selected block is highlighted with a colored bullet.
- In Insert mode, a `▏` caret marks cursor position inside the block.
- In Normal mode on the selected block, a block cursor (white bg) sits on the character under `cursor_col`.
- Other (non-focused) blocks render markdown prettily: `**bold**` shows as bold without asterisks,
  `*italic*` as italic,
  `~~strike~~` struck through,
  `` `code` `` in green,
  `[text](url)` blue-underlined,
  `[[ref]]` cyan-underlined (no brackets),
  `#tag` magenta-underlined,
  `((blk-XXXXXX))` resolves to the source block's text + page icon (orphan handles render dimmed),
  `:shortcode:` renders as the unicode glyph (`:tada:` → 🎉; unknown shortcodes never tokenize so the literal stays visible).
- The selected/editing block keeps `:` (dim) + shortcode (raw) + `:` (dim) so cursor columns match source bytes 1:1 — the pretty render is the only place the glyph appears.
- `!((blk-XXXXXX))` — when a block contains a single embed token (whitespace OK around it) — expands the source block **and its children** read-only below the carrying block.
  Conventions:
  - Every embed row carries a `↳ ` prefix (root + descendants) so the expansion reads as one cohesive block.
  - Descendants get `2 * (depth + 1)` spaces of padding before `↳ ` so children align under the source root's *text*, not under its `↳ `.
  - Outer indent (`│ ` guides) matches the carrying block's outline depth.
  - Task checkboxes (`☐` TODO / `◐` DOING / `☑` DONE), page refs and tags render with their normal styling inside the expansion (via `render_pretty_block_text`).
  - Recursion is capped at depth 4 to break embed cycles.
  - Expansion runs in every render mode — but the carrying block's first row keeps the raw `!((…))` literal under the cursor so column-byte alignment holds.
- The selected/editing block renders **raw** (delimiters visible, dimmed) so cursor columns map 1:1 to source bytes — including the literal `((blk-XXXXXX))` and `!((blk-XXXXXX))` forms.
- A block whose text starts with the CommonMark `"> "` prefix renders with a left `│ ` bar (dimmed, `theme.dim`) and full body colour — the `│` is enough of a cue, dimming refs / tags / bold would erase their affordance.
  Same affordance as `TODO`/`DONE`.
  The chrome lives in **`view::inline::render_pretty_block_text_impl`** and is the **only owner** of the bar + checkbox + token rendering pipeline;
  the outline view's `BlockRowKind::Bullet if single_line_pretty` branch delegates to it directly (one owner, every caller wraps).
  The bar composes with the task checkbox (`│ ☐ foo`) and `view::inline::split_block_prefixes` accepts the prefixes in **either order**, so `"> TODO foo"` and `"TODO > foo"` render the same.
- **Task states draw as `☐` (TODO), `◐` (DOING), `☑` (DONE).**
  DOING shares `theme.todo_open`'s colour rather than claiming a third palette entry — it is unfinished work and the glyph already says which kind.
  Only DONE dims and strikes the body.
  `Ctrl+T` walks `none → TODO → DOING → DONE → none`; in Insert mode `cycle_todo_inline` shifts the caret by the **difference between the two prefixes** (`DOING ` is one wider than its neighbours), never by a constant.
- IDs are **never** shown.
- Mode tag (`NORMAL`/`INSERT`) appears in the header.
- **Block text word-wraps to the pane width** (issue #99).
  Terminals don't reflow, and `Paragraph::wrap` can't be used because it expands lines *after* layout and would desync the `selected_line` scroll index.
  So `view::wrap::push_wrapped` emits the wrapped `Line`s up front: the first visual row keeps the bullet/fold `head`, continuations re-indent under the text column, and the `│ ` indent rails repeat on every row.
  Wrapping runs on the already-styled `Span`s (post-tokenization), so a break never splits a `**bold**` token back into literal asterisks.
  **Cursor rows (Insert / Normal-selected) wrap too** — `emit_row_with_cursor` bakes the caret / block cursor into the row's `Span`s *before* `push_wrapped` runs.
  Reflowing just carries the cursor onto its wrapped visual row: the char offset was already consumed turning it into a span, so there's nothing left to desync.
  The earlier "cursor rows pass width `0`" workaround was the actual #99 regression: the selected block stayed on one overflowing line and only wrapped once the cursor left it (`viewing mode won't wrap until I navigate away`).
  `text_width == 0` is still the "don't wrap" sentinel, but only headless renders pass it now.

## Reuse across UI surfaces (Tauri, mobile)

The TUI is **the first UI surface**, not the only one.
Every piece of logic that's not strictly about ratatui rendering lives in `outl-md` (or `outl-core`) so Tauri and the mobile apps can consume it later:

| Layer | Owns |
|-------|------|
| `outl-core` | Op log, CRDT, storage, workspace |
| `outl-md` | Parse/render, sidecar, matching, reconcile, **inline tokens (`InlineTok`, `RefTarget`)**, **slugify** |
| `outl-actions` | UI-agnostic workspace operations (edit, indent, move, toggle TODO, page model, backlinks). **TUI now imports from here** — `cycle_todo`, `split_todo`, and `TodoState` live in `outl-actions`, and so does the date slash-command math (`dates::parse_date_arg`, `week_tag`, `days_until_next_weekday`, `journal_ref`); `commands/builtins/dates.rs` keeps only the Insert-mode wiring. |
| `outl-tui` | Terminal-specific: ratatui mapping, key handling, raw-mode lifecycle |
| `outl-mobile` (shipping today) | Tauri 2 + Solid: consumes `InlineTok` / `RefTarget` from `outl-md` and renders to JSX. Shares every workspace operation with the TUI via `outl-actions`. |
| `outl-desktop` (shipping today) | Tauri 2 + Solid for macOS / Linux / Windows. Shares the entire `outl-actions` surface plus the `@outl/shared` TS lib (DTOs, `MarkdownInline`, paste / autocomplete helpers, command wrappers) with mobile. Adds OS-standard shortcuts, `outl-exec` code-block execution, and a cross-platform FS watcher (`notify`) that emits `peer-ops-changed` so the frontend reloads automatically when a peer's `ops-*.jsonl` arrives. |

Pattern when adding a new feature:

1. **Grep first.**
   Before writing a helper here, `rg "fn <name>"` / `rg "struct <Name>"` across `crates/outl-core`, `crates/outl-md`, `crates/outl-actions`.
   The thing you're about to write probably exists upstream — wrap it instead of cloning the logic.
2. If it's data or pure logic → put in `outl-md` (or `outl-core`).
3. If it's a workspace mutation two clients would call the same way → put it in `outl-actions`.
4. If it's how it's drawn on a terminal → put in `outl-tui`.
5. Never write a function in `outl-tui` that a Tauri/mobile client would also need byte-for-byte.
   Extract upstream first.

**Concrete example:** `EditBuffer::move_up` / `move_down` (cursor nav across `\n` inside a multi-line block) are TUI-specific primitives,
but the `(line, col) ↔ char_idx` math underneath isn't — it's shared with how the renderer maps a cursor onto a [`outl_md::view::BlockRow`].
Both directions live in `outl_md::view::{char_to_line_col, line_col_to_char}` and the `EditBuffer` methods are thin wrappers.
**Don't** add a `line_start_and_column` helper here; extract the inverse to `outl-md` if it's missing and wrap from here.

## Persistence model

Editing is **AST-first**: edits mutate an in-memory `ParsedPage`.
On commit boundaries (Esc, Enter, dd, Tab/Shift-Tab, structural ops), the TUI persists in two decoupled steps:

1. **Mark dirty + repaint (`App::save`).**
   The commit boundary only records that the AST diverged from disk (`App::dirty_since`) and lets the next frame paint the result — no disk I/O.
   This is what makes a burst of edits (`Esc o … Esc o …`) feel instant: the user never waits on a per-commit fsync.
2. **Drain (`App::flush_pending_save` → `App::persist`).**
   The heavy work runs when it won't stall input:
   - renders the AST back to `.md` via `outl_md::render`,
   - writes the `.md` file,
   - calls `outl_md::reconcile_md` (matching → diff → applies ops to the workspace → updates the sidecar).

The event loop drains the moment it goes idle (no keystroke waiting in the terminal buffer).
`runtime::MAX_SAVE_DEFER` (600 ms) forces a flush even mid-burst so an unsaved edit can't linger longer than that, bounding crash loss.

**Every path that reads persisted state flushes first**, so a reader never sees a stale `.md` / op log.
Those paths are navigation (`load_current_no_autorun`), peer reload (`reload_workspace_from_disk`), quit (`Ctrl+C`, `q q`, overlay-driven exit), explicit `Ctrl+S`, `call:` re-run (`commit_insert`), and code-block exec (`run_current_block`).
**If you add an action that reads the workspace or `.md` after an edit, call `flush_pending_save()` first** — otherwise it acts on pre-edit state.
Regression tests for the invariant live at the bottom of `actions/lifecycle/persistence.rs`.

This means concurrent `outl serve` is OK — both go through the same reconcile path; the sidecar `last_synced_hash` short-circuits no-ops.

### Automatic backups

`runtime::run_with_theme_override` calls `outl_actions::backup::spawn_auto_pass(root, cfg.backup.enabled, cfg.backup.interval_minutes)` once at startup — the TUI is the **only** client running the pass today.
It is a detached background thread on purpose: a snapshot walks the whole workspace and forks `git`, which on a large graph is seconds, so it must never sit on the event loop, a commit boundary, or the quit path.
Don't "improve" this by hanging it off the idle tick or `flush_pending_save` — the interval floor is read back out of the git history, so a session that ends before a snapshot simply gets one on the next launch.
Policy + the "which client snapshots" table live in [`docs/clients.md` → Automatic backups](../../docs/clients.md#automatic-backups).

## Layout

```
src/
├── main.rs              # binary entry (clap + outl_tui::run)
├── lib.rs               # exposes `run` so outl-cli can reuse the TUI
├── app.rs               # thin re-export shim + cross-module tests
├── state.rs             # plain data: App, Mode, Focus, Overlay, snapshots
├── actions/             # impl App { ... } blocks, one per concern
│   ├── lifecycle.rs     # load / save / external-edit polling / new
│   ├── nav.rs           # page/journal jumps, cursor, ref open, Focus-aware move
│   ├── block.rs         # Insert mode, create/indent/outdent/delete blocks
│   ├── history.rs       # undo / redo snapshots
│   ├── visual.rs        # Visual mode + range ops
│   ├── yank.rs          # yank register, copy-to-OS-clipboard (arboard + OSC 52)
│   ├── mouse.rs         # mouse capture: click-selects, wheel moves, drag-selects and copies markdown on release
│   ├── paste.rs         # external-clipboard paste (bracketed paste → outl_actions::paste_markdown)
│   ├── exec.rs          # run code block via outl_exec
│   ├── plugins.rs       # outl_plugins::PluginHost wiring (load, slash dispatch, op-hook sweep)
│   ├── properties.rs    # the `g p` property editor (key catalogue completion, `[[` in the value)
│   └── overlay.rs       # quick switcher, search, palette, autocomplete
├── input/               # key → action routing (normal/insert/visual/overlay)
│   ├── chord_adapter.rs # crossterm KeyEvent → outl_shortcuts::Chord
│   └── plugin_chord.rs  # plugin keybinding dispatch (Normal-only, never shadows native)
├── view.rs              # render_app orchestrator; thin
├── view/
│   ├── inline.rs        # span-level markdown (highlight + pretty)
│   ├── outline.rs       # outline rendering (render_outline, render_block, …)
│   ├── wrap.rs          # width-aware word wrap of styled spans (push_wrapped)
│   ├── overlays.rs      # every modal popup
│   ├── properties.rs    # the `g p` property editor popup
│   ├── warnings_banner.rs # yellow banner above the outline when the current page has ParseWarnings
│   └── backlinks.rs     # inline backlinks section (below outline, ─ rule)
├── outline_ops.rs       # one-line re-export shim — helpers moved to outl_md::outline_ops so the mobile client can share them
├── edit_buffer.rs       # cursor + chars; isolated, well-tested
├── editor.rs            # placeholder for block-level editor widgets (not yet built)
└── ui/                  # legacy placeholders; logic lives in view/
```

## Dependencies

- `ratatui` + `crossterm` (UI).
- `outl-core`, `outl-md` (workspace, parse/render/reconcile).
- `outl-ws` (workspace layout + the device-actor protocol).
  `runtime::open_workspace` calls `outl_ws::layout::read_or_init_config` and `outl_ws::actor::resolve_device_actor` instead of parsing `.outl/config.toml` by hand.
  The hand-rolled version drifted from the CLI's schema: it seeded a config with `actor_id` but no `created_at`, which `outl-ws` then refused to deserialize.
  It also read the write actor straight out of the workspace, which is silent op loss on any transport that replicates `.outl/` (see `outl-core/CLAUDE.md` → "Actor id is device-local").
  Only the optional per-workspace `[theme]` is still read off the raw TOML, since `outl_ws::layout::Config` models `[workspace]` alone.
- `arboard` (OS clipboard for `y r` / `/refer` / `/refer-embed` and for yank-to-clipboard; degrades to status-line-only on headless).
- `base64` (encodes the OSC 52 escape sequence that writes to the clipboard in SSH / tmux / Crostini environments where `arboard` has no display server).
- `outl-plugins` (JS plugin runtime — `PluginHost`, `load_installed`; default `js`/Boa feature on).
- `walkdir`, `toml`, `ulid`, `chrono`, `anyhow`.

## Plugins

JS plugins are loaded at boot from `<root>/.outl/plugins/` into an `outl_plugins::PluginHost` held directly in `App.plugin_host` (`Option`, single-threaded — no `Arc`/`Mutex`, the Boa context is `!Send`).
Boot / slash / op-hook / content-transform wiring lives in `actions/plugins.rs`; keybinding dispatch lives in `input/plugin_chord.rs`.
The five touch points are:

- **Boot** (`App::load_plugins`, called at the end of `App::new`).
  Declares the client capabilities the TUI honors (`slash-command`, `op-hook`, `keybinding`, `content-transformer:text`, `toolbar-button`), runs `load_installed`, then `mark_synced` so pre-existing ops don't fire hooks on startup.
  A `toolbar-button` has no chrome bar in a terminal, so its command is surfaced in the **slash menu** instead (deduped against `slash-command` entries) — a runnable command is never dropped just because its only affordance was a GUI button.
  `ctx.net`, `ctx.storage`, and the gas limits are host-level (the engine), so they work in the TUI with no per-capability wiring — only HTML surfaces (`ui-render`, `content-transformer:rich`) stay undeclared, since a terminal can't draw them.
  Best-effort: a load failure toasts a warning and the TUI runs normally; a workspace with no plugins is unchanged.
  **`content-transformer:rich` is deliberately *not* declared** — `rich` output is HTML for a GUI iframe, meaningless in a terminal; the host filters those out of `host.transformers()` automatically.
- **Slash commands** (`App::slash_candidates` in `actions/overlay.rs`).
  The slash menu concatenates `host.commands()` onto the built-in registry list; each plugin command carries a `SlashOrigin::Plugin { plugin_id, command_id }` tag (vs `SlashOrigin::Builtin`).
  `accept_slash` routes a plugin pick to `App::run_plugin_command`, which surfaces `notify`/error output as toasts and re-projects if it mutated.
- **Keybindings** (`input/plugin_chord.rs::try_plugin_binding`, called first inside `handle_normal_key`).
  A plugin's `contributes.keybindings[].key` is parsed by `outl-plugins` into an `outl_shortcuts::ChordSequence`; `input/chord_adapter.rs` maps the live `crossterm::KeyEvent` into the same `outl_shortcuts::Chord` so we can compare them.
  A matching single-chord binding runs `App::run_plugin_command` immediately.
  A two-chord binding (`Ctrl+G A`) buffers the first chord in `App::pending_plugin_chord` (a **separate** field from the native `pending_chord` vim accumulator so the two never interfere) and fires on the second key.
  **Plugin chords are scoped to Normal mode** — they're `Mode::Global` in the catalog, but the TUI deliberately won't steal keys mid-edit.
  They **never shadow a native action**: `native_normal_chord` mirrors what `handle_normal_key` consumes, so a plugin can't rebind `j`, `dd`, `Ctrl+T`, `Ctrl+P`, etc. (use a free chord like `Ctrl+G` or a two-chord sequence).
  No host / no bindings / a key with no `Chord` form all short-circuit to native handling.
- **Op hooks** (`App::run_plugin_op_hooks`).
  Called once per iteration at the **single post-mutation point** in `runtime.rs`'s event loop (after the mode key handler, before the next draw).
  Deferred while in `Mode::Insert` (same reason as `pending_reload`: a hook-driven `load_current` would clobber the in-flight buffer; the edit isn't in the op log until commit anyway).
- **Content transformers** (`App::recompute_transforms`).
  **Pre-compute, not render-time.**
  When a block's text is a single closed code fence (`` ```<lang> `` … `` ``` ``) whose language a loaded `text` transformer claims, its body runs through `host.transform_block` *at load time*.
  The result is cached in `App::transform_cache`, keyed by `NodeId`.
  The render walk (`view/outline.rs`) only has `&App`, and `transform_block` is `&mut self` (it runs Boa) — so the transform **cannot** happen during render.
  It's done in `recompute_transforms`, called from `load_current_no_autorun` (every reparse), after `load_plugins` at boot, and on the reproject paths (plugin + peer mutations).
  The render path is then a pure `HashMap` lookup: a read-only block with a cache hit renders the transformed text/markdown (`RenderMode::Transformed`) in place of the raw fence; the bullet stays.
  **A block under the cursor (Insert / Normal-selected) always renders the raw fence source** so the user edits what they see — the cursor cases win over a cache hit.
  Lang match: the fence's raw info-string first (custom langs like `mermaid`), then the canonical alias via `outl_md::lang::canonical` (so a transformer registered as `rust` fires on `` ```rs ``).
  Best-effort: a plugin error or `Ok(None)` (declined) leaves the block to render as a raw fence — never crashes.

A plugin mutation lands in the op log via `outl-actions` but does **not** write `.md`, so `reproject_after_plugin` runs `outl_actions::apply_all_pages_md` (a plugin can touch any page) then `load_current`.
If a plugin declares a capability the TUI lacks, the host filters it; `host.missing_capabilities(id)` lists the gap.

## Peer sync coordination

The TUI is a peer in a multi-device workspace.
Two threads' worth of sync logic live here on top of `outl_actions::SyncEngine`:

| Thread / path | Responsibility |
|---------------|----------------|
| `wire_sync_transport` (startup, before the poller) | Asks `outl_sync_iroh::build_transport` for this device's iroh endpoint and stores the result in `App::sync_transport`. Every decision lives there — `[sync] transport` (iroh is the **default**), the identity (`~/.outl/identity.key`) + per-workspace peer store, the relay, and the **device endpoint lease**: a device binds one endpoint at a time, so a co-resident desktop / `outl mcp serve` that started first keeps it and the TUI stays on the poller (status line says so). All three non-`Ready` outcomes leave `sync_transport` as `None`; the filesystem poller covers detection either way, and startup never aborts. |
| `spawn_jsonl_poller` (startup) | Runs **both** change-detection backends, not one or the other. `outl_actions::FileSyncTransport.start(...)` always runs (polls peer `ops-<actor>.jsonl` every ~2 s, own file filtered out, signals on growth); when `App::sync_transport` is set (iroh) `transport.start(...)` runs too (QUIC → local `ops/` → signal). They cover different paths and neither subsumes the other: iroh signals only on its OWN wire receipts, the poller signals on ANY disk write — including ops a **co-resident** process (a desktop / MCP / CLI sharing this workspace's `ops/`) wrote. That co-resident case is load-bearing: the desktop and the TUI share `~/.outl/identity.key` (one node id per device), so the relay routes a mobile peer's inbound to whichever endpoint holds the route (usually the desktop); the desktop writes the received ops to the shared `ops/`, but the TUI's iroh transport never saw them on the wire, so without the always-on poller the TUI stayed blind to them (the "TUI ↔ mobile doesn't sync" bug). Reopen is idempotent, so the occasional overlap of both signals just reconfirms convergence. |
| `save` / `save_page_with` post-commit | After the local edit lands in the op log, calls `transport.announce_local_ops(slug, hlc)` when a transport is set. **Both** commit paths announce: `save` (the hot path — current page) and `save_page_with` (cross-page backlink edits / Insert into a source page), so a backlink edit wakes peers too instead of only converging on the catch-up re-sync. No-op for `FileSyncTransport`; `IrohSyncTransport` gossips the new HLC so peers pull over QUIC. |
| `poll_jsonl_updates` (main loop, per tick) | Drains the signal. **In Insert mode**, sets `pending_reload = true` and returns — the in-flight `ParsedPage` would be clobbered by a reload mid-edit. Outside Insert, calls `engine.reload_workspace()` and `engine.reproject_page()`. |
| `commit_insert` | After the user's edit lands in the op log, drains `pending_reload` and runs the deferred reload. The CRDT merges peer ops with the freshly-committed local edit. |
| `spawn_orphan_md_scanner` (worker thread, 10s tick) | Calls `engine.scan_for_orphans()` to find `.md` files whose sidecar is missing or stale (Roam import, peer-shipped projection without sidecar, vim edits). Signals the main loop, which runs `outl_md::reconcile::reconcile_md` on each path (also deferred during Insert mode). |

The two filters that make this safe:

- `snapshot_peers` (not `snapshot`) — never react to your own jsonl growing, or every save closes a reload-race loop.
- `pending_reload` flag — never swap the workspace while an Insert buffer has unsaved keystrokes; the unsaved buffer is not in any op log yet and the CRDT can't help with state it doesn't know about.

Mobile shares the same `SyncEngine` but does not need the `pending_reload` flag: every mutation is one atomic Tauri command, so there's no multi-keystroke window.
Different policy, same engine.

## What this crate does NOT do

- ❌ Mutate the op log directly — every change goes through `outl_md::reconcile_md`, which routes ops through `Workspace`.
- ❌ Parse markdown by hand — use `outl-md`.
- ❌ Re-implement workspace operations that another client needs — put them in `outl-actions` first.
- ❌ Render outside the AST — the AST is the source of truth between the TUI and disk.

## Things to be careful about

- **Cursor accounting**: `EditBuffer.cursor` is a char index, not a byte offset.
  When converting to ratatui spans for rendering, use `byte_index_for_char` to slice the string correctly.
  Skipping this step crashes on multi-byte UTF-8.
- **Empty page**: `save()` always re-adds a single empty bullet when the page would otherwise be empty, so the cursor never has nowhere to go.
- **Chord state**: `pending_chord` is cleared on every key press.
  Don't let it persist past one event or `gj` becomes "g + (anything)".

## When you're done

1. `cargo fmt`
2. `cargo clippy -p outl-tui --all-targets -- -D warnings`
3. `cargo test -p outl-tui` (lib + bin + e2e tests)
4. **`RUSTDOCFLAGS="-D warnings" cargo doc -p outl-tui --no-deps`** — CI runs this and it catches things `clippy` won't.
   Most common bite: ``[`SomeType`]`` in a `//!` module doc where the type is `pub(crate)` triggers `rustdoc::private_intra_doc_links`.
   Drop the brackets, keep the backticks: `` `SomeType` ``.
5. Manual smoke in a real terminal: `outl init /tmp/x && outl --path /tmp/x`
