# RFC 0557 - Merging pages: move the blocks, trash the source, leave its names behind as aliases

| | |
|---|---|
| **Status** | Proposed |
| **Issue** | none yet. The number is the PR that carries this RFC, [#557](https://github.com/outlmd/outl/pull/557) |
| **PR** | [#557](https://github.com/outlmd/outl/pull/557) |
| **Date** | 2026-11-15 |
| **Reference doc** | `crates/outl-actions/CLAUDE.md` → "Merging and renaming pages", [`cli.md`](../cli.md) (`page rename`), [`concepts.md`](../concepts.md) (`alias::`), [RFC 0107](0107-page-identity.md) |
| **Invariant** | root `CLAUDE.md` invariant 1 (every step is an `Op`, no `.md` is edited to fix state), invariant 6 (delete is `Move` to the trash, so the source keeps its history), invariant 7 (a merge converges through the op log, never through a shared file) |
| **Guarded by** | `an_alias_naming_another_page_does_not_steal_its_backlinks`, `a_person_alias_naming_another_page_does_not_steal_the_at_forms`, `reindexing_one_page_keeps_the_rule_in_force` (`crates/outl-actions/tests/alias_shadow.rs`); `blocks_move_after_the_targets_own_in_order_with_ids_and_subtrees_intact`, `a_merge_copies_nothing`, `a_second_merge_is_a_no_op_that_says_so`, `a_block_that_reached_the_trashed_source_late_is_moved_by_the_rerun` (`crates/outl-actions/src/merge_pages/tests.rs`); `every_link_form_across_the_workspace_follows_the_merge`, `a_namespace_child_is_reported_not_rewritten`, `an_old_name_written_after_the_merge_is_a_backlink_of_the_target` (`merge_pages/tests_links.rs`); `a_scalar_conflict_keeps_the_target_and_reports_the_dropped_value`, `the_sources_identity_keys_never_reach_the_target` (`merge_pages/tests_props.rs`); `a_ref_by_slug_is_rewritten_to_the_new_title`, `a_backtick_fence_is_untouched`, `a_name_that_slug_resolves_to_another_page_is_not_a_mention`, `a_second_run_rewrites_nothing` (`crates/outl-actions/src/link_rewrite/tests.rs`); `a_slug_rename_moves_the_page_and_keeps_its_id_and_blocks`, `a_journal_cannot_be_renamed`, `a_rename_onto_an_existing_slug_is_refused_and_writes_no_op`, `known_limit_reopening_the_old_slug_renames_the_page_back` (`crates/outl-actions/src/page_rename/tests.rs`); `merge_into_folds_the_old_page_into_the_existing_one` (`page_rename/merge_tests.rs`); `a_merge_racing_an_edit_and_a_late_append_converges_and_loses_neither`, `rerunning_the_merge_on_the_converged_state_rescues_the_late_block_once`, `two_replicas_merging_the_same_pages_concurrently_converge_on_one_placement` (`crates/outl-actions/tests/page_merge_convergence.rs`); `rename_rewrites_refs_and_tags_on_other_pages_and_moves_the_file`, `rename_onto_an_existing_page_is_slug_conflict_and_says_merge_is_possible`, `rename_over_mcp_reports_the_rewritten_blocks` (`crates/outl-cli/tests/page_rename_cmd.rs`) |

## Why

Two pages end up holding one subject.
`alpha` and `beta` both exist because a graph was imported, or because two devices named the same idea differently, or because a typo became a page.
The user wants one page.
Before this change outl had no way to get there.

- **Rename onto an existing name was refused**, with `SLUG_CONFLICT` and nothing else to try.
- **Rename onto a free name left every mention behind.**
  `outl page rename` moved the file and returned the blocks that still said `[[old]]` in `affected_refs`, and left rewriting them to the caller.
  Every `[[old]]` and `#old` in the workspace became a link to a page that no longer existed, which `[[old]]` then recreated empty on the next click.
  [RFC 0107](0107-page-identity.md) lists the slug rename as out of scope for exactly this reason: it touches the file path, the sidecar and every inbound ref.
- **An alias could steal a page's backlinks.**
  A page can answer to other names through `alias::`.
  `keys_for_page` gave a page a backlink key for every alias, even when the alias named a different live page.
  Navigation sends `[[Beta]]` to the page titled `Beta`, because titles win over aliases.
  The backlink index sent the same mention to the page that merely listed `Beta` as an alias.
  Clicking the link and reading the backlinks panel disagreed about which page owned the mention.
  Merging by hand (copy the blocks, add the old name as an alias) walks straight into that, so the alias bug had to be fixed before a merge could rely on aliases at all.

## What we chose

Three pieces, each with one owner.

**1. The alias verdict.**
`alias_conflicts::PageClaims::shadowed_by` answers "does another live page already answer to this alias".
It compares the alias against slugs verbatim, against slugified forms, and against lower-cased titles, which is the order navigation uses.
The backlink index drops a shadowed alias, including the `@alias` forms of a person page.
`outl_actions::alias_conflicts(workspace, page)` lists the shadowed aliases of a page for a UI, by asking the same predicate.
What the user is warned about is exactly what the index ignores.

**2. Mention rewriting.**
`link_rewrite::rewrite_mentions` owns "what is a mention of page X, and how is it respelled".
It reads mentions off `outl_md::inline::tokenize`, the tokenizer that renders the block and decides backlinks.
It rewrites `[[Name]]`, `[[@Name]]` and `#tag` forms, writes one `Op::Edit` per changed block inside one batch, and is idempotent.
It never touches code spans, fenced code, link text or urls, block refs, or names that merely contain the slug.
`link_rewrite::rewrite_text` is the pure half, and `PageNames` is the (slug, title, person) triple both halves read.

**3. The two actions that use it.**

- `outl_actions::merge_pages(ws, hlc, source, target) -> MergeReport` does, in one batch:
  1. moves every top-level block of the source under the target, appended after the target's own children, with ids and subtrees intact;
  2. merges page properties onto the target: `alias` and `tags` are unions, an absent scalar is copied, and a scalar both pages hold with different values keeps the target's and reports the dropped one in `MergeReport::property_conflicts`;
  3. adds the source's slug and title to the target's `alias::`;
  4. rewrites every mention of the source so it names the target, through `rewrite_mentions`;
  5. deletes the source with `Move` to `TRASH_ROOT`.

  A journal as source is refused (`ActionError::CannotMergeJournal`), because a journal day is identified by its date.
  A journal as target is allowed.
  Merging a page into itself is `ActionError::MergeIntoSelf`.
  Running it again after it finished is a no-op, or moves only the blocks that arrived late.
- `outl_actions::rename_page` moved out of the CLI into this crate and now rewrites mentions through the same function.
  A name that is already taken is `ActionError::PageExists { existing }` instead of a CLI-local check.
  `rename_or_merge(.., OnExisting::MergeInto)` turns that refusal into a `merge_pages` call, with the renamed page as the source.
  A journal cannot be renamed (`ActionError::CannotRenameJournal`), because its slug is its date.

The callers project.
Both reports carry `touched_pages`, and a caller runs `commit_page` over each and removes the stale projection (`MergeReport::source`, `RenameReport::stale_projection`) last, so a crash in between leaves two copies of a page and never none.
`outl page rename` (and so `outl_page_rename` and the batch `page_rename` op) is the first caller.
It keeps the `SLUG_CONFLICT` code, now words the message as an offer to merge, and adds `rewritten_refs`, `skipped_namespace_refs` and `touched_pages` to its output.
`affected_refs` stays, and now lists what still names the old slug after the rewrite.

## Why not the alternatives

**Leave a stub redirect page under the old slug.**
It keeps a second root alive for every merge.
That root shows in every page list and picker, owns a `.md` and a sidecar, and needs a rule somewhere that says "this one is not a real page".
Every list, search and index would have to learn that rule, and the first one that did not would show the user two pages for one subject, which is the state the merge was meant to end.
The alias already does the one job a stub has, which is making `[[Alpha]]` land on the merged page.

**Merge at the file level, by concatenating the `.md` files.**
It would write bytes the op log never saw, and root `CLAUDE.md` invariant 1 and 8 exist to refuse exactly that.
It would also drop the block ids, which live in the `.outl` sidecar, so a concurrent edit from a peer to a block of the source would have nothing to land on.
Ids have to survive the merge for the sync story below to hold, and only an op-level move keeps them.

**Copy the blocks and delete the originals.**
A copy mints new ids.
A peer editing the old block while the merge propagates would edit a block that is now in the trash, and the edit would be invisible.
It would also double the ops for a large page.
`Move` preserves the id and the whole subtree for the price of one op per top-level block.

**Physically remove the source, or skip the trash.**
Invariant 6: delete is a move to the trash.
It keeps history, keeps the source's own properties readable (the conflict report points at them), and is what lets a re-run find the source again.

**Rewrite mentions in the client, or per surface.**
The CLI, MCP, the batch op, the TUI and both GUIs all need it.
Two implementations of "what is a mention" drift, and the user finds out when a link survives a rename on one client and not on another.
The backlink index already had to share one tokenizer for that reason.

## The opposite direction

**Unmerge is not an operation, and a merge leaves three things that nothing undoes.**
Be precise about what is and is not true.

- The source root is in the trash, under its own slug, with its properties.
  `outl trash restore` refuses a page root today (`ActionError::TrashPageRestoreUnsupported`), so that is not a route.
  Reopening the old slug through `page::open_or_create` revives the empty root (`page/revive.rs`, [issue #473](https://github.com/outlmd/outl/issues/473)).
  That is a code path, not a feature: no test in this change pins merge followed by reopen, and what it gives back is an empty page, not the blocks.
- The moved blocks stay under the target.
  Nothing moves them back, because after the merge a peer may have edited them or added children, and "the blocks that were once the source's" is not recorded anywhere but the log.
  `MergeReport::moved_block_ids` has them, and `outl block move` can move them by hand.
- The rewritten links and the added aliases are not undone.
  A user who wanted the old page back has the old name as an alias of the target, and mentions that now say the target's name.

**An alias that names another page is now inert.**
This is the fix for the stolen backlinks, and it has a mirror.
Someone who wrote `alias:: Beta` on `alpha` on purpose, to collect every `[[Beta]]` mention on `alpha`, no longer gets them there.
They are listed under the page titled `Beta`, which is where navigation already sent the reader.
Nothing on screen tells the author their alias went quiet: `alias_conflicts` exists for clients, and no client calls it, the GUI rename and merge included.
`doctor`'s alias check names names shared between pages, which is a different question.

**Rename now writes to other pages.**
A rename used to edit one page and its file.
It now emits an `Op::Edit` for every block that mentions the old name, anywhere in the workspace, and the projection of every page those blocks live on changes.
A rename of a heavily linked page is a large batch.
It is one batch, but it is still a write the user did not make in each of those pages.

**Rename of a journal now fails.**
The journal's slug is its date, so renaming one detaches the page from the day that identifies it.
Anyone scripting a journal rename now gets `CannotRenameJournal`, which is a behaviour change for them.

**Sync.**
Every step is an ordinary op, so a merge converges the way any batch of moves, property writes and edits does.
Ids are preserved, so a peer's edit to a moved block, made before the merge reached them, lands on the block under its new parent (`a_merge_racing_an_edit_and_a_late_append_converges_and_loses_neither`).
Two replicas that merge the same pair concurrently converge on one placement (`two_replicas_merging_the_same_pages_concurrently_converge_on_one_placement`).
What a merge cannot do is reach into the future.
A peer that appends a block to the source after the merge landed on their device puts it under a trashed page.
Re-running the merge moves it (`rerunning_the_merge_on_the_converged_state_rescues_the_late_block_once`), and nothing runs it automatically.

## Known limits

These are real, they are recorded here so they are not discovered one at a time, and none of them is fixed by this change.

1. **A late append lands under the trashed source** until the merge is re-run, as above.
2. **Namespace children are reported, not rewritten.**
   `[[source/child]]` and `#source/child` belong to a sub-page that is not being merged or renamed.
   They come back in `skipped_namespace_refs`, one entry per block and name.
3. **Page refs in property values and in fenced code are not rewritten.**
   A `key:: value` is an `Op::SetProp`, not block text, and the backlink index does not read it either.
   Code is never edited, even where backlinks still count a `[[x]]` inside a fence.
4. **The backlink index matches an alias against `[[X]]` case-sensitively, while navigation is case-insensitive.**
   A merge whose source title differs from its slug only by case leaves a `[[Alpha]]` written later resolving to the target without appearing in the target's backlinks.
5. **A renamed page keeps its node id, so reopening the old slug renames it back.**
   `page_id_from_slug` derives the id from the slug a page was created under.
   `open_or_create("old", ..)` finds no live page with that slug, finds the deterministic id live, and writes the old slug onto the renamed page.
   This is how the CLI rename already behaved.
   It is now documented and pinned by `known_limit_reopening_the_old_slug_renames_the_page_back`, so a fix has to be deliberate.
   A merge has no such problem, because the merged-away root is trashed under its own slug.
6. **Only the GUI clients reach a merge.**
   The desktop and mobile apps call `rename_page` and `merge_pages` (rename, a merge target picker, and a confirm when a rename lands on a taken name).
   There is still no `outl page merge`, no MCP tool, no batch op and no TUI affordance, so no terminal command runs a merge, and `outl page rename` onto a taken name keeps refusing with `SLUG_CONFLICT`.
   Those follow as separate changes and are thin wrappers over `merge_pages` and `MergeReport`.
   The GUI clients compose the two themselves (`rename_page`, then `merge_pages` after a confirm), so no surface calls `rename_or_merge` with `OnExisting::MergeInto` yet.
7. **A title given to a rename that turns into a merge has nowhere to go.**
   The target keeps its own title.

## How it cannot regress

1. **The invariant.**
   The single owners are written into `crates/outl-actions/CLAUDE.md` → "Merging and renaming pages", the file anyone editing this code reads.
   A second page-name matcher, in any crate or client, is the thing to refuse in review.
2. **The tests.**
   They are in the **Guarded by** row.
   The ones that fail on the pre-RFC behaviour:
   - `an_alias_naming_another_page_does_not_steal_its_backlinks` fails on the previous `keys_for_page`, which emitted alias keys unconditionally.
   - `a_merge_copies_nothing` fails if the merge ever mints block ids.
   - `a_name_that_slug_resolves_to_another_page_is_not_a_mention` fails if the rewriter answers "is this a mention" with a second opinion that disagrees with navigation.
   - `a_backtick_fence_is_untouched` fails if the rewriter starts editing code.
   - `rename_onto_an_existing_page_is_slug_conflict_and_says_merge_is_possible` pins the wire code scripts already match on.
   - `a_merge_racing_an_edit_and_a_late_append_converges_and_loses_neither` fails if the merge stops preserving ids.

## Scope

**Not covered: a `page merge` command, MCP tool, batch op, or a TUI surface.**
They are separate changes over `merge_pages` and `MergeReport`, and each owns its own refusal wording through the `refusal` catalog ([RFC 0255](0255-operation-vocabulary.md)).
The desktop and mobile apps now call the actions, and `outl_shortcuts::capability_support` records `PageRename` and `PageMerge` as missing in the TUI.

**Not covered: showing a shadowed alias to the user.**
`alias_conflicts` is the function a client calls.
No client does yet, and the GUI change that added rename and merge deliberately left the alias warning out.

**Not covered: fixing the renamed-page-flips-back limit.**
It needs `open_or_create` to notice that the deterministic id is live under a different slug, and every caller of `page_id_from_slug` leans on the id being a function of the slug.

**Not covered: rewriting mentions inside property values, or a namespace subtree.**
See the known limits above.

**Not covered: slug collisions from slugification, or per-device timezone journals.**
[RFC 0107](0107-page-identity.md) owns those.
