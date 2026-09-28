//! `outl doctor` — workspace integrity check.
//!
//! Default mode is **read-only**: it reports, it does not fix.
//! `--repair` opts into a deliberately narrow set of fixes that cannot
//! lose data — see [`repair`] for the exact allow-list and the backup
//! policy.
//!
//! The check pipeline is exposed as [`collect_scoped`], returning a
//! serializable [`DoctorReport`]. The CLI surface ([`run`] for human
//! output, [`run_json`] for `--json`) wraps it, and the MCP shim
//! routes `outl_workspace_doctor` into [`collect_in_session_json`].
//!
//! ## Layout
//!
//! - `report` — the vocabulary every check writes into: severity,
//!   finding, the accumulating `Builder`, the serialized
//!   `DoctorReport`. No check lives there.
//! - `oplog` — raw `.jsonl` line scan, snapshot decode, offset-index
//!   coherence. Bytes on disk.
//! - `files` — `.md` ↔ sidecar pairing, parse warnings, orphan block
//!   refs, sync-conflict copies. Files on disk.
//! - `tree` — trash contents, ops that never materialized, projection
//!   drift. Needs a booted `Workspace`.
//! - `theme` — `[theme]` pair validation (light slot actually light,
//!   dark slot actually dark). The global user config, not this
//!   workspace's.
//! - `device_store` — the actor bindings. The one subject that is not
//!   in this workspace at all.
//! - `gate` — what takes a planned page write back out of the plan: a
//!   damaged op log, and a deletion past the ceilings.
//! - `repair` — the `--repair` pass.
//! - `print` — the human listing, the `--json` envelope, exit codes.

mod device_store;
mod files;
mod gate;
mod oplog;
mod ops_guard;
mod print;
mod repair;
mod report;
#[cfg(test)]
mod tests;
mod theme;
mod tree;

use crate::output::ApiError;
use crate::workspace_layout::{read_config, Paths};
use outl_core::device::DeviceStore;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use outl_md::index::WorkspaceIndex;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

pub use gate::RepairScope;
pub use print::{run, run_json};
use repair::Plan;
pub use repair::{RepairReport, RepairVolume};
use report::Builder;
pub use report::{DoctorReport, Severity};

/// Run every doctor check and return a structured report, optionally
/// applying the safe repairs.
///
/// Used by the CLI human path and the `--json` flag — those acquire the
/// workspace lock fresh, so the lock probe at the end can tell apart
/// "free" / "held by another outl process". The MCP shim uses
/// [`collect_in_session`] because it is already holding the lock through
/// its cached `WsCtx`, so the probe would always return `AlreadyHeld`
/// and lie.
///
/// `scope` is [`RepairScope::Forced`] only from the CLI's `--force`
/// flag; every other caller passes [`RepairScope::Guarded`].
pub fn collect_scoped(
    path: &Path,
    do_repair: bool,
    scope: RepairScope,
) -> Result<DoctorReport, ApiError> {
    let global = outl_config::load_result();
    collect_internal(
        path,
        true,
        do_repair,
        scope,
        &DeviceStore::open_default(),
        &global.config.theme,
        global.notice(),
    )
}

/// Same as [`collect_scoped`] but skips the workspace-lock probe.
///
/// The MCP server holds the lock for its whole session through the
/// cached `WsCtx`, so a second `WorkspaceLock::acquire` from inside
/// the same process would always return `AlreadyHeld` and the probe
/// would always report a non-existent contention. Skipping the probe
/// keeps the doctor signal honest when invoked from within a long-
/// running process.
///
/// Never repairs: a tool call is not the place to start rewriting
/// files on the user's disk. `--repair` is CLI-only and explicit.
pub fn collect_in_session(path: &Path) -> Result<DoctorReport, ApiError> {
    let global = outl_config::load_result();
    collect_internal(
        path,
        false,
        false,
        RepairScope::Guarded,
        &DeviceStore::open_default(),
        &global.config.theme,
        global.notice(),
    )
}

/// `store` is the device store the binding check reads and prunes.
///
/// It is a parameter, not a `DeviceStore::open_default()` call inside the
/// pass, for one reason: the device store is **machine-global**, so a
/// pass that resolved it itself would make every test in this suite share
/// the developer's store — and a `--repair` test would then delete real
/// bindings and assert against a count nobody controls. That is issue
/// #211 exactly, reintroduced by its own fix. Root `CLAUDE.md`
/// invariant 9's third question (*how does a test get its own copy?*) has
/// to be answered where the state is reached, and this is that place.
///
/// `theme` is the same story. It is the `[theme]` section of the
/// **global** `~/.config/outl/config.toml` (`outl_config::load()`), not
/// `cfg.theme` — the workspace-local `Config` this function reads two
/// lines below (`outl_ws::layout::Config`) carries only `workspace`, no
/// theme section at all. Resolving the global config inside this pass
/// would make every test in the battery judge whatever theme pair
/// happens to be on the machine running the suite, the exact bug the
/// `store` parameter above exists to avoid for the device store.
#[allow(clippy::too_many_arguments)]
fn collect_internal(
    path: &Path,
    probe_lock: bool,
    do_repair: bool,
    scope: RepairScope,
    store: &DeviceStore,
    theme: &outl_config::ThemeCfg,
    config_notice: Option<String>,
) -> Result<DoctorReport, ApiError> {
    let paths = Paths::at(path.to_path_buf());
    let cfg = read_config(&paths).map_err(|e| {
        ApiError::new(
            crate::output::codes::NO_WORKSPACE,
            format!("workspace config missing — run `outl init` first ({e})"),
        )
    })?;
    // Report the actor this DEVICE writes under, not the one seeded in
    // `config.toml` — on a shared/synced workspace they differ on every
    // device but the one that claimed it (see `outl_ws::actor`).
    // `store`, not `open_default()`. This call *writes* — it binds an
    // actor for the workspace when the device has none — so resolving the
    // store here would leave one record per test workspace in the shared
    // dev store, which is the leak this issue is about. It would also let
    // the report name an actor from one store while the binding check
    // judges another.
    let actor = outl_ws::actor::resolve_device_actor(&paths, &cfg, store).map_err(|e| {
        ApiError::new(
            crate::output::codes::INTERNAL,
            format!("could not resolve this device's actor: {e}"),
        )
    })?;
    let mut b = Builder::new(paths.root.display().to_string(), actor.to_string());
    let mut plan = Plan::default();
    let mut health = oplog::OpLogHealth::default();

    // 0. The global user config: can it be read, and is its `[theme]` pair
    //    the right way round. Neither is this workspace's file, and neither
    //    needs the op log or a booted tree — check them before anything
    //    that does.
    //
    //    The unreadable notice comes first, and `doctor` only *phrases* it:
    //    the sentence is `outl_config::Loaded::notice`, so this report and
    //    the TUI's boot line cannot describe the same file differently
    //    (issue #284). A warning, not an error — the workspace is intact,
    //    and nothing overwrote the file. When it fires, `theme` below is a
    //    default nobody chose, which is why it is announced first.
    if let Some(notice) = config_notice {
        b.warn(notice);
    }
    theme::check_theme_pair(&mut b, theme);

    // 1. Raw op-log sweep, before any storage open. `JsonlStorage::open`
    //    skips malformed records and reports them only through
    //    `tracing::warn!` — intentional, so one torn tail line can't
    //    lock a user out of their workspace, but it also means this is
    //    the only place a corrupt record is nameable.
    let scans = oplog::check_jsonl_lines(&mut b, &paths.ops, &mut health);
    oplog::check_offset_indexes(&mut b, &paths.ops, &scans);
    let snapshots = oplog::check_snapshots(&mut b, &paths.root, actor);
    plan.drop_snapshots = snapshots.drops;
    plan.prune_snapshot_tmp = snapshots.stale_tmp;
    // Dead `ops/` index caches. Surveyed HERE, before the storage open
    // below rebuilds the live sidecars — a survey taken afterwards would
    // be judging files this command itself created.
    plan.prune_index_sidecars = repair::collect_dead(&mut b, &paths.ops);
    repair::report_actor_locks(&mut b, &paths.ops);
    // Housekeeping over `--repair`'s own output. Collected here, not
    // inside `repair::run`, so it is announced in `repairable[]` and so
    // a workspace with nothing else wrong still gets its old backup
    // generations reclaimed.
    plan.prune_backups = repair::collect_prunable(&paths.root);
    // The one check whose subject lives outside this workspace: the
    // machine-global device store. See `device_store`.
    device_store::check(&mut b, store, &mut plan);

    // 2. The op log as the storage layer sees it: how many ops survive
    //    the skip-on-malformed read, and which nodes they touch.
    //
    //    `JsonlStorage::open` is a read for us and a write on disk — it
    //    persists rebuilt `.idx` sidecars for every actor it finds. The
    //    guard photographs `ops/` here and restores it at the end of the
    //    run, in BOTH modes, so `doctor` (repairing or not) leaves that
    //    directory byte-identical. See `ops_guard`.
    // The dead caches above are the one thing in `ops/` this command may
    // remove, so the guard is told about them rather than restoring them
    // from a photograph it would otherwise have to hold 134 MB of.
    let announced = repair::announced_paths(&plan.prune_index_sidecars);
    let ops_guard = ops_guard::OpsDirGuard::capture(&paths.ops, &announced);
    let mut storage_for_ws: Option<Box<dyn Storage>> = None;
    let known_node_ids: HashSet<outl_core::id::NodeId> =
        match JsonlStorage::open(paths.ops.clone(), actor) {
            Ok(storage) => match storage.all_ops() {
                Ok(ops) => {
                    b.op_count = ops.len();
                    b.ok(format!("op log has {} ops", ops.len()));
                    let mut ids = HashSet::new();
                    for op in &ops {
                        let node = match &op.op {
                            outl_core::op::Op::Move { node, .. }
                            | outl_core::op::Op::Edit { node, .. }
                            | outl_core::op::Op::SetProp { node, .. }
                            | outl_core::op::Op::Create { node, .. }
                            | outl_core::op::Op::SetCollapsed { node, .. }
                            | outl_core::op::Op::SnoozeRemind { node, .. } => *node,
                        };
                        ids.insert(node);
                    }
                    storage_for_ws = Some(Box::new(storage));
                    ids
                }
                Err(e) => {
                    b.err(format!("could not read op log: {e}"));
                    health.compromise(format!("the op log could not be read back: {e}"));
                    HashSet::new()
                }
            },
            Err(e) => {
                b.err(format!(
                    "could not open ops dir at {}: {e}",
                    paths.ops.display()
                ));
                health.compromise(format!(
                    "the ops dir at {} could not be opened: {e}",
                    paths.ops.display()
                ));
                HashSet::new()
            }
        };

    // 3. Materialized tree. `root: None` forces a **full replay** — the
    //    doctor has to judge the op log, not a snapshot's opinion of it
    //    — and as a side effect guarantees this command never writes a
    //    snapshot of its own.
    let workspace = match storage_for_ws {
        Some(storage) => match Workspace::open_with_storage(actor, storage, None) {
            Ok(ws) => Some(ws),
            Err(e) => {
                b.err(format!("could not replay the op log into a tree: {e}"));
                None
            }
        },
        None => None,
    };
    match &workspace {
        Some(ws) => {
            tree::check_trash(&mut b, ws);
            tree::check_unmaterialized_ops(&mut b, ws, &known_node_ids);
            let projection =
                tree::check_projections(&mut b, ws, &paths.root, health.is_compromised());
            plan.reproject = projection.reproject;
            plan.rebuild_sidecar = projection.rebuild_sidecar;
        }
        None => b.warn(
            "skipped the tree checks (trash, unmaterialized ops, projection drift) — \
             the op log could not be replayed",
        ),
    }

    // 4. Pages and journals: `.md` ↔ sidecar pairing.
    //
    // The parse-warning tally is accumulated **across** both
    // directories and reported once at the end. Emitting the
    // all-clear per directory printed "every `.md` parses cleanly"
    // right after listing a page's bad lines, because the journals
    // pass happened to be clean — a flatly wrong statement to show a
    // user who was just told otherwise.
    let mut parse_warning_total = 0usize;
    for dir in [&paths.pages, &paths.journals] {
        if !dir.is_dir() {
            continue;
        }
        let mut md_files = Vec::new();
        let mut sidecar_files = Vec::new();
        for entry in walkdir::WalkDir::new(dir).max_depth(1) {
            let Ok(entry) = entry else { continue };
            let p = entry.path();
            if !entry.file_type().is_file() {
                continue;
            }
            let name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Sidecars come in two spellings: the modern un-hidden
            // `foo.outl` (what `sidecar_path_for` writes today) and the
            // legacy dotted `.foo.outl`. Collecting only the dotted one
            // made the orphan-sidecar check dead on every workspace
            // written by a current build.
            if name.ends_with(".outl") {
                sidecar_files.push(p.to_path_buf());
                continue;
            }
            if name.starts_with('.') {
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) == Some("md") {
                md_files.push(p.to_path_buf());
            }
        }
        files::check_md_files(&mut b, &md_files, &known_node_ids);
        files::check_orphan_sidecars(&mut b, &sidecar_files, &md_files);
        // The orphans-log rows are a `--repair` side effect, never a
        // read-only one: `.outl/orphans.log` is also where level-3
        // matching orphans (the record of blocks that could not be
        // matched back into the log) live, and appending thousands of
        // parse-warning rows on every diagnostic run buries them.
        parse_warning_total +=
            files::check_parse_warnings(&mut b, &md_files, &paths.orphans, do_repair);
    }
    if parse_warning_total == 0 {
        b.ok("no parser warnings — every `.md` parses cleanly in the outl dialect");
    }

    // 5. Sync-conflict copies. Loud on purpose: user content outl will
    //    never read.
    //
    //    A fork under `ops/` is a different animal from a fork under
    //    `pages/`: it is a slice of op log the tree never replayed, so
    //    the tree is short exactly the way a torn line makes it short.
    //    That is a repair-blocking condition, not just a warning.
    let conflicts = files::check_sync_conflicts(
        &mut b,
        &[&paths.pages, &paths.journals, &paths.ops, &paths.assets],
    );
    for forked in conflicts.iter().filter(|p| p.starts_with(&paths.ops)) {
        health.compromise(format!(
            "{} is a forked op log — its ops never reached the tree",
            forked.display()
        ));
    }

    // 6. Block ref integrity — every `((blk-XXXXXX))` mentioned must
    //    resolve to an indexed block. Build the index once.
    //
    //    Deliberately the **disk** build, not `outl_actions::index::derive`,
    //    while every other reader moved to the derived one. Doctor's job is
    //    to compare the projection against the tree; deriving both sides
    //    from the tree would make this check agree with itself by
    //    construction and stop reporting the divergence it exists to find.
    //    An orphan `((blk-…))` in a `.md` whose target the log never had is
    //    precisely a finding, not noise.
    let workspace_index = WorkspaceIndex::build(&paths.root);
    files::check_orphan_block_refs(&mut b, &workspace_index);

    // 7. Orphan log presence (informational).
    if paths.orphans.exists() {
        let bytes = std::fs::metadata(&paths.orphans)
            .map(|m| m.len())
            .unwrap_or(0);
        if bytes == 0 {
            b.ok("orphans.log is empty");
        } else {
            b.info(format!(
                "orphans.log has {bytes} bytes — run `outl reconcile` to triage"
            ));
        }
    }

    // 8. Lock file: warn if held by another process.
    //    Skipped when running inside an outl process that already
    //    holds the lock (e.g. MCP server) — `AlreadyHeld` would just
    //    report itself.
    if probe_lock {
        match outl_core::WorkspaceLock::acquire(&paths.root) {
            Ok(_lock) => b.ok("workspace lock is free (no other outl process attached)"),
            Err(outl_core::LockError::AlreadyHeld(_)) => {
                b.warn("another outl process is holding the workspace lock");
            }
            Err(e) => b.warn(format!("could not test workspace lock: {e}")),
        }
    } else {
        b.info("workspace lock probe skipped (running inside an outl session)");
    }

    // 9. Two gates over the page writes planned above: a damaged op
    //    log has no authority over its own projection, and a deletion
    //    past the ceilings is a decision rather than a repair. Both
    //    withhold, neither is silent. See `gate`.
    gate::check_damaged_log(&mut b, &mut plan, &health);
    gate::check_volume(&mut b, &mut plan, do_repair, scope);

    let repairable = plan.describe();
    let repair_report = match (do_repair, plan.is_empty(), &workspace) {
        (false, _, _) | (true, true, _) => None,
        (true, false, Some(ws)) => Some(repair::run(ws, &paths.root, actor, &plan, store)),
        // No replayed tree, so page re-projection is off the table, but
        // dropping a corrupt snapshot, pruning stale backups and dropping
        // a dead device-store binding still are not — none needs a tree,
        // and the binding prune does not even read this workspace.
        (true, false, None) => {
            let treeless = Plan {
                drop_snapshots: std::mem::take(&mut plan.drop_snapshots),
                prune_snapshot_tmp: std::mem::take(&mut plan.prune_snapshot_tmp),
                prune_index_sidecars: std::mem::take(&mut plan.prune_index_sidecars),
                prune_backups: std::mem::take(&mut plan.prune_backups),
                prune_bindings: std::mem::take(&mut plan.prune_bindings),
                prune_scratch: std::mem::take(&mut plan.prune_scratch),
                ..Plan::default()
            };
            if treeless.is_empty() {
                None
            } else {
                Workspace::open_in_memory(actor)
                    .ok()
                    .map(|ws| repair::run(&ws, &paths.root, actor, &treeless, store))
            }
        }
    };

    // Storage (and the tree that borrowed it) is done with; put `ops/`
    // back exactly as we found it before anything is reported.
    drop(workspace);
    ops_guard.restore(&mut b);

    Ok(b.into_report(repairable, repair_report))
}

/// MCP entry point — returns the report as JSON `data` without the
/// workspace-lock probe. The MCP shim already owns the lock for the
/// session, so a fresh `acquire` would always report contention
/// against itself.
pub fn collect_in_session_json(path: &Path) -> Result<Value, ApiError> {
    let report = collect_in_session(path)?;
    serde_json::to_value(&report).map_err(ApiError::internal)
}
