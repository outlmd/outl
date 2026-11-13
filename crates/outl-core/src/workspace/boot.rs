//! How a workspace comes up: adopt a snapshot and replay the delta, or
//! replay the whole op log.
//!
//! Split out of `workspace.rs` because it is the one part of `Workspace`
//! a user waits on. Everything else in that file answers a question
//! about the document; this answers "how long until the app is usable",
//! which is the premise the project is built on and the thing most
//! likely to be measured, argued about and changed.
//!
//! The two paths are not variants of one algorithm and must not be
//! merged into one. A full replay rebuilds from ops and leaves block
//! text deferred against the **resident** log. A snapshot boot hydrates
//! settled state and leaves the delta's edited nodes deferred against
//! **storage** — the resident log holds only the delta there, so
//! rebuilding one of those from it would silently drop every pre-cutoff
//! edit (#129). `ContentStore` keeps the two deferred sets apart for
//! exactly that reason.
//!
//! Per-phase timings land on one `info!` line via
//! [`crate::boot_timing`]; read them on a device rather than guessing
//! from a developer machine.

use std::collections::HashSet;

use tracing::warn;

use crate::content::ContentStore;
use crate::id::NodeId;
use crate::op::Op;
use crate::snapshot;
use crate::tree::Tree;

use super::{Workspace, WorkspaceError};

impl Workspace {
    /// Whether this workspace booted by adopting a snapshot (so the
    /// in-memory log holds only the post-cutoff delta) rather than a full
    /// replay (log holds every op).
    ///
    /// A receive-only device whose snapshot was rejected/absent/stale
    /// full-replays, leaving a huge resident log that makes `block_text`
    /// materialization O(log) per block (it scans the log for each node's
    /// `Edit`s). The caller uses this to persist a FRESH snapshot right
    /// after such a boot, so the next boot adopts it (delta-only log) and
    /// materialization stays cheap.
    pub fn booted_from_snapshot(&self) -> bool {
        !self.log_complete
    }

    /// Try to hydrate the workspace from a snapshot + the ops posted
    /// since its cutoff. Returns `Ok(false)` when there's nothing to
    /// load (so the caller falls through to [`Self::boot_from_full_replay`]).
    pub(super) fn boot_from_snapshot(&mut self) -> Result<bool, WorkspaceError> {
        // Snapshots are a local boot cache owned by the workspace, read
        // straight from `<root>/.outl/snapshots` — not routed through the
        // storage backend (the op log). An in-memory workspace
        // (`root = None`) has nowhere to read from, so it never boots
        // from snapshot.
        let snapshots_dir = match self.snapshots.dir() {
            Some(d) => d.to_path_buf(),
            None => return Ok(false),
        };
        // Prefer this device's own snapshot; when it has none yet (a fresh
        // paired device), adopt a peer's snapshot so we skip a full replay
        // of a 200k-op log. Local input is preserved either way — the
        // per-actor delta below replays every op above the snapshot's
        // cutoff, including this device's own. See `read_best_from_disk`.
        let mut timer = crate::boot_timing::BootTimer::start();
        let body = match snapshot::read_best_from_disk(&snapshots_dir, self.actor) {
            Ok(Some(b)) => b,
            Ok(None) => return Ok(false),
            Err(e) => {
                // Corrupt / stale / unreadable snapshot is never fatal —
                // it's a cache. Fall back to a full op-log replay.
                warn!("snapshot unusable, falling back to full replay: {e}");
                return Ok(false);
            }
        };

        // Compute the delta the snapshot hasn't seen — per actor, so a
        // low-HLC op from an actor the snapshot never saw isn't dropped
        // below another actor's high-water mark (#156).
        timer.phase("snap_decode");
        let delta = match self.ops_since_per_actor_combined(&body.cutoff) {
            Ok(d) => d,
            Err(e) => {
                // An op the index lists but the file won't return. NOT
                // fatal, and emphatically not something to skip past:
                // the cutoff of the snapshot we write next comes from
                // the index, so adopting a delta that quietly omitted
                // this op would record it as "already folded in" and no
                // future boot would replay it. Fall back to a full
                // sequential replay, which re-reads the file and can
                // still recover everything around the damaged line.
                warn!("snapshot delta unreadable, falling back to full replay: {e}");
                return Ok(false);
            }
        };

        // CONVERGENCE GUARD (invariant #1). The snapshot body is an opaque
        // MATERIALIZED tree, not a reorderable log, so applying the delta on
        // top equals a full replay ONLY when the delta is a pure temporal
        // SUFFIX — every delta op newer than every op folded into the body.
        // If a delta op sorts at/below a body op, the CRDT would need to
        // reorder it against ops that live only in the (opaque) tree, so a
        // cycle-forming `Move` can resolve the opposite way and the tree
        // diverges from a full replay. The per-actor cutoff makes this
        // reachable — a peer op below our cutoff, or an adopted peer
        // snapshot that excludes all our local ops. Bail to a full replay:
        // correct over fast. (No-silent-loss #5 holds regardless — the op is
        // always in the log; this only decides the materialization order.)
        //
        // Also load-bearing for `Tree::created_by`: it is what guarantees
        // the resident log holds no op that could undo past the snapshot
        // body, which is why `from_parts` may leave `created_by` empty.
        // RFC 0263. So this guard now has two independent reasons, and
        // relaxing it for the first would silently re-arm the second —
        // a duplicate `Create` that `undo_op` removes unconditionally,
        // the exact defect RFC 0263 fixed.
        //
        // An ABSENT or EMPTY `cutoff` is not "nothing to compare against" —
        // it is a body we CANNOT check, and an uncheckable body is exactly
        // what this guard exists to refuse. `build_snapshot_body` returns
        // `None` rather than an empty cutoff, so no honest producer makes
        // one: the only way to hold it is a forged or corrupt peer snapshot.
        // And it is worse than merely "unchecked" — every actor absent from
        // the cutoff reads as "seen none of your ops", so
        // `ops_since_per_actor` yields the WHOLE local log, replayed on top
        // of an attacker-chosen tree. That is the maximal version of the
        // divergence above, plus a duplicate `Create` for every node the
        // forged body seeds. `if let Some(..)` read that as permission.
        //
        // `let .. else`, not a second `if`, so the guard cannot be re-nested
        // into an `if let` by a later edit — that nesting *was* the bug.
        let Some(max_body_hlc) = body.cutoff.values().max().copied() else {
            warn!("snapshot carries no per-actor cutoff; falling back to full replay");
            return Ok(false);
        };
        if delta.iter().any(|op| op.ts <= max_body_hlc) {
            return Ok(false);
        }

        // Safe to adopt: hydrate the materialized tree + block text from the
        // snapshot body, then apply the suffix delta on top. No `Edit` ops
        // are replayed for the body nodes; `block_text` already carries
        // their settled string. Convert BTreeMap → HashMap to match the
        // runtime representation (ContentStore's hot path is a HashMap).
        timer.phase("delta_read");
        let delta_len = delta.len();
        self.tree = Tree::from_parts(
            body.nodes.into_iter().collect(),
            body.properties.into_iter().collect(),
            body.collapsed.into_iter().collect(),
            body.snoozed.into_iter().collect(),
        );
        timer.phase("tree_hydrate");
        let text_len = body.block_text.len();
        self.content = ContentStore::from_text_map(body.block_text.into_iter().collect());
        timer.phase("text_hydrate");

        // `Edit` ops in the delta are re-materialized in the loop below;
        // the rest of the snapshot's text is still authoritative.
        let mut edited_in_delta: HashSet<NodeId> = HashSet::new();
        for op in delta {
            if let Op::Edit { node, .. } = &op.op {
                edited_in_delta.insert(*node);
            }
            self.tree.apply_op(&mut self.log, op);
        }

        // Record which nodes the delta edited, and rebuild each on first
        // read rather than here.
        //
        // The rebuild itself is unchanged and still mandatory: the
        // snapshot's string for one of these predates an `Edit` above the
        // cutoff, and the resident log holds only the delta, so the FULL
        // `Edit` history has to come back from storage. Replaying all of
        // it through a fresh `Doc` is what makes the text match the
        // full-replay path (#129). What moved is *when*.
        //
        // Doing it here made a launch pay for every block a peer had
        // touched since the last snapshot, to paint a journal that shows
        // about twenty. On a real workspace that is 2,298 nodes and 68 ms
        // — and, far worse, it is the only thing on the boot path that
        // reads the per-node index at all, so it forced 98 MB of sidecar
        // to load before the first frame (`storage/jsonl/index_load.rs`).
        //
        // `set_stale` is what keeps the deferral honest: the cached string
        // for these is present and wrong, so `block_text` checks the set
        // *before* the cache, and `force_materialize_pending` drains it
        // before a snapshot body can be written from it.
        timer.phase("delta_apply");
        let rematerialized_count = edited_in_delta.len();
        self.content.set_stale(edited_in_delta);

        timer.phase("mark_stale");
        timer.finish(&format!(
            "snapshot actors={} nodes={} delta_ops={delta_len} text_entries={text_len} \
             rematerialized={rematerialized_count}",
            body.cutoff.len(),
            self.tree.node_count(),
        ));

        Ok(true)
    }

    /// Boot by replaying every op in storage. The historical path; used
    /// as the snapshot-miss fallback and by `open_in_memory`.
    pub(super) fn boot_from_full_replay(&mut self) -> Result<(), WorkspaceError> {
        // Pass 1: structural. Apply every op to the tree (`Edit` is a
        // no-op there) and the log. This keeps the tree FULLY materialized;
        // only block text goes lazy below.
        let ops = self.all_ops_combined()?;
        for op in ops {
            self.tree.apply_op(&mut self.log, op);
        }

        // Pass 2: defer text. Materializing every block's `Doc` here was
        // O(all blocks) — a major boot freeze on large snapshotless vaults
        // (a 66k-block install-clean froze the mobile app, #179). Instead
        // record which nodes carry `Edit` history (indices-free, one cheap
        // scan) and let `block_text` rebuild each string lazily on first
        // read. `self.log` is complete here (`log_complete == true`), so a
        // lazy rebuild sees the node's full `Edit` set and produces
        // byte-identical text to the old eager pass.
        let edited: HashSet<NodeId> = self
            .log
            .iter()
            .filter_map(|logged| match &logged.op {
                Op::Edit { node, .. } => Some(*node),
                _ => None,
            })
            .collect();
        self.content.set_pending(edited);

        // Seed the snapshot counter so a long-lived workspace opened
        // without a snapshot on disk doesn't have to wait a full
        // `threshold` worth of new ops before producing one. If the log
        // already crosses the threshold, the next `apply` snapshots.
        self.snapshots.seed_from_log_len(self.log.len());

        Ok(())
    }
}
