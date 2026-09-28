//! Opening the workspace: the locks, the device actor, the storage
//! backend, and the caps applied to the materialized tree.
//!
//! One function, called once per launch, whose return value the rest
//! of the session is built on — including two RAII guards whose
//! ownership contract is spelled out on it.

use anyhow::{Context, Result};
use outl_core::id::ActorId;
use outl_core::storage::{JsonlStorage, Storage};
use outl_core::workspace::Workspace;
use std::fs;
use std::path::Path;

/// Open the workspace at `root` and return everything the TUI needs
/// to run for the rest of its lifetime.
///
/// **Lock ownership is on the caller.** The returned tuple's last
/// two fields are RAII guards (`WorkspaceLock` is shared,
/// `ActorWriteLock` is exclusive per actor). They must outlive every
/// write against the workspace — that is, the entire TUI session.
/// In practice that means binding them in
/// [`run_with_theme_override`]'s top scope so they drop after the
/// event loop returns. **Don't** pass the workspace into another
/// function without forwarding the locks, and **don't** rebind the
/// guards in a tighter scope.
///
/// Returns `(workspace, actor, cfg, workspace_lock, actor_write_lock)`.
pub(super) fn open_workspace(
    root: &Path,
) -> Result<(
    Workspace,
    ActorId,
    toml::Value,
    outl_core::WorkspaceLock,
    outl_core::ActorWriteLock,
)> {
    // Shared workspace lock — every well-behaved `outl` opener piles
    // on. Concurrent TUI + MCP server + sink-outl plugin is the
    // supported case; per-actor write isolation comes from the
    // ActorWriteLock below.
    let lock = outl_core::WorkspaceLock::acquire(root)
        .with_context(|| format!("could not acquire workspace lock at {}", root.display()))?;

    let paths = outl_ws::layout::Paths::at(root.to_path_buf());
    if !paths.dot_outl.is_dir() {
        anyhow::bail!(
            "no outl workspace at {} — run `outl init` first",
            root.display()
        );
    }
    // `read_or_init_config` seeds a config when the `.outl/` dir exists
    // but `config.toml` doesn't — a workspace created by a GUI client or
    // by P2P sync, which only seed `.outl/workspace-id`. Going through
    // `outl-ws` instead of parsing the TOML here is what keeps the TUI
    // and the CLI on one schema (the hand-rolled seed used to omit
    // `created_at`, which `outl-ws` then refused to deserialize).
    let cfg = outl_ws::layout::read_or_init_config(&paths)?;
    // Which actor this DEVICE owns. Read from the device store, never
    // from `.outl/config.toml`: that file rides the file-sync surface,
    // and two devices reading one actor id is silent op loss. See
    // `outl_ws::actor`.
    let device_actor = outl_ws::actor::resolve_device_actor(
        &paths,
        &cfg,
        &outl_core::device::DeviceStore::open_default(),
    )?;
    // `outl_ws::layout::Config` models only the `[workspace]` section;
    // `resolve_theme` reads the optional per-workspace `[theme]` off the
    // raw document. A malformed file degrades to "no override" — the
    // theme falls back to the global config, never a failed open.
    let cfg_raw: toml::Value = fs::read_to_string(&paths.config)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_else(|| toml::Value::Table(Default::default()));

    let ops_dir = paths.ops.clone();
    fs::create_dir_all(&ops_dir)
        .with_context(|| format!("creating ops dir at {}", ops_dir.display()))?;

    // Exclusive per-actor write lock. Falls back to an ephemeral
    // actor when another `outl` on this machine already owns the device
    // actor — that's how a TUI + MCP server share the same workspace
    // without racing on `ops-<device_actor>.jsonl`.
    let (actor_lock, actor) = outl_core::resolve_write_actor(&ops_dir, device_actor)
        .with_context(|| format!("acquiring per-actor write lock at {}", ops_dir.display()))?;
    if actor != device_actor {
        tracing::info!(
            "another outl process owns the device actor {device_actor}; this TUI writes under ephemeral actor {actor}"
        );
    }

    // JsonlStorage is the only persistent backend (see CHANGELOG
    // 0.5.0). The directory is created above because sync transports
    // sometimes garbage-collect empty dirs between runs. Not named
    // `.ops/` because iCloud skips dotted paths.
    let storage: Box<dyn Storage> = Box::new(
        JsonlStorage::open(ops_dir.clone(), actor)
            .with_context(|| format!("opening jsonl storage at {}", ops_dir.display()))?,
    );
    let mut ws = Workspace::open_with_storage(actor, storage, Some(root.to_path_buf()))?;
    let lru_cap = outl_config::load().storage.lru_cap;
    // Register per-page shards BEFORE applying the LRU cap. Shards
    // open unbounded (cap = 0) so the reboot replays the full history.
    outl_actions::storage_scope::register_per_page_storages(&mut ws, &ops_dir, actor, root);
    if ws.has_page_storages() {
        ws.reboot_with_all_storages()?;
    }
    // Shed cold history AFTER the materialized tree is complete.
    ws.apply_lru_cap(lru_cap);
    // Snapshot boot-cache policy (#128/#109): a long-lived client writes
    // background snapshots so the next open boots from one instead of
    // replaying the whole op log. `Drop for App` also flushes a final
    // snapshot on exit. Defaults (enabled, 10k) unless `[snapshot]`
    // overrides them.
    let snap_cfg = outl_config::load().snapshot;
    ws.set_snapshot_policy(snap_cfg.enabled, snap_cfg.op_threshold);
    Ok((ws, actor, cfg_raw, lock, actor_lock))
}
