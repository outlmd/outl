//! Reconcile a single `.md` file against a `Workspace`.
//!
//! Used by `outl serve` (file watcher), `outl init` (initial seed), and
//! `outl-tui` (after editor commits). Reads the `.md`, optionally loads
//! the sidecar, runs 3-level matching, emits the minimal op sequence,
//! applies it, and writes back the refreshed sidecar.
//!
//! Orphan ids are logged before being moved to `TRASH_ROOT`, so a
//! deletion is never silent.
//!
//! This file is the **pass** — the order the steps run in, and the two
//! decisions only the pass can make: whether the page can be
//! short-circuited, and whether `last_synced_hash` may be advanced
//! (invariant 8). Each step it calls lives next to it:
//!
//! - `outcome` — what a pass hands back ([`ReconcileReport`],
//!   [`ReconcileError`]).
//! - `page_root` — the page node: its id, its rooting under
//!   `NodeId::root`, and its page-level properties, frontmatter fence
//!   included ([`ensure_page_root_in_tree`]).
//! - `text_sync` — the `Op::Edit` pass that gives the created nodes
//!   their text.
//! - `orphan_log` — the record written before an orphan is trashed.

mod orphan_log;
mod outcome;
mod page_root;
mod text_sync;

pub use outcome::{ReconcileError, ReconcileReport};
pub use page_root::ensure_page_root_in_tree;

use crate::parse::parse;
use crate::sidecar::{self, file_hash, sidecar_path_for, Sidecar, SIDECAR_VERSION};
use outcome::io_err;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::op::LogOp;
use outl_core::workspace::Workspace;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Reconcile a single `.md` file with the workspace, refusing a
/// deletion large enough to be an accident.
///
/// `orphan_log_path` receives one line per orphan id surfaced during
/// matching. Pass `None` to suppress logging (mostly useful for tests).
///
/// This is the only production path that turns matching orphans into
/// `Move(node, TRASH_ROOT)`, so it is where the volume guard has to be —
/// see `matching::guard` for the thresholds and the reasoning. Use
/// [`reconcile_md_with_guard`] with
/// [`OrphanGuard::Disabled`](crate::matching::guard::OrphanGuard::Disabled)
/// to apply a deletion the user has confirmed.
pub fn reconcile_md(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    orphan_log_path: Option<&Path>,
) -> Result<ReconcileReport, ReconcileError> {
    reconcile_md_with_guard(
        ws,
        hlc,
        md_path,
        orphan_log_path,
        &crate::matching::guard::OrphanGuard::Enforced,
    )
}

/// [`reconcile_md`] with an explicit orphan-volume policy.
///
/// Separate entry point rather than a parameter on `reconcile_md`
/// because the guarded form is what every caller should want; only the
/// one wired to a user saying "yes, I meant to delete that" passes
/// [`OrphanGuard::Disabled`](crate::matching::guard::OrphanGuard::Disabled).
pub fn reconcile_md_with_guard(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    md_path: &Path,
    orphan_log_path: Option<&Path>,
    guard: &crate::matching::guard::OrphanGuard,
) -> Result<ReconcileReport, ReconcileError> {
    let md_text = fs::read_to_string(md_path).map_err(|e| io_err(md_path, e))?;
    let new_ast = parse(&md_text);
    let md_hash = file_hash(&md_text);

    let sidecar_path = sidecar_path_for(md_path);
    // Read the sidecar **once** — both the diff (which needs `page_id`
    // and `old_blocks`) and the short-circuit below (which needs
    // `last_synced_hash` + `pipeline_version`) consume the same JSON.
    // A previous version read it twice with no lock between the
    // reads, leaving a window where another process could rewrite
    // the file mid-call and the two reads would disagree.
    let existing_sidecar: Option<Sidecar> = match sidecar::read(&sidecar_path) {
        Ok(sc) => Some(sc),
        Err(sidecar::SidecarError::Io(e)) if e.kind() == io::ErrorKind::NotFound => None,
        // A sidecar written by a **newer** binary is not corruption —
        // it is a mixed-version workspace (the mobile build on
        // TestFlight lags the desktop by days). Rebuilding "from
        // scratch" here would hand every block a fresh ULID while the
        // old ids stay in the tree: the page duplicates, every
        // `((blk-…))` handle changes, and the next boot of the newer
        // binary does the same in reverse. Refuse the page loudly and
        // leave the workspace untouched; the caller logs and skips it.
        // See `SIDECAR_VERSION` for why a version bump is a
        // fleet-coordinated break rather than a patch.
        Err(e @ sidecar::SidecarError::UnsupportedVersion(_)) => return Err(e.into()),
        Err(_) => None, // Corrupt JSON — rebuild from scratch.
    };
    let (page_id, old_blocks, created_sidecar) = match &existing_sidecar {
        Some(sc) => (sc.page_id, sc.blocks.clone(), false),
        // No sidecar → derive the page-root id **from the slug**, never
        // a fresh `NodeId::new()`. A page/journal root's identity is its
        // slug (see `NodeId::from_slug`): minting a time-based ULID here
        // is exactly what split the day's journal across two competing
        // roots — one deterministic, one time-based — when the same
        // `journals/YYYY-MM-DD.md` was reconciled on a device that had
        // no `.outl` yet (external editor, peer that shipped only the
        // `.md`, crash before the sidecar landed). Deriving from the
        // slug makes every such path converge on the one node.
        None => (page_root::page_id_from_stem(md_path), Vec::new(), true),
    };

    // Short-circuit: file unchanged since last sync AND the sidecar
    // was produced by the current reconcile pipeline (or newer). The
    // `pipeline_version` clause is what triggers the one-shot
    // migration for sidecars predating `diff_to_ops_with_page_props`
    // and `ensure_page_root_in_tree` — without it, legacy pages whose
    // hash hasn't changed (the common case: fixtures, imports,
    // anything authored before the page-prop pipeline) skip the
    // migration silently and the desktop / mobile keep seeing empty
    // page properties while the `.md` shows them.
    if let Some(existing) = &existing_sidecar {
        if existing.last_synced_hash == md_hash
            && existing.pipeline_version >= crate::sidecar::CURRENT_PIPELINE_VERSION
        {
            return Ok(ReconcileReport {
                md_path: md_path.to_path_buf(),
                ops_applied: 0,
                orphans: 0,
                created_sidecar: false,
                unlogged_lines: 0,
            });
        }
    }

    // Guarded, not raw: the orphans this produces become
    // `Move(node, TRASH_ROOT)` a few lines down, and level 3 reports one
    // orphan and five thousand the same way. A `.md` that arrived
    // truncated — an undownloaded iCloud placeholder, a half-flushed
    // write — would empty the page as quietly as deleting a bullet.
    // Refusing here refuses before any op exists, since `match_blocks`
    // is pure.
    //
    // One exemption from the *count*, never from the deletion: a sidecar
    // from before the fence parser (issue #281) holds each YAML line as a
    // block, and those orphan on the first pass. They are the fence still on
    // disk, logged as `page-frontmatter` below, so counting them would
    // refuse the one pass that migrates the page. Only asked when this `.md`
    // has a fence, so a truncated file (no fence, no body) gets no discount.
    let fence_lines = match new_ast.frontmatter {
        Some(_) => crate::frontmatter::legacy_fence_lines(&md_text),
        None => Default::default(),
    };
    let (matches, orphans) = crate::matching::guard::match_blocks_guarded_except(
        &new_ast.blocks,
        &old_blocks,
        guard,
        |b| crate::frontmatter::is_legacy_fence_block(&fence_lines, &b.text),
    )?;

    if !orphans.is_empty() {
        if let Some(log_path) = orphan_log_path {
            orphan_log::log_orphans(log_path, md_path, &orphans, &old_blocks)?;
        }
    }

    let mut ops_applied = 0usize;

    // **Materialise the page root** as a child of `NodeId::root` with
    // `page-slug` + `page-kind` set. Without this, a `.md` authored
    // externally (vim, peer via iCloud, Roam import) emits `Create`
    // ops for the blocks (whose `parent` is the `page_id`) but leaves
    // the page node itself as an unrooted ghost. The CRDT happily
    // stores blocks under it, but `children_of(root)` doesn't list
    // it as a page — so `list_all_pages`, `search_persons`, and the
    // sidebar all miss it silently. The `WorkspaceIndex`-driven
    // surfaces (TUI autocomplete, picker preview) still see the page
    // because they parse `.md` from disk; that hid the bug.
    //
    // Each call is idempotent: we emit `Op::Move` / `Op::SetProp` only
    // when the workspace tree disagrees with what the filesystem says
    // the page should look like. Pages created via the UI
    // (`open_or_create_by_name`) already carry the right state, so
    // this is a no-op for them.
    ops_applied += ensure_page_root_in_tree(ws, hlc, page_id, md_path)?;

    // The YAML frontmatter fence is page metadata, not outline, so it
    // rides the op log as one `SetProp` on the page root — see
    // `page_root::sync_page_frontmatter` for why leaving it on disk
    // only is not an option.
    ops_applied +=
        page_root::sync_page_frontmatter(ws, hlc, page_id, new_ast.frontmatter.as_deref())?;

    // Feed the diff the nodes' CURRENT positions so an unchanged block
    // keeps its position and its `Move` stays a filtered-out no-op — see
    // `Workspace::op_is_noop` and the walk in `diff.rs`.
    let plan = {
        let current_pos = |id: NodeId| ws.tree().position(id).cloned();
        crate::diff::diff_to_ops_with_page_props(
            &new_ast.blocks,
            &matches,
            &orphans,
            page_id,
            &md_hash,
            &old_blocks,
            &new_ast.properties,
            &current_pos,
        )
    };

    // Coalesce the whole structural-diff pass into one storage flush.
    // This is the largest single burst of ops in the system (a fresh
    // boot or import emits Create/Move/SetProp for every block of every
    // page); paying one fsync per op here was the write bottleneck. The
    // batch keeps every op flowing through the CRDT one at a time (the
    // materialized tree stays correct for `op_is_noop`) but defers the
    // persist to a single `append_ops` on commit. On an error mid-loop
    // the guard drops and flushes the prefix best-effort, so the on-disk
    // state matches the pre-batch behaviour (the prefix was already
    // persisted op-by-op there too).
    {
        let mut batch = ws.begin_batch();
        for op in plan.ops {
            // The diff defensively re-emits `Create` + `Move` (+ `SetProp`)
            // for every block; skip the ones that wouldn't change the tree so
            // a one-block edit persists one op, not two per block in the page
            // — and the op log stops growing by the whole page on every
            // commit. See `Workspace::op_is_noop`.
            if batch.op_is_noop(&op) {
                continue;
            }
            let ts = hlc.next();
            let log_op = LogOp {
                ts,
                actor: ts.actor,
                op,
            };
            batch.apply(log_op)?;
            ops_applied += 1;
        }
        batch.commit()?;
    }

    // Synchronise block text with the workspace.
    //
    // `diff_to_ops` only knows about tree structure (Create / Move /
    // SetProp). It never emits `Op::Edit` because computing the Yrs
    // delta needs the live workspace, which isn't in its scope. The
    // result is a tree of nodes that exist but have empty text — fine
    // when the only consumer is the local sidecar (which carries the
    // content hash) but **catastrophic across devices**: a peer
    // replaying the op log materialises empty blocks, regenerates
    // `.md` from that empty state, and iCloud syncs the empty `.md`
    // back to us. Every text edit silently turns into a deletion.
    //
    // Fix: walk the new AST in lockstep with the freshly built
    // sidecar block list (same DFS preorder) and emit one
    // `Op::Edit` per block whose text doesn't match what the
    // workspace already has. Idempotent: `build_text_replace_update`
    // returns an empty update when text is unchanged.
    ops_applied += text_sync::sync_block_text(ws, hlc, &new_ast.blocks, &plan.new_sidecar.blocks)?;

    // **Invariant 8, enforced.**
    //
    // Writing `last_synced_hash` is a claim: *the op log holds what is in
    // this file*. Every consumer downstream believes it —
    // `apply_page_md_with_sidecar_if_stale` reads it as permission to
    // re-render the tree over these bytes, and `doctor` reads it as proof
    // the page is healthy. This is the one place that both reads a `.md`
    // and rewrites its sidecar, so it is the one place that can make the
    // claim falsely.
    //
    // Until now it made the claim unconditionally. When the parser
    // dropped a line it could not place, that line reached the `.md` and
    // never the log, the hash was stamped over the whole file anyway, and
    // the page became indistinguishable from a healthy one. Measured on a
    // real 2,560-page workspace: 41 pages holding 387 lines that existed
    // in no op, invisible to every check, deleted by the next
    // re-projection.
    //
    // The parser fix above closes the case we know about. This closes the
    // *class*: any future path that reads content it cannot emit an op
    // for leaves the page dirty instead of lying about it. A page that
    // reconciles twice is a nuisance; a page that lies about its own
    // state is a data-loss bug.
    //
    // Same question, same owner as both consumers
    // (`crate::unlogged::content_lines_missing_from`) — a second opinion
    // here about what counts as logged is how the producer and the guard
    // would drift apart.
    let unlogged = crate::unlogged::content_lines_missing_from(&md_text, &plan.new_sidecar.blocks);
    let unlogged_lines = unlogged.len();
    if let Some(sample) = unlogged.first() {
        tracing::warn!(
            path = %md_path.display(),
            lines = unlogged_lines,
            sample = %sample,
            "`.md` holds content this reconcile could not turn into ops; \
             leaving the page unsynced rather than claiming the log has it"
        );
    }

    let new_sidecar = Sidecar {
        version: SIDECAR_VERSION,
        page_id,
        // An empty hash never equals `file_hash(_)`, so the short-circuit
        // above misses on the next pass and the page is looked at again.
        // Deliberately not the *previous* hash: that would describe a
        // revision of the file that no longer exists on disk, and level-2
        // matching reads the sidecar as "what the log held at the last
        // agreement" — there was no agreement here.
        last_synced_hash: if unlogged.is_empty() {
            md_hash
        } else {
            String::new()
        },
        last_synced_at: plan.new_sidecar.last_synced_at,
        blocks: plan.new_sidecar.blocks,
        pipeline_version: plan.new_sidecar.pipeline_version,
    };
    sidecar::write(&sidecar_path, &new_sidecar)?;

    Ok(ReconcileReport {
        md_path: md_path.to_path_buf(),
        ops_applied,
        orphans: orphans.len(),
        created_sidecar,
        unlogged_lines,
    })
}

/// Scan a directory for `.md` files and reconcile each one.
pub fn reconcile_dir(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    dir: &Path,
    orphan_log_path: Option<&Path>,
) -> Result<Vec<ReconcileReport>, ReconcileError> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    let mut entries: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().is_file()
                && e.path().extension().is_some_and(|x| x == "md")
                && !e.file_name().to_string_lossy().starts_with('.')
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    entries.sort();
    for path in entries {
        out.push(reconcile_md(ws, hlc, &path, orphan_log_path)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
