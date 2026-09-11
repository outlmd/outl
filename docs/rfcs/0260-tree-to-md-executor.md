# RFC 0260 — an executor for the `tree → .md` direction

**Status:** implemented
**Related:** root `CLAUDE.md` invariants 8, 10, 11; [RFC 0210](0210-md-content-outside-op-log.md); [issue #210](https://github.com/outlmd/outl/issues/210)

## The measurement

`outl doctor` on a real 2,574-page daily-driver workspace reports 745 warnings.
704 of them are one line repeated:

```
pages/<slug>.md: `.md` is a stale projection — the op log renders different content
 (re-projecting removes 0 content line(s) from disk)
```

**702 of those 704 remove zero content lines** — the guard says they are entirely safe to re-project.
Two would remove three lines between them.
Four more pages are in the op log with no `.md` on disk at all.

27% of a real workspace's pages had been silently out of date on disk for months, and the fix already existed.

## What was and was not missing

`outl doctor --repair` already re-projects stale pages from the op log, backs every written file into `.outl/repair-backup/<timestamp>/`, enforces the invariant-8 guard and applies volume ceilings.
None of that was missing.

What was missing is that **nothing ever ran it.**

The `.md → tree` direction has a permanent executor: the `outl serve` file watcher, plus every GUI client on page open.
The `tree → .md` direction had none.
When ops arrive by sync, or a render-affecting fix lands in the renderer, the `.md` on disk simply stays wrong until a human happens to open that page — or happens to run a maintenance command they have no reason to run.

A direction of reconciliation with no executor is not a bug in any one function.
It is a gap between two of them, which is why no test caught it and why the report that measures it is the same report nobody runs.

## Where the executor lives, and why there

**`outl serve`, in the passes it already has.**

The obvious alternative is a new subcommand.
Invariant 11 says attribute the cost before letting it decide, so:

1. **Would the alternative pay it too?**
   The expensive part is running something permanently, and a second daemon runs permanently just as hard.
   A new subcommand buys a second `launchd` job and a second thing that can die quietly, for a pass `serve` is already awake for.
2. **Does the cost track the feature or the lifetime?**
   The write-actor cost `serve` carries comes from being a daemon, not from what it does while it is one.
   Adding a projection sweep to an already-running loop costs nothing new on that axis.
3. **What is the smallest thing that works?**
   No flag, no subcommand, no new entry point.
   `serve --once` already exists as a batch reconcile pass and becomes the batch projection pass for free; the watcher loop already wakes on peer ops and becomes the continuous one.

So the wiring is: `run()` sweeps after the initial `.md → tree` scan, and again whenever `reload_if_peer_ops` reports that the tree moved.

**`serve --no-watch` deliberately does not sweep.**
That mode exists to hold the iroh endpoint beside a GUI you are also using, takes no per-actor write lock, and its documented contract is "hold the endpoint and nothing else".
Giving that name a second job is invariant 10's fifth question answered wrong.
A GUI already projects the pages it opens; a headless box runs plain `outl serve` and gets the sweep.

## The default, and the argument for it

**Automatic, unprompted — but only for writes that provably remove nothing.**

Writing 702 files on someone's next boot is a big action, and the honest way to make it small is not to ask permission: it is to narrow what the pass is allowed to do until permission stops being the question.

The executor writes a page only when re-projecting it removes **zero** content lines from disk.
On the measured workspace that is 702 of the 704 stale pages, plus the 4 with no `.md` at all.
A page whose re-projection would remove content — a peer genuinely deleted blocks — is **withheld** and left to `outl doctor --repair`.
That command copies every file into `.outl/repair-backup/` first, applies the 100-line / 20-page ceilings, and lists what it will do before it does it.

That split is what makes the background pass need no backup of its own.
A write that removes nothing has nothing to undo: every content line on disk is reproduced by the new render, by construction, measured with the same `content_lines_missing_from` the doctor uses.

It also makes a **torn op log** safe without the sweep knowing anything about op-log health.
A truncated replay renders *less* than the disk holds, so every page it touches reports `lines_removed > 0` and is withheld.
Invariant 8's precedence order — a damaged log is reported as a damaged log, never as thousands of pages of "unlogged content" — stays entirely with `outl doctor`, which is the surface that can actually diagnose it.
The one case the gate does not catch is a log that lost only reordering ops: the same lines in a different order re-project, and the next sync corrects them.
That is a reorder, not a deletion, and it is the recoverable direction.

**What the user sees.**
An all-quiet pass says nothing.
A pass that wrote logs one summary line with the count.
Every refusal and every withheld page is **named individually**, capped at 20 each, with the command that fixes it.

## Who was standing on "nothing re-projects in the background"

Invariant 10's question, answered per consumer:

| Consumer | What changes |
|---|---|
| GUI page open (`reproject_stale_md`) | Finds fewer stale pages. The ahead-of-log banner is unaffected: a page holding unlogged content is still refused by `apply_page_md_with_sidecar_if_stale` on open, because the sweep never wrote it. |
| `outl doctor` / `--repair` | Finds fewer stale pages, and the ones it does find are the content-removing ones — exactly the set that needs its backup and its ceilings. |
| `outl serve`'s own file watcher | Sees its own writes. Cannot turn one into an op: see the race analysis below. |
| The TUI / an editor with the file open | A *saved* external edit is protected by the hash gate (the page classifies as `PendingExternalEdit` and is skipped). An unsaved buffer was never at risk from anything on disk. |
| `outl reconcile --ahead-of-log` | Unchanged. It is still the only route for content the log never saw, and the sweep now names the pages that need it. |

## Race analysis

**The write that could hurt** is one where a `tree → .md` projection is read back by the `.md → tree` direction as an external edit, because that turns a stale render into `Op::Delete`s for real content.

It cannot happen, and the reason is structural rather than timing-based.
Every write goes through `write_page_projection_unlocked`, which replaces the `.md` **and** rebuilds its sidecar from the same tree snapshot.
`needs_reconcile` compares the sidecar's `last_synced_hash` against the file, and after a projection they agree by construction — so the watcher event the write generates reconciles to zero ops.
A projection this pass writes is never readable as an edit.

**The write that could flap** is two outl processes projecting one page from different tree snapshots — a GUI that just committed an op the daemon has not replayed yet.
Three things bound it:

- `ProjectionLock` is an `flock` on a stable sibling of the `.md`, held across the check *and* the rename, so two outl processes serialise.
- `write_page_projection_if_unchanged` re-reads the file after rendering and refuses if the bytes moved, so a projection authorised against one revision never lands on another.
- The sweep runs immediately after `reload_workspace()`, so its snapshot is as fresh as the op log on disk.

The residual case is a daemon whose snapshot predates ops a peer has not shipped yet.
It writes an older render, the op log still holds the truth, and the next sweep corrects the file.
No content leaves the op log, which is the only loss that is not recoverable.

**Throttle.**
The sweep renders every page to compare it with disk, and peer ops arrive in bursts.
A 30-second floor coalesces a catch-up sync into one sweep instead of one per batch.
A projection lagging by half a minute is invisible: every client reads the op log, not the `.md`.

## What was extracted, and who owns the verdict now

`outl doctor`'s page classification was inline and private in `doctor/tree.rs`.
It is now `outl_actions::journal::survey_page_projections` — one exhaustive `PageProjectionState`, consumed by both the doctor's read-only listing and the executor.
`doctor/tree.rs` went from 348 lines to 293; `doctor/repair.rs` is untouched.

**The verdict itself did not move.**
`content_lines_missing_from` is still the single owner of "does the op log know this line", still asked of the *sidecar's* blocks and never of a render.
The survey is a **selector**; `apply_page_md_with_sidecar_if_stale` is still the authority, and the executor hands every candidate back to it rather than trusting its own classification.
That is deliberate: it re-reads under the page lock, so a page that changed between the two reads is refused rather than raced, and there is exactly one place that decides whether a `.md` may be overwritten.

The extraction closed one real gap it was not aimed at.
A sidecar written before 0.11 carries `text: ""` and cannot answer the question at all.
`apply_page_md_with_sidecar_if_stale` declines those; the doctor's old inline code classified them as ordinary stale pages and offered a repair, which the pass then silently skipped with `ok: false`.
`PageProjectionState::SidecarCannotAnswer` is its own state now, reported as a warning rather than as repairable work — a listing that no longer promises something the writing pass refuses.

## What this deliberately does not do

- **No new CLI surface.**
  No flag to turn the sweep off.
  If that turns out to be wanted, `--no-project` on `serve` is the shape, and the sweep is already a single call site behind a boolean.
- **No re-projection from the GUI clients in bulk.**
  They project the page they open, which is the right granularity for a foreground process.
- **No backup for the background pass.**
  It writes nothing that could need one.
  The moment that stops being true, the pass has become `doctor --repair` and should be it.
- **No op-log health check inside the sweep.**
  The lossless gate covers the damage a torn log can do here, and duplicating `OpLogHealth` outside the doctor would be a second opinion about a thing that already has an owner.

## Measured on the real workspace

A byte copy of the 2,574-page daily driver, `outl serve --once`, release build.

| | before | after |
|---|---|---|
| `outl doctor` warnings | 745 | **9** |
| Stale pages | 705 | 1 |
| Pages in the op log with no `.md` | 6 | 0 |
| Pages the pass refused (ahead of log) | — | 0 |
| Pages withheld (would remove content) | — | 1, named |

The sweep re-projected **710 pages** in 30s over 2,860 surveyed, withheld `journals/2026-07-23.md` (3 content lines) for `outl doctor --repair`, and refused nothing.
Passes 2, 3 and 4 wrote **zero** pages: the executor is a fixed point.

The 9 warnings that remain are 5 unmaterialized node ids (an unrelated defect) plus the one withheld page and its two volume lines.

### A determinism bug the measurement exposed

The first run was **not** a fixed point: 1–3 journals were rewritten on every pass, forever.

`outl_actions::tree::children_of` sorted siblings by `Fractional` alone.
That is not a total order once two devices append while offline and produce the same position.
`Tree::iter_nodes` walks a `HashMap` whose iteration order is seeded per process, so a stable sort over a partial comparator rendered the same op log in a different block order on every boot.

This is a **convergence** bug before it is a churn bug: two devices replaying one op log disagreed about sibling order, which is the thing the op log exists to prevent (invariant 7).
It was invisible because nothing re-rendered pages often enough to notice.
Fixed by the same tiebreak the HLC already uses — `a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0))` — in `children_of` and in `build_children_index`, pinned by `siblings_sharing_a_position_are_ordered_by_node_id`.

### What the lossless gate does not cover

`content_lines_missing_from` deliberately excludes `key:: value` lines: a property is not block text, it lives in the tree's property map, and counting it would flag every page that has one.
So "removes 0 content line(s)" is a promise about **block content**, not about property lines — and this is a pre-existing property of `doctor --repair`'s gate, unchanged here.

Measured consequence on the real workspace: **zero** block content lines lost across 2,848 files, and **9 property values** in 9 files changed.
Every one is a duplicate-key collision in a `.md` that had accumulated the same property block two or three times (`gustavo-verdeli.md` carried `icon:: 👤` 22 times), where the two copies disagreed and the op log's map holds one value per key.
The re-projection collapsed the file to what the log already said.

That is the right outcome under invariant 1, and it is worth naming because making the pass automatic is what makes it happen without anyone asking.
Whether the gate should also protect property lines is a separate question about `content_lines_missing_from`, which has one owner and a documented reason for the current rule.
It deserves its own issue, not a change folded into this one (invariant 11, question 4).

## The mirror direction, stated before merging

Root `CLAUDE.md` invariant 8: when you fix one direction of a `.md` ↔ tree divergence, say what happens in the opposite direction.

This change makes `tree → .md` run without being asked.
The opposite direction, `.md → tree`, is **untouched**: the sweep never emits an op, never advances a `last_synced_hash` over content it did not log, and never visits a page whose `.md` disagrees with its sidecar.
What it makes *worse* is nothing on disk and one thing in the logs: a daemon that previously said nothing about frozen pages now names them on every sweep, up to twenty at a time.
That is the intended direction of the trade — invariant 8's "a refusal has to reach the user" is the reason the names are there — but a workspace with hundreds of ahead-of-log pages will be noisy until `outl reconcile --ahead-of-log` runs.
