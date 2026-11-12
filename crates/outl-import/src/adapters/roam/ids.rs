//! `--keep-source-ids`: what changes when every Roam block must land as
//! exactly one outl block that carries its Roam uid.

use super::is_pure_prop_block;
use crate::ir::ImportBlock;

/// The property a page's and a block's Roam uid is written under.
///
/// A property and not a sidecar field or a side file: it is the one
/// place that travels with the block through every outl write, move and
/// peer sync, so the mapping a Roam sync needs can't drift from the
/// block it names.
pub(super) const ROAM_UID_KEY: &str = "roam-uid";

/// The page header copy of the leading run of attribute blocks
/// (`icon::`, `alias::`, `type::` at the head of a Roam page), when the
/// blocks themselves stay in the outline.
///
/// Roam models a page's attributes as such blocks, each with its own
/// uid and timestamps; outl reads them from the page header (sidebar
/// icon, `@` mention type, aliases). Lifting the blocks into the header
/// served outl and lost the blocks, so with source ids kept the blocks
/// stay and the header gets a copy. `title::` is the emitter's own line.
pub(super) fn page_attributes(blocks: &[ImportBlock], keep_ids: bool) -> Vec<(String, String)> {
    if !keep_ids {
        return Vec::new();
    }
    blocks
        .iter()
        .take_while(|b| is_pure_prop_block(b))
        .flat_map(|b| b.props.iter().filter(|(k, _)| k != "title").cloned())
        .collect()
}
