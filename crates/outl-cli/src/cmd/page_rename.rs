//! `outl page rename`: the wrapper around [`outl_actions::rename_page`].
//!
//! Split out of `page.rs` to keep that file under the size ratchet. It is
//! re-exported there as `page::rename`, which is the path the batch and MCP
//! dispatchers call.

use serde_json::{json, Value};

use outl_actions::find_by_slug;

use super::page::{page_meta_inner, resolve, write_projection};
use crate::output::ApiError;
use crate::ws::WsCtx;

/// Rename a page's slug and rewrite every mention of the old name.
///
/// The CRDT half lives in [`outl_actions::rename_page`], which also
/// rewrites `[[old]]` / `#old` / person mentions in every block through
/// ordinary edit ops. This wrapper owns the projection of every page the
/// rewrite touched, the removal of the old `.md` + sidecar, and the JSON
/// envelope.
///
/// A new slug already owned by another page is `SLUG_CONFLICT`, and the
/// message says a merge is possible: the page can be folded into the
/// existing one instead (`outl_actions::rename_or_merge`).
pub fn rename(ctx: &mut WsCtx, old_slug: &str, new_slug: &str) -> Result<Value, ApiError> {
    let id = resolve(ctx, old_slug)?;
    let report = outl_actions::rename_page(&mut ctx.workspace, &ctx.hlc, old_slug, new_slug, None)?;

    // Project the renamed page and every page whose blocks were
    // rewritten. The old projection is only removed once the renamed
    // page itself projected, so a refused write never costs the user the
    // only `.md` that still holds the page.
    let mut first_failure: Option<ApiError> = None;
    let mut renamed_projected = false;
    for slug in &report.touched_pages {
        let Some(page) = find_by_slug(&ctx.workspace, slug) else {
            continue;
        };
        match write_projection(ctx, page) {
            Ok(()) => renamed_projected |= *slug == report.meta.slug,
            Err(e) => {
                first_failure.get_or_insert(e);
            }
        }
    }
    if renamed_projected {
        if let Some(old_meta) = &report.stale_projection {
            // Logged on failure for the same reason as in `delete`.
            if let Err(e) = outl_actions::remove_page_projection(&ctx.root, old_meta) {
                tracing::warn!(
                    target: "outl::cmd::page",
                    "could not remove old page projection for {}: {e}",
                    old_meta.slug
                );
            }
        }
    }
    if let Some(e) = first_failure {
        return Err(e);
    }

    // Whatever still names the old slug after the rewrite (a namespace
    // child, for one) stays visible so the caller can decide.
    let affected = outl_actions::backlinks_for_target(&ctx.workspace, &ctx.root, old_slug);
    Ok(json!({
        "meta": page_meta_inner(ctx, id)?,
        "old_slug": old_slug,
        "rewritten_refs": report.rewritten_blocks,
        "skipped_namespace_refs": report.skipped_namespace_refs,
        "touched_pages": report.touched_pages,
        "affected_refs": affected
            .iter()
            .map(|b| json!({
                "block_id": b.block_id,
                "block_text": b.block_text,
                "source_page": b.source_page,
            }))
            .collect::<Vec<_>>(),
    }))
}
