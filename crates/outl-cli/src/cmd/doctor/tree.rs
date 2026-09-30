//! Checks that need the **materialized tree**, i.e. a booted
//! [`Workspace`].
//!
//! Everything here answers a question the on-disk bytes cannot:
//! "what did the op log actually build?".
//!
//! - [`check_trash`] — how much of the graph is deleted. Today the user
//!   has no way at all to see this: `Delete` is `Move(node, TRASH_ROOT)`
//!   (root `CLAUDE.md` invariant 6), so a deleted block is still in the
//!   tree, just parked under a sentinel that nothing renders.
//! - [`check_unmaterialized_ops`] — node ids the op log mentions that
//!   never landed in the tree.
//! - [`check_projections`] — every page in the tree vs its `.md` on
//!   disk, which is also where the `--repair` plan comes from.

use std::collections::HashSet;

use outl_actions::journal::{survey_page_projections, PageProjectionState};
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use super::repair::PageWrite;
use super::{Builder, Plan};

/// Cap on how many individual items each listing names.
const MAX_LISTED: usize = 20;

/// Report what sits in the trash, with a preview.
///
/// The walk, the subtree count and the preview all belong to
/// `outl_actions::trash::list` — the same listing `outl trash list` and
/// the `outl_trash_*` MCP tools render. This used to be its own
/// traversal, which is how the doctor could count 683 blocks that no
/// other surface could name (issue #287).
pub(super) fn check_trash(b: &mut Builder, ws: &Workspace) {
    let entries = outl_actions::trash::list(ws);
    if entries.is_empty() {
        b.ok("trash is empty — nothing has been deleted in this workspace");
        return;
    }

    let total: usize = entries.iter().map(|entry| entry.subtree_len).sum();
    b.info(format!(
        "trash holds {total} block(s) across {} top-level deletion(s) — \
         deletes are `Move(node, TRASH_ROOT)`, so nothing was physically removed",
        entries.len()
    ));

    for entry in entries.iter().take(MAX_LISTED) {
        b.info(format!("  trashed {}: {}", entry.node, entry.preview));
    }
    if entries.len() > MAX_LISTED {
        b.info(format!(
            "  … and {} more top-level deletion(s)",
            entries.len() - MAX_LISTED
        ));
    }

    let restorable = entries.iter().filter(|e| e.refusal.is_none()).count();
    b.info(format!(
        "  {restorable} of {} can be put back with `outl trash restore <id>`",
        entries.len()
    ));
}

/// Node ids the op log touches that are absent from the materialized
/// tree.
///
/// A node reaches the tree through `Create` or `Move`; an `Edit` /
/// `SetProp` / `SetCollapsed` on a node that never got either is an op
/// whose effect the user will never see. On a healthy workspace this is
/// zero.
pub(super) fn check_unmaterialized_ops(
    b: &mut Builder,
    ws: &Workspace,
    op_nodes: &HashSet<NodeId>,
) {
    if op_nodes.is_empty() {
        return;
    }
    let mut missing: Vec<NodeId> = op_nodes
        .iter()
        .copied()
        .filter(|id| !ws.tree().contains(*id))
        .collect();
    if missing.is_empty() {
        b.ok(format!(
            "every node the op log touches is in the materialized tree ({} node(s))",
            op_nodes.len()
        ));
        return;
    }
    missing.sort();
    b.warn(format!(
        "{} node id(s) appear in the op log but not in the materialized tree — \
         their ops (Edit / SetProp / SetCollapsed) never took effect",
        missing.len()
    ));
    for id in missing.iter().take(MAX_LISTED) {
        b.warn(format!("  unmaterialized node {id}"));
    }
    if missing.len() > MAX_LISTED {
        b.warn(format!("  … and {} more", missing.len() - MAX_LISTED));
    }
}

/// Compare every page in the tree against its `.md` projection, and
/// record what `--repair` may safely act on.
///
/// Three repairable shapes, and one deliberately-not-repairable one:
///
/// - `.md` absent → the page exists in the op log but was never
///   projected here. Safe to write: nothing on disk to lose.
/// - `.md` present, hash matches its sidecar (a *faithful* projection),
///   but the tree now renders differently → the projection is stale.
///   Safe to rewrite: the on-disk bytes carry no unreconciled edit.
/// - `.md` present, no sidecar, and its bytes equal what the tree
///   renders → only the sidecar is missing. Safe: the `.md` is
///   rewritten byte-identical and the sidecar rebuilt from the tree.
/// - `.md` present, no sidecar, bytes differ from the tree → the file
///   may hold content the log never saw. **Never** repaired here;
///   `outl reconcile` owns the `.md → tree` direction.
///
/// The final call at repair time belongs to
/// `outl_actions::apply_page_md_with_sidecar_if_stale`, which re-runs
/// the faithful/stale test itself. What lives here is detection only —
/// `outl-actions` exposes no dry-run of that decision.
///
/// `log_damaged` comes from the caller's [`super::oplog::OpLogHealth`]: a
/// torn op log replays a truncated tree, which makes every page look like
/// it carries unlogged content. That verdict belongs to the log, not the
/// pages, so the unlogged-content check stands down and the caller's gate
/// reports the recoverable cause instead.
pub(super) fn check_projections(
    b: &mut Builder,
    ws: &Workspace,
    root: &std::path::Path,
    log_damaged: bool,
) -> Plan {
    let mut plan = Plan::default();
    let mut absent = 0usize;
    let mut stale = 0usize;
    let mut sidecar_only = 0usize;
    let mut pending_edit = 0usize;
    let mut ahead = 0usize;
    let mut ahead_lines = 0usize;
    let mut removed_lines = 0usize;

    for page in survey_page_projections(ws, root, log_damaged) {
        let path = page.path;
        match page.state {
            PageProjectionState::InSync => {}
            PageProjectionState::Absent => {
                absent += 1;
                b.warn(format!(
                    "{}: page `{}` is in the op log but has no `.md` on disk",
                    path.display(),
                    page.slug
                ));
                // Nothing on disk, so nothing to remove. Counting it as
                // a zero keeps the common bulk case — a device that just
                // paired and has the whole graph unprojected — from
                // tripping a guard aimed at deletion.
                plan.reproject
                    .push(PageWrite::additive(page.page_root, path));
            }
            PageProjectionState::Unreadable { error } => {
                b.warn(format!("{}: unreadable: {error}", path.display()));
            }
            // Neither of the next two adds to `plan.reproject`, and that
            // is the point of both: a re-projection stops at the same
            // gate the survey already hit, so offering one would promise
            // a repair the writing pass refuses (root `CLAUDE.md`
            // invariant 8).
            PageProjectionState::MarkdownNotHereYet { reason } => {
                b.warn(format!("{}: {reason}", path.display()));
            }
            PageProjectionState::HashWithheldButClean => {
                b.warn(format!(
                    "{}: its sidecar still carries the withheld-hash sentinel, but the \
                     content it was withheld for is no longer on disk — `outl reconcile` \
                     restamps it; `--repair` cannot (a re-projection stops at the same gate)",
                    path.display()
                ));
            }
            PageProjectionState::SidecarMissingButFaithful => {
                sidecar_only += 1;
                // Byte-identical by precondition — the sidecar is what
                // is missing, not the content.
                plan.rebuild_sidecar
                    .push(PageWrite::additive(page.page_root, path));
            }
            PageProjectionState::SidecarMissingAndDrifted => {
                b.warn(format!(
                    "{}: no sidecar AND content differs from the op log — \
                     `--repair` will not touch it, run `outl reconcile` so the `.md` \
                     is matched back into the log first",
                    path.display()
                ));
            }
            // An external edit is pending. `outl reconcile` owns it;
            // saying it twice as a warning would drown the real signal.
            PageProjectionState::PendingExternalEdit => pending_edit += 1,
            PageProjectionState::SidecarCannotAnswer => {
                // Not "nothing at risk" — "I cannot tell". The write
                // guard declines these, so offering a repair here would
                // be a listing promising something the pass refuses.
                b.warn(format!(
                    "{}: the `.md` is stale but its sidecar cannot say whether the op log \
                     knows the content on disk (written before 0.11) — `--repair` leaves \
                     it alone; `outl reconcile` rebuilds the sidecar",
                    path.display()
                ));
            }
            PageProjectionState::AheadOfLog { lines, sample } => {
                ahead += 1;
                ahead_lines += lines;
                b.warn(format!(
                    "{}: `.md` holds {lines} line(s) that exist in no op (e.g. {sample:?}) — \
                     `--repair` will not touch it, run `outl reconcile --ahead-of-log` so they enter \
                     the op log first",
                    path.display(),
                ));
            }
            PageProjectionState::Stale { lines_removed } => {
                stale += 1;
                // Measured before the plan is even offered, because
                // `--repair` printing `708 fixed` after the fact is exactly
                // how 1,426 lines went unnoticed (RFC 0210).
                removed_lines += lines_removed;
                b.warn(format!(
                    "{}: `.md` is a stale projection — the op log renders different content \
                     (re-projecting removes {lines_removed} content line(s) from disk)",
                    path.display()
                ));
                plan.reproject.push(PageWrite {
                    page_root: page.page_root,
                    path,
                    lines_removed,
                });
            }
        }
    }

    if ahead > 0 {
        b.warn(format!(
            "{ahead} page(s) hold {ahead_lines} line(s) of content that reached the `.md` but \
             never the op log — they do not sync to other devices, and `--repair` leaves them \
             alone. `outl reconcile --ahead-of-log` is what brings them into the log"
        ));
    }

    if removed_lines > 0 {
        // The headline the old output never had. `--repair` used to
        // print a page count and nothing about content, so a pass that
        // removed 1,426 lines read as `708 fixed`. Stated in both modes:
        // a read-only run is where the user decides whether to authorise
        // the write at all.
        b.warn(format!(
            "re-projecting the stale page(s) above removes {removed_lines} content line(s) \
             across {} page(s) — read the list before running `--repair`",
            plan.reproject
                .iter()
                .filter(|p| p.lines_removed > 0)
                .count()
        ));
    }

    if pending_edit > 0 {
        b.info(format!(
            "{pending_edit} page(s) carry an unreconciled external edit — run `outl reconcile`"
        ));
    }
    if sidecar_only > 0 {
        b.warn(format!(
            "{sidecar_only} page(s) have a correct `.md` but no sidecar — `--repair` rebuilds them"
        ));
    }
    if absent + stale + sidecar_only == 0 {
        b.ok("every page in the op log has a matching `.md` projection on disk");
    }
    plan
}
