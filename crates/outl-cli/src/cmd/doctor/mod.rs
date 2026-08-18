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
//! - `oplog` — raw `.jsonl` line scan, snapshot decode, offset-index
//!   coherence. Bytes on disk.
//! - `files` — `.md` ↔ sidecar pairing, parse warnings, orphan block
//!   refs, sync-conflict copies. Files on disk.
//! - `tree` — trash contents, ops that never materialized, projection
//!   drift. Needs a booted `Workspace`.
//! - `repair` — the `--repair` pass.

mod files;
mod oplog;
mod ops_guard;
mod repair;
#[cfg(test)]
mod tests;
mod tree;

use crate::output::{emit, ApiError};
use crate::workspace_layout::{read_config, Paths};
use anyhow::{Context, Result};
use outl_core::device::{ActorBinding, DeviceStore, STALE_BINDING_TTL, STALE_SCRATCH_TTL};
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use outl_md::index::WorkspaceIndex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

use repair::Plan;
pub use repair::{RepairReport, RepairVolume};

/// How much authority a `--repair` run carries.
///
/// Re-projection removes content by design — a peer deleted a block, the
/// log is right, the `.md` is behind. What it must not do is remove
/// *thousands* of lines because something systemic is wrong, print a
/// page count, and leave nothing to compare against afterwards. So the
/// volume is measured first and a large one needs a second, deliberate
/// act.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepairScope {
    /// Default. Page writes stand down when the measured volume is past
    /// [`RepairVolume::needs_confirmation`]; everything else still runs.
    #[default]
    Guarded,
    /// `--force`: the user has read the count and authorised it.
    ///
    /// Reachable only from an explicit flag. Never a retry, never a
    /// fallback — "attempted twice" is not consent.
    Forced,
}

/// Severity of a single finding.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Healthy state — no action needed.
    Ok,
    /// Informational, not a problem.
    Info,
    /// Possible problem, user should look.
    Warn,
    /// Definite problem, blocks "integrity OK".
    Error,
}

/// One workspace check result.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Severity bucket.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
}

/// Aggregate of every finding from a doctor run.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    /// Workspace root path (display form).
    pub workspace: String,
    /// Actor id from `config.toml`.
    pub actor: String,
    /// Number of ops in the persisted log.
    pub op_count: usize,
    /// Every check emitted, in execution order.
    pub findings: Vec<Finding>,
    /// Convenience counts so callers don't have to count.
    pub error_count: usize,
    /// Number of warning findings.
    pub warn_count: usize,
    /// What `--repair` *would* do, in execution order. Filled on every
    /// run, so a read-only report already tells the user what is
    /// fixable and what is not.
    pub repairable: Vec<String>,
    /// What `--repair` actually did. `None` when the flag was off or
    /// there was nothing to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair: Option<RepairReport>,
}

struct Builder {
    workspace: String,
    actor: String,
    op_count: usize,
    findings: Vec<Finding>,
    errors: usize,
    warnings: usize,
}

impl Builder {
    fn new(workspace: String, actor: String) -> Self {
        Self {
            workspace,
            actor,
            op_count: 0,
            findings: Vec::new(),
            errors: 0,
            warnings: 0,
        }
    }
    fn push(&mut self, severity: Severity, message: impl Into<String>) {
        if matches!(severity, Severity::Error) {
            self.errors += 1;
        }
        if matches!(severity, Severity::Warn) {
            self.warnings += 1;
        }
        self.findings.push(Finding {
            severity,
            message: message.into(),
        });
    }
    fn ok(&mut self, msg: impl Into<String>) {
        self.push(Severity::Ok, msg);
    }
    fn info(&mut self, msg: impl Into<String>) {
        self.push(Severity::Info, msg);
    }
    fn warn(&mut self, msg: impl Into<String>) {
        self.push(Severity::Warn, msg);
    }
    fn err(&mut self, msg: impl Into<String>) {
        self.push(Severity::Error, msg);
    }
    fn into_report(self, repairable: Vec<String>, repair: Option<RepairReport>) -> DoctorReport {
        DoctorReport {
            workspace: self.workspace,
            actor: self.actor,
            op_count: self.op_count,
            findings: self.findings,
            error_count: self.errors,
            warn_count: self.warnings,
            repairable,
            repair,
        }
    }
}

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
    collect_internal(path, true, do_repair, scope, &DeviceStore::open_default())
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
    collect_internal(
        path,
        false,
        false,
        RepairScope::Guarded,
        &DeviceStore::open_default(),
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
fn collect_internal(
    path: &Path,
    probe_lock: bool,
    do_repair: bool,
    scope: RepairScope,
    store: &DeviceStore,
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

    // 1. Raw op-log sweep, before any storage open. `JsonlStorage::open`
    //    skips malformed records and reports them only through
    //    `tracing::warn!` — intentional, so one torn tail line can't
    //    lock a user out of their workspace, but it also means this is
    //    the only place a corrupt record is nameable.
    let scans = oplog::check_jsonl_lines(&mut b, &paths.ops, &mut health);
    oplog::check_offset_indexes(&mut b, &paths.ops, &scans);
    plan.corrupt_snapshots = oplog::check_snapshots(&mut b, &paths.root);
    // Housekeeping over `--repair`'s own output. Collected here, not
    // inside `repair::run`, so it is announced in `repairable[]` and so
    // a workspace with nothing else wrong still gets its old backup
    // generations reclaimed.
    plan.prune_backups = repair::collect_prunable(&paths.root);
    // The one check whose subject is outside this workspace. The device
    // store is machine-global and has never had a GC, so a workspace the
    // user deleted keeps its actor binding forever (issue #211 item 3).
    // Reported here because `doctor` is the surface a user already runs,
    // and because the store's health is what decides whether the *next*
    // open of any workspace forks an actor.
    // An error here is an empty list, never a finding: the store is
    // outside this workspace, so a permission problem there says nothing
    // about this graph and must not fail an otherwise-clean run.
    plan.prune_bindings = store
        .stale_actor_bindings(STALE_BINDING_TTL)
        .unwrap_or_default();
    plan.prune_scratch = store.stale_scratch(STALE_SCRATCH_TTL).unwrap_or_default();
    report_stale_bindings(&mut b, &plan.prune_bindings);

    // 2. The op log as the storage layer sees it: how many ops survive
    //    the skip-on-malformed read, and which nodes they touch.
    //
    //    `JsonlStorage::open` is a read for us and a write on disk — it
    //    persists rebuilt `.idx` sidecars for every actor it finds. The
    //    guard photographs `ops/` here and restores it at the end of the
    //    run, in BOTH modes, so `doctor` (repairing or not) leaves that
    //    directory byte-identical. See `ops_guard`.
    let ops_guard = ops_guard::OpsDirGuard::capture(&paths.ops);
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

    // 9. The gate. Every projection repair writes a `.md` rendered from
    //    the materialized tree, and the tree is only ever as complete as
    //    the op log it replayed. `JsonlStorage` skips unreadable records
    //    by design, so a damaged log boots a **truncated** tree that
    //    looks perfectly healthy from the inside — and a `.md` that is
    //    still a faithful projection of the whole page then reads as
    //    "stale", because the truncated tree renders less than it.
    //
    //    Repairing that overwrites the user's content with the render of
    //    a log we just told them is broken. The op log is the source of
    //    truth (root `CLAUDE.md` invariant 1); when it is damaged it has
    //    no authority over the projection, and the correct move is to
    //    stop and recover the log first.
    //
    //    Snapshot deletion stays allowed: it is a pure boot cache, and
    //    dropping it only forces the full replay a damaged log wants
    //    anyway.
    let held_back = plan.reproject.len() + plan.rebuild_sidecar.len();
    if health.is_compromised() && held_back > 0 {
        plan.reproject.clear();
        plan.rebuild_sidecar.clear();
        b.warn(format!(
            "{held_back} page repair(s) suppressed — the op log is damaged, so the tree replayed \
             from it may be missing blocks, and re-projecting a page would overwrite a good \
             `.md` with an incomplete render. Recover the op log first (restore `ops/` from a \
             backup, or let a healthy peer sync it back), then re-run. Reasons: {}",
            health.compromised_by.join("; ")
        ));
    }

    // 10. The volume guard. Everything above decided *whether* a page
    //     may be rewritten; this decides whether the total is small
    //     enough to happen without being asked.
    //
    //     Re-projection removes content legitimately — a peer deleted a
    //     block and this device is behind — so the gate cannot be "any
    //     deletion". It is the scale: on a healthy workspace the total
    //     is single digits, and the run that motivated this removed
    //     1,426 lines from 233 pages while printing `708 fixed`
    //     (RFC 0210). A destructive operation that scales silently turns
    //     a small bug into an unrecoverable one.
    //
    //     Announced in **both** modes and suppressed only when actually
    //     repairing, so the read-only listing keeps naming what it found
    //     and states the condition attached to it — rather than offering
    //     a repair `--repair` then silently refuses.
    let volume = plan.volume();
    if volume.is_destructive() {
        let (max_pages, max_lines) = RepairVolume::ceilings();
        if volume.needs_confirmation() {
            let held_back = plan.reproject.len() + plan.rebuild_sidecar.len();
            if do_repair && scope == RepairScope::Guarded {
                plan.reproject.clear();
                plan.rebuild_sidecar.clear();
            }
            let tail = match (do_repair, scope) {
                (true, RepairScope::Guarded) => format!(
                    "{held_back} page repair(s) suppressed — re-run with \
                     `outl doctor --repair --force` once the list above reads right"
                ),
                (true, RepairScope::Forced) => "proceeding: `--force` was given".to_string(),
                (false, _) => {
                    "`outl doctor --repair` will refuse this without `--force`".to_string()
                }
            };
            b.err(format!(
                "`--repair` would remove {} content line(s) from {} page(s), past the \
                 point this runs unattended (ceilings: {max_lines} line(s), {max_pages} \
                 page(s)). Every write is backed up under `.outl/repair-backup/`, but a \
                 deletion this size is a decision, not a repair. {tail}",
                volume.lines_removed, volume.pages_losing_content,
            ));
        } else {
            b.warn(format!(
                "`--repair` would remove {} content line(s) from {} page(s) — under the \
                 ceilings ({max_lines} line(s), {max_pages} page(s)), so it runs without \
                 `--force`",
                volume.lines_removed, volume.pages_losing_content,
            ));
        }
    }

    let repairable = plan.describe();
    let repair_report = match (do_repair, plan.is_empty(), &workspace) {
        (false, _, _) | (true, true, _) => None,
        (true, false, Some(ws)) => Some(repair::run(ws, &paths.root, &plan, store)),
        // No replayed tree, so page re-projection is off the table, but
        // dropping a corrupt snapshot, pruning stale backups and dropping
        // a dead device-store binding still are not — none needs a tree,
        // and the binding prune does not even read this workspace.
        (true, false, None) => {
            let treeless = Plan {
                corrupt_snapshots: std::mem::take(&mut plan.corrupt_snapshots),
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
                    .map(|ws| repair::run(&ws, &paths.root, &treeless, store))
            }
        }
    };

    // Storage (and the tree that borrowed it) is done with; put `ops/`
    // back exactly as we found it before anything is reported.
    drop(workspace);
    ops_guard.restore(&mut b);

    Ok(b.into_report(repairable, repair_report))
}

/// Say what the device store is carrying, in the read-only pass too.
///
/// **Info, never a warning.** A stale binding costs ~190 bytes and breaks
/// nothing: the workspace it names is gone, so there is no sync to be
/// wrong about. Ranking tidiness alongside a torn op log is how the loud
/// lines in this report stop being read. The user learns the number, and
/// `--repair` is where they act on it.
fn report_stale_bindings(b: &mut Builder, stale: &[ActorBinding]) {
    if stale.is_empty() {
        return;
    }
    b.info(format!(
        "device store: {} actor binding(s) name a workspace that no longer exists — \
         `outl doctor --repair` drops them (a binding whose volume is merely unmounted \
         is never counted here)",
        stale.len()
    ));
}

/// MCP entry point — returns the report as JSON `data` without the
/// workspace-lock probe. The MCP shim already owns the lock for the
/// session, so a fresh `acquire` would always report contention
/// against itself.
pub fn collect_in_session_json(path: &Path) -> Result<Value, ApiError> {
    let report = collect_in_session(path)?;
    serde_json::to_value(&report).map_err(ApiError::internal)
}

/// CLI entry point with human output. Exits with status 1 when the
/// report has errors so scripts can detect failure.
pub fn run(path: &Path, do_repair: bool, scope: RepairScope) -> Result<()> {
    let report = collect_scoped(path, do_repair, scope)
        .with_context(|| format!("running doctor on {}", path.display()))?;
    println!("workspace: {}", report.workspace);
    println!("actor:     {}", report.actor);
    println!();
    for finding in &report.findings {
        let tag = match finding.severity {
            Severity::Ok => "ok:  ",
            Severity::Info => "info:",
            Severity::Warn => "warn:",
            Severity::Error => "err: ",
        };
        println!("{tag} {}", finding.message);
    }

    if !report.repairable.is_empty() {
        println!();
        let suffix = if do_repair {
            ""
        } else {
            " — run `outl doctor --repair`"
        };
        println!("{} repairable item(s){suffix}:", report.repairable.len());
        for line in &report.repairable {
            println!("  - {line}");
        }
    }
    if let Some(rep) = &report.repair {
        println!();
        println!("repair: backups under {}", rep.backup_dir);
        for action in &rep.actions {
            let tag = if action.ok { "done" } else { "SKIP" };
            println!(
                "  {tag} {} {} ({})",
                action.kind, action.path, action.detail
            );
        }
        println!("repair: {} fixed, {} not fixed", rep.repaired, rep.failed);
    }

    println!();
    match (report.error_count, report.warn_count) {
        (0, 0) => println!("integrity OK"),
        (0, w) => println!("integrity OK with {w} warning(s)"),
        (e, w) => {
            println!("{e} error(s), {w} warning(s) — see lines above");
            std::process::exit(1);
        }
    }
    Ok(())
}

/// `outl doctor --json` shape — emits the envelope and exits 1 when
/// the report has errors.
pub fn run_json(path: &Path, do_repair: bool, scope: RepairScope) -> i32 {
    let result = collect_scoped(path, do_repair, scope)
        .and_then(|r| serde_json::to_value(&r).map_err(ApiError::internal));
    let exit = emit(true, result.clone(), |_| {});
    // `emit` already used the JSON branch; force an error exit when
    // the report itself carried errors even though the call succeeded.
    if exit == 0
        && result
            .ok()
            .and_then(|v| v.get("error_count").and_then(|n| n.as_u64()))
            .unwrap_or(0)
            > 0
    {
        return 1;
    }
    exit
}
