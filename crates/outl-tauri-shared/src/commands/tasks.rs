//! The Tauri command behind the clients' task list.
//!
//! The selection, the ordering and the wire shapes live in
//! [`outl_actions::tasks`]; they moved there so the TUI could reach
//! them ([#454](https://github.com/outlmd/outl/issues/454)). What is
//! left here is the half that needs Tauri: the `AppHost` lookup and
//! the offload, because the index build reads and parses every `.md`
//! in the workspace and the IPC thread may not do that work.

pub use outl_actions::tasks::{collect_open_tasks, OpenTasks, TaskHit, TASK_LIMIT};

use outl_md::index::WorkspaceIndex;

use crate::host::AppHost;

/// List the workspace's open tasks.
///
/// Async: the index build reads and parses every `.md` + sidecar in the
/// workspace, and mobile fires this from the bottom bar on mount — the
/// IPC thread may not do that work ([`crate::offload`]).
pub async fn list_open_tasks<S: AppHost>(state: &S) -> Result<OpenTasks, String> {
    let root = state.storage_root()?;
    crate::offload::run("list_open_tasks", move || {
        let index = WorkspaceIndex::build(&root);
        Ok(collect_open_tasks(index.iter_blocks(), &index, TASK_LIMIT))
    })
    .await
}
