//! `outl page update`: set a page's title and/or icon.
//!
//! Split out of `page.rs` to keep that file under the size ratchet. It is
//! re-exported there as `page::update`, which is the path the batch and MCP
//! dispatchers call.

use serde_json::Value;

use outl_actions::page::TITLE_KEY;

use super::page::{page_meta_value, resolve, write_page_property, write_projection};
use crate::output::{codes, ApiError};
use crate::ws::WsCtx;

/// Update a page's title and/or icon. At least one of `--title` /
/// `--icon` must be provided.
///
/// The title is written to the `title::` property, the one `page_meta`
/// reads first. Writing the root node's text instead would be invisible on
/// any page that already has the property, and would make concurrent
/// offline edits concatenate (see `TITLE_KEY`).
pub fn update(
    ctx: &mut WsCtx,
    slug: &str,
    title: Option<&str>,
    icon: Option<&str>,
) -> Result<Value, ApiError> {
    if title.is_none() && icon.is_none() {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            "update requires at least one of --title or --icon".to_string(),
        ));
    }
    // `page_meta` trims the property and falls back past a blank one, so a
    // blank title would be written and then silently ignored.
    let title = title.map(str::trim);
    if title == Some("") {
        return Err(ApiError::new(
            codes::INVALID_ARG,
            "a page title cannot be blank".to_string(),
        ));
    }
    let id = resolve(ctx, slug)?;

    if let Some(t) = title {
        write_page_property(ctx, id, TITLE_KEY, Some(t))?;
    }
    if let Some(icon_value) = icon {
        if icon_value.is_empty() {
            // Setting the property to None clears it.
            write_page_property(ctx, id, "icon", None)?;
        } else {
            write_page_property(ctx, id, "icon", Some(icon_value))?;
        }
    }

    write_projection(ctx, id)?;
    page_meta_value(ctx, id)
}
