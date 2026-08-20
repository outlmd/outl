//! The property-key catalogue, as a command.
//!
//! Every "add a property" surface needs the same answer to "which
//! key?", and the useful answer is what this workspace already uses,
//! ranked. The ranking itself lives in `outl_actions::known_keys` so
//! the three clients cannot drift on it; this is the IPC wrapper.

use serde::Serialize;

use crate::helpers::with_ws;
use crate::host::AppHost;

/// One entry of the catalogue: the key as most users of it spell it,
/// and how many properties in the workspace use it.
///
/// The count rides along so a client can decide how many to show
/// (mobile shows the top few as tappable chips) without a second call.
#[derive(Debug, Clone, Serialize)]
pub struct PropertyKey {
    pub key: String,
    pub uses: usize,
}

/// Property keys used anywhere in the workspace, most-used first.
///
/// Read-only, and `O(total properties)` — a scan of the property map,
/// no block text materialized and no tree walk, so it is cheap enough
/// to call when a menu opens rather than caching it into staleness.
pub fn known_property_keys<S: AppHost>(state: &S) -> Result<Vec<PropertyKey>, String> {
    with_ws(state, |ws| {
        Ok(outl_actions::known_keys(ws)
            .into_iter()
            .map(|(key, uses)| PropertyKey { key, uses })
            .collect())
    })
}
