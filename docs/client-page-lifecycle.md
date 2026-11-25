# A page over its life: history, deletion, revival, rename

What every client does with a page that already exists: reading its history, getting a deleted block back, reopening a page that was deleted, and renaming or merging one.

**On the desktop, in short:** the clock icon in the page header is the history, with a **Restore** on edits and deletions.
The trash button in the top-right capsule lists every deleted block, and **Restore** puts one back.
Clicking the page title renames it, and **Merge into...** on a sidebar row merges it.
Nothing is destroyed by a delete, but a merge cannot be undone, and a deleted *page* cannot be restored from the trash: opening it by name gives back an empty page, and the blocks it held are in the trash, one entry each.
How to do each, step by step: [Using outl on the desktop](desktop.md#organise).

Moved out of [`docs/clients.md`](clients.md), which had grown to the point where no edit to it was cheap.
That page keeps the cross-client architecture: the stack, who owns which fact, the locks an open takes, and the two conditions every client has to surface.

## Page history

The op log holds every revision of every block. `outl_actions::timeline` is the single owner of turning that into "what happened to this page" — which blocks count as the page's, which ops are not events, and the order. No client re-derives any of it.

**A restore is new ops, never a rewind**, and `@outl/shared/trust`'s `restorePlan` is the single owner of which rows carry one.
Two do: an edit puts the block's previous text back, a deletion brings the lost text back as a new block at the page end.
A move, a property change, or an edit on a since-deleted block gets no button — "the version before" is a position or a value whose meaning depends on its neighbours, and the deletion row offers that text back instead.
`outl recover` is the narrower CLI tool, and writes back a revision only when the recovered text *contains* the current text as a prefix.

Two rules that decide what a reader sees:

- **A deleted block stays in its page's history**, with the text it held when it went. A history that omits deletions answers "what changed" with everything except the change people open a history to find. A block moved to a *different* page goes with it; `block_timeline` follows one block wherever it has lived.
- **Not every op is an event.** Folds, snoozes, `page-slug` / `page-kind` writes, an `Op::Edit` that re-emitted the block's existing text, and a re-emitted `Create` / `Move` that changed nothing are all skipped. A reconcile produces them in volume — on a real 64k-block workspace one block's history was six such rows around a single real edit.

| Client | Surface |
|--------|---------|
| CLI | `outl page history <slug>` and `outl block history <id>`, both with `--limit` and `--json`. See [`docs/cli.md`](cli.md#outl-page-history--outl-block-history). |
| Desktop | The `⏱` button in the page-header eyebrow opens `<TimelinePanel />`; `Esc` closes. No chord yet. |
| Mobile | `<HistorySheet />`, reached from the page properties sheet (long-press the page title) — the same `page_timeline` read, with restores. |
| TUI | Not yet. |

Related but different: **undo / redo** (`outl_actions::history`) is this session's mutations, and `outl backup` is workspace-granular git snapshots. Neither answers "what did this page say last Tuesday" — that is this.

## Restoring a deleted block

Delete is `Move(node, TRASH_ROOT)` ([invariant 6](../CLAUDE.md#critical-invariants-never-violate)), so nothing is ever physically removed. `outl_actions::trash` is the single owner of reading that back — what is in the trash, where each entry came from, and whether putting it back would work.

**Deliberately narrower than it looks.** Three things it does not do, each for its own reason:

- **A page cannot be restored.** It needs a re-projected `.md` on top of the `Move`, and on a real 2,576-page workspace 16 of the 18 deleted pages have their slug taken by a live page today. Picking a free slug would make this a second owner of the slug rule, so it refuses and names where the content still is (`outl block tree <id>`, since `outl page history` only resolves live slugs).
- **A block whose parent is also in the trash cannot be restored on its own** — 89 of 393 top-level deletions on that workspace. Restoring it would succeed structurally and change nothing the user can see; the refusal names the ancestor to restore first.
- **A restored block lands as the last child**, not in the slot it held. `Move.old_position` is the originating replica's local derivation for `undo_op`, not something a reader of the log may trust.

**The listing and the mutation share one verdict function** (`trash::refusal_for`). A listing that decided "restorable" for itself would drift towards offering an action that then fails — the same rule invariant 8 applies to which pages are safe to overwrite.

**A refusal is a snapshot, not a promise.** The verdict reads the live tree, so a peer can trash the landing parent between a listing and a click, and the mutation declines a row the list showed as restorable. Every surface therefore handles the refusal *as well as* filtering on it, and the two sentences are the same one — both are `ActionError::to_string()` off `trash::refusal_for`. A GUI puts it on the row it belongs to and re-reads the listing; nothing re-words it.

| Client | Surface |
|--------|---------|
| CLI | `outl trash list` and `outl trash restore <id>`. See [`docs/cli.md`](cli.md#trash). |
| MCP | `outl_trash_list`, `outl_trash_restore` — an agent that can delete a block can undo it. |
| Desktop | `TrashPanel`, from the trash button in the top-right capsule. See [`desktop-ux.md`](desktop-ux.md#trash-panel). |
| TUI | `/trash` (and `:trash`), `r` to restore, `Enter` to open the page a deleted block came from. See [`tui.md`](tui-overlays.md#trash-trash-trash). |
| Mobile | Not yet — the commands are registered, no sheet calls them. |

Both GUI commands are `list_trash` / `restore_trash_entry` (`outl_tauri_shared::commands::trash`), which neither client may second-guess: `refusal` rides each row, and `restore_trash_entry` re-asks under its own lock.

Neither surface has a chord. A chord needs an `Action` in `outl-shortcuts`, whose `support()` match is at its frozen size baseline, so it waits on that split — the per-client verdicts it would force are the mechanism, not the cost. The desktop's affordance is a button and the TUI's is its command registry, which is what `Capability::Trash` records (it was the one row in `CLI_ONLY`, now empty). [Issue #287](https://github.com/outlmd/outl/issues/287).

Related but different: **undo / redo** is this session's mutations and does not survive a restart; this reaches a deletion from any device, on any day, because it reads the op log.

## Reopening a deleted page

A page root's id is derived from its slug, and delete keeps the node, so a deleted page's slug still resolves to that same node sitting in the trash.
Opening it again therefore has to *move the root back*, not create it: `Op::Create` is idempotent, so it found the node and left it exactly where it was — in the trash.
The page opened looking normal and every keystroke under it was dropped with no error, every morning, on a journal day the user had deleted ([issue #473](https://github.com/outlmd/outl/issues/473)).

`outl_actions::page::open_or_create` revives it, so every client gets this without asking for it.
Two things it deliberately does **not** do:

- **It does not bring the deleted blocks back.**
  They went to the trash with the page in one `Op::Move`, so reviving the root alone would undo the user's delete behind their back.
  Each one is trashed in its own op instead, which is also what makes it an ordinary [trash](#restoring-a-deleted-block) entry: `outl trash restore <id>` puts it back under the revived page, with its subtree.
  The one exception is a child that is itself a page root, from a page nested under another: `trash restore` refuses a page root, and reopening that page by its own slug is what brings it back.
  Either way the content is hidden, never destroyed — the asymmetry [invariant 8](../CLAUDE.md#critical-invariants-never-violate) argues from.
- **It does not append an empty block.**
  A revived page is empty, and a client's own "this page is blank" affordance is what fills it.

The page's `.md` is the caller's business as usual.
A revive emits `Op::Move`s only, so a `.md` holding lines the op log never saw still refuses to be overwritten — see [Surfacing a page that stopped syncing](clients.md#surfacing-a-page-that-stopped-syncing).
A `.md` still showing blocks the log holds *in the trash* is re-projected without them, which is the delete reaching the file.

## Renaming and merging pages

`outl_actions::rename_page` and `outl_actions::merge_pages` own both operations, including the rewrite of every `[[old]]` and `#old` mention ([RFC 0557](rfcs/0557-page-merge-and-rename.md)).
The desktop and mobile apps reach them through the `rename_page` and `merge_pages` commands, and the caller projects every page in `report.touched_pages` and removes the old projection last, as `outl page rename` does.
`@outl/shared/pages/rename-merge` owns the flow (rename, confirm a merge when the name is taken, merge) and the wording of the confirmations and summaries, so the two clients do not each invent it.
Which clients can do it is recorded as `Capability::PageRename` and `Capability::PageMerge`, and both are full on all three.
Neither has a chord, so there is no `Action`: every door below is a click, a tap or a typed command.

### Where a rename starts

**The page's own title is the door on both GUI clients**, because that is where a reader is already looking when the name is wrong.

| Client | The door | The name field |
|---|---|---|
| Desktop | Click the page title | Edits in place, prefilled and selected. Enter commits, Escape restores, clicking away commits |
| Desktop | The sidebar row's hover `✎` | `PageActionDialog`, which also owns the merge-target picker behind `⇥` |
| Mobile | Tap the page title (a pencil glyph marks it) | `PageNameSheet`, a bottom sheet |
| Mobile | Long-press a page row in the page switcher | The same sheet, from the 3-row action menu |
| TUI | `/rename <new name>` | The `:` line, prefilled with the command name only |

A **journal is offered none of them**.
Its slug is its date, `outl_actions::rename_page` refuses it, and reaching a refusal through an affordance is the defect rather than the guard: on the desktop the title is simply not clickable, on mobile the title tap still means "back to today", and the TUI refuses the argument before it writes.

**Mobile's field is a sheet rather than an inline input on purpose.**
A focused `<input>` living in the page chrome makes WKWebView scroll the document to keep the caret above the keyboard, and what it scrolls away is the header the field is in.
A `position: fixed` sheet sits outside the document flow and cannot do that.

**A no-op is not a write.**
Submitting the name a page already has runs no command on any client.

### What the user is told afterwards

Both reports carry counts, never sentences, and `outl_actions::page_summary` owns **what those counts are called** so four surfaces name one fact one way.
`rewritten_blocks` is counted in **blocks** ("links updated in 3 blocks"), because one block can carry several mentions of the page and "3 links updated" is a different, larger number.
A mention deliberately left alone is a **nested-page link** (`[[old/child]]` names a page that is not being renamed), the word [the glossary](../UX.md) uses.
`@outl/shared/pages/rename-merge` keeps a declared mirror of those nouns, pinned from Rust by `the_shared_frontend_copy_counts_the_same_things`.

### Reopening the old name undoes the rename

A page id is derived from the slug it was *created* under and a rename does not change it ([RFC 0557](rfcs/0557-page-merge-and-rename.md) known limit 5).
So anything that later opens the **old** slug — a stale `[[link]]`, a Recent row, a script — makes `open_or_create` write that slug back onto the renamed page, which flips the name back and leaves every rewritten mention pointing at nothing.
A client that remembers a page by file name has to repoint or forget what it remembers; the TUI does both for `recent_paths` and its persisted search hits, and only onto a file that is really on disk.
