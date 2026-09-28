//! The orphan record.
//!
//! An id that reaches level 3 is about to become
//! `Move(node, TRASH_ROOT)`. Crate invariant 2 is that no such block
//! disappears silently, so the line lands in `orphans.log` **before**
//! the move is applied — this module is the only writer, and it is
//! deliberately the only part of a pass that touches a file other than
//! the page's own `.md` and sidecar.

use super::outcome::{io_err, ReconcileError};
use crate::sidecar::SidecarBlock;
use outl_core::id::NodeId;
use std::fs;
use std::io::Write;
use std::path::Path;

pub(super) fn log_orphans(
    log_path: &Path,
    md_path: &Path,
    orphans: &[NodeId],
    old_blocks: &[SidecarBlock],
) -> Result<(), ReconcileError> {
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| io_err(log_path, e))?;
    let now = chrono::Local::now().to_rfc3339();
    for id in orphans {
        let hash_snippet = old_blocks
            .iter()
            .find(|b| b.id == *id)
            .map(|b| b.content_hash.as_str())
            .unwrap_or("?");
        writeln!(
            f,
            "{now}\tmd={}\tid={}\thash={}",
            md_path.display(),
            id,
            hash_snippet,
        )
        .map_err(|e| io_err(log_path, e))?;
    }
    Ok(())
}
