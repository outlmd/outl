# RFC 0266 — The most dangerous stale fact is one that was true

Status: informational.
Records what a core-hardening pass measured, and — more importantly — what it **refuted**.

## Why an RFC for a measurement pass

Most of this document is negative results.
That is the point.

A refuted hypothesis leaves no trace in the code, so the next person forms it again, spends the same week, and reaches the same answer.
Every section below that says *"this is not the problem"* exists so nobody re-derives it.

## The thesis

Every defect in this pass reduces to one shape: **a claim that was true when written, and was never re-derived after the thing it described changed.**

Not a lie, not carelessness — a fact with no owner and no expiry.
Four instances, all found within one pass:

- **`uhlc`.**
  Root `CLAUDE.md` lists "`uhlc` for time" under *Decisions you don't get to revisit*, and `outl-core/CLAUDE.md` called `hlc.rs` a "wrapper over `uhlc`" in two places.
  `grep uhlc Cargo.toml crates/*/Cargo.toml` is **empty**; `hlc.rs` is hand-rolled.
  True as a plan, never true as code, asserted in six places for months.
  The cost was not the wrong word: `docs/storage.md` listed "uhlc clamps to avoid runaway logical counter" as a **mitigation** in a failure-mode table — a risk register claiming a control that does not exist.
- **Invariant 3's coverage rule.**
  Five documents state 100% coverage on `do_op` / `undo_op` / `apply_op` / `creates_cycle`, "no exceptions".
  Measured: 100%, 100%, 100%, and **96.55%**.
  The uncovered line is `creates_cycle`'s `return true` under `if steps > max_steps`, and a `debug_assert!` one line above fires first — so in any debug build (which is what `cargo llvm-cov` builds) it is unreachable **by construction**.
  The rule was true when written; a later refactor moved the functions and added the assert.
- **`Tree::property_count` as "dead API".**
  A coverage run reported it called by nothing.
  True at the time — and false by the time it was repeated, because this pass's own `benches/tree.rs` calls it.
  Worth recording precisely because it is the least dramatic instance: the author re-asserted an inherited finding without re-deriving it against a tree they had themselves changed.
- **`docs/shortcuts.md`'s TUI column**, `docs/sync.md`'s revocation section, and the capability catalog's `Calendar` row — each accurate when written, each describing a client that has since gained or lost the feature.

**The general rule:** a fact that was *never* true tends to get caught, because someone trips over it early.
A fact that *was* true is load-bearing, quoted, and trusted — and nothing re-checks it.
When you fix something, the question is not only "is the doc right?" but "which claims elsewhere were true **because** of what I just changed?"

This is the same family as invariants 9, 10 and 11 in the root `CLAUDE.md`: a fix relocates a problem more often than it removes one.

## Refuted: the `apply_op` reorder window is not a cost

**Hypothesis:** `apply_op` pops and undoes every op newer than an arriving one, so under sync the undo window gets deep and expensive.

**Measured, on the real 217,811-op / 20-actor workspace:** the window is **0 for 217,663 of 217,663 ops.**
Sorted replay is 107.5 ms total, 494 ns/op, **zero ops undone**.

Every path feeds `apply_op` pre-sorted: `all_ops_combined` sorts, and so does `ops_since_per_actor`.
Peer ops never reach `apply_op` incrementally at all — `outl-sync-iroh` writes received ops straight to the `.jsonl` and the client rebuilds the whole `Workspace`.

Three candidate optimizations were evaluated and all three rejected:

- **Short-circuiting undo for ops in "different subtrees" is unsound.**
  It is a case split on `do_undo_op_inv`, whose only hypothesis is a well-formed tree and which has no such split.
  Skipping X's undo also leaves its `old_parent` / `old_position` derived against the pre-reorder state, so the *next* reorder restores a parent that never existed.
- **A skip index to find the split point faster misdiagnoses the cost.**
  The loop is already O(window), not O(n) — `last()` and `pop()` are O(1) and `contains_ts` is already a binary search.
  Finding the boundary faster does not remove the obligation to undo everything inside it.
- **Batching a sorted arrival set is sound and provably equivalent, and worth zero today**, because production already gets the same effect by rebuilding from a sorted disk merge.
  It becomes load-bearing only if incremental in-memory ingest replaces the full rebuild — land it *then*, and route it through `paper-verifier`.

**Do not spend effort here** until incremental ingest exists.

## Refuted: nothing quadratic fires during boot

Cold boot on the reference workspace measured **70 seconds**, and the CRDT was 0.1% of it.
Fixed during this pass: `sidecar::write_atomic` now buffers.
The numbers below are the pre-fix measurement, which is the part worth keeping.

| phase | ms | % of boot |
|---|---:|---:|
| index sidecar persist | 71,499 | **99.31%** |
| serde_json full deserialization | 174 | 0.24% |
| `apply_op` loop (entire CRDT) | 68 | 0.095% |
| global sort of 217k ops | 3.6 | 0.005% |
| Yrs text hydration | 0 | 0% (correctly deferred, #179) |

`contains_ts` is a real binary search at 87 ns/call, `do_op` is 0.1 µs/op, `OpLog::append`'s `edits_by_node` upkeep is O(1), and Yrs hydration is 0 ms.

The whole of cold boot was **one missing `BufWriter`**: `sidecar::save_entries` did `writeln!` per entry against a bare `File` — 435,622 unbuffered `write(2)` calls.
CPU accounting confirms it: 0.62 s user + 3.07 s sys against 72.00 s wall.
Isolated micro-benchmark, same entries, same bytes: **27.17 s unbuffered vs 0.54 s buffered, 50×.**

Two consequences worth carrying:

- The 84 MB of abandoned `.tmp.<ulid>` scratch in `ops/` is a **consequence** of this, not bad luck.
  A 70-second write gets interrupted — laptop asleep, Ctrl+C, app closed.
  Sixteen abandoned temps is the expected outcome of a 70-second window.
- Compaction deletes the index sidecars, so **every `compact --apply` armed a 70-second freeze on the next open.**

## The measurements that stand

Each is a ratio or an order of magnitude, so its conclusion survives a noisy machine.

| measurement | result | reading |
|---|---|---|
| `OpLog::contains_ts`, 10k → 218k ops | 36 → 41 ns | binary search confirmed |
| `OpLog::edit_updates`, 68× log growth | 49 → 44 ns | the per-node index holds its documented O(edits-of-node) |
| `creates_cycle`, real tree | p50 depth 3, max 8 | ~3.4 map lookups; not a cost |
| `apply_op` reorder, window 10 → 10,000 | 1.64 µs → 1.65 ms | exactly linear; nothing scans inside the loop |
| `children_of`, 1k → 10k → 68k nodes | 815 ns → 9.7 µs → 105 µs | worse than linear — the 68k map exceeds cache |
| whole-tree walk via `children_of` vs a prebuilt index | 11,759 ms vs 23.2 ms | **506×** (the diagnosis; what shipped is below) |
| per-node `properties_of` vs one `iter_properties` pass | 546 ms vs 0.917 ms | **596×** |

### What shipped, measured the same way

| | before | after |
|---|---|---|
| `walk_subtree(ROOT)` | 6,832 ms | **13.2 ms (516×)** |
| `project_outline(ROOT)` | 6,857 ms | **61.6 ms (111×)** |
| `walk_subtree`, one page | 2.831 ms | **0.544 ms (5.2×)** |
| every page in a loop | 7,174 ms | **1,429 ms (5.0×)** |
| `render_page_md`, one page at 64k nodes | 12.5 ms | **3.9 ms (3.2×)** |

The fix is a **level-batched, subtree-scoped** children index — one `iter_nodes` scan per level of *depth*, not per *node*.
Not a whole-workspace map, for the reason below.

### The trap inside the second one

The 596× figure is for a **whole-workspace** pass, where one grouping is amortized over every node.
Applying the same fix per page made rendering **2.7× slower**, because a page render then allocates for all 192k properties to read the ~80 that are its own.
Scoping the scan to the subtree being walked is what actually paid — 3.2× measured.

**Two people hit this trap from opposite directions, and both caught it only by measuring.**
The second measured the whole-workspace children index at 4.99 ms and showed that calling it per page would make a page walk ~5 ms where it was 2.83 ms — the same regression, arrived at independently.
Its first scoped attempt then came in at 1.56 ms/page, of which hashing 64k parents per level was ~70%; a sorted-`Vec` frontier with a single-parent fast path took it to 0.54 ms.

**A speedup measured at one scope does not transfer to another.**
The obvious reading of a real measurement produced a regression twice, in two different modules, and re-measuring is the only thing that caught either.
Note what the two share: an amortized cost is only amortized over the traversal you actually do.
A figure quoted from a whole-workspace pass is not a property of the helper — it is a property of the caller.

## An untested refusal was untested because it did not work

`storage/compact/` shipped with 45% function coverage, and **18 of 33 never-executed functions were the refusal arms** — `exclusive_workspace_lock`, `lock_every_actor`, `apply_compaction`, `invalidate_indexes`, `holds_op_log`.

Its doc comment promised "a damaged log is reported, never rewritten".
When a test was finally written for that promise, **it failed.**
Compaction rewrote a healthy actor file, and dropped a `Move` from it, while a different file in `ops/` was damaged.

The mechanism is worth stating, because it is not "someone forgot a check".
Compaction decides that a `Move` restating its own `Create` is **inert**, and it makes that judgement against the **merged** log across all actors.
A damaged file's ops are missing from the merged view, so a `Move` that is meaningful in the real log looks inert against the truncated one.
The fix was an **ordering** change, not an added condition: parse every file before any rewrite begins, so a damaged record surfaces before the first byte of `ops/` moves.

This is the sharpest argument in the whole pass for a standard of **no unexecuted refusal**, stated as a property rather than as a coverage percentage.
A guard is worth exactly what it refuses, and one that has never refused anything has never been shown to work.
The number was never the point — the untested branch was.

## Two things about the test suite

**The naive tree invariant is false for this CRDT.**
"Every node reaches ROOT or TRASH_ROOT" cannot be asserted: an op whose parent has not arrived yet materializes against a phantom parent, and the only way to make the assertion pass is to drop the op — which violates invariant 5.
The assertable property is **"every parent chain terminates"**.

**The shared convergence generator barely exercises the cycle path.**
Measured by replaying in HLC order and asking `Tree::creates_cycle` *before* each apply — which isolates genuine rejections from ops merely superseded by a later move:

- dedicated cycle generator: **10.2%** of structural ops rejected, 328/400 programs affected;
- shared `program_strategy()`: **1.5%**, and only **13%** of programs contain any rejection.

So ~87% of the broad convergence cases never touch invariant 4's "no-op on the tree, still in the log" clause.
If someone deleted a cycle-rejected op from the log, most generated cases would not notice.

The first version of that measurement was **wrong**, and is worth recording as a method note.
It counted `Move` ops whose `new_parent` differed from the final tree's parent, which conflates "rejected by the cycle guard" with "superseded by a later move".
It reported a meaningless 55%.

## What this pass changed about how to argue

1. **Attribute a cost before letting it decide** (invariant 11).
   "Boot is slow, so the op log is too big" was the available story; the op log was 0.3% of boot.
2. **A refutation is a deliverable.**
   Write down what you proved is *not* the problem, with the number, or it gets re-hypothesized.
3. **Re-derive inherited facts**, especially the ones that were true.
4. **When outputs are identical and only cost differs, no correctness test can help.**
   `log.iter().filter_map(..)` and `log.edit_updates(node)` return the same bytes; that is why the whole-log scan was reintroduced twice after being fixed once.
   Guarding it needs a test that measures a **ratio across input sizes** rather than a duration — see `crates/outl-core/tests/block_text_is_indexed_not_scanned.rs`.
5. **Property tests find the sites hand-written cases miss.**
   Fixing one blank-line defect in the markdown parser left two more copies of the same guard; the property test found both.

## A heuristic promoted to a rule pointed everyone away from the bug

Invariant 8 has a standing instruction, and it is a good one: **the guard is right, the producer is wrong — fix `render`/`parse`, never `unlogged.rs`.**
It exists because relaxing that gate once cost 1,426 lines of real user content.

Applied as an absolute, it was wrong exactly once, and it cost most of a day.

A property test failed on `"a\n```\n- j\n```\nj"`.
Under the rule, that had to be the producer manufacturing a line it could not emit an op for.
It was not.
`parse(render(text))` is an **exact roundtrip** for that input — the `.md` holds precisely what the log holds — and the guard still reported a line missing.

The defect was **greedy multiset assignment** in `content_lines_missing_from`.
An indented line has two readings (`"- j"` verbatim, `"j"` marker-stripped); `known` is a multiset that **decrements**; trying the stripped reading first spent the single `"j"` the log held for the *last* line, which then matched nothing.
A perfect assignment existed and greedy never found it.
The control that proves it is positional rather than structural: change only the last line to `k` and it always passed.

Two things follow, and the second is the general one.

**The fix was safe for a reason worth stating, because "we edited the guard" is otherwise alarming.**
Both orders try **both** keys, so the set of lines that find a hit is identical — only *which entry gets consumed* changes.
The swap therefore **cannot introduce a false negative**, the byte-deleting direction; it can only shuffle false positives.
Nothing about which lines count as known was widened.

**The partitioning pointed every agent away from where the bug was.**
One was scoped to the parser and forbidden `unlogged.rs`; the reviewer's lead was an ordering question inside `parse.rs`, which a control disproved.
The bug sat in the one file the rule had declared innocent.
It was found only because someone tested the premise instead of applying it.

So: **a rule that says "the bug is never here" is a rule that stops you looking there.**
Keep the default — it is right almost always, and the one time it was wrong it cost a day, while the failure it prevents costs a thousand lines.
But state it as a **prior**, not a fact, and make "verify the producer is actually innocent" the first step rather than an assumption.
Here that check was one line of output: print what `parse` returns.

## The thesis applied to this pass itself

Two "done and green" reports in this campaign did not survive independent verification.

The first was benign — a stale checkout, where nothing had actually changed underneath.
The second was not.
A property test reported green had been run on one seed; a later seed found a fenced `- ` line that made `content_lines_missing_from` report a false positive.
That is invariant 8's guard freezing a page whose content the op log *does* hold.

Both failures share the shape this document is about.
A verification result is a **fact with a timestamp**, and it decays exactly like a documented one: the gap between "I ran it" and "I reported it" is where both lived.

Three rules follow, and they are cheap:

- **Run the suite against the current tree immediately before reporting**, not against the state you remember.
- **For a property test, green on one seed is not green.**
  When proptest finds a case, pin the minimal input as a **deterministic** test as well, so it survives seed rotation.
- **A test that passes with the feature removed is pinning nothing.**
  Check it per-test, not in aggregate — one agent here found three of its own tests were vacuous and deleted them rather than keep them, which is the right trade.
- **Check the artifact, not the instruction.**
  Twice in this pass someone reported "an agent is on it" on the strength of having *sent* the instruction, while `ls -lT` showed the files untouched for over an hour.
  Sending is not doing.

The most useful habit to come out of it is smaller than any of those, and it is a **contradiction check**.

A failing test went green once, and the reason it was not believed is that the source files had not changed in over an hour.
A green test with no source change is impossible, so one of the two readings had to be wrong.
Re-running six times showed the green was the artifact — a stale binary during a concurrent rebuild — while the failure was deterministic and correctly seed-pinned.

Taken alone, the green would have been reported as a fix.
It was caught by holding two cheap observations against each other rather than by trusting either.
That generalizes past tests: **when a result would be good news, look for a second fact that must also be true if it is.**

It would be convenient to treat this as a process footnote rather than evidence.
It is evidence: the pass that set out to find claims nobody re-derived produced two of its own inside a day.
