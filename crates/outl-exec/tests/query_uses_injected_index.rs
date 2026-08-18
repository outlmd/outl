//! The `query` runtime must answer from `ExecContext::index` when the
//! caller supplied one, and only fall back to a disk build when it did
//! not.
//!
//! Why this is worth a test rather than a code comment: the fallback is
//! *correct*, just expensive — it walks every `.md` and every sidecar in
//! the workspace, and `query` auto-runs on every page load, so a
//! regression here is invisible in behaviour and shows up only as the
//! app getting slower on large workspaces. Nothing else would fail.
//!
//! The two are told apart by giving them **different** answers: the
//! index is populated by hand and the on-disk workspace is left empty,
//! so a hit can only have come from the injected index.

#![cfg(feature = "lang-query")]

use std::path::{Path, PathBuf};

use outl_core::id::NodeId;
use outl_exec::{ExecContext, Runtime, RuntimeRegistry};
use outl_md::block_index::IdentifiedNode;
use outl_md::index::{PageEntry, WorkspaceIndex};
use tempfile::TempDir;

/// An index holding one TODO block, built without touching any file.
fn index_with_one_todo(root: &Path) -> (WorkspaceIndex, NodeId) {
    let mut idx = WorkspaceIndex::default();
    let page_path = root.join("pages/notes.md");
    idx.insert_page(PageEntry {
        path: page_path.clone(),
        slug: "notes".to_string(),
        title: "Notes".to_string(),
        icon: None,
        is_journal: false,
        pinned: false,
        page_type: None,
    });
    let id = NodeId::new();
    let blocks = vec![IdentifiedNode {
        id,
        text: "TODO only in the injected index".to_string(),
        properties: Vec::new(),
        children: Vec::new(),
    }];
    idx.collect_page_blocks_from_tree("notes", &page_path, &blocks);
    idx.collect_refs_from_indexed();
    (idx, id)
}

fn query_runtime() -> std::sync::Arc<dyn Runtime> {
    RuntimeRegistry::with_builtins()
        .get("query")
        .expect("the query runtime is registered")
}

fn empty_workspace() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    std::fs::create_dir_all(dir.path().join("journals")).unwrap();
    let root = dir.path().to_path_buf();
    (dir, root)
}

#[test]
fn the_query_runtime_answers_from_the_injected_index() {
    let (_dir, root) = empty_workspace();
    let (index, id) = index_with_one_todo(&root);

    let ctx = ExecContext {
        workspace_root: root.clone(),
        index: Some(&index),
        ..Default::default()
    };
    let out = query_runtime()
        .execute("status: todo", &ctx)
        .expect("query runs");

    // The workspace on disk is empty, so a hit proves the index was
    // consulted rather than the filesystem.
    let handle = index
        .block_by_id(id)
        .expect("the block is indexed")
        .ref_handle
        .clone();
    assert_eq!(
        out.stdout,
        format!("!(({handle}))"),
        "the runtime must emit the injected index's block as an embed"
    );
}

#[test]
fn without_an_injected_index_it_falls_back_to_the_workspace_on_disk() {
    let (_dir, root) = empty_workspace();
    // Same query, same (empty) workspace, no index handed in.
    let ctx = ExecContext {
        workspace_root: root,
        index: None,
        ..Default::default()
    };
    let out = query_runtime()
        .execute("status: todo", &ctx)
        .expect("query runs");

    assert_eq!(
        out.stdout, "",
        "with no index and no pages on disk there is nothing to match"
    );
}

#[test]
fn only_the_query_runtime_asks_for_a_workspace_index() {
    // The signal that lets a caller skip deriving an index for a fence
    // that will not read one. Without it every `run_code_block` — a
    // python block, a lisp block — pays for a full workspace
    // derivation to serve a facility only `query` uses.
    //
    // Asserted over the whole registry rather than on `query` alone, so
    // a runtime that starts reading `ctx.index` without saying so shows
    // up here.
    let registry = RuntimeRegistry::with_builtins();
    let asking: Vec<String> = registry
        .languages()
        .filter(|lang| {
            registry
                .get(lang)
                .is_some_and(|r| r.needs_workspace_index())
        })
        .map(String::from)
        .collect();

    assert_eq!(
        asking,
        vec!["query".to_string()],
        "only `query` reads ExecContext::index; a new one here means \
         every caller's derive gate needs revisiting"
    );
}
