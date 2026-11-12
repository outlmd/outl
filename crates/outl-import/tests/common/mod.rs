//! Shared harness for the importer end-to-end tests: a real
//! `Workspace` over `JsonlStorage` in a tempdir, no CLI involved.
#![allow(dead_code)]

pub mod roam;

use outl_core::hlc::HlcGenerator;
use outl_core::id::ActorId;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
use outl_import::{run_import, ImportDest, ImportOptions, ImportReport, SourceAdapter};
use std::fs;
use std::path::{Path, PathBuf};

pub struct TestWs {
    pub _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub workspace: Workspace,
    pub hlc: HlcGenerator,
}

pub fn open_test_ws() -> TestWs {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("ws");
    for sub in ["pages", "journals", "ops", ".outl"] {
        fs::create_dir_all(root.join(sub)).expect("mkdir");
    }
    let actor = ActorId::new();
    let storage = JsonlStorage::open(root.join("ops"), actor).expect("storage");
    let workspace = Workspace::open_with_storage(actor, Box::new(storage), Some(root.clone()))
        .expect("workspace");
    let hlc = HlcGenerator::new(actor);
    TestWs {
        _dir: dir,
        root,
        workspace,
        hlc,
    }
}

pub fn import_with(adapter: &dyn SourceAdapter, src: &Path) -> (TestWs, ImportReport) {
    import_with_opts(adapter, src, &ImportOptions::default())
}

pub fn import_with_opts(
    adapter: &dyn SourceAdapter,
    src: &Path,
    opts: &ImportOptions,
) -> (TestWs, ImportReport) {
    let mut ws = open_test_ws();
    let report = import_into(adapter, src, &mut ws, opts);
    (ws, report)
}

/// Import into an existing workspace (for reimport/idempotency tests).
pub fn import_into(
    adapter: &dyn SourceAdapter,
    src: &Path,
    ws: &mut TestWs,
    opts: &ImportOptions,
) -> ImportReport {
    let dest = ImportDest {
        workspace: &mut ws.workspace,
        hlc: &ws.hlc,
        root: ws.root.clone(),
        pages_dir: ws.root.join("pages"),
        journals_dir: ws.root.join("journals"),
        orphans: ws.root.join(".outl").join("orphans.log"),
    };
    run_import(adapter, src, dest, opts).expect("import runs")
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Write a tree of `(relative_path, content)` files under a tempdir.
pub fn fixture_tree(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for (rel, content) in files {
        let p = dir.path().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&p, content).expect("write fixture");
    }
    dir
}
