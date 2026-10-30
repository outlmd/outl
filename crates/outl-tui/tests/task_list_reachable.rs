//! The TUI can compute the workspace's open tasks.
//!
//! `collect_open_tasks` used to live in `outl-tauri-shared`, which this
//! crate cannot depend on — the dependency edge points the other way.
//! So `Capability::TaskList` was `Missing` on the TUI for a reason that
//! was about crate layout rather than about the terminal
//! ([#454](https://github.com/outlmd/outl/issues/454)).
//!
//! This is a reachability test, not a UI one. It fails to **compile**
//! if the function goes back behind that edge, which is the regression
//! worth catching: the ordering is a product decision, and a client
//! that cannot call it either re-derives it or does without.
//!
//! The TUI still has no task list overlay, so the capability verdict
//! stays `Missing` until one exists. What changed is that the verdict
//! is now about missing UI instead of an unreachable crate.

use outl_core::hlc::HlcGenerator;
use outl_core::{ActorId, Workspace};
use outl_md::index::WorkspaceIndex;
use outl_md::reconcile::reconcile_md;
use std::fs;
use tempfile::TempDir;

#[test]
fn the_tui_can_collect_open_tasks_from_a_workspace() {
    let dir = TempDir::new().unwrap();
    let pages = dir.path().join("pages");
    fs::create_dir_all(&pages).unwrap();
    let page = pages.join("work.md");
    fs::write(
        &page,
        "- TODO ship the thing\n- DOING halfway there\n- DONE already shipped\n- just a note\n",
    )
    .unwrap();

    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).unwrap();
    let hlc = HlcGenerator::new(actor);
    reconcile_md(&mut ws, &hlc, &page, None).unwrap();

    let index = WorkspaceIndex::build(dir.path());
    let out = outl_actions::tasks::collect_open_tasks(
        index.iter_blocks(),
        &index,
        outl_actions::tasks::TASK_LIMIT,
    );

    assert_eq!(out.total, 2, "DONE and the plain note are not open tasks");
    let texts: Vec<&str> = out.tasks.iter().map(|t| t.text.as_str()).collect();
    assert!(texts.contains(&"halfway there"), "got {texts:?}");
    assert!(texts.contains(&"ship the thing"), "got {texts:?}");
    assert_eq!(
        out.tasks.first().map(|t| t.status.as_str()),
        Some("doing"),
        "started work is ordered first, and that ordering is the shared decision"
    );
}
