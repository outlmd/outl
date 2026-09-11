#!/usr/bin/env bash
# Stage and commit this branch in context-sized pieces, ready to push.
#
# Written because the branch landed as one 212-file working tree: a CRDT
# correctness fix, op-log compaction, two cache GCs, a projection
# executor, a security fix and a performance pass. One commit for that is
# unreviewable and unrevertable; this splits it along the seams the work
# actually has.
#
# WHAT IT DOES NOT DO: push. Nothing here talks to a remote.
#
# HONEST LIMITATION, read before trusting `git bisect` on the result:
# these commits group by **context**, not by "each one builds". Everything
# was authored in a single working tree, so a few files carry changes
# belonging to two groups — `cli.rs` holds both the clap split and
# `compact`'s `--force` flag, and there is no way to separate them without
# hand-staging hunks. The order below puts dependencies first so the tree
# builds at the END of the run, which the script verifies. Intermediate
# commits may not compile.
#
# Usage:
#   ./scripts/commit-branch.sh            # commit everything, grouped
#   ./scripts/commit-branch.sh --dry-run  # print the plan, touch nothing
set -euo pipefail

DRY_RUN=0
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1

cd "$(git rev-parse --show-toplevel)"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [ "$BRANCH" = "main" ]; then
  printf 'refusing to run on main. switch to the feature branch first.\n' >&2
  exit 1
fi

if [ -z "$(git status --porcelain)" ]; then
  printf 'nothing to commit — working tree is clean.\n' >&2
  exit 1
fi

# Start from an empty index. Every group stages its own paths, so
# anything already staged would silently ride along in whichever group
# runs first. `git reset` only unstages — it never touches the working
# tree, so no change is lost.
STAGED_BEFORE="$(git diff --cached --name-only | wc -l | tr -d ' ')"
if [ "$STAGED_BEFORE" != "0" ]; then
  printf 'unstaging %s file(s) so each group stages its own (working tree untouched):\n' "$STAGED_BEFORE"
  git diff --cached --name-only | sed 's/^/  /'
  [ "$DRY_RUN" = "1" ] || git reset -q
  printf '\n'
fi

committed=0
skipped=0
CLAIMED="$(mktemp)"
trap 'rm -f "$CLAIMED"' EXIT

# commit <<'EOF' ... EOF  — message on stdin, paths as arguments.
#
# `git add -A` on each path so a deletion (a file that became a
# directory, e.g. `cmd/serve.rs` → `cmd/serve/`) is staged too.
commit() {
  local msg paths=() p
  msg="$(cat)"
  shift 0
  for p in "$@"; do paths+=("$p"); done

  local existing=()
  for p in "${paths[@]}"; do
    # keep paths git knows about (tracked, deleted, or untracked)
    if git ls-files --error-unmatch "$p" >/dev/null 2>&1 \
       || [ -e "$p" ] \
       || git status --porcelain -- "$p" | grep -q .; then
      existing+=("$p")
    fi
  done

  if [ ${#existing[@]} -eq 0 ]; then
    printf '  skip (no files): %s\n' "$(printf '%s' "$msg" | head -1)"
    skipped=$((skipped + 1))
    return 0
  fi

  # Record what this group claims, so the leftover check below is exact
  # rather than a guess from the group's path prefixes.
  git status --porcelain -- "${existing[@]}" \
    | sed 's/^...//' | sed 's/.* -> //' >> "$CLAIMED"

  if [ "$DRY_RUN" = "1" ]; then
    printf '\n── %s\n' "$(printf '%s' "$msg" | head -1)"
    git status --porcelain -- "${existing[@]}" | sed 's/^/     /'
    return 0
  fi

  git add -A -- "${existing[@]}"
  if git diff --cached --quiet; then
    printf '  skip (nothing changed): %s\n' "$(printf '%s' "$msg" | head -1)"
    skipped=$((skipped + 1))
    return 0
  fi
  printf '%s' "$msg" | git commit -F -
  committed=$((committed + 1))
}

# ---------------------------------------------------------------------
# 1. CRDT correctness — the deepest change, nothing depends on the rest.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-core/src/tree crates/outl-core/src/op.rs crates/outl-core/src/log.rs \
  crates/outl-core/tests/convergence_property.rs crates/outl-core/tests/convergence_gen \
  crates/outl-core/tests/create_undo_symmetry.rs crates/outl-core/tests/create_tree_invariants.rs \
  crates/outl-core/tests/create_undo_after_snapshot_boot.rs \
  crates/outl-core/tests/duplicate_create_convergence_property.rs \
  crates/outl-core/tests/tree_shape_property.rs crates/outl-core/tests/no_silent_loss_property.rs \
  crates/outl-core/tests/old_field_divergence_property.rs \
  docs/rfcs/0263-create-is-invertible.md docs/crdt.md
fix(core): make Op::Create invertible so a duplicate cannot delete a node

do_op(Create) is idempotent and undo_op(Create) removed the node
unconditionally, so the pair was not an inverse. apply_op's reorder loop
then diverged for any node carrying two Creates.

That is not an exotic shape. Page and journal roots are addressed by
NodeId::from_slug, a deterministic hash, precisely so two devices
creating the same page offline land on one node — so every journal opened
on two devices produces a duplicate Create. The effect was worse than
reordering: after the spurious remove, the following Move found no node
and became a no-op, so a page deleted on one device reappeared under root
depending on delivery order.

Tree::created_by records which op actually created a node, kept on the
local side rather than on the wire — where the paper keeps it (§3.2), and
where Op::Move::old_parent already demonstrates the cost of the
alternative. No Op variant changed, so the JSONL is byte-identical in
both directions and there is nothing to migrate.
EOF

# ---------------------------------------------------------------------
# 2. HLC — seeding, plus the two defects seeding made reachable.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-core/src/hlc.rs crates/outl-core/src/workspace/clock.rs \
  crates/outl-core/tests/hlc_total_order_property.rs \
  crates/outl-ws/src crates/outl-ws/tests crates/outl-ws/Cargo.toml \
  crates/outl-tui/src/actions/lifecycle \
  crates/outl-actions/src/sync.rs crates/outl-actions/src/sync
fix(core): seed the HLC from the log, and bound what it will absorb

The generator started from the wall clock with no reference to the log,
so after a backwards clock jump a new op sorted below ops already on
disk. A real workspace carried 11 such events, the widest 2.7 days. This
was never a convergence bug — apply_op reorders to the same tree — it
cost the undo/redo window on a foreground keystroke.

Seeding made two previously unreachable defects reachable, so both are
fixed here rather than left for later:

- the logical counter is raised from a u32 read off disk, so saturating
  at u32::MAX would pin next() on one value forever; Workspace::apply
  then dedups every later local op by ts and returns Ok(()) without
  persisting. Silent total write loss reported as success. It now carries
  into physical_ms instead.
- the seed folded every actor's maximum with no ceiling, and the result
  is persisted, so one far-future timestamp pinned the clock forward
  permanently and irreversibly for that workspace. It is clamped to
  hlc::MAX_CLOCK_SKEW_MS, which is now the single owner of that window —
  outl-sync-iroh's ingest gate uses the same constant instead of its own
  literal.
EOF

# ---------------------------------------------------------------------
# 3. Storage: one owner for index sidecars, and the write that was 50x slow.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-core/src/storage/sidecar crates/outl-core/src/storage/index.rs \
  crates/outl-core/src/storage/node_index.rs crates/outl-core/src/storage/mod.rs \
  crates/outl-core/src/storage/jsonl crates/outl-core/tests/sidecar_lifecycle.rs \
  crates/outl-config docs/rfcs/0265-index-sidecar-lifecycle.md
perf(core): buffer sidecar writes and give index sidecars one owner

A cold boot on a 217k-op workspace took 70 seconds, and 99.31% of it was
sidecar::save_entries: write_atomic handed out a bare File and the writer
did one writeln! per entry, so building the indexes cost 435,622
unbuffered write(2) calls. A 256 KiB BufWriter takes that to ~1s,
measured at 50x on the same fixture. The flush is into_inner(), not Drop,
because Drop discards the error and this path's whole job is durability.

The 84 MB of abandoned .idx.tmp scratch in a real ops/ was a consequence
of that window, not an independent leak: a 70-second write gets
interrupted.

index.rs and node_index.rs each derived their own sidecar paths and
repeated the temp-and-rename dance, which is how an entire earlier
generation of undotted sidecars survived unnoticed — 50 MB of files no
reader composes. sidecar:: now owns the naming, the write and the
lifecycle, so anything invalidating byte offsets asks for the complete
set instead of listing names it happens to remember.
EOF

# ---------------------------------------------------------------------
# 4. Snapshot cache GC.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-core/src/snapshot.rs crates/outl-core/src/snapshot \
  crates/outl-core/src/workspace/snapshot_policy.rs \
  crates/outl-core/tests/snapshot_without_cutoff_is_refused.rs \
  docs/rfcs/0258-snapshot-cache-lifecycle.md
feat(core): collect boot snapshots the selector can no longer choose

Nothing ever deleted a snapshot, so .outl/snapshots/ gained one
snap-<actor>.bin per actor that ever wrote there and lost none — 54 MB in
four files on a real workspace, one of them a schema-3 bincode body no
current build can read.

The rule is about the reader, not the actor. "Drop the snapshots of
actors that are gone" is unanswerable (an unpulled peer log, an
undownloaded iCloud placeholder and a dead actor are one observation) and
also irrelevant, because a snapshot is never read by its own actor. The
deciding question is whether the boot selector can compose that name
again, which the directory alone answers.

Prunable is Superseded or Unusable. Own, Selected, a newer schema, and
anything we failed to read are kept: could not read is not read and
proved bad.
EOF

# ---------------------------------------------------------------------
# 5. Compaction — depends on sidecar (3) for invalidation.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-core/src/storage/compact crates/outl-cli/src/cmd/compact.rs \
  crates/outl-core/tests/compaction.rs crates/outl-core/tests/compaction_property.rs \
  crates/outl-core/tests/compaction_refusals.rs \
  crates/outl-core/tests/compaction_property.proptest-regressions \
  docs/rfcs/0256-op-log-compaction.md docs/storage.md
feat(core): outl compact, dropping ops that provably changed nothing

The op log is append-only and nothing compacted it, so it replays on
every boot and ships whole to every newly paired device. A real workspace
carried 262 MB of ops/ for 28 MB of markdown, and 62,209 of its 217,811
ops were Moves restating the placement their own Create had just made.

The predicate is six conditions, not one, because Op::Create is
idempotent: the same adjacent pair reads as "the Move is inert" or as
trashed-then-restored, where the Move is the op doing the work and
dropping it relocates a user's block. The naive rule matched 99.9% of
Moves; the sound one matches 94.7%, and the 2,074 it declines are exactly
the restored ones.

Two things it refuses, both found by writing the refusal tests:

- it rewrites only this device's ops-<actor>.jsonl. Every file is
  append-only and owned by one writer, which is what makes
  transport = "file" safe; rewriting a peer's file makes a shortened
  competing version of one path, and iCloud resolves that last-write-
  wins. --force is there for workspaces with no file transport.
- the 30-day horizon anchors on min(newest op, wall clock). Anchored on
  the log alone, one peer with a fast clock disarmed it for the whole
  log and the user silently got --no-horizon behaviour.
EOF

# ---------------------------------------------------------------------
# 6. Block text: index instead of scanning.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-core/src/content.rs crates/outl-core/src/workspace.rs \
  crates/outl-core/src/workspace/tests.rs \
  crates/outl-core/tests/block_text_is_indexed_not_scanned.rs
perf(core): read a block's edits from the index, not the whole log

ensure_doc and materialize_text_from_log each walked every op in the log
to rebuild one block's text, so the first keystroke on a cold block after
a full-replay boot paid an O(217k) scan. Both now route through one
doc_from_log built on OpLog::edit_updates, which removes the duplication
and both scans together.

This is the third instance of the bug #179 fixed in block_text, so it
comes with a guard. A scan and an index return identical bytes — which is
exactly why it came back twice and why no correctness test can catch it —
so the guard measures the ratio across log sizes rather than a duration,
and does not care how fast the machine is.
EOF

# ---------------------------------------------------------------------
# 7. Benchmarks (Cargo.toml declares them; keep with the harness).
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-core/benches crates/outl-core/Cargo.toml Cargo.lock
test(core): add a benchmark harness for the CRDT and boot paths

outl-core had no benchmarks, so "the CRDT is fast" was unverifiable and
every performance claim about it was an argument rather than a number.

The first results are already worth having, and two of them refute
hypotheses this branch started from: the apply_op reorder window is 0 for
all 217,663 ops on a real log, and nothing quadratic fires during boot —
the whole CRDT is 68 ms of what was a 70-second cold boot. contains_ts is
flat at 36→41 ns from 10k to 218k ops, and edit_updates flat at 49→44 ns
as the surrounding log grows 68x.
EOF

# ---------------------------------------------------------------------
# 8. Markdown: the producer and the invariant-8 guard.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-md/src crates/outl-md/tests crates/outl-md/CLAUDE.md \
  docs/markdown-format.md
fix(md): stop the guard freezing a page whose every line the log holds

content_lines_missing_from tried the bullet-stripped reading of an
indented line first. known is a multiset that decrements, so on a block
whose text holds a fenced bullet, the continuation "- j" stripped to "j"
and spent the single "j" the log held for the last line, which then
matched nothing. A page the log fully accounts for reported one unlogged
line and froze in both directions, and the recovery the error names
re-runs the same computation and refuses again.

The renderer only writes "- " as a marker on a block's first line, which
is never indented, so for an indented line the verbatim reading is the
likely one. Both orders try both keys, so the set of lines that find a
hit is unchanged and only which entry is consumed differs: the change
cannot introduce a false negative, which is the byte-deleting direction.

Also here: the parser now preserves a leading newline, both CommonMark
fence characters and a UTF-8 BOM, each of which produced a line the
producer could emit but not read back.
EOF

# ---------------------------------------------------------------------
# 9. Actions: the tree -> .md executor.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-actions/src/journal crates/outl-actions/src/lib.rs \
  crates/outl-actions/src/refusal.rs docs/rfcs/0260-tree-to-md-executor.md \
  docs/clients.md
feat(actions): give the tree -> .md direction an executor

The .md -> tree direction has had a permanent executor since the file
watcher existed. The reverse had none: when ops arrived by sync, or a
render-affecting fix landed, a page's .md stayed wrong until somebody
happened to open it. A real 2,574-page workspace carried 704 stale pages,
702 of them safe to re-project with nothing removed.

The sweep writes only pages whose re-projection removes nothing from
disk. A page that would remove content is withheld for
`outl doctor --repair`, which backs every file up and applies volume
ceilings: a background pass that deletes is one that needs an undo, and
the command with the undo already exists. That gate also makes a torn op
log safe without the sweep knowing anything about op-log health, since a
truncated replay renders less than disk holds.

An undownloaded iCloud file reads as NotFound, which is indistinguishable
from "page deleted", so both the survey and the writer now ask
guard_absent_markdown — the same function, so a read-only listing cannot
promise a repair the writing pass refuses.
EOF

# ---------------------------------------------------------------------
# 10. Actions: the 500x traversal.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-actions/src/tree.rs crates/outl-actions/src/backlinks_index.rs \
  crates/outl-actions/src/backlinks.rs crates/outl-actions/src/outline.rs \
  crates/outl-actions/src/mentions.rs crates/outl-actions/src/asset.rs \
  crates/outl-actions/src/timeline.rs crates/outl-actions/src/reminders \
  crates/outl-actions/tests crates/outl-actions/CLAUDE.md
perf(actions): stop rescanning every node to list one node's children

Tree exposes no children accessor, so children_of scanned the whole node
map per call and every caller that forgot to build an index by hand paid
O(n^2). Measured on 64k nodes: walk_subtree(ROOT) 6,832 ms against 13.2
ms through a scoped index, and the outl serve sweep 8,273 ms against
~500 ms.

The index is level-batched and scoped, not a whole-workspace map:
building the global map costs ~5 ms, so using it per page made a page
walk slower than the quadratic it replaced. Two agents hit that trap from
opposite ends and both caught it by measuring, which is why the seam sits
where it pays.

Sibling order gained a NodeId tiebreak in the same pass. Fractional alone
is not a total order once two devices append offline, and Tree::iter_nodes
walks a per-process-seeded HashMap, so one op log rendered a different
block order on every boot and on every device.
EOF

# ---------------------------------------------------------------------
# 11. Sync: authorization.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-sync-iroh crates/outl-mobile/src-tauri/src \
  docs/rfcs/0155-peer-trust.md docs/iroh-internals.md docs/sync.md
fix(sync): authorize snapshot and asset reads, so revocation revokes

SNAPSHOT_ALPN and ASSET_ALPN did no authorization at all — they served
the whole graph and every asset to any dialer — while SYNC_ALPN on the
same endpoint did a full fail-closed peers.json check. outl peer remove
therefore revoked the op exchange and nothing else: a removed device kept
pulling everything.

All four call sites now route through one authz::authorize_or_close, and
the asset path checks per request rather than per connection, so
revocation lands mid-stream instead of waiting for the peer to hang up.
Membership gossip could also add arbitrary node ids to peers.json,
manufacturing authorization; that hole is pinned by an ignored test
rather than asserted as current behaviour, because closing it is a
protocol decision.

Deny cases outnumber allow cases in the new suite, and the allow cases
assert real payload bytes so a check that refuses everything fails loudly.
EOF

# ---------------------------------------------------------------------
# 12. CLI: the clap split (also carries compact's --force flag).
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-cli/src/main.rs crates/outl-cli/src/cli.rs crates/outl-cli/src/startup.rs \
  crates/outl-cli/src/cmd/mod.rs crates/outl-cli/src/cmd/init.rs \
  crates/outl-cli/src/cmd/reconcile.rs crates/outl-cli/CLAUDE.md
refactor(cli): split main.rs into a clap surface and a dispatcher

main.rs was 1,085 lines holding the whole clap declaration and the
dispatch for every subcommand, which made it the file every unrelated
change had to touch. The declarations move to cli.rs (with the parse
tests) and the path/bootstrap/tracing helpers to startup.rs, leaving
main.rs at 510 lines of dispatch.
EOF

# ---------------------------------------------------------------------
# 13. CLI: doctor.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-cli/src/cmd/doctor crates/outl-cli/tests
feat(cli): doctor reports and repairs the caches that had no owner

check_snapshots kept its own read-and-decode loop and its own notion of
"corrupt", which made it a second owner of a verdict outl-core now owns.
It asks snapshot::gc::survey instead and only phrases the result, and
--repair re-asks at write time rather than trusting the plan.

A superseded snapshot reports as info, not a warning: nothing is wrong,
there is disk to reclaim, and a healthy workspace should not read as sick
every time the background writer publishes. One that could not be judged
is reported and never deleted.

The same pass gained index-sidecar collection, which is where the 134 MB
of dead generations in a real ops/ goes.
EOF

# ---------------------------------------------------------------------
# 14. CLI: serve.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-cli/src/cmd/serve.rs crates/outl-cli/src/cmd/serve docs/cli.md
feat(cli): serve sweeps the tree -> .md direction

serve already held the watcher and the endpoint; it now also runs the
projection sweep, so the direction that had no executor gets one without
a second daemon, a second launchd job and a second thing that can die
quietly. --no-watch deliberately does not sweep: its contract is holding
the endpoint and nothing else.

It reports the change, not the state — pages needing attention are named
on the first sweep that sees them, again when the set differs, and once
when it empties. A daemon naming a frozen page 2,880 times a day is the
silence this was built to end, with more lines.
EOF

# ---------------------------------------------------------------------
# 15. Shortcuts + clients parity.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-shortcuts crates/outl-frontend-shared crates/outl-mobile/src \
  crates/outl-desktop docs/shortcuts.md docs/client-parity.md
fix(shortcuts): correct chords and capability rows that claimed too much

The parity table declared WrapBold, WrapItalic, WrapCode, WrapStrike and
InsertLink as supported on all three clients; the TUI has no chord for
any of them. The table that exists so one fact has one answer was itself
asserting support that does not exist.

zR and zM were spelled with pair(), which lowercases, so the catalog
emitted z then lowercase r while the TUI matched uppercase. Worse, z then
lowercase r falls through to the bare-key arm that arms replace-char, so
the catalog was handing out a chord that edits the user's block under the
name "unfold all".
EOF

# ---------------------------------------------------------------------
# 16. Tauri wire boundary.
# ---------------------------------------------------------------------
commit <<'EOF' \
  crates/outl-tauri-shared
test(tauri): pin the hand-written Rust <-> TS wire boundary

The DTO mirror is hand-written on purpose, and the pins are what make
that safe. Pin coverage was 42% rather than the 76% the gate reported
about itself, and the mechanism could not pin enums at all — which is why
ParseWarningKind shipped one of its six variants to TypeScript.
EOF

# ---------------------------------------------------------------------
# 17. CI + docs that are not tied to one change above.
# ---------------------------------------------------------------------
commit <<'EOF' \
  scripts/check-docs-index.sh scripts/commit-branch.sh .github
ci: fail when an RFC is not reachable from the docs index

The index was silently wrong five times in a row: one branch added five
RFCs, including the two documenting the changes most capable of
destroying data, and listed none of them. It was complete each time it
was last edited — five separate changes each forgot the same step, and
nothing failed when they did.

check-docs-index.sh checks reachability, not wording: on failure it
prints the RFC's real title but tells the author to write the line, since
a script that generated the summary would become a second owner of that
fact.

EOF

# ---------------------------------------------------------------------
# 18. Documentation not tied to one change above.
# ---------------------------------------------------------------------
commit <<'EOF' \
  docs CLAUDE.md crates/outl-core/CLAUDE.md crates/outl-sync-iroh/CLAUDE.md \
  crates/outl-shortcuts/CLAUDE.md crates/outl-tauri-shared/CLAUDE.md \
  crates/outl-tui crates/outl-ws
docs: correct claims this branch falsified, and one that was never true

uhlc was named in six places, including the "decisions you don't get to
revisit" table and a failure-mode row listing it as the mitigation for
HLC clock skew. It is in no manifest and never was, so that row promised
a control that does not exist — which is worse than an untracked risk,
because it stops anyone looking.

Four documents were also past the size ceiling that blocks edits to
them, two of them pushed there by this branch. The detail moved to where
it belongs and the rule stayed: device-store internals to
docs/device-store.md, the endpoint lease and blob protocol to
docs/iroh-internals.md, the parser's preservation rules to
docs/markdown-format.md, and outl-actions' module reference to
docs/outl-actions-surface.md — that last one because primitives-actions
was carrying two taxonomies, an intent catalog and a module reference,
in one 77k file.
EOF

# ---------------------------------------------------------------------
# Anything left over is a bug in this script, not something to ignore.
# ---------------------------------------------------------------------
printf '\n'
LEFT="$(git status --porcelain | wc -l | tr -d ' ')"
if [ "$DRY_RUN" = "1" ]; then
  # The one failure mode of this script that loses work silently is a
  # file no group claims, so the dry run checks for it too — that is the
  # whole reason to run it before the real thing.
  left="$(comm -23 \
    <(git status --porcelain | sed 's/^...//' | sed 's/.* -> //' | sort -u) \
    <(sort -u "$CLAIMED"))"
  rm -f "$CLAIMED"
  if [ -n "$left" ]; then
    printf 'UNCLAIMED — no group would commit these:\n\n'
    printf '%s\n' "$left" | sed 's/^/  /'
    printf '\nadd them to a group before running for real.\n'
    exit 1
  fi
  printf 'every changed file is claimed by a group. nothing was staged or committed.\n'
  printf 'run without --dry-run to apply.\n'
  exit 0
fi
rm -f "$CLAIMED"

if [ "$LEFT" != "0" ]; then
  printf 'WARNING: %s file(s) were not claimed by any group:\n\n' "$LEFT" >&2
  git status --porcelain >&2
  printf '\nthey are still uncommitted. add them to a group in this script,\n' >&2
  printf 'or commit them by hand before pushing.\n' >&2
  exit 1
fi

printf '%s commit(s) created on %s.\n\n' "$committed" "$BRANCH"
git --no-pager log --oneline "main..$BRANCH"

printf '\nverifying the tree builds at the tip (intermediate commits may not):\n'
if cargo fmt --all -- --check >/dev/null 2>&1; then
  printf '  fmt     ok\n'
else
  printf '  fmt     FAILED — run `cargo fmt --all`\n' >&2
fi
if cargo clippy --workspace --all-targets -- -D warnings >/dev/null 2>&1; then
  printf '  clippy  ok\n'
else
  printf '  clippy  FAILED\n' >&2
fi

printf '\nnothing was pushed. when you are ready:\n'
printf '  git push -u origin %s\n' "$BRANCH"
