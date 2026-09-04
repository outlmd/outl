//! Structural template command bodies.
//!
//! Thin adapters over `outl_actions::{list_templates, instantiate_template}`
//! so both GUI clients can list workspace templates and deep-copy one
//! under a target block without a plugin. Every workspace mutation lives
//! in `outl-actions`; this module only parses ids, locks the workspace,
//! and reprojects.

use outl_actions::{
    enclosing_page_id, instantiate_template, list_templates as action_list_templates, page_meta,
    PageKind,
};

use crate::helpers::{build_page_view, parse_node_id, with_ws, with_ws_mut};
use crate::host::AppHost;
use crate::state::{PageView, TemplateDto};

/// List every structural template defined in the workspace, sorted by
/// invocation name. Wraps `outl_actions::list_templates` — a template is
/// any page with a non-empty `template::` property, and its outline is
/// the body deep-copied on instantiation.
pub fn list_templates<S: AppHost>(state: &S) -> Result<Vec<TemplateDto>, String> {
    with_ws(state, |ws| {
        Ok(action_list_templates(ws)
            .into_iter()
            .map(|t| TemplateDto {
                name: t.name,
                slug: t.slug,
                duplicate: t.duplicate,
            })
            .collect())
    })
}

/// Instantiate the template `name` under `target_block`, returning a
/// refreshed [`PageView`] of the block's enclosing page.
///
/// Deep-copies the template's subtree under `target_block` with built-in
/// variable substitution (`{{date}}` / `{{page}}` / …), stamping
/// `from-template:: <slug>` on each root clone so the instance is
/// traceable via backlinks. The heavy lifting is
/// `outl_actions::instantiate_template`; this adapter resolves the page
/// context the action needs (slug + optional journal date) from the
/// target block's enclosing page.
///
/// An unknown template name surfaces as a string error (the action
/// returns `PageNotFound`); a malformed / stale block id fails at parse
/// time or as "not in tree". The frontend shows a toast — this command
/// does not create anything on a miss.
pub fn instantiate_template_at<S: AppHost>(
    state: &S,
    name: String,
    target_block: String,
) -> Result<PageView, String> {
    let root = state.storage_root()?;
    let node = parse_node_id(&target_block)?;

    // Resolve the page the target block lives on: the action stamps the
    // page slug / journal date into `{{page}}` / `{{date}}` and we need
    // the same page id to reproject + build the view.
    let (page_id, slug, page_date) = with_ws(state, |ws| {
        let page_id = enclosing_page_id(ws, node)
            .ok_or_else(|| format!("block {target_block} is not on a page"))?;
        let meta = page_meta(ws, page_id)
            .ok_or_else(|| format!("page for block {target_block} not found"))?;
        // A journal page carries a date the template's `{{date}}` /
        // `{{today}}` tokens resolve against; regular pages pass `None`.
        let date = match meta.kind {
            PageKind::Journal => outl_actions::date_from_slug(&meta.slug),
            PageKind::Page => None,
        };
        Ok((page_id, meta.slug, date))
    })?;

    let projection_failure = with_ws_mut(state, |ws| {
        instantiate_template(ws, state.hlc(), &name, node, &slug, page_date)
            .map_err(|e| e.to_string())?;
        // Guarded — same reason as every other post-mutation projection:
        // the write is required, deleting unlogged bytes to achieve it is
        // not (invariant 8).
        let failure = outl_actions::apply_page_md_with_sidecar_guarded(ws, &root, page_id).err();
        if let Some(e) = &failure {
            tracing::warn!("instantiate_template_at: md+sidecar sync skipped for {slug}: {e}");
        }
        Ok(failure)
    })?;

    // Announce the new ops so a peer pulls the instantiated subtree over
    // iroh without waiting for the catch-up loop.
    if let Some(transport) = state.sync_transport() {
        transport.announce_local_ops(&slug, state.hlc().next());
    }

    with_ws(state, |ws| {
        let mut view = build_page_view(ws, &root, page_id).map_err(|e| e.to_string())?;
        if let Some(e) = projection_failure {
            if let Some(writer) = state.projection_writer() {
                writer.report_failure(page_id, &e);
            }
            let failure = crate::state::ProjectionWriteFailed::from_error(page_id, &e);
            view.md_ahead_of_log = failure.md_ahead_of_log.clone();
            view.projection_error = failure.md_ahead_of_log.is_none().then_some(failure.error);
        }
        Ok(view)
    })
}
