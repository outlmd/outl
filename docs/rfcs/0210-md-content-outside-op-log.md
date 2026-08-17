# RFC 0210 — A sidecar hash match is not evidence the `.md` came from the op log

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#210](https://github.com/outlmd/outl/issues/210), regression from [#166](https://github.com/outlmd/outl/issues/166) |
| **PR** | — |
| **Date** | 2026-08-06; producer, recovery and volume guards 2026-08-07; detector false positive, post-mutation guard and fleet analysis 2026-08-08 |
| **Reference doc** | [storage.md](../storage.md), [doctor.md](../doctor.md) |
| **Invariant** | root `CLAUDE.md` invariant 8; `outl-md/CLAUDE.md` invariant 8 |
| **Guarded by** | **Producer** — `no_non_blank_line_is_ever_lost_between_parse_and_render`, `render_then_parse_is_a_fixpoint_not_just_lossless`, `a_trailing_blank_line_does_not_end_up_in_the_blocks_text`, `a_four_space_indented_bullet_is_still_a_block`, `the_hash_is_still_advanced_for_every_shape_a_real_workspace_holds` (`crates/outl-md/tests/multiline_block_roundtrip.rs`). **Consumers** — `if_stale_refuses_when_the_md_carries_content_the_log_lacks`, `if_stale_still_reprojects_when_the_md_holds_no_unlogged_content`, `if_stale_ignores_whitespace_only_differences_when_deciding`, `if_stale_declines_when_the_sidecar_cannot_answer`, `if_stale_still_projects_a_page_whose_sidecar_has_no_blocks` (`crates/outl-actions/src/journal/tests.rs`), `recovery_does_not_reproject_over_text_the_log_never_saw` (`crates/outl-actions/tests/desync_recovery.rs`). **Volume** — `a_repair_that_would_delete_a_lot_of_content_stops_and_asks`, `the_same_repair_runs_once_it_is_explicitly_forced`, `an_ordinary_amount_of_deletion_repairs_without_a_flag`, `the_volume_is_announced_by_a_read_only_run_too`, `a_torn_op_log_never_lets_repair_overwrite_a_good_md` (`crates/outl-cli/src/cmd/doctor/tests/safety.rs`), `the_escape_hatch_applies_a_deletion_the_guard_refused` (`crates/outl-cli/src/cmd/reconcile.rs`) |

## Why

On a real workspace — 2,560 pages, 213,859 ops, in daily use since June — **233 pages held 1,426 lines of content that existed in no op**.

> **Which detector produced that number matters, and this document later proves it wrong twice.**
> The 233 / 1,426 above came from the render-based comparison, in the round that motivated this work.
> The sidecar-based one that replaced it measured **41 pages / 387 lines** over the same graph.
> Both are recorded because both were acted on; neither should be quoted without the method attached.
> Final residue after everything below: **0 / 0**.

The content was not scratch: infrastructure learnings with run ids, operational briefings, root-cause notes spanning 2022 to 2026.
It read correctly on disk, in the editor, and in any `grep`.
It had simply never been recorded as an `Op`.

Three consequences, in increasing severity:

- **It never reached another device** — peers exchange ops, not files, so all 1,426 lines existed on exactly one machine, with no second copy for a user who trusts the sync story.
- **Nothing surfaced it** — not `doctor`, not any consistency check, not one log line, because `sidecar.last_synced_hash` agreed with the bytes on disk, and that agreement is what every downstream check tests.
- **Re-projection deleted it and reported success** — `outl doctor --repair` printed `708 fixed` while removing content from 233 pages, then rebuilt each sidecar from the same render, so afterwards nothing could tell the page had ever held more.

The deletion was not limited to `--repair`.
`apply_page_md_with_sidecar_if_stale` runs on every GUI open path (`open_page_by_slug`, `open_journal_for`, `open_today_journal`, `open_ref`), so opening the page in the desktop or mobile app was enough.

The root cause is a conflated question.
`sidecar.last_synced_hash == file_hash(disk)` answers *did outl write these bytes last?*
It was read as answering *did these bytes come from the op log?*
Every page in the state above answers yes to the first and no to the second.

## What we chose

`outl_actions::content_lines_missing_from(disk, sidecar_blocks) -> Vec<String>` is the single owner of the verdict "would re-projecting delete something?".

**The reference is the sidecar's blocks, and the first version of this got it wrong.**
It compared against a fresh render of the tree, which answers a different question: *do disk and tree disagree?*
Every remote edit answers yes to that, since the pre-edit line is on disk and absent from the render.
So does every remote delete and every reorder.
The guard therefore refused to re-project any page a peer had touched, and the page froze showing pre-edit text with nothing surfaced — issue #166 reintroduced for the most ordinary sync case there is.
The recovery command the error message recommended made it worse.
Reconciling such a page wrote the pre-edit text back as ops, reverting the peer permanently, since the log is append-only.
Caught in code review by an executable probe, after 1,687 tests and a green `/check` had not.
The sidecar's blocks are what the log held at the last agreement, so they answer the question actually being asked.
`apply_page_md_with_sidecar_if_stale` calls it after the hash gate passes and returns `ActionError::PageMarkdownAheadOfLog { path, lines, sample }` instead of writing.

Two details that carry weight:

**It compares a multiset of trimmed non-blank lines, not a diff.**
A line the renderer merely *moved* is not at risk, and an LCS diff reports it as unique to disk.
On the measured workspace that is the difference between flagging 616 pages and the 233 that genuinely hold unlogged content.

**Whitespace-only drift is ignored on purpose.**
The renderer's trailing-newline behaviour changed between releases, so a large share of "stale" pages differ from the log by exactly that.
Treating it as content would strand every genuine re-projection behind noise, and a guard that fires constantly gets disabled.

`outl doctor` calls the same function in its read-only listing, so the listing can no longer offer a repair the `--repair` pass then refuses — the "announced before they run" invariant in `outl-cli/CLAUDE.md`.

## Why not the alternatives

**Return `Ok(None)` and skip the write silently.**
The cheapest change, and it fails the test this project cares most about: the user learns nothing.
A page quietly not re-projecting looks identical to a page that needed nothing, so the 1,426 lines stay unsynced and undiscovered.
Silence is the defect, not the write.

**Compare block ids from the sidecar against the tree instead of text.**
Structurally cleaner, and blind to the actual case: the sidecar in this state has *already* been rewritten to describe the file, so its ids and the tree agree while the text does not.
The evidence lives in the content, which is why the check reads content.

**Make `reconcile_md` correct and trust the hash again.**
Right, and not sufficient.
It fixes the producer for the future while leaving every workspace already in this state to be emptied by the next page open.
It also leaves the false inference — "hash matches, therefore the log holds it" — in place for the next caller to repeat.
The producer fix is still needed — see Scope.

**Refuse on any hash-faithful page whose render differs.**
This is the pre-#166 behaviour, and it re-breaks #166: a peer's ops land, the `.md` is never refreshed, the page renders empty.
The point of the multiset check is to separate those two cases rather than choosing which one to lose.

## The opposite direction

**What this makes worse:** a page holding unlogged content now refuses to re-project, so a genuine tree-ahead update to that same page does not reach the `.md` either.
That is deliberate — a stale view is recoverable, deleted content is not — but the user is stuck until `outl reconcile` runs, and they only learn this from `doctor` or an error surfaced by the client.
`doctor` names the count and one sample line for exactly that reason.

**The mirrored case, stated explicitly:** this RFC fixes "`.md` ran ahead of the tree" (content deleted).
The mirror is "tree ran ahead of the `.md`" (content hidden), which is #166, and it stays fixed: `if_stale_still_reprojects_when_the_md_holds_no_unlogged_content` pins it.
Both directions are now pinned by a test, and neither can be reintroduced by simplifying the other away.

**One precedence trap found while implementing this.**
A torn op log replays a truncated tree, which makes *every* page look like it holds unlogged content.
The first version of this guard therefore hijacked the report on a damaged log, and the message telling the user how to recover the log disappeared — caught by the existing `a_torn_op_log_never_lets_repair_overwrite_a_good_md`.
The check now stands down when `OpLogHealth::is_compromised()`, so a damaged log is reported as a damaged log.
That test was written for a different defect and caught this one; it is the strongest argument in this RFC for naming tests as a required section.

## What actually found these

Worth stating plainly, because it decided the outcome four separate times.

Across this issue, **every** defect that reached the user — the original one and each of the four regressions introduced while fixing it — was found by running code over a real workspace, and **none** by the unit suite, which was green (237 tests in `outl-md` alone) at every one of those moments.

| found by | defects |
|---|---|
| probe over 2,827 real `.md` files | the producer; the residue that turned out to be the detector's false positive |
| adversarial review, then confirmed by probe | the fixpoint break, the trailing `\n`, the marker-in-text growth |
| unit tests | none of them |

The split matters, and an earlier version of this section blurred it by crediting the probe with all five.
Review found three of them by *reading* the diff; the probe is what settled each one in minutes and turned it into a failing case.
Neither is sufficient: review without a corpus argues, and a corpus without review only checks what someone already suspected.

What both share is that the unit suite — 237 tests in this crate alone, green at every one of those moments — reported nothing. That is not an argument against unit tests; they pin the shapes once known. It is an argument that the shapes are not knowable in advance, and that "green suite" is not evidence about a workspace nobody ran it against.

So the *properties* stop being a throwaway — the probes themselves stayed disposable, and it is worth being precise about which part survived.
Each one was written, run, and deleted; what is committed is the corpus and the three assertions over it.
`crates/outl-md/tests/corpus_gate.rs` runs three properties over `tests/corpus/`, a set of files reduced from the real shapes — odd indentation, tabs, Roam leftovers, bullets inside fences, unicode separators pasted from PDFs:

1. **no line is lost** across `parse → render`;
2. **`render → parse` is a fixpoint** — not merely lossless, because a document that changes shape on every save is worse than the bug it replaced, which at least converged;
3. **the unlogged-content check does not cry wolf** — a page the log fully accounts for must not be reported, since that verdict freezes the page.

Property 3 is the one that catches the defect this RFC closed last, and it did not exist until that defect had already shipped.

The maintenance rule is one line: **when a `.md` bug is found in the wild, its shape becomes a file in `tests/corpus/`.**

## What this costs a mixed-version fleet

Withholding `last_synced_hash` is a message to *this* binary. A peer running an older one reads the same field and draws the opposite conclusion, and that was measured rather than reasoned about: a worktree at `275c322` (v0.11.0-beta.151, the build users actually have) run against the same files.

Two facts, both executable:

1. **The shipped parser is lossy** in every one of the six shapes this RFC closed. `- head\n\n  body\n` renders back as `- head\n`.
2. **An empty hash invites it in.** The shipped `reconcile_md`'s short-circuit misses, it reconciles with that parser, and emits an `Op::Edit` truncating a block the log held correctly:

```text
before (withheld):  tree = ["head\ndetail\n  deeper detail", "second"]
after  (withheld):  ops=3  hash=real  pv=2  tree = ["head\ndetail", "second"]
after  (real hash): ops=0                   tree unchanged
```

So this PR introduces a real harm on a mixed fleet. It is worth stating plainly rather than filing under "future work".

**And there is no value of the field that avoids it.** The shipped binary has two complementary gates: `reconcile_md` runs when the hash does *not* match, and its `apply_page_md_with_sidecar_if_stale` — which in beta.151 has no `content_lines_missing_from`, only the hash — re-projects when it *does*. No single value disarms both. A real hash is strictly worse: it authorises the old peer to render the tree over the `.md` and delete the unlogged content outright, which is #210 with no guard at all. A distinct sentinel is equivalent, since the comparison is equality. A `SIDECAR_VERSION` bump is off the table for the reason that section already gives.

**Two things bound the damage.** No write path in the old binary preserves a `pipeline_version` it does not understand, so the page stays queued and this binary heals it from the `.md` — that is what makes the harm bounded rather than permanent, and it is pinned by `a_shipped_reconcile_cannot_leave_the_page_looking_healthy_to_this_binary`. And the truncation arrives as an `Op::Edit`, so `outl recover` can read the revision before it.

**Calibration.** Running `reconcile_md` over a copy of all 2,827 real `.md` files: **0 pages trigger the withholding today**. The state is unreachable with the current parser. This is a guard for the *next* parser gap — which is precisely when a fleet is most mixed, and precisely when nobody will remember this trade-off. That is why it is written down here instead of in a commit message.

## How it cannot regress

1. **Invariants.**
   Root `CLAUDE.md` invariant 8 states the rule for consumers (never overwrite a `.md` on the hash gate alone) and carries the *why*, so it cannot be argued away as paranoia.
   `outl-md/CLAUDE.md` invariant 8 states it for the producer: `last_synced_hash` may only advance over content the same call emitted ops for.
   Three anti-patterns in the root `CLAUDE.md` name the specific mistakes.

2. **Tests.**
   Those in **Guarded by** above, in three groups that fail for three different reasons.
   The **producer** group pins `render → parse` as a roundtrip, so the state cannot be created again.
   The **consumer** group covers refusal, the still-must-reproject case, whitespace tolerance, and the two ways a sidecar answers nothing — pre-0.11 entries carrying no text, and a page with no blocks.
   The **volume** group pins the ceilings, the escape hatch, and the precedence order against a damaged log.
   The root `CLAUDE.md` says outright that the consumer group exists to fail if the gate is re-simplified back to a hash comparison, and must not be relaxed.

## Scope

**Covered — the producer, and it was not where this RFC guessed.**

The suspect named here was `reconcile_md` rewriting the sidecar to agree with a file it did not fully emit ops for.
That is the *mechanism*, and it is real, but it is not the cause: `reconcile_md` was rewriting the sidecar to agree with what the **parser handed it**, and the parser was the one dropping content.

`render → parse` was not a roundtrip.
An executable probe is what settled it, in the shape this project keeps relearning is the only one that settles anything:

```text
input:    a block whose text carries a blank line and its own indentation
render:   correct — every line emitted, continuation at indent + 1
parse:    one block, first line only
warnings: 0
```

Six lines gone, and the `warnings: 0` is the whole story — the parser's contract is that nothing is dropped in silence, and this arm broke it with no record for the user, for `doctor`, or for the log.
The next `reconcile_md` then wrote that truncation into the op log as an `Op::Edit`, so the loss reached the one place this RFC had described as still holding the content.
Verified on the real workspace: block `01KX0SSS…` on `journals/2026-07-08.md` carries a `Create`, an `Edit` with the full 34-line briefing, and a later `Edit` from a different actor holding one line.

Three defects in `parse_block_list`, each of which alone produces the state:

- **An over-indented line was recovered only at `indent == 0`** and skipped in silence at every deeper level.
  The comment justifying it said "the same recovery upstream caught the parent line, so nothing leaks".
  It leaks; that arm is where most of the measured lines went.
- **A blank line inside a block's text was read as a separator.**
  The renderer writes each line of `text` after the first at `indent + 1`, so an empty line *within* the text comes back as indented whitespace, while a separator between siblings is genuinely empty.
  The indent distinguishes them; reading both as "separator" closed continuation and discarded everything after it.
- **A continuation line's own indentation pushed it out of reach.**
  `"head\n  detail"` renders with the renderer's indent *plus* the text's own, comes back over-indented, and was handed to a child list that could not place it either.
  `strip_indent_levels` removes exactly what the renderer added, so the block's internal indentation survives.

The fix for each is "preserve, and say so": recover at every depth, distinguish blank-in-text from separator by indent, and strip only the renderer's contribution.
The arm that warned about an unplaceable line but dropped it from the AST now also keeps it, as a recovered child block — warning *and* content, not one or the other.
That arm's own comment had argued the warning was enough because the reconcile would refuse to advance the hash, which made not losing bytes depend on a guard in another crate.

**Covered — the claim itself, enforced.**

`outl-md/CLAUDE.md` invariant 8 said `last_synced_hash` may only advance over content the same call emitted ops for.
It was prose: no code checked it, no test pinned it, and `reconcile_md` stamped the hash over the whole file unconditionally.
It now asks `content_lines_missing_from` before writing the sidecar and **withholds the hash** when anything is unaccounted for, leaving the page dirty so the next pass looks at it again.
`ReconcileReport.unlogged_lines` carries the count so a caller can surface it.

This is defence in depth, not a second fix for the same bug: with the parser corrected, none of the shapes that produced the 387 lines trip it.
Which makes its own failure mode the one worth testing — a **false positive** leaves a page permanently dirty, and across 2,560 pages that is the whole graph.
`the_hash_is_still_advanced_for_every_shape_a_real_workspace_holds` asserts the hash advances for 17 shapes taken from the pages that actually held unlogged content.
Roam's tab-indented ordinals and `*` bullets, a `U+2029` pasted from a PDF, non-breaking spaces, fences, properties followed by prose.

`content_lines_missing_from` moved from `outl-actions` to `outl_md::unlogged` to make this possible.
The producer is one crate below and cannot depend upwards, and a second copy of the verdict is exactly the drift the "one owner" rule exists to prevent.
`sidecar_can_answer` moved with it, so the stand-down condition and the predicate that names it cannot be changed apart.

**Covered, but only because measuring proved the parser fix was not enough.**
Reading those lines again recovers nothing on its own: the sidecar already carries the hash of the file *with* the content, so the page reads as in-sync and the reconcile never looks at it.
Measured: `serve --once` applied **0 ops** to all 233 pages.
Two things close it.
`CURRENT_PIPELINE_VERSION` goes 2 → 3, which makes every existing sidecar stale by pipeline and turns the first boot into a one-shot migration (323 ops, 233 pages down to 29 on the measured workspace).
It goes **3 → 4** in the round that fixed the parser, for the same reason and by the same mechanism.
Measured cost of that second migration on the reporting workspace: **56 seconds, 2,827 files, 679 ops** — background on the desktop, but **inline on the open path on mobile** (`outl-mobile/src-tauri/src/workspace_open.rs`), so the first cold boot after the bump holds the phone's UI for that long.
`outl reconcile --ahead-of-log` is the explicit escape hatch for whatever the migration leaves, clearing the recorded hash on exactly the pages `doctor` names.
**Covered — and the residue turned out not to be a residue.**

After the parser fix the count stood at 8 pages / 49 lines, down from 41 / 387, and this RFC characterised them as a class the parser still could not read: fences imported from Roam, bodies at odd indentation, loose bullets inside.

That characterisation was wrong in the way that matters.
The shape was right — every one of the 8 is a bullet living inside a block's text, in a code fence or a pasted list — but **the parser reads them correctly.**
Checked directly on the representative case (`pages/2024-04-04.md`): the line is in the AST, it survives `render`, the page is a `render → parse` fixpoint, and disk and render have the same 55 non-blank lines.
Nothing was lost.
The whole-corpus check had been saying so the entire time — `losing_a_line = 0` across all 2,827 files — and it was read past.

What was wrong was **the detector**.
`disk_line` strips the `- ` marker because the renderer *adds* one, which is true for a block's first line and false everywhere else.
Inside a fence the renderer adds nothing, so the marker is part of the text verbatim, and `- endpoint:` on disk never matched the `- endpoint:` the log held.

So those 8 pages were being told they carried unlogged content they did not carry — and that verdict is not advisory.
It withholds `last_synced_hash`, refuses re-projection, and reconciles the page on every boot forever.
**A false positive here freezes a healthy page**, which this RFC had already named as the worst failure mode available, one section above, while shipping one.

The fix is to try both shapes for each disk line — stripped, then verbatim.
That widens *how* a known line may match, never *which* lines are known, so a line the log genuinely lacks still fails both lookups (`content_the_log_does_not_have_is_still_reported`).
Residue after the fix, same measurement: **0 pages / 0 lines**.

That number came from re-running the measurement against the final diff rather than from the round that motivated the design, and the two disagreed — which is why the re-run is now the step, not the estimate.

**It disagreed twice, and the second time is the instructive one.**
The first figure written here (10 pages / 105 lines) was copied from a review report rather than measured, and it predated three regression fixes to the same parser.
So this RFC stated an unmeasured number *in the paragraph that argues for measuring*, one edit after adding the rule.
The published figures now come from one inlined copy of the normalisation run over both branches in the same session, which is also why they are lower than any earlier estimate: the two sides finally answered the same question.
A number nobody re-derived is a claim, whoever wrote it and however recently.
`outl reconcile --ahead-of-log` is safe to run now that a reconcile no longer writes the truncation back, and on the reporting workspace it has nothing left to do.

**Covered — the second recovery route, which does not need the `.md`.**

Everything above recovers content by reading the `.md`, which only works while the `.md` still holds it.
A page overwritten before the guard existed is out of reach that way — and was assumed lost.
It is not, because the op log is append-only: the truncating `Op::Edit` did not replace the earlier one, it followed it.

`outl recover` reads that history back.
The signature it looks for is narrow on purpose.
Not "the text shrank" — users delete text, and that is not a defect — but "the current text is a **prefix**, character for character, of an earlier revision", which is precisely what dropping everything after a point produces.
Two properties follow from the narrowness.
It is quiet: on the reporting workspace the whole-graph scan returns 8 blocks out of 67,213, four of them the multi-line briefings the original report was about.
And restoring cannot lose anything, since the recovered text contains the current text as a prefix, which `restore_truncated_block` re-checks rather than assumes.

Read-only by default, `--apply` to write, and writing goes through `block::edit_text` like any other edit — a new op, never a rewrite of the log.

**The two routes have an order, and it is `reconcile --ahead-of-log` first.**
Reconcile is the wider net — it recovers whole blocks the log never saw, not only truncated ones — and it rebuilds sidecars on the way through.
Measured on the reporting workspace: reconcile-first recovered 40 pages / 623 ops and left `recover` reporting 4 blocks holding 4 lines, every one of them a one-line editing artifact.
Run the other way round, `recover` restored the same briefings as 4 blocks / 77 lines, and then two of those pages could not re-project.
Their `.md` still held the full text, so the guard correctly refused, and the sidecar stayed stale until a reconcile ran anyway.
`outl serve --once` does not resolve that state (0 ops applied, `.md` unchanged); `doctor` names the page and points at `--ahead-of-log`, which is the same instruction `recover` prints when it withholds a page.

**Covered — volume guards.**
Both halves of "this operation scales silently" are now answered, on the same principle: measure first, refuse the whole pass, and leave one explicit way to say yes.

*Matching.*
`outl_md::matching::guard::match_blocks_guarded` wraps `match_blocks` and checks how much of the page the orphan list would trash.
Ceilings: 500 orphans absolute, or 75% of the page once it has at least 20 known blocks.
The absolute arm catches the partial-but-huge case (5,000 of 20,000 blocks is only 25%); the relative arm catches the truncated read, which is the failure that motivated this — an iCloud placeholder or half-flushed write orphans everything.
The ratio is set high, and the floor exists, for the reason this RFC already recorded about the unlogged-content check: a guard that fires on ordinary editing gets disabled, and then it guards nothing.
Refusal is an `Err` over the whole pass.
`OrphanGuard::Disabled` is the opt-out, reached by `outl reconcile --allow-bulk-delete` — a guard with no way to say "yes, I meant that" is a wall, which root `CLAUDE.md` invariant 9 names as its own defect class.
The refusal is safe by construction because `match_blocks` is pure — refusing after it ran is refusing before anything exists to apply.

*Re-projection.*
`outl doctor` measures, per page, how many content lines on disk the new projection would not reproduce, and reports the number **before** writing — per action and as a workspace total, in `--repair` and in the default read-only mode alike.
Past 100 content lines or 20 pages that lose content, the page repairs stand down and `--repair --force` is required.
Pages the write only adds to count as zero, so a device that just paired and has the whole graph unprojected never trips it.
That closes the specific gap named here: the run that removed 1,426 lines from 233 pages printed `708 fixed` and no line count at all.

*Armed.*
`reconcile_md` is the one production caller that turns orphans into `Move(node, TRASH_ROOT)`, and it now goes through `match_blocks_guarded`.
`reconcile_md_with_guard` is the entry point that takes an explicit `OrphanGuard`, so the opt-out reaches a user-facing flag without every caller having to think about it.
A guard that exists but is not wired to the call site is a guard that guards nothing — worth saying, because it shipped in exactly that state for one commit.
