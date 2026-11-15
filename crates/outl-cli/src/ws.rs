//! Shared workspace bootstrap used by every machine-shaped subcommand.
//!
//! The open logic lives in [`outl_ws`] (shared with embedders); this
//! module keeps the CLI-facing signature, mapping every boot failure to
//! an [`ApiError`] with a stable code.

use std::path::Path;

pub use outl_ws::WsCtx;
use outl_ws::WsError;

use crate::output::{codes, ApiError};

fn map_open(result: Result<WsCtx, WsError>) -> Result<WsCtx, ApiError> {
    result.map_err(|e| match e {
        WsError::NoWorkspace(_) => ApiError::new(codes::NO_WORKSPACE, e.to_string()),
        WsError::Config(_) => ApiError::new(codes::NO_WORKSPACE, e.to_string()),
        WsError::InvalidActor(_) => ApiError::new(codes::INTERNAL, e.to_string()),
        WsError::Internal(msg) => ApiError::internal(msg),
        WsError::DeviceActorHeld => ApiError::new(codes::INTERNAL, e.to_string()),
    })
}

/// Open the workspace at `path` for writing. Returns a stable
/// [`ApiError`] on any boot failure (missing workspace, bad config,
/// filesystem error).
///
/// See [`outl_ws::open`] for the lock/actor policy. A command that only
/// reads uses [`open_read`] so a held write lock does not mint an actor.
pub fn open(path: &Path) -> Result<WsCtx, ApiError> {
    // Writing opens persist the boot cache; reads do not.
    //
    // A CLI command is short-lived, so it was excluded from snapshot
    // writes wholesale. But the exclusion left a real hole: on a
    // machine driven only from the terminal nothing else writes one
    // either, so **every** `outl` invocation full-replayed the log
    // forever — 330-357ms for `outl page get` on a 10k-page workspace
    // against 132-166ms with a snapshot present.
    //
    // The write is enabled here and not in `open_read` because that is
    // where the cost is attributable (root `CLAUDE.md` invariant 11):
    // this command is already encoding and fsyncing ops, and
    // `Workspace::apply` only spawns the write when the threshold is
    // crossed — which after a full replay means "there was no usable
    // snapshot", i.e. exactly the case that needs one. A read paying
    // for the next process is the trade that stays refused.
    //
    // Two things make it safe now rather than when #109 refused it:
    // `write_to_disk` uses a per-write scratch, so a concurrent GUI
    // writer is no longer a shared inode; and `WsCtx::drop` waits out
    // the worker, so a 200ms command cannot exit mid-fsync.
    //
    // **The user's `[snapshot]` section decides, not this function.**
    // `outl-ws` has no `outl-config` dependency, so enabling writes
    // there would run on `SnapshotPolicy`'s hardcoded default threshold
    // and ignore `enabled = false` outright — a documented opt-out
    // silently overridden, on the first op, because `seed_from_log_len`
    // leaves the counter at the threshold after a full replay. So the
    // policy is applied here, by the crate that already reads config.
    let snapshot = outl_config::load().snapshot;
    let opts = outl_ws::OpenOptions {
        write_snapshots: snapshot.enabled,
        ..Default::default()
    };
    let mut ctx = map_open(outl_ws::open_with(path, opts))?;
    if snapshot.enabled {
        ctx.workspace
            .set_snapshot_policy(snapshot.enabled, snapshot.op_threshold);
    }
    Ok(ctx)
}

/// Open the workspace at `path` for reading.
///
/// Same boot failures as [`open`], and no per-actor write lock. See
/// [`outl_ws::open_read`].
pub fn open_read(path: &Path) -> Result<WsCtx, ApiError> {
    map_open(outl_ws::open_read(path))
}

/// Open the workspace for a **resident** writer that must stay on the device
/// actor: the embedded MCP server of `outl serve`.
///
/// Returns the context and whether it is read-only. When another process
/// already holds the device actor's write lock, a normal open would mint an
/// ephemeral actor and leave an `ops-<ulid>.jsonl` plus a lock file behind
/// for good. This one refuses that and falls back to a read open instead, so
/// the server still answers reads and every write fails with an explicit
/// error. Snapshots are written, since the caller outlives the command.
pub fn open_resident(path: &Path) -> Result<(WsCtx, bool), ApiError> {
    let opts = outl_ws::OpenOptions {
        write_snapshots: true,
        writes: true,
        device_actor_only: true,
    };
    match outl_ws::open_with(path, opts) {
        Ok(wc) => Ok((wc, false)),
        Err(WsError::DeviceActorHeld) => {
            tracing::warn!(
                "mcp: another outl process holds the device actor; the embedded server is read-only until it exits"
            );
            open_read(path).map(|wc| (wc, true))
        }
        Err(e) => map_open(Err(e)).map(|wc| (wc, false)),
    }
}

/// Open the workspace for writing under the device actor and nothing else.
///
/// The on-demand counterpart of [`open_resident`]: when another process holds
/// the device actor it fails with an explicit error instead of falling back
/// to a read open, and it never mints an ephemeral actor. Used by the
/// embedded MCP server of `outl serve --no-watch` when a mutating tool
/// arrives.
pub fn open_device_writer(path: &Path) -> Result<WsCtx, ApiError> {
    let opts = outl_ws::OpenOptions {
        write_snapshots: true,
        writes: true,
        device_actor_only: true,
    };
    match outl_ws::open_with(path, opts) {
        Err(WsError::DeviceActorHeld) => Err(ApiError::new(
            codes::INTERNAL,
            "another outl process (a GUI, TUI or CLI) holds the device actor's write lock, \
             so this write cannot run; close it or retry once it exits. \
             Reads keep working."
                .to_string(),
        )),
        other => map_open(other),
    }
}
