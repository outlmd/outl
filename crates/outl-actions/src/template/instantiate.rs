//! Structural template instantiation.
//!
//! Deep-copies a template page's subtree under a target block,
//! applying built-in variable substitution and cloning block
//! properties. Each cloned block gets a fresh `NodeId` via
//! `Op::Create` — there is no ongoing link to the template after
//! instantiation.

use chrono::NaiveDate;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;

use crate::block::append_block;
use crate::error::ActionError;
use crate::page::{read_text_prop, set_property, SLUG_KEY};
use crate::template::list::find_template_by_name;
use crate::template::vars::{substitute_vars, VarContext};
use crate::template::{FROM_TEMPLATE_KEY, PARAMS_KEY, TEMPLATE_KEY};
use crate::tree::children_of;

/// Recursion cap for structural instantiation. A template's block subtree is a
/// finite tree (the CRDT forbids cycles), so this only trips on a pathologically
/// deep template — the guard turns a would-be stack overflow into a clean error.
const MAX_TEMPLATE_DEPTH: usize = 256;

/// Instantiate a structural template under `target_block`.
///
/// Walks the template page's child subtree, creates fresh blocks
/// under `target_block` with variable substitution applied, and
/// copies block-level properties (excluding internal page keys and
/// template metadata keys). The first root block of each cloned
/// top-level child gets `from-template:: <slug>` so the instance
/// is traceable via backlinks.
///
/// Returns the [`NodeId`]s of the top-level blocks created.
pub fn instantiate_template(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    template_name: &str,
    target_block: NodeId,
    page_slug: &str,
    page_date: Option<NaiveDate>,
) -> Result<Vec<NodeId>, ActionError> {
    instantiate_template_traced(
        workspace,
        hlc,
        template_name,
        target_block,
        page_slug,
        page_date,
        true,
    )
}

/// Like [`instantiate_template`], but lets the caller opt out of the
/// `from-template::` provenance property.
///
/// The journal auto-stamp ([`crate::page::open_journal`]) passes
/// `trace = false`: every daily note comes from the same `journal`
/// template, so the property would be pure noise on every note — it
/// would clutter each daily's `.md` and flood the journal template's
/// backlinks with one entry per day. Deliberate `/template` invocations
/// keep `trace = true` so the instance is discoverable.
pub(crate) fn instantiate_template_traced(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    template_name: &str,
    target_block: NodeId,
    page_slug: &str,
    page_date: Option<NaiveDate>,
    trace: bool,
) -> Result<Vec<NodeId>, ActionError> {
    let template_page = find_template_by_name(workspace, template_name)
        .ok_or_else(|| ActionError::PageNotFound(template_name.to_string()))?;

    let template_slug = read_text_prop(workspace, template_page, SLUG_KEY)
        .unwrap_or_else(|| template_name.to_string());

    let ctx = VarContext::new(page_slug, page_date);

    // Instantiating a template is one user-visible action that deep-copies
    // a whole subtree (append_block + property ops per node). Batch it so
    // the clone flushes once per destination instead of per op. The
    // recursion runs off `begin_batch` so the depth counter is pushed once.
    let mut batch = workspace.begin_batch();
    let new_ids = clone_children_recursive(
        &mut batch,
        hlc,
        template_page,
        target_block,
        &ctx,
        &template_slug,
        true,
        trace,
        0,
    )?;
    batch.commit()?;
    Ok(new_ids)
}

/// Recursively clone the children of `template_parent` under
/// `target_parent`, applying var substitution and copying
/// properties. When `is_root_level && trace`, each top-level child
/// gets the `from-template::` traceability property.
#[allow(clippy::too_many_arguments)]
fn clone_children_recursive(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    template_parent: NodeId,
    target_parent: NodeId,
    ctx: &VarContext,
    template_slug: &str,
    is_root_level: bool,
    trace: bool,
    depth: usize,
) -> Result<Vec<NodeId>, ActionError> {
    if depth > MAX_TEMPLATE_DEPTH {
        return Err(ActionError::Exec(format!(
            "template recursion exceeded {MAX_TEMPLATE_DEPTH} levels"
        )));
    }
    let template_children = children_of(workspace, template_parent);
    let mut new_ids = Vec::with_capacity(template_children.len());

    for (template_id, _) in template_children {
        let raw_text = workspace.block_text(template_id).unwrap_or_default();
        let substituted = substitute_vars(&raw_text, ctx);

        let new_id = append_block(workspace, hlc, Some(target_parent), Some(&substituted))?;

        copy_block_properties(workspace, hlc, template_id, new_id, ctx)?;

        if is_root_level && trace {
            set_property(
                workspace,
                hlc,
                new_id,
                FROM_TEMPLATE_KEY,
                Some(PropValue::Text(template_slug.to_string())),
            )?;
        }

        clone_children_recursive(
            workspace,
            hlc,
            template_id,
            new_id,
            ctx,
            template_slug,
            false,
            trace,
            depth + 1,
        )?;

        new_ids.push(new_id);
    }

    Ok(new_ids)
}

/// Copy every textual block property from `source` to `target`,
/// skipping internal page keys (`page-slug`, `page-kind`) and
/// template metadata keys (`template`, `params`).
///
/// Text property **values** get the same variable substitution the
/// block text does (`date:: {{date}}` on a template lands as the real
/// date on the instance), so a token never survives half-substituted
/// depending on whether it sat in the body or a property. Structured
/// refs (`PageRef`/`Tag`) are copied verbatim — they're resolved
/// handles, not free text with tokens.
fn copy_block_properties(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    source: NodeId,
    target: NodeId,
    ctx: &VarContext,
) -> Result<(), ActionError> {
    let props_to_copy: Vec<(String, PropValue)> = workspace
        .tree()
        .properties_of(source)
        // Page-model book-keeping goes through `crate::tree`'s predicate
        // rather than a second spelling of it here: that list had
        // `page-slug` / `page-kind` and never learned `page-source` or
        // `page-frontmatter`, and a copy of either onto an instance claims
        // a provenance the instance does not have. `template` / `params`
        // stay explicit — they are this module's own metadata, not the
        // page model's.
        .filter(|(k, _)| {
            !crate::tree::is_page_model_key(k) && *k != TEMPLATE_KEY && *k != PARAMS_KEY
        })
        .filter_map(|(k, v)| match v {
            PropValue::Text(s) => Some((k.to_string(), PropValue::Text(substitute_vars(s, ctx)))),
            PropValue::PageRef(_) | PropValue::Tag(_) => Some((k.to_string(), v.clone())),
            PropValue::List(_) => None,
        })
        .collect();

    for (key, value) in props_to_copy {
        set_property(workspace, hlc, target, &key, Some(value))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::append_block;
    use crate::page::{open_or_create as open_or_create_page, PageKind};
    use outl_core::id::ActorId;

    fn ws() -> (Workspace, HlcGenerator) {
        let actor = ActorId::new();
        (
            Workspace::open_in_memory(actor).unwrap(),
            HlcGenerator::new(actor),
        )
    }

    #[test]
    fn instantiates_blocks_under_target() {
        let (mut workspace, hlc) = ws();

        // Create template page with blocks
        let tpl_page = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-1on1",
            "1:1 Template",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            tpl_page,
            TEMPLATE_KEY,
            Some(PropValue::Text("1on1".into())),
        )
        .unwrap();
        append_block(&mut workspace, &hlc, Some(tpl_page), Some("Person:")).unwrap();
        append_block(
            &mut workspace,
            &hlc,
            Some(tpl_page),
            Some("TODO follow up {{tomorrow}}"),
        )
        .unwrap();

        // Create target page with a host block
        let daily = open_or_create_page(
            &mut workspace,
            &hlc,
            "2026-07-08",
            "2026-07-08",
            PageKind::Journal,
        )
        .unwrap();
        let host = append_block(&mut workspace, &hlc, Some(daily), Some("#1on1 host")).unwrap();

        let new_ids = instantiate_template(
            &mut workspace,
            &hlc,
            "1on1",
            host,
            "2026-07-08",
            NaiveDate::from_ymd_opt(2026, 7, 8),
        )
        .unwrap();

        assert_eq!(new_ids.len(), 2);

        // Check the children of host
        let children: Vec<NodeId> = children_of(&workspace, host)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(children.len(), 2);
    }

    #[test]
    fn substitutes_variables_in_cloned_text() {
        let (mut workspace, hlc) = ws();

        let tpl_page = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-todo",
            "TODO Template",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            tpl_page,
            TEMPLATE_KEY,
            Some(PropValue::Text("todo-tpl".into())),
        )
        .unwrap();
        append_block(
            &mut workspace,
            &hlc,
            Some(tpl_page),
            Some("Page is {{page}}"),
        )
        .unwrap();

        let target = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();

        instantiate_template(&mut workspace, &hlc, "todo-tpl", target, "my-page", None).unwrap();

        let children = children_of(&workspace, target);
        let text = workspace.block_text(children[0].0).unwrap_or_default();
        assert!(
            text.contains("my-page"),
            "variable should be substituted: {text}"
        );
        assert!(!text.contains("{{page}}"), "token should be gone: {text}");
    }

    #[test]
    fn sets_from_template_on_root_blocks() {
        let (mut workspace, hlc) = ws();

        let tpl_page = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-interview",
            "Interview",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            tpl_page,
            TEMPLATE_KEY,
            Some(PropValue::Text("interview".into())),
        )
        .unwrap();
        append_block(&mut workspace, &hlc, Some(tpl_page), Some("Question 1")).unwrap();

        let target = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();

        instantiate_template(
            &mut workspace,
            &hlc,
            "interview",
            target,
            "2026-07-08",
            NaiveDate::from_ymd_opt(2026, 7, 8),
        )
        .unwrap();

        let children = children_of(&workspace, target);
        let prop = workspace.tree().property(children[0].0, FROM_TEMPLATE_KEY);
        assert!(matches!(prop, Some(PropValue::Text(s)) if s == "template-interview"));
    }

    #[test]
    fn deep_clones_nested_children() {
        let (mut workspace, hlc) = ws();

        let tpl_page = open_or_create_page(
            &mut workspace,
            &hlc,
            "template-nested",
            "Nested",
            PageKind::Page,
        )
        .unwrap();
        set_property(
            &mut workspace,
            &hlc,
            tpl_page,
            TEMPLATE_KEY,
            Some(PropValue::Text("nested".into())),
        )
        .unwrap();
        let parent = append_block(&mut workspace, &hlc, Some(tpl_page), Some("parent")).unwrap();
        append_block(&mut workspace, &hlc, Some(parent), Some("child-a")).unwrap();
        append_block(&mut workspace, &hlc, Some(parent), Some("child-b")).unwrap();

        let target = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();

        instantiate_template(&mut workspace, &hlc, "nested", target, "page", None).unwrap();

        let top = children_of(&workspace, target);
        assert_eq!(top.len(), 1);
        let grandkids = children_of(&workspace, top[0].0);
        assert_eq!(grandkids.len(), 2);
    }

    #[test]
    fn fails_when_template_not_found() {
        let (mut workspace, hlc) = ws();
        let target = append_block(&mut workspace, &hlc, None, Some("host")).unwrap();

        let result =
            instantiate_template(&mut workspace, &hlc, "nonexistent", target, "page", None);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ActionError::PageNotFound(s) if s == "nonexistent"
        ));
    }

    /// Build a template page `name` and run `body` to append its blocks.
    fn template_with(
        workspace: &mut Workspace,
        hlc: &HlcGenerator,
        slug: &str,
        name: &str,
    ) -> NodeId {
        let page = open_or_create_page(workspace, hlc, slug, name, PageKind::Page).unwrap();
        set_property(
            workspace,
            hlc,
            page,
            TEMPLATE_KEY,
            Some(PropValue::Text(name.into())),
        )
        .unwrap();
        page
    }

    #[test]
    fn deep_clone_preserves_sibling_order() {
        let (mut w, hlc) = ws();
        let tpl = template_with(&mut w, &hlc, "template-order", "order");
        for t in ["a", "b", "c"] {
            append_block(&mut w, &hlc, Some(tpl), Some(t)).unwrap();
        }
        let target = append_block(&mut w, &hlc, None, Some("host")).unwrap();
        instantiate_template(&mut w, &hlc, "order", target, "page", None).unwrap();

        let texts: Vec<String> = children_of(&w, target)
            .into_iter()
            .filter_map(|(id, _)| w.block_text(id))
            .collect();
        assert_eq!(texts, vec!["a", "b", "c"], "sibling order must survive");
    }

    #[test]
    fn copies_and_substitutes_text_properties() {
        let (mut w, hlc) = ws();
        let tpl = template_with(&mut w, &hlc, "template-props", "props");
        let block = append_block(&mut w, &hlc, Some(tpl), Some("item")).unwrap();
        set_property(
            &mut w,
            &hlc,
            block,
            "due",
            Some(PropValue::Text("{{date}}".into())),
        )
        .unwrap();

        let target = append_block(&mut w, &hlc, None, Some("host")).unwrap();
        instantiate_template(
            &mut w,
            &hlc,
            "props",
            target,
            "2026-07-08",
            NaiveDate::from_ymd_opt(2026, 7, 8),
        )
        .unwrap();

        let clone = children_of(&w, target)[0].0;
        // The `{{date}}` token in the property value is substituted too.
        assert!(matches!(
            w.tree().property(clone, "due"),
            Some(PropValue::Text(s)) if s == "2026-07-08"
        ));
    }

    #[test]
    fn from_template_only_on_root_not_descendants() {
        let (mut w, hlc) = ws();
        let tpl = template_with(&mut w, &hlc, "template-depth", "depth");
        let parent = append_block(&mut w, &hlc, Some(tpl), Some("parent")).unwrap();
        append_block(&mut w, &hlc, Some(parent), Some("child")).unwrap();

        let target = append_block(&mut w, &hlc, None, Some("host")).unwrap();
        instantiate_template(&mut w, &hlc, "depth", target, "page", None).unwrap();

        let root_clone = children_of(&w, target)[0].0;
        let child_clone = children_of(&w, root_clone)[0].0;
        assert!(
            w.tree().property(root_clone, FROM_TEMPLATE_KEY).is_some(),
            "root clone is traced"
        );
        assert!(
            w.tree().property(child_clone, FROM_TEMPLATE_KEY).is_none(),
            "descendant clones are NOT traced"
        );
    }

    #[test]
    fn untraced_variant_skips_from_template() {
        let (mut w, hlc) = ws();
        let tpl = template_with(&mut w, &hlc, "template-untraced", "untraced");
        append_block(&mut w, &hlc, Some(tpl), Some("body")).unwrap();

        let target = append_block(&mut w, &hlc, None, Some("host")).unwrap();
        instantiate_template_traced(&mut w, &hlc, "untraced", target, "page", None, false).unwrap();

        let root_clone = children_of(&w, target)[0].0;
        assert!(
            w.tree().property(root_clone, FROM_TEMPLATE_KEY).is_none(),
            "untraced instantiation writes no from-template (journal path)"
        );
    }
}
