# Backlinks on every client

What the "Linked from" panel is on each client: a place you can work, in what order, with what breadcrumb, plus the nested-pages section that rides the same reply and the index that makes it cheap.

Moved out of [`docs/clients.md`](clients.md), which had grown to the point where no edit to it was cheap.
That page keeps the cross-client architecture: the stack, who owns which fact, the locks an open takes, and the two conditions every client has to surface.

## Editing a backlink where it is cited

A backlink row shows a block that lives on **another** page.
It used to be a destination: you acted on it by leaving, and the page that assembled the references was gone.
It is a place you can work now, on every client.

Ten operations reach a row, and each lands on the citing block on **its own** page: the text edit, task state, mark done (`g D`), delete, indent, outdent, move up, move down, and create a sibling below and above.
Mobile's long-press sheet has no mark-done row today (`Action::MarkDone` is `Missing` there in [`client-parity.md`](client-parity.md)), so it offers nine.
`Capability::BacklinkInlineEdit` covers exactly those nine, leaving mark done to the action row that owns it rather than claiming an operation one client has nowhere.
Opening the source page stays reachable on the desktop and mobile, on the group header rather than on the row; the TUI has no gesture for it at all, which is the whole of why its column reads `Partial`.
Per-client reach is `Capability::BacklinkInlineEdit` in [`client-parity.md`](client-parity.md), which is generated from `outl_shortcuts::capability_support`.

| Client | Edit in place | Everything else | Open the source page |
|---|---|---|---|
| TUI | `i` / `Enter` on the focused row | the same Normal-mode chords the outline uses | nothing in the panel opens it — `Ctrl+P`, then the page name from the row's header |
| Desktop | click the row, or `i` / `Enter` on it | the same chords, consulted on the row first | the group header (a `<button>`, so a keyboard stop) |
| Mobile | tap the row | long-press for the same sheet the outline raises | the card header |

Four rules hold across all three, and each was the alternative:

- **A mutation is an `Op` on a `NodeId`, never a `.md` the client rewrote.**
  The TUI used to render the source page's whole `.md`, write it, and let `reconcile_md` infer which ops that implied.
  Under concurrency that is not the same operation: it depends on `outl-md`'s matching heuristics to decide which block the new text belongs to, where an op *names* the block.
  Root [`CLAUDE.md`](../CLAUDE.md#critical-invariants-never-violate) invariant 1 says the op log is the source of truth, so all three clients now reach `outl-actions` by id, through `outl_actions::commit_page`.
- **A cross-page write refuses rather than re-targets — all ten of them, not just the text edit.**
  A backlink's `block_id` can be a transient `NodeId` — the index is built from the `.md` on disk, and a page whose sidecar is absent or misaligned mints one per uncovered block — so the id may name no node in the tree.
  The ordinary `block.rs` commands tolerate that, because they act on the page you are reading: `edit_block` and `create_block` append at the page's end, which keeps the result where you can still see it there and moves it to a page you are not looking at here.
  So the GUI clients call the cross-page family instead — `edit_backlink_block`, `backlink_row_op`, `create_backlink_sibling` — which refuse, and the refusal reaches you.
  A source page that has stopped syncing is refused **before** anything is written too, and **which gestures ask is decided per operation, not per client**.
  The criterion is whether the operation's result is computed from text the draft or the row showed — never whether it is "structural".
  Two shapes answer yes.
  A **text edit** on the desktop and mobile, because both seed the draft from the backlink row, which is built from the `.md` on disk: on a frozen page the user types over bytes the op log does not hold.
  And **task state / mark done on every client**, because both read the block's logged text, move a marker and write the result back, while the row the user acted on was rendered from that same stale `.md`.
  Everything else names a node by id — delete, indent, outdent, both moves, create a sibling — reads no text, and converges through the op log whatever the local `.md` says.
  **The TUI's text edit is the one place the answer differs between clients, and the reason is where the draft comes from.** `enter_insert_backlink` seeds its buffer from the op log, so there the logged text *is* what the user edited and the question has nothing to protect.
  Gating it anyway cost the one unrecoverable thing in this feature: the Insert buffer is consumed before the commit runs, so the refusal dropped the keystrokes while the status line described a *file* and told the user to run a command.
  **What is left is a notice gap, not a capability one.**
  A structural op still converges, because peers sync through the op log and never through the `.md`.
  What stays behind is the local `.md` of a page that is not on screen, and the user is told nothing: the refusal rides the reply (`outl-tauri-shared/src/commit.rs`), which reaches the screen only when the source page *is* the page on screen.
  Nobody has built that notice yet, and the fix is a notice rather than a wider gate — withholding an op that names a node buys nothing and the stale `.md` is the pre-existing condition.
- **The refusal's words come from `outl_actions::refusal`**, so a stale row says the same thing in a terminal, a window and a phone.
  Two sentences, because the cost differs: a text edit lost your keystrokes, a chord typed nothing and only failed to land.
  Both GUI clients reached that catalog for the text edit and printed a raw block id for the other nine, until each was routed through the family above — the desktop first, the phone a day later.
- **A refused text edit keeps the editor open on what you typed.**
  Nothing was written, so that text exists nowhere else; closing the editor would be the one loss in this feature that cannot be undone.
  You leave by navigating, closing the panel, or editing another row, each of which gives you the chance to copy it first.
- **The list is re-asked per commit, never per keystroke.**
  `backlinks_for_page` is an `O(blocks-in-workspace)` scan (invariant 15), and the section keeps its rows across that refetch rather than blanking and coming back.

Two limits are deliberate, and written down rather than discovered:

- **Indent, outdent and move have no visible result on a GUI row.**
  The row is one flat block, its siblings and children are not drawn, and the effect lands on a page you are not looking at.
  Shipping them anyway is a decision taken against declaring them `Partial`, and it is **not** what the TUI's `Partial` records — that one is the missing way to the source page, and nothing else.
- **Creating a sibling does not put the cursor in it.**
  The new block cites nothing, so it is a backlink of no page and no row will ever render it, which leaves nothing on screen to aim an editor at — so you are told which page it landed on instead.
  **The two chord clients make one exception, and mobile does not**, because a page can cite itself and the new block is then in the outline in front of you.
  The TUI drops out of the panel and opens Insert on it; the desktop writes it to `editingBlockId`.
  Mobile applies the view and says nothing at all — there is no cursor to place and the keyboard deliberately stays down, since a `.focus()` outside the tap's gesture window raises it over nothing.
  That asymmetry is the right behaviour on each, not a gap: the notice would be a lie on a page you are looking straight at, which is why it is skipped on all three.

**Namespace-mention rows stay read-only.**
That list is capped and answers a different question, so it is a different surface.

## Backlinks order (issue #142)

Every client sorts the "Linked from" list the same way: `outl_actions::sort_backlinks` groups backlinks by source page (each page's blocks stay contiguous, in document order) and orders the pages by how recently each was referenced.
The direction — `newest` (default, most recently referenced page on top) or `oldest` — is a pure display preference stored in `[display] backlinks_order` (`config.toml`), same non-converging policy as `theme.preset`.

| Client | Toggle | Persistence |
|---|---|---|
| TUI | `Ctrl+O` (Normal mode) | writes `config.toml` directly via `outl_config::save` |
| Desktop | direction button in the `InlineBacklinks` header | `set_backlinks_order` Tauri command |
| Mobile | direction button in the `BacklinksSection` header | `set_backlinks_order` Tauri command |

See [config.md → `[display]`](config.md#display) for the schema and [shortcuts.md](shortcuts.md) for the TUI chord.

## Backlink breadcrumbs

When a citing block sits nested inside an outline (not at the page's root level), the "Linked from" panel shows the chain of parent blocks as a dimmed breadcrumb directly above the citing block, root-first — e.g. `Planejamento Q3 › Objetivos`.
A block at root level has no breadcrumb.
Consecutive references from the same branch collapse: the trail renders once, and the following reference(s) sit under it silently until the branch changes.

**The trail is rendered markdown, not source.**
A crumb carries inline tokens beside its text, so `#alinhamento [[buser/team/vendas]] internet 0800 [email](https://mail.google.com/...)` reads as a tag, a page name and a link label — the same way that block reads on its own page.
It shipped as raw text until then, which made the one line describing *where* a reference lives the one line written in markup, with a URL swallowing the rest of a trail that is truncated to a single line anyway.

*Dimmed* stays a styling rule: the trail is inert on every client (nothing in it opens a page — the row already has one destination, the citing block), and the dimming composes with the token colours underneath.
`@outl/shared/markdown::CrumbTrail` is the one renderer both GUI clients use, with `crumbTrailText` for the plain reading a `title=` tooltip can hold; the TUI's `render_crumb_trail` puts the dim on the ratatui `Line` so prose inherits it while a ref keeps its hue.

`outl_actions::backlinks_for_page` / `backlinks_for_target` populate `Backlink::ancestors` (a `Vec<BacklinkCrumb>`) while walking the page tree, so every client reads the same data — nothing is recomputed client-side, and no client holds a tokenizer to recompute it with.
`@outl/shared/outline::sameCrumbTrail` (mirrored locally by the TUI as `same_trail`) is the one comparison every client uses to decide whether to collapse consecutive breadcrumbs, comparing trails by block id.

Mobile's `BacklinksSection` groups references by source page and renders the breadcrumb the same way desktop and TUI do — it no longer shows a flat list of blocks.

## Nested pages (issue #275)

A page whose title carries a `/` — `os/linux`, `programming-language/rust` — is nested under the namespace to its left.
Two things follow from that, and they are separate features that happen to share a source:

1. **The parent lists its descendants.**
   Every client renders a "Nested pages" section below the backlinks, showing every page under the open one (all levels, not just direct children), title-sorted and indented by depth.
2. **The parent collects its descendants' mentions.**
   A block tagged `#os/linux` is a backlink of `os` as well as of `os/linux`, through the backlink index's `TargetKey::Namespace` channel.
   Exact-tag matching is unchanged: `#os` and `#os/linux` are still different tags.

**The hierarchy is derived from the page `title`, never from the slug**, and `outl_actions::namespace` is its single owner.
A slug is one filesystem path component (`pages/os-linux.md`), so `slugify` folds `/` to `-`; the `/` the user typed survives only in `title::`.
Comparison is per **slugified segment**, so `OS/Linux` and `os/linux` are one namespace and `oscar/wilde` is not under `os`.

`descendants` hands each client rows that already carry `depth` and `label`, so nothing splits a title on `/` client-side — the desktop and mobile share one `<NestedPages />` from `@outl/shared/namespace`.

| Client | Section | Open a row |
|---|---|---|
| TUI | `view/namespace.rs`, below the backlinks | **No** — `j`/`k` stop at the backlinks section; open one with the picker (`Ctrl+P`) |
| Desktop | `<NestedPages />` in `OutlineView` | Click |
| Mobile | `<NestedPages />` in `Journal` | Tap |

The TUI gap is recorded as `Capability::NestedPages` → `Partial` in `outl_shortcuts::capability_support` (see [client-parity.md](client-parity.md)), and the section header carries the nudge so the user is not left wondering whether the keys are broken.

The rows ride the `page_backlinks` reply (`PageBacklinks.namespace_children`) rather than a command of their own: they answer the same question at the same moment, off the page list that call already read.

## Backlinks index (performance)

> **One definition of a mention, one index, four clients:** [RFC 0169](rfcs/0169-backlinks.md).

`outl_actions::backlinks_for_page` walks the whole workspace (`O(blocks)`) on every call.
Recomputing that on each page open made opening a page — or today's journal — noticeably slower as a workspace grew.

Every client now keeps a **pre-computed inverted index** (`outl_actions::BacklinkIndex`) instead of scanning per open.
Once built, a page's backlinks are an `O(refs-of-the-page)` lookup.

The index is built from the **`.md` files on disk** (`outl_actions::build_backlink_index_from_disk`), not from the in-memory `Workspace`.
An earlier version built it from the workspace (`Workspace::block_text` on every block).
That forced a lazy-boot vault (#179) to materialize entirely and held the workspace lock across the whole `O(blocks)` walk — the "opening the journal / pressing Esc freezes" regression.
The `.md` projection already carries every block's text, properties, and sidecar id, so the from-disk build needs no lock and no Yrs materialization.

| Client | Where the index lives | Rebuild |
|---|---|---|
| TUI | `App::backlink_index` (`RefCell<Option<BacklinkIndex>>`) | Inline, on the next dirty read (`ensure_backlink_index`), from disk. The TUI owns its `Workspace` single-threaded, so it can't hand the build to a worker the way the GUIs do — the from-disk walk stays fast enough (disk I/O + parse, no lock, no materialization) to run inline. |
| Desktop / Mobile | `AppState::backlink_index` (`Arc<Mutex<Option<BacklinkIndex>>>`), exposed through `AppHost::backlink_index()` | Off the IPC thread, inside the `page_backlinks` command (`spawn_blocking`, `compute_backlinks_offloaded`): a brief workspace lock reads the page list + this page's meta, the index rebuild itself runs from disk with **no lock held**, then a brief lock does the `O(refs)` lookup. |

A regression test (`outl-actions/tests/backlinks_index_disk.rs`) asserts the from-disk build never materializes the workspace, via `Workspace::resident_text_count()` (the same counter #179's fix introduced, now `pub` for this exact check).

The index is a **dirty-flag cache, never a sync surface** — it is dropped (set to `None`) on every event that can change what a page's backlinks are, and rebuilt lazily on the next read:

- a local mutation (`App::invalidate_backlinks_cache` on the TUI, `finish_in_page*` on desktop/mobile),
- a peer or workspace reload,
- a plugin run that applied ops.

The index stores each referencing block as a **shallow leaf** — body, tokens, `ancestors`, no subtree.
Every client renders a backlink row from `source_block.tokens`, and materializing the subtree of every referencing block across the workspace (under the workspace lock) is exactly what froze input, so the build stops at the leaf.
One visible consequence: a backlink shows only the citing block, not its child blocks, on every client (the TUI no longer expands a backlink into a mini-outline).
