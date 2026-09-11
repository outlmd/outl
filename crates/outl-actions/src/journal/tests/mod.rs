//! Tests for the journal projection family (render + apply + paths).

use std::path::PathBuf;

use super::*;
use crate::block::append_block;
use crate::page::{open_journal, open_or_create, page_meta, PageKind};
use chrono::NaiveDate;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

/// Build a page projected while it held `initial`, then return the
/// `(workspace, hlc, root, page_id, md_path)` so a test can drive the
/// stale-projection scenarios. The `TempDir` is returned to keep it alive.
fn projected_page(initial: &str) -> (TempDir, Workspace, HlcGenerator, NodeId, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let actor = ActorId::new();
    let hlc = HlcGenerator::new(actor);
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let page = open_or_create(&mut ws, &hlc, "notes", "Notes", PageKind::Page).unwrap();
    append_block(&mut ws, &hlc, Some(page), Some(initial)).unwrap();
    apply_page_md_with_sidecar(&ws, tmp.path(), page).unwrap();
    let md_path = page_md_path(tmp.path(), &page_meta(&ws, page).unwrap());
    (tmp, ws, hlc, page, md_path)
}

/// Re-stamp `path`'s sidecar so it declares the bytes currently on disk as
/// the last faithful projection, without changing its block entries. This
/// reproduces the state a `reconcile_md` leaves behind when it rewrites the
/// sidecar to agree with a `.md` whose content never became ops.
fn restamp_sidecar_as_faithful(md_path: &PathBuf) {
    let sidecar_path = outl_md::sidecar::sidecar_path_for(md_path);
    let mut sc = outl_md::sidecar::read(&sidecar_path).unwrap();
    let disk = std::fs::read_to_string(md_path).unwrap();
    sc.last_synced_hash = outl_md::sidecar::file_hash(&disk);
    outl_md::sidecar::write(&sidecar_path, &sc).unwrap();
}

mod guard;
mod if_stale;
mod render;
