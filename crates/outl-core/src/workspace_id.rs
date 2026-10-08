//! Stable, shared identity of a workspace.
//!
//! A workspace is identified by a `WorkspaceId` — a ULID generated **once**
//! and persisted inside the workspace directory, **not** derived from the local
//! filesystem path. Two paired devices that share one workspace must compute the
//! same id, even though their local paths differ (desktop `~/outl-p2p`, mobile
//! `…/app.outl.mobile-app/outl`). The id is what the P2P transport hashes into
//! the gossip topic, so a stable shared id is what makes two real devices land
//! on the same topic and sync as one workspace.
//!
//! ## Where it lives
//!
//! `<root>/.outl/workspace-id` — a single-line plaintext ULID. It sits in the
//! `.outl/` metadata directory next to `config.toml` and `peers.toml`, so it
//! never pollutes the clean markdown (invariant #2) and never reaches a rendered
//! page. It is its own file (not folded into `config.toml`) because
//! `config.toml` carries the **per-device** `actor_id` — a value that must
//! *differ* per device — whereas the workspace id must be **identical** across
//! devices. Keeping them separate means pairing-adoption (the joiner overwriting
//! its workspace id with the host's) can rewrite the id without touching the
//! device's own actor.
//!
//! ## Pairing adoption
//!
//! When two devices pair, the joiner [`WorkspaceId::write`]s the host's id into
//! its own `.outl/workspace-id`, so both sides then derive the same gossip topic
//! and reconcile as one workspace. The op logs CRDT-merge afterwards — content
//! from both devices converges, which is the intended outcome.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Stable identifier of a workspace, shared by every device that syncs it.
///
/// A ULID, persisted at `<root>/.outl/workspace-id`. Unlike [`crate::ActorId`]
/// (one per device) there is exactly one `WorkspaceId` per workspace, the same
/// bytes on every paired device.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(String);

/// Errors reading or writing the workspace id file.
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceIdError {
    /// The `.outl/workspace-id` file could not be read or written.
    #[error("workspace id io at {path}: {source}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying io error.
        source: std::io::Error,
    },
}

impl WorkspaceId {
    /// Generate a fresh workspace id.
    pub fn new() -> Self {
        Self(ulid::Ulid::generate().to_string())
    }

    /// Wrap a raw string as a workspace id (e.g. one received over the wire from
    /// a pairing host). The value is trusted as-is; it is opaque to the
    /// transport beyond equality + hashing.
    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// The id as a string slice — the form hashed into the gossip topic and put
    /// on the sync-request wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Path of the workspace-id file under a workspace root
    /// (`<root>/.outl/workspace-id`).
    pub fn path_for(root: &Path) -> PathBuf {
        root.join(".outl").join("workspace-id")
    }

    /// Read the workspace id from `<root>/.outl/workspace-id`, generating and
    /// persisting a fresh one if the file does not exist yet.
    ///
    /// This is the migration path: an existing workspace with no id file gets
    /// one on first open and keeps it stable thereafter. A blank/whitespace-only
    /// file (e.g. truncated by a crash) is treated as missing and regenerated.
    pub fn read_or_create(root: &Path) -> Result<Self, WorkspaceIdError> {
        let path = Self::path_for(root);
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                let trimmed = contents.trim();
                if trimmed.is_empty() {
                    let id = Self::new();
                    id.write(root)?;
                    Ok(id)
                } else {
                    Ok(Self(trimmed.to_string()))
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let id = Self::new();
                id.write(root)?;
                Ok(id)
            }
            Err(source) => Err(WorkspaceIdError::Io { path, source }),
        }
    }

    /// Persist this id to `<root>/.outl/workspace-id`, creating the `.outl/`
    /// directory if needed. Used both on first generation and by pairing
    /// adoption (the joiner overwriting its id with the host's).
    pub fn write(&self, root: &Path) -> Result<(), WorkspaceIdError> {
        let path = Self::path_for(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| WorkspaceIdError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(&path, format!("{}\n", self.0))
            .map_err(|source| WorkspaceIdError::Io { path, source })
    }
}

impl Default for WorkspaceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn read_or_create_generates_then_is_stable() {
        let dir = TempDir::new().unwrap();
        let first = WorkspaceId::read_or_create(dir.path()).unwrap();
        let second = WorkspaceId::read_or_create(dir.path()).unwrap();
        assert_eq!(first, second, "id must be stable across opens");
        assert!(WorkspaceId::path_for(dir.path()).exists());
    }

    #[test]
    fn distinct_workspaces_get_distinct_ids() {
        let a = TempDir::new().unwrap();
        let b = TempDir::new().unwrap();
        let ida = WorkspaceId::read_or_create(a.path()).unwrap();
        let idb = WorkspaceId::read_or_create(b.path()).unwrap();
        assert_ne!(ida, idb);
    }

    #[test]
    fn write_overwrites_for_pairing_adoption() {
        let dir = TempDir::new().unwrap();
        let original = WorkspaceId::read_or_create(dir.path()).unwrap();
        let adopted = WorkspaceId::from_raw("HOST00000000000000000000000");
        adopted.write(dir.path()).unwrap();
        let reread = WorkspaceId::read_or_create(dir.path()).unwrap();
        assert_eq!(reread, adopted);
        assert_ne!(reread, original);
    }

    #[test]
    fn blank_file_is_regenerated() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join(".outl")).unwrap();
        std::fs::write(WorkspaceId::path_for(dir.path()), "  \n").unwrap();
        let id = WorkspaceId::read_or_create(dir.path()).unwrap();
        assert!(!id.as_str().is_empty());
    }
}
