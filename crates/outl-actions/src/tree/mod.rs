//! Read-only navigation over the materialised tree.
//!
//! Nothing here applies an op. Four concerns, one owner each:
//!
//! - `props` — which of a node's properties are the user's and which
//!   are page-model book-keeping, and how a value renders.
//! - `children` — the canonical sibling order (the `NodeId` tiebreak is
//!   convergence-critical, not tidiness) and the two ways to ask for a
//!   parent's children: one lookup, or an index for a whole walk.
//! - `position` — minting a [`outl_core::fractional::Fractional`] slot
//!   for a new node, including the shapes where no slot exists.
//! - `traverse` — walking down into a subtree and up to the page.
//!
//! Re-exported here so every `crate::tree::*` path keeps resolving.

mod children;
mod position;
mod props;
mod traverse;

pub(crate) use children::{
    children_index, children_index_unordered, previous_sibling, sort_siblings, subtree_ids,
};
pub use children::{children_of, next_sibling, ChildrenIndex};
pub use position::{position_after, position_before, position_for_new_last_child};
pub use props::is_page_model_key;
pub(crate) use props::{renderable_prop_value, text_properties_of};
pub(crate) use traverse::is_under;
pub use traverse::{enclosing_page_id, is_trashed, page_slug_of, walk_subtree};
