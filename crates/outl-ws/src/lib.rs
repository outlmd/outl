//! # outl-ws
//!
//! Workspace bootstrap for anything that embeds outl: opening a
//! workspace directory with the correct lock/actor protocol, per-page
//! shard registration, and slug repair. Extracted from the CLI so the
//! TUI, MCP server, and external embedders (e.g. ai-memory) share one
//! implementation instead of re-deriving the multi-process contract.
//!
//! ## Actor policy
//!
//! Two independent questions, resolved in order:
//!
//! 1. **Which actor does this *device* own?** [`actor::resolve_device_actor`]
//!    answers from [`outl_core::DeviceStore`] — a directory outside the
//!    workspace, keyed by (workspace id, workspace directory), so neither
//!    two devices syncing one `.outl/` nor two copies of one directory
//!    can read the same answer. `config.toml`'s `actor_id` is a legacy
//!    value adopted only by the device whose `actor_claimed_by` marker is
//!    already in the file; everyone else forks.
//! 2. **Which actor may this *process* write under?**
//!    [`outl_core::resolve_write_actor`] takes the exclusive per-actor
//!    flock; when another `outl` on this machine already holds it (TUI
//!    running, MCP server proxy active, a peer CLI in flight) the
//!    process gets an ephemeral actor and a fresh
//!    `ops-<ephemeral>.jsonl`.
//!
//! The shared workspace lock itself accepts multiple concurrent openers.
//! All readers merge every `ops-*.jsonl`, so peers see every write
//! regardless of how many actors a workspace accumulates.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod actor;
pub mod layout;

use std::path::{Path, PathBuf};

use outl_core::device::DeviceStore;
use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_core::{resolve_write_actor, ActorWriteLock, WorkspaceLock};

use outl_actions::{commit_page, ActionError, CommitHooks};
use outl_core::id::NodeId;

use crate::layout::{ensure_ops_dir, read_or_init_config, Paths};

/// Why a workspace failed to open.
#[derive(Debug, thiserror::Error)]
pub enum WsError {
    /// The directory holds no `.outl/` marker — not a workspace.
    #[error("no outl workspace at {0} — run `outl init {0}` first")]
    NoWorkspace(PathBuf),
    /// `.outl/config.toml` is missing or unreadable.
    #[error("workspace config missing or unreadable: {0}")]
    Config(String),
    /// `.outl/config.toml` carries an invalid actor id.
    #[error("invalid actor in config: {0}")]
    InvalidActor(String),
    /// Locking, storage, or op-log failure while booting.
    #[error("{0}")]
    Internal(String),
}

impl WsError {
    fn internal(e: impl std::fmt::Display) -> Self {
        Self::Internal(e.to_string())
    }
}

/// Per-open runtime context. The shared workspace lock plus an
/// exclusive per-actor write lock are held for the whole lifetime of
/// the context — two processes against the same workspace coexist as
/// long as each one ends up writing under a distinct actor.
pub struct WsCtx {
    /// Workspace path layout (root, .outl/, pages/, journals/, …).
    /// Consumed by surfaces that need on-disk paths (doctor, export,
    /// workspace info) without re-deriving them from `root`.
    pub paths: Paths,
    /// Loaded `Workspace` ready for reads or `apply`.
    pub workspace: Workspace,
    /// HLC generator bound to whatever actor this process resolved to
    /// (device actor on the first opener; ephemeral on the rest).
    pub hlc: HlcGenerator,
    /// Workspace path on disk.
    pub root: PathBuf,
    /// Actor id this process writes under. Equal to the device actor
    /// ([`actor::resolve_device_actor`]) when this process is the
    /// workspace's primary holder on this machine; a fresh
    /// `ActorId::new()` when another `outl` (TUI, MCP server, peer
    /// CLI) is already attached.
    pub actor: ActorId,
    /// Whether [`Self::actor`] is this device's actor (`false` signals
    /// "ephemeral, this process spun a new ops jsonl"). Used by
    /// diagnostics and doctor reporting.
    pub ephemeral_actor: bool,
    /// Hold the shared workspace lock for the lifetime of the context.
    #[allow(dead_code)]
    lock: WorkspaceLock,
    /// Hold the exclusive per-actor write lock too.
    #[allow(dead_code)]
    actor_lock: ActorWriteLock,
}

impl WsCtx {
    /// Mutate `page` and run the four steps that go around it.
    ///
    /// Thin wrapper over [`outl_actions::commit_page`] bound to this
    /// context. Reach for it instead of calling
    /// `apply_page_md_with_sidecar_guarded` after a mutation: that is
    /// step 5 of five, and a caller that takes it alone has silently
    /// decided the other four do not apply
    /// ([#264](https://github.com/outlmd/outl/issues/264)).
    ///
    /// Two of those four are genuinely no-ops here and stay defaulted.
    /// A one-shot CLI command has no undo stack to snapshot into and no
    /// cached backlink index to invalidate, both of which are per-process
    /// state a long-lived client keeps. The **peer announce** is the one
    /// worth naming: this context holds no transport, so a CLI write
    /// reaches other devices on the next catch-up sweep rather than
    /// immediately. That is the pre-existing behaviour, not a regression
    /// introduced here, and wiring `SyncEngine` into the hook is the
    /// obvious follow-up.
    ///
    /// A projection failure is returned as an error, matching what the
    /// hand-written call sites did with `?`.
    pub fn commit_with<T, F>(&mut self, page: NodeId, mutate: F) -> Result<T, ActionError>
    where
        F: FnOnce(&mut Workspace) -> Result<T, ActionError>,
    {
        let root = self.root.clone();
        let mut hooks = CtxCommit {
            root: &root,
            failure: None,
        };
        let value = commit_page(&mut self.workspace, &mut hooks, page, mutate)?;
        match hooks.failure {
            Some(e) => Err(e),
            None => Ok(value),
        }
    }

    /// Run the pipeline for a page whose mutation already happened.
    ///
    /// Most CLI commands mutate through a plain `outl_actions::` call and
    /// then need the steps around it, so they would otherwise all pass
    /// `|_ws| Ok(())` to [`Self::commit_with`]. Same sequence, without
    /// the empty closure at eight call sites.
    pub fn commit(&mut self, page: NodeId) -> Result<(), ActionError> {
        self.commit_with(page, |_ws| Ok(()))
    }
}

/// [`WsCtx`]'s half of the commit pipeline.
///
/// `project` is the only hook implemented. It cannot return the failure
/// directly (the trait keeps `project` infallible on purpose, since it
/// runs after the op log already holds the mutation), so it parks it
/// here and [`WsCtx::commit`] reads it back.
struct CtxCommit<'a> {
    root: &'a Path,
    failure: Option<ActionError>,
}

impl CommitHooks for CtxCommit<'_> {
    fn project(&mut self, workspace: &Workspace, page: NodeId) {
        if let Err(e) = outl_actions::apply_page_md_with_sidecar_guarded(workspace, self.root, page)
        {
            self.failure = Some(e);
        }
    }
}

/// Tuning knobs for [`open_with`]. The derived `Default` is the
/// short-lived contract: snapshots are read, never written.
#[derive(Debug, Clone, Copy, Default)]
pub struct OpenOptions {
    /// Whether this process may **write** boot snapshots.
    ///
    /// Defaults to `false`: short-lived openers (CLI commands,
    /// embedders) read snapshots but must not write them — a write
    /// here would race with the long-lived TUI/desktop/mobile that
    /// own the workspace (#109). A long-lived surface adopting this
    /// crate opts in, otherwise its boot/reload cost on a large
    /// workspace silently regresses to full log replay.
    pub write_snapshots: bool,
}

/// Open the workspace at `path` with the short-lived defaults
/// ([`OpenOptions::default`]). Returns a typed [`WsError`] on any
/// boot failure (missing workspace, bad config, filesystem error).
pub fn open(path: &Path) -> Result<WsCtx, WsError> {
    open_with(path, OpenOptions::default())
}

/// Like [`open`], but the caller picks the [`OpenOptions`] — a
/// long-lived surface (TUI, desktop, an embedder that stays resident)
/// sets `write_snapshots: true` to keep boot O(delta) on large
/// workspaces.
pub fn open_with(path: &Path, opts: OpenOptions) -> Result<WsCtx, WsError> {
    let paths = Paths::at(path.to_path_buf());
    if !paths.dot_outl.exists() {
        return Err(WsError::NoWorkspace(paths.root.clone()));
    }

    // Shared workspace lock first — every well-behaved opener takes one,
    // and nothing below should touch `.outl/` before it is held.
    let lock = WorkspaceLock::acquire(&paths.root).map_err(WsError::internal)?;

    let cfg = read_or_init_config(&paths).map_err(|e| WsError::Config(e.to_string()))?;
    // Which actor this DEVICE owns. Read from the device store, never
    // from the workspace — `.outl/config.toml` travels with the
    // workspace on every file transport except iCloud, and two devices
    // reading one actor id is silent op loss (see `crate::actor`).
    //
    // The lock does not *serialize* this (it admits multiple openers by
    // design), so resolution must be safe against a concurrent one on its
    // own: it never writes `config.toml`, and the device-local binding it
    // does write is an atomic compare-and-swap.
    let device_actor = actor::resolve_device_actor(&paths, &cfg, &DeviceStore::open_default())
        .map_err(|e| WsError::InvalidActor(e.to_string()))?;

    ensure_ops_dir(&paths).map_err(WsError::internal)?;

    // Exclusive per-actor write lock: the device actor if available,
    // ephemeral otherwise. This is the contract that lets multiple
    // `outl` processes on ONE machine share the workspace safely.
    let (actor_lock, actor) =
        resolve_write_actor(&paths.ops, device_actor).map_err(WsError::internal)?;
    let ephemeral_actor = actor != device_actor;

    let storage = JsonlStorage::open(paths.ops.clone(), actor).map_err(WsError::internal)?;
    let mut workspace =
        Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))
            .map_err(WsError::internal)?;
    // Snapshot writes are opt-in (see `OpenOptions::write_snapshots`):
    // short-lived openers read snapshots but must not write them — a
    // write here would race with the long-lived owner of the
    // workspace, and reads happen regardless of this policy
    // (`Workspace::open_with_storage` → `snapshot::read_from_disk`).
    if !opts.write_snapshots {
        workspace.set_snapshot_policy(false, 0);
    }
    // Register per-page shards if the workspace has been migrated
    // (RFC #137 Phase B). No-op for legacy Global workspaces.
    outl_actions::storage_scope::register_per_page_storages(
        &mut workspace,
        &paths.ops,
        actor,
        &paths.root,
    );
    // If per-page shards were registered, re-boot so the materialized
    // tree includes their ops. The initial boot only saw the Global
    // storage (which is empty after migration).
    if workspace.has_page_storages() {
        workspace
            .reboot_with_all_storages()
            .map_err(WsError::internal)?;
        if !opts.write_snapshots {
            workspace.set_snapshot_policy(false, 0);
        }
    }
    let hlc = HlcGenerator::new(actor);
    // Raise the fresh generator above every timestamp already on disk,
    // before anything below emits an op. A generator built by
    // `HlcGenerator::new` knows nothing about the log, so after the wall
    // clock moves backwards between two runs it issues timestamps that
    // sort *below* what the log holds — a cost, not a divergence (see
    // `Workspace::seed_clock`).
    //
    // Propagated rather than warned about: this reads the per-actor
    // maxima off the index the boot above just built, so a failure here
    // means storage stopped being readable between two statements, and a
    // workspace that cannot answer "what is the newest op" has a bigger
    // problem than an unseeded clock.
    workspace.seed_clock(&hlc).map_err(WsError::internal)?;

    // Repair split-brain page/journal roots (two roots sharing one slug, e.g. a
    // sidecar-less `.md` reconciled to a fresh id) before any command reads or
    // writes. Idempotent and a no-op when clean; only emits Ops when a duplicate
    // exists, and those converge across devices via the op log. Covers every CLI
    // command, the long-lived MCP server, and embedders (all open through here).
    //
    // Cost: an O(roots) scan grouping page roots by slug. It runs on every open,
    // but the boot above already replays the whole op log (O(ops), far larger),
    // so this scan is noise next to it. If a huge workspace ever makes it show
    // up in a profile, gate it on a persisted "merged at op-count N" marker in
    // `.outl/` rather than paying the scan when nothing changed.
    match outl_actions::merge_duplicate_slug_roots(&mut workspace, &hlc) {
        Ok(0) => {}
        Ok(n) => tracing::warn!("merged {n} duplicate slug root(s)"),
        Err(e) => tracing::warn!("duplicate-slug-root repair: {e}"),
    }

    Ok(WsCtx {
        root: paths.root.clone(),
        paths,
        workspace,
        hlc,
        actor,
        ephemeral_actor,
        lock,
        actor_lock,
    })
}
