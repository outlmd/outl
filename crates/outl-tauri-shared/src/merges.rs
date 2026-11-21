//! Host glue for the merged edits log. The log itself (detection, cap,
//! on-disk state) lives in `outl_actions::merges` so the terminal shares
//! it; this file adds the two commands the GUI clients call.

pub use outl_actions::merges::{detect, record, MergedEditDto, MERGE_LOG_CAP};
use outl_actions::merges::{list, take};

use crate::commit::finish_in_page;
use crate::helpers::parse_node_id;
use crate::host::AppHost;
use crate::state::PageView;

/// The merged edits recorded on this device, newest first.
pub fn merged_edits<S: AppHost>(state: &S) -> Result<Vec<MergedEditDto>, String> {
    let root = state.storage_root()?;
    Ok(list(&root, state.hlc().actor()))
}

/// Put this device's pre-merge text back and drop the entry.
pub fn undo_merged_edit<S: AppHost>(state: &S, id: String) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let entry = take(&root, state.hlc().actor(), &id)
        .ok_or_else(|| "that merge is no longer listed".to_string())?;
    let page = parse_node_id(&entry.page_id)?;
    let node = parse_node_id(&entry.block_id)?;
    let hlc = state.hlc().clone();
    finish_in_page(state, page, |ws| {
        outl_actions::edit_text(ws, &hlc, node, &entry.ours)
    })
}
