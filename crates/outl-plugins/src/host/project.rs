//! Read-only projections of the workspace into the shapes the JS side sees.
//!
//! The engine never holds a `Workspace`. Each turn the host hands it a
//! [`ReadModel`] (blocks + pages + templates), and an `onOp` sweep hands it one
//! [`LogOpView`] per applied op. This module is that translation and nothing
//! else — no mutation, no engine call, no permission check — so the shapes the
//! plugin API promises have exactly one producer.
//!
//! Two of those promises are easy to re-derive slightly differently elsewhere
//! and are pinned here on purpose: page nodes are *not* blocks (a plugin
//! operates on blocks), and a block whose parent is a page reports `parent:
//! null` rather than the page's id, so "no addressable parent block" is one
//! value instead of one per caller's guess.

use outl_actions::page;
use outl_actions::template as tpl_actions;
use outl_actions::todo::{split_todo, TodoState};
use outl_core::id::NodeId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

use crate::model::{BlockView, LogOpView, PageView, ReadModel, TemplateView};

/// Build the read-only snapshot the JS side queries this turn.
pub(super) fn build_read_model(workspace: &Workspace) -> ReadModel {
    let pages: Vec<PageView> = page::list_all(workspace)
        .into_iter()
        .map(|m| PageView {
            slug: m.slug,
            title: m.title,
            kind: m.kind.as_str().to_string(),
        })
        .collect();

    let templates: Vec<TemplateView> = tpl_actions::list_templates(workspace)
        .into_iter()
        .map(|t| TemplateView {
            name: t.name,
            slug: t.slug,
            params: t.params,
        })
        .collect();

    let mut blocks = Vec::new();
    for (node, parent, _pos) in workspace.tree().iter_nodes() {
        if node == NodeId::root() || node == NodeId::trash() {
            continue;
        }
        // Skip page nodes themselves — plugins operate on blocks, not pages.
        if page::page_meta(workspace, node).is_some() {
            continue;
        }
        let Some(raw) = workspace.block_text(node) else {
            continue;
        };
        let (todo, body) = split_todo(&raw);
        // A block whose parent is the page root (or root/trash) is top-level:
        // report `null` so the plugin sees "no addressable parent block".
        let parent_id = if parent == NodeId::root()
            || parent == NodeId::trash()
            || page::page_meta(workspace, parent).is_some()
        {
            None
        } else {
            Some(parent.to_string())
        };
        blocks.push(BlockView {
            id: node.to_string(),
            text: body.to_string(),
            todo: todo.map(|t| t.as_str().to_string()),
            parent: parent_id,
            page: page_slug_of(workspace, node).unwrap_or_default(),
        });
    }
    ReadModel {
        blocks,
        pages,
        templates,
        op: None,
    }
}

/// Climb parents until one is a page; return its slug.
pub(super) fn page_slug_of(workspace: &Workspace, node: NodeId) -> Option<String> {
    let mut cur = node;
    loop {
        let parent = workspace.tree().parent(cur)?;
        if let Some(meta) = page::page_meta(workspace, parent) {
            return Some(meta.slug);
        }
        if parent == NodeId::root() {
            return None;
        }
        cur = parent;
    }
}

/// Project an applied [`LogOp`] to the stable JS shape.
pub(super) fn project_op(workspace: &Workspace, lo: &LogOp) -> Option<LogOpView> {
    let mk = |kind: &str, node: NodeId| LogOpView {
        kind: kind.to_string(),
        node: node.to_string(),
        text: None,
        todo: None,
    };
    Some(match &lo.op {
        Op::Create { node, .. } => mk("Create", *node),
        Op::Move { node, .. } => mk("Move", *node),
        Op::SetProp { node, .. } => mk("SetProp", *node),
        Op::SetCollapsed { node, .. } => mk("SetCollapsed", *node),
        Op::SnoozeRemind { node, .. } => mk("SnoozeRemind", *node),
        Op::Edit { node, .. } => {
            let raw = workspace.block_text(*node).unwrap_or_default();
            let (todo, body) = split_todo(&raw);
            LogOpView {
                kind: "Edit".to_string(),
                node: node.to_string(),
                text: Some(body.to_string()),
                todo: todo.map(|t: TodoState| t.as_str().to_string()),
            }
        }
    })
}
