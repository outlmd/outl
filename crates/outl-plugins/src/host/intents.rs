//! Applying what a plugin turn emitted.
//!
//! The JS engine never holds `&mut Workspace`. It emits [`HostIntent`]s into a
//! buffer and the host drains it here, which makes this the **only** place in
//! the host that mutates a workspace. Both rules that make a plugin op
//! trustworthy therefore live in one file and are checked in one order: the
//! intent is refused unless the approved [`PermissionSet`] covers it, and then
//! it runs through `outl-actions` → `Workspace::apply` → op log — never against
//! `outl-core` directly, never by editing a `.md`.
//!
//! A refused or failing intent is a line in [`PluginRun::errors`], not a
//! `Result::Err`: one bad intent does not abandon the rest of the turn, and the
//! client shows the user which plugin was denied what.

use std::str::FromStr;

use outl_actions::block;
use outl_actions::page::{self, PageKind};
use outl_actions::template as tpl_actions;
use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;

use crate::error::{PluginError, Result};
use crate::model::{HostIntent, MoveTarget, TreeNode};
use crate::permission::PermissionSet;

use super::project::page_slug_of;
use super::PluginRun;

/// Apply a plugin's intents, gating each on the approved permission set.
pub(super) fn apply_intents(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    perms: &PermissionSet,
    plugin_id: &str,
    intents: &[HostIntent],
    run: &mut PluginRun,
) {
    for intent in intents {
        if !perms.check(&intent.required_permission()) {
            run.errors.push(format!(
                "{plugin_id}: denied `{}` for intent",
                intent.required_permission()
            ));
            continue;
        }
        match apply_one(workspace, hlc, intent) {
            Ok(()) => run.applied += 1,
            Err(e) => run.errors.push(format!("{plugin_id}: {e}")),
        }
    }
}

fn apply_one(workspace: &mut Workspace, hlc: &HlcGenerator, intent: &HostIntent) -> Result<()> {
    match intent {
        HostIntent::EditText { node, text } => {
            block::edit_text(workspace, hlc, parse_node(node)?, text).map_err(act)
        }
        HostIntent::CreateUnder { parent, text } => {
            block::create_under(workspace, hlc, parse_node(parent)?, Some(text))
                .map(|_| ())
                .map_err(act)
        }
        HostIntent::CreateAfter { after, text } => {
            block::create_after(workspace, hlc, parse_node(after)?, Some(text))
                .map(|_| ())
                .map_err(act)
        }
        HostIntent::ToggleTodo { node } => {
            block::toggle_todo(workspace, hlc, parse_node(node)?).map_err(act)
        }
        HostIntent::Delete { node } => {
            block::delete(workspace, hlc, parse_node(node)?).map_err(act)
        }
        HostIntent::EnsurePage { slug } => {
            page::open_or_create(workspace, hlc, slug, slug, PageKind::Page)
                .map(|_| ())
                .map_err(act)
        }
        HostIntent::InstantiateTemplate { name, under } => {
            let target = parse_node(under)?;
            let slug = page_slug_of(workspace, target).unwrap_or_default();
            // Derive the page date from the target slug so `{{date}}`
            // resolves to the journal's own date on a daily note, matching
            // the CLI/TUI path — passing `None` here made it always render
            // today's date regardless of which page the block lives on.
            let page_date = outl_actions::dates::date_from_slug(&slug);
            tpl_actions::instantiate_template(workspace, hlc, name, target, &slug, page_date)
                .map(|_| ())
                .map_err(act)
        }
        HostIntent::Move { node, target } => {
            let n = parse_node(node)?;
            let parent = match target {
                MoveTarget::ToParent { to_parent } => parse_node(to_parent)?,
                MoveTarget::ToPage { to_page } => {
                    page::open_or_create(workspace, hlc, to_page, to_page, PageKind::Page)
                        .map_err(act)?
                }
            };
            block::move_under(workspace, hlc, n, parent).map_err(act)
        }
        HostIntent::AppendTree { target, tree } => {
            let parent = match target {
                MoveTarget::ToParent { to_parent } => parse_node(to_parent)?,
                // `toPage` is a slug, same as `Move`/`EnsurePage`. Pages are
                // flat (`pages/<slug>.md`); the slug is also the page title, so
                // the plugin reads the day back with `query({ page: slug })`.
                MoveTarget::ToPage { to_page } => {
                    page::open_or_create(workspace, hlc, to_page, to_page, PageKind::Page)
                        .map_err(act)?
                }
            };
            append_tree(workspace, hlc, parent, tree).map_err(act)
        }
    }
}

/// Recursively create `nodes` under `parent`, descending into children with the
/// id the host gets back from each create. This is what lets `AppendTree`
/// materialize a nested structure in one turn — the plugin never sees the ids,
/// the host threads them through here.
fn append_tree(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    nodes: &[TreeNode],
) -> std::result::Result<(), outl_actions::error::ActionError> {
    for node in nodes {
        let id = block::create_under(workspace, hlc, parent, Some(&node.text))?;
        if !node.children.is_empty() {
            append_tree(workspace, hlc, id, &node.children)?;
        }
    }
    Ok(())
}

fn act(e: outl_actions::error::ActionError) -> PluginError {
    PluginError::Engine(e.to_string())
}

fn parse_node(s: &str) -> Result<NodeId> {
    // `NodeId` is a `NodeId(pub Ulid)` newtype with no `FromStr`; parse the
    // ULID and wrap it, same as the desktop's `helpers::parse_node_id`.
    ulid::Ulid::from_str(s)
        .map(NodeId)
        .map_err(|_| PluginError::BadNodeId(s.to_string()))
}

#[cfg(all(test, feature = "js"))]
mod tests {
    use super::*;
    use crate::host::project::build_read_model;
    use crate::host::tests::ws;

    /// A plugin `InstantiateTemplate` intent on a journal page must resolve
    /// `{{date}}` to the journal's OWN date (derived from its slug), not to
    /// today — matching the CLI/TUI path. Regression for the footgun where
    /// the host passed `page_date: None` and every plugin instantiation
    /// rendered today's date regardless of which page the block lived on.
    #[test]
    fn instantiate_template_intent_uses_journal_page_date() {
        use outl_core::property::PropValue;

        let (mut ws, hlc) = ws();

        // A template whose body echoes `{{date}}`.
        let tpl =
            page::open_or_create(&mut ws, &hlc, "template-daily", "daily", PageKind::Page).unwrap();
        page::set_property(
            &mut ws,
            &hlc,
            tpl,
            tpl_actions::TEMPLATE_KEY,
            Some(PropValue::Text("daily".into())),
        )
        .unwrap();
        block::append_block(&mut ws, &hlc, Some(tpl), Some("day is {{date}}")).unwrap();

        // A journal page dated well in the past, with a host block.
        let journal =
            page::open_or_create(&mut ws, &hlc, "2020-01-02", "2020-01-02", PageKind::Journal)
                .unwrap();
        let host_block = block::append_block(&mut ws, &hlc, Some(journal), Some("host")).unwrap();

        let intent = HostIntent::InstantiateTemplate {
            name: "daily".into(),
            under: host_block.to_string(),
        };
        apply_one(&mut ws, &hlc, &intent).unwrap();

        // The cloned block must carry the journal's date, never today's.
        let clone_text = outl_actions::tree::children_of(&ws, host_block)
            .into_iter()
            .filter_map(|(id, _)| ws.block_text(id))
            .find(|t| t.starts_with("day is"))
            .expect("template block was cloned under the host");
        assert!(
            clone_text.contains("2020-01-02"),
            "`{{{{date}}}}` should resolve to the journal's date, got: {clone_text}"
        );
    }

    // --- AppendTree: seed a fresh page in one turn --------------------------

    #[test]
    fn append_tree_seeds_a_fresh_page_in_one_turn() {
        let (mut ws, hlc) = ws();
        // The page does not exist yet — this is exactly the case a plugin can't
        // handle with `create` (no parent id to hand it mid-turn).
        let intent = HostIntent::AppendTree {
            target: MoveTarget::ToPage {
                to_page: "ouraring-2025-11-29".into(),
            },
            tree: vec![TreeNode {
                text: "#ouraring anchor".into(),
                children: vec![
                    TreeNode {
                        text: "Sleep — Score 85".into(),
                        children: vec![],
                    },
                    TreeNode {
                        text: "Readiness — Score 78".into(),
                        children: vec![],
                    },
                ],
            }],
        };
        apply_one(&mut ws, &hlc, &intent).unwrap();

        // The page now exists — its title keeps the pretty `/`-name, its slug is
        // the slugified filename-safe form.
        let rm = build_read_model(&ws);
        assert!(
            rm.pages.iter().any(|p| p.slug == "ouraring-2025-11-29"),
            "flat page created from the slug, got: {:?}",
            rm.pages.iter().map(|p| &p.slug).collect::<Vec<_>>()
        );
        // …with the anchor as a top-level block on it.
        let anchor = rm
            .blocks
            .iter()
            .find(|b| b.text == "#ouraring anchor")
            .expect("anchor block created on the fresh page");

        // …and the two section lines nested under it (verified via the tree, not
        // just page membership).
        let anchor_id = NodeId(ulid::Ulid::from_str(&anchor.id).unwrap());
        let kids = outl_actions::tree::children_of(&ws, anchor_id);
        assert_eq!(kids.len(), 2, "anchor has its two children");
        let texts: Vec<String> = kids
            .into_iter()
            .filter_map(|(id, _)| ws.block_text(id))
            .collect();
        assert!(texts.iter().any(|t| t.starts_with("Sleep")));
        assert!(texts.iter().any(|t| t.starts_with("Readiness")));
    }
}
