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
    })
}

/// Open the workspace at `path` for writing. Returns a stable
/// [`ApiError`] on any boot failure (missing workspace, bad config,
/// filesystem error).
///
/// See [`outl_ws::open`] for the lock/actor policy. A command that only
/// reads uses [`open_read`] so a held write lock does not mint an actor.
pub fn open(path: &Path) -> Result<WsCtx, ApiError> {
    map_open(outl_ws::open(path))
}

/// Open the workspace at `path` for reading.
///
/// Same boot failures as [`open`], and no per-actor write lock. See
/// [`outl_ws::open_read`].
pub fn open_read(path: &Path) -> Result<WsCtx, ApiError> {
    map_open(outl_ws::open_read(path))
}
