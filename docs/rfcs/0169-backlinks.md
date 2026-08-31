# RFC 0169 — Backlinks: one definition of a mention, one index, four clients

| | |
|---|---|
| **Status** | Shipped |
| **Issue** | [#169](https://github.com/outlmd/outl/issues/169) (anchor, labelled `RFC`), [#180](https://github.com/outlmd/outl/issues/180), [#142](https://github.com/outlmd/outl/issues/142) |
| **PR** | — |
| **Date** | 2026-08-06 |
| **Reference doc** | [clients.md § Backlinks index](../clients.md#backlinks-index-performance) |
| **Invariant** | none in root `CLAUDE.md` — the rule lives in `outl-actions/CLAUDE.md` (`backlinks` / `backlinks_index` / `backlinks_sort` rows) and in `outl-md/CLAUDE.md` ("Does not carry backlinks") |
| **Guarded by** | `index_matches_the_on_demand_path`, `dedups_block_matched_by_slug_and_title`, `one_index_serves_many_pages`, `empty_workspace_has_empty_index` (`crates/outl-actions/src/backlinks_index.rs`), `from_disk_matches_workspace_build`, `from_disk_build_does_not_materialize_workspace`, `workspace_build_materializes_everything_the_disk_build_avoids`, `reindex_page_reflects_an_edit_incrementally` (`crates/outl-actions/tests/backlinks_index_disk.rs`), `backlinks_include_self_references_inside_the_same_page`, `backlink_carries_shallow_source_block_and_path`, `tag_mentions_match_through_slugify`, `longer_tag_does_not_false_match_a_prefix_target`, `tag_inside_inline_code_is_not_a_mention`, `block_with_ref_and_tag_emits_one_backlink`, `block_with_repeated_reference_only_emits_one_backlink`, `structural_instance_shows_in_template_backlinks`, `callable_site_shows_in_template_backlinks`, `non_template_page_ignores_call_and_provenance`, `person_page_picks_up_at_alias_mentions`, `backlinks_for_a_future_journal_finds_blocks_in_past_journals` (`crates/outl-actions/src/backlinks.rs`), `root_level_reference_has_no_ancestors`, `one_level_deep_carries_the_immediate_parent`, `deeper_reference_carries_the_full_chain_root_first`, `ancestor_todo_prefix_is_stripped_from_the_crumb`, `siblings_share_the_same_ancestor_prefix` (`crates/outl-actions/tests/backlinks_breadcrumb.rs`), `newest_first_puts_newest_page_on_top_dfs_within`, `oldest_first_is_the_group_reverse_dfs_within`, `interleaved_sources_become_contiguous_runs`, `orphan_blocks_cluster_under_the_empty_group` (`crates/outl-actions/src/backlinks_sort.rs`), `the_index_agrees_with_a_naive_scan_on_generated_workspaces`, `a_page_nobody_links_to_has_no_backlinks` (`crates/outl-actions/tests/backlinks_index_property.rs` — the randomised agreement check this RFC proposed and #213 recorded as never written), `simulate_760_backlinks` (`crates/outl-actions/tests/bench_backlinks.rs`, `#[ignore]` — measurement harness, not a CI guard) |

## Why

Three user reports, one subject, and one prior failure that constrains every fix.

**It got slow, and it got slow with the wrong variable.**
A user opening a template page with 760 backlinks reported "things are rather slow" ([#169](https://github.com/outlmd/outl/issues/169), @gitdaveuk).
The committed bench (`crates/outl-actions/tests/bench_backlinks.rs`, median of 7, `--release`) holds the backlinks fixed at 760 and grows the workspace around them:

| scenario | backlinks | workspace blocks | `backlinks_for_page` |
|---|--:|--:|--:|
| ref, no subtree, no noise | 760 | 760 | 4.7 ms |
| ref, no subtree, noise ×10 | 760 | 8 360 | 89 ms |
| ref, no subtree, noise ×30 | 760 | 23 560 | 730 ms |
| template, subtree 5, noise ×10 | 760 | 12 160 | 426 ms |
| template, subtree 15, noise ×30 | 760 | 34 960 | **3 830 ms** |

The cost tracked the *workspace*, not the results, because `collect_backlinks` walked every page and every block on every open.
The report's own shape — the template channel, a large workspace — took **3.8 seconds to open one page**.
The same bench found a second waste: 70–87% of the IPC payload was subtree the renderer immediately discarded, because each `Backlink` shipped `source_block` as a full `OutlineNode` tree while desktop and mobile render only `source_block.tokens`.

**It was fast and meaningless.**
[#180](https://github.com/outlmd/outl/issues/180): a reference nested inside an outline arrived in the panel as a bare leaf.

```
- Planejamento Q3
  - Objetivos
    - bater as [[metas]] de receita
```

Opening `metas` showed `bater as [[metas]] de receita` and nothing else.
"Bater as metas de receita" — under what?
Which plan?
A backlink is supposed to answer *where* the page is mentioned **and** *in what context*, and a leaf pulled out of its branch answers only the first.
Root-level references were fine; in a real outline most references are not root-level.
Mobile was worse — a flat list that did not even group references by their source page.

**It was in the wrong order.**
[#142](https://github.com/outlmd/outl/issues/142): "when I have a lot of backlinks, I have to scroll down to see the latest."

**And the prior failure that shapes all three.**
There used to be a backlinks cache on `outl_md::index`, and it was deleted on purpose.
Not for performance — the *policy* existed twice, once in `outl-md` and once in `outl-actions`, and the two drifted on whether a reference inside the same page counts.
Self-references were visible on one surface and invisible on the other, and **the user found it, not a test**.
Any index proposal that re-implements "what counts as a backlink" walks straight back into that.
So the first thing this RFC has to fix is not the walk.
It is the definition.

## What we chose

### A backlink is a block that mentions the page, and "mentions" has exactly one owner

`mentions_of(text, from_template)` in `crates/outl-actions/src/backlinks_index.rs` is the single owner of *what does this block mention*.
`keys_for_page` is the single owner of *what keys does a page look itself up under*.
Both are `pub(crate)`, so no caller outside the module can fork them, and `backlinks::backlinks_for_page` / `backlinks_for_target` delegate here rather than keeping a second walk.

Four mention channels, and the reason each is a channel rather than a client feature:

| Channel | Rule | Why it is not a client concern |
|---|---|---|
| `[[target]]` | literal ref text | — |
| `#tag` | the tag's `slugify` form resolves to the page | Same `slugify` a tag *click* goes through, so navigation and "Linked from" cannot disagree about what a tag points at |
| `call:<name>` fence | the template's name | A template's backlinks are its invocation sites |
| `from-template:: <slug>` | provenance property | A structural instantiation is a reference to the template it came from |

A block mentioning the same page twice, or by slug *and* by title, emits **one** backlink.
A reference inside the target page itself **counts** — that is the exact policy the deleted cache got wrong, and it now has a test named after it.

### The index carries facts, and there is only one code path

`BacklinkIndex` is an inverted map from target key to referencing blocks.
`build_backlink_index_from_disk(metas, root)` is the client-facing builder: one `O(blocks)` pass reading each page's `.md` plus sidecar, touching no `Workspace`, holding no lock, and `Send`.
`for_page` / `for_target` are `O(refs)` lookups, `count_for_page` counts without cloning, and `reindex_page_from_disk` refreshes a single page after a local edit instead of rebuilding everything.

The structural move that prevents the old bug is stronger than the one the issue proposed.
The issue's design kept the brute-force walk as the policy owner and made the index a *candidate filter*, with a parity property test to notice divergence.
What shipped removed the second path instead: `backlinks_for_page` and `backlinks_for_target` are now thin one-shot wrappers that build a fresh index and look it up.
There is no second walk to drift, so parity is a property of the code rather than of a test.

### Entries are shallow leaves

`project_outline_node_shallow` (from-workspace) and `shallow_parsed` (from-disk) produce body, tokens and properties, with **no children**.
Two independent reasons, both load-bearing:

- **The lock.**
  Materializing the subtree of every referencing block across the workspace, under the workspace lock, is what froze input.
  A shallow projection makes the walk `O(refs)` in materialization instead of `O(refs × subtree)`.
- **The wire.**
  70–87% of what crossed IPC was dropped by the renderer, which reads only `source_block.tokens`.

`Backlink::into_shallow()` survives as a defensive no-op on the GUI wire path, so a future full projection cannot silently reach the clients.

### Context is the ancestor chain, not the subtree

This is the part of [#180](https://github.com/outlmd/outl/issues/180) that had to be answered *without* undoing the decision above.
The context a nested reference is missing is **upward**, not downward, and upward is `O(depth)` while downward is `O(subtree)`.

`Backlink::ancestors: Vec<BacklinkCrumb>` is root-first, excludes the page root, and is empty when the block sits at the page root.
`BacklinkCrumb { id, text }` is plain text with any `TODO ` / `DONE ` marker stripped, because a breadcrumb is context and not a task list.
Every client renders it as a dimmed trail above the citing block.
`@outl/shared/outline::sameCrumbTrail` (mirrored by `outl-tui`'s render-local `same_trail`) collapses consecutive references inside one branch, so the trail prints once per branch instead of once per reference.

### Order is one function

`outl_actions::sort_backlinks(links, newest_first)` (`crates/outl-actions/src/backlinks_sort.rs`) is **group-stable**.
Each source page's blocks stay contiguous and in DFS order; pages sort by their most recently referenced block.
`block_id` is a ULID, so lexicographic order already tracks creation time and no extra read is needed.
The direction comes from `[display] backlinks_order` and is a pure display preference that never converges between devices — same class as `theme.preset`.
Every client calls this one function, so the direction cannot drift: the TUI toggles it with `Ctrl+O`, the desktop and mobile panels expose a header button, and all three persist through `outl_config`.

Client wiring lives with the clients: [`docs/clients.md` → Backlinks order](../clients.md), plus `outl-tauri-shared/CLAUDE.md` for the lazy `page_backlinks` command.
`PageView.backlinks` always comes back empty, because computing backlinks on the first-paint path was what blocked the first journal render.

## Why not the alternatives

**Put the index back on `outl_md::index` / `WorkspaceIndex`.**
The natural home for an index, and the one that already failed.
The cost is measured rather than hypothetical: policy in two crates drifted on self-references, one surface hid them, and a user reported it.
Issue [#81](https://github.com/outlmd/outl/issues/81) proposes deriving the *existing* `WorkspaceIndex` from the op log, and lists backlinks as something that could live there.
If it lands, the two coexist with ownership split: `outl-md` owns facts about `((blk))` handles and text, `outl-actions` owns page-backlink keys **plus** the policy, and neither re-derives the other's.

**Let the index decide what a backlink is** — store resolved verdicts keyed by page instead of mentions keyed by target.
Cheaper lookup, and it makes staleness *unsafe*: a stale index would answer with a wrong verdict rather than a mention the caller still has to interpret.
The shipped shape keeps the index answering a factual question, which is why a slightly stale index degrades into a missing row instead of a wrong one.

**Keep the brute-force walk as the policy owner, with the index as a candidate filter** (the design in #169's own body).
It works, and it keeps two code paths alive whose agreement is asserted by a test rather than guaranteed by construction.
Collapsing the on-demand API into a wrapper over the index removed the second path entirely; what is left to test is the narrower question of whether the two *builders* (from-disk and from-workspace) agree.

**Build the index from the in-memory `Workspace` on the client.**
This is what #169 specified — "a structure derived from the `Workspace`, not from `.md`/disk" — and it shipped, and it had to be changed.
Reading block text through `Workspace::block_text` on an `O(blocks)` walk forces a lazy-boot vault ([#179](https://github.com/outlmd/outl/issues/179)) to materialize entirely, and it holds the workspace lock across the whole walk.
Together that pair is the "opening the journal / pressing Esc freezes" regression.
`build_backlink_index` still exists for the one-shot wrappers and for in-memory tests with no `.md` on disk, and `workspace_build_materializes_everything_the_disk_build_avoids` is a control test that keeps the difference honest.

**Client-side virtualization or lazy-load of the list.**
Named in the issue and rejected there: it helps the subtree render and the JS parse, and it does not touch the compute layer.
The backend still has to find the 760.

**Ship the full subtree and let each client trim it.**
The pre-#169 shape.
The client cannot un-walk what the server already walked under the lock, and the bench puts the wasted payload at 70–87%.

**Materialize the subtree lazily, behind an "expand this backlink" gesture.**
A real option, and #180 is the argument against it: the missing context was the ancestor trail, not the children.
Building an expand affordance to recover meaning a breadcrumb gives for free is more UI for a worse answer.

**Persist the index** — in the sidecar, or as an `Op`.
Refused by invariant 7's reasoning read in the other direction.
The index is a pure projection of the tree, like the `.md` files and `WorkspaceIndex`, so it is rebuilt rather than replicated.
In the sidecar it would become a last-write-wins surface; as an `Op` it would let two devices disagree about a value neither of them authored.

**Sort alphabetically, or by last edit.**
Alphabetical does not answer #142 at all.
Sorting by last edit would reorder the whole panel whenever someone fixes a typo in an old block, which is worse than the scrolling it saves.
ULID order gives creation time for free and is stable.

**Newest-first with no option.**
[#142](https://github.com/outlmd/outl/issues/142) asked for the setting explicitly as its own fallback, and the direction is a per-device display preference — a config key is the honest home, not a converged value.

## The opposite direction

**The panel can be behind, and nothing says so.**
`reindex_page_from_disk` runs after a local edit, so a mention the user just typed appears.
A mention that arrived **from a peer** changes the tree, and the rebuild is tied to the client's reload and invalidation seams (`invalidate_backlink_index` after local mutations, plus a rebuild when the host's cached slot is stale).
Until one of those fires, the panel is missing a backlink that exists.
The direction was chosen — missing is recoverable by reopening the page, wrong is not, because the user believes it — but the user is never told the list may be stale, and nothing surfaces the index's age.

**Reading from disk makes the read path trust a projection instead of the op log.**
The index is built from the `.md` files, and the op log is the source of truth (invariant 1).
A page whose `.md` is behind the tree therefore contributes *stale* backlinks.
That is deliberate: the alternative is the workspace lock plus full materialization, and that combination froze input.
The mirrored case is where it bites hardest.
[RFC 0210](0210-md-content-outside-op-log.md) makes re-projection **refuse** on a `.md` that holds content the log lacks.
Such a page stays behind indefinitely, and its mentions are read from the wrong side of the divergence until `outl reconcile` runs.
Two correct decisions compose into a stale row nobody is watching, and this is the place that says so.

**A shallow leaf cannot be read in full.**
A reference whose substance lives in its *children* — a parent block that cites the page while the detail sits underneath — now shows one line plus a breadcrumb pointing upward.
That is a real loss for a real shape, and the recovery is navigating to the source, not expanding in place.

**Group-stable ordering puts old blocks above new ones.**
A page whose newest reference is recent floats to the top with *all* its references, including one from a year ago.
#142 asked for "the latest" and got "the page with the latest", because splitting one page's references apart across the list is worse than the scroll.
`interleaved_sources_become_contiguous_runs` fails if someone later "simplifies" this into a flat chronological sort, which is the shape a reader of the issue title would reach for.

**The breadcrumb is text, not navigation.**
`BacklinkCrumb` carries an `id`, and the TUI trail is not clickable.
A user who can now *see* the right branch still cannot jump to it from the crumb.

**Nothing refuses, and nothing caps.**
A page with 100,000 backlinks renders 100,000 rows.
`count_for_page` exists and no client uses it as a gate, so the failure mode at extreme scale is a slow paint rather than a message.

## How it cannot regress

1. **The rule.**
   There is no root-`CLAUDE.md` invariant for backlinks, and that is worth stating plainly rather than inventing one.
   The enforcing surface is `outl-actions/CLAUDE.md`, whose `backlinks` / `backlinks_index` / `backlinks_sort` rows carry the single-owner rule, name the deleted `outl_md::index` cache as the reason, and state why entries are shallow leaves.
   Its "Reuse-first" section uses the `outl_md::index::Backlink` → `outl_actions::Backlink` consolidation as its worked example of promote-don't-fork.
   On the other side of the boundary, `outl-md/CLAUDE.md`'s workspace-index row says **"Does not carry backlinks"** so an editor adding a cache there reads the rule before writing it.
   Client wiring is pinned in `outl-tauri-shared/CLAUDE.md` — lazy `page_backlinks`, plus the explicit note that `compute_backlinks` no longer exists in `helpers.rs` — and in [`docs/clients.md` → Backlinks order](../clients.md).
   The config key is [`docs/config.md` → `[display]`](../config.md), and the reuse rows are [`docs/primitives-actions.md`](../primitives-actions.md) §3.
   This RFC is the rationale layer those five point at.

2. **Tests.**
   Grouped by what they refuse to let happen:
   - **Policy cannot fork.**
     `index_matches_the_on_demand_path` asserts the indexed lookup and the on-demand call return the same set across the ref, tag and title channels.
     `from_disk_matches_workspace_build` does the same for the two builders.
     `backlinks_include_self_references_inside_the_same_page` is the test named after the original bug; deleting it is how the old divergence comes back.
   - **The mention rule cannot loosen.**
     Tag boundaries: `tag_mentions_match_through_slugify`, `longer_tag_does_not_false_match_a_prefix_target`, `tag_inside_inline_code_is_not_a_mention`.
     Dedup: `block_with_ref_and_tag_emits_one_backlink`, `block_with_repeated_reference_only_emits_one_backlink`.
     Template channels: `structural_instance_shows_in_template_backlinks`, `callable_site_shows_in_template_backlinks`, `non_template_page_ignores_call_and_provenance`.
   - **The lock and the payload cannot come back.**
     `from_disk_build_does_not_materialize_workspace` fails if the builder starts reading the workspace.
     `workspace_build_materializes_everything_the_disk_build_avoids` is its control: it asserts the *old* path really did materialize, so the first test cannot silently become vacuous.
     `backlink_carries_shallow_source_block_and_path` fails if entries regain their children.
   - **Context and order.**
     The five breadcrumb tests pin root-level emptiness, depth, root-first order, marker stripping, and shared prefixes between siblings.
     The four sort tests pin both directions plus group-stability.

   **Named gaps, both real.**
   `#169` asked for a **property test over random workspaces** (`assert_eq!(indexed(p), bruteforce(p))`).
   That does not exist — `outl-actions` has no `proptest` dependency, and the parity guards above are fixed-fixture unit tests.
   Second, there is **no CI performance gate**: `bench_backlinks` is `#[ignore]`d and run by hand, so a reintroduced `O(blocks)`-per-open path would pass every test named above.
   The 3.8 s measurement that motivated this RFC would not fail the suite if it returned.

## Scope

**Not covered — block-ref (`((blk))`) backlinks.**
Already `O(1)` through `outl_md::block_index`'s reverse-ref map, and the dialect side of it belongs to [RFC 0008](0008-markdown-dialect-and-sidecar-tokens.md).

**Not covered — deriving `WorkspaceIndex` from the op log.**
Issue [#81](https://github.com/outlmd/outl/issues/81).
This RFC states how the two indices coexist; it does not decide #81.

**Not covered — incremental maintenance as ops apply.**
`reindex_page_from_disk` is per-page after a local commit.
A hook that updates the index as each `Op` lands (the "Phase 2" in #169) is still open, and its placement question — `outl-actions` observing `Workspace::apply`, or an `outl-core` notification — is unanswered.

**Not covered — surfacing index staleness to the user.**
See The opposite direction.
No issue tracks it.

**Not covered — a result cap or a count-first affordance for very large panels.**
No issue tracks it.

**Not covered — the panel's chrome.**
Sections, collapsing, keyboard traversal across the separator, and the mobile grouping fix from [#180](https://github.com/outlmd/outl/issues/180) are per-client.
Owners: [`docs/clients.md`](../clients.md), [`docs/tui.md`](../tui.md), and the client `CLAUDE.md` files.
