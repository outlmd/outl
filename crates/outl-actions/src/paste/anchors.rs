//! Anchor-specific paste implementations.
//!
//! `paste_markdown` (in [`super`]) decides which anchor variant the
//! caller asked for and dispatches to one of the functions here.
//! Splitting them out keeps the entry point small and each variant
//! single-purpose.

use outl_core::hlc::HlcGenerator;
use outl_core::id::NodeId;
use outl_core::property::PropValue;
use outl_core::workspace::Workspace;
use outl_md::parse::OutlineNode as ParsedNode;

use super::{PasteAnchor, PasteOutcome};
use crate::block::{
    append_forest, create_after, create_under, edit_text, BlockTreeOutcome, BlockTreeSpec,
};
use crate::error::ActionError;
use crate::page::set_property;

/// Graft a set of parsed nodes at `anchor`, picking the anchor-specific
/// operation. Both the outline path and the plain-multi-paragraph path in
/// [`super::paste_markdown`] funnel through here so the anchor `match`
/// lives once.
pub(super) fn paste_nodes(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    anchor: PasteAnchor,
    blocks: &[ParsedNode],
) -> Result<PasteOutcome, ActionError> {
    match anchor {
        PasteAnchor::AsLastChildOf(parent) => paste_as_children(workspace, hlc, parent, blocks),
        PasteAnchor::AfterBlock(after) => paste_after(workspace, hlc, after, blocks),
        PasteAnchor::AtCaret { block, caret } => {
            paste_at_caret(workspace, hlc, block, caret, blocks)
        }
    }
}

/// Append every parsed block as a new last child of `parent`.
pub(super) fn paste_as_children(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parent: NodeId,
    blocks: &[ParsedNode],
) -> Result<PasteOutcome, ActionError> {
    let specs: Vec<BlockTreeSpec> = blocks.iter().map(to_spec).collect();
    let outcomes = append_forest(workspace, hlc, parent, &specs)?;
    apply_properties_forest(workspace, hlc, blocks, &outcomes)?;
    Ok(PasteOutcome {
        new_blocks: collect_ids(&outcomes),
        host_text: None,
        root_count: blocks.len(),
    })
}

/// Insert each parsed block as a sibling after the previous one,
/// starting with `after`.
pub(super) fn paste_after(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    after: NodeId,
    blocks: &[ParsedNode],
) -> Result<PasteOutcome, ActionError> {
    let mut new_blocks: Vec<NodeId> = Vec::new();
    let mut prev = after;
    for parsed in blocks {
        let id = create_after(workspace, hlc, prev, Some(&parsed.text))?;
        new_blocks.push(id);
        apply_properties_node(workspace, hlc, parsed, id)?;
        attach_children(workspace, hlc, parsed, id, &mut new_blocks)?;
        prev = id;
    }
    Ok(PasteOutcome {
        new_blocks,
        host_text: None,
        root_count: blocks.len(),
    })
}

/// Caret-aware paste: the first parsed bullet merges into the host's
/// text at the caret; remaining bullets become siblings; the host's
/// right-hand text becomes one more sibling at the end.
pub(super) fn paste_at_caret(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    block: NodeId,
    caret: usize,
    blocks: &[ParsedNode],
) -> Result<PasteOutcome, ActionError> {
    let original = host_text_for_caret(workspace, block)?;
    let (left, right) = split_at_char(&original, caret);

    // First parsed bullet merges into the host's left-side text.
    let first = &blocks[0];
    let host_text = join_caret_left_with(&left, &first.text);
    edit_text(workspace, hlc, block, &host_text)?;

    let mut new_blocks: Vec<NodeId> = Vec::new();

    // Children of the first bullet become children of the host block.
    if !first.children.is_empty() {
        let child_specs: Vec<BlockTreeSpec> = first.children.iter().map(to_spec).collect();
        let child_outcomes = append_forest(workspace, hlc, block, &child_specs)?;
        apply_properties_forest(workspace, hlc, &first.children, &child_outcomes)?;
        for o in &child_outcomes {
            push_ids(&mut new_blocks, o);
        }
    }
    // The first bullet's own properties apply to the host block.
    apply_properties_node(workspace, hlc, first, block)?;

    // Remaining root-level bullets become siblings after the host.
    let mut prev = block;
    for parsed in &blocks[1..] {
        let id = create_after(workspace, hlc, prev, Some(&parsed.text))?;
        new_blocks.push(id);
        apply_properties_node(workspace, hlc, parsed, id)?;
        attach_children(workspace, hlc, parsed, id, &mut new_blocks)?;
        prev = id;
    }

    // Right-hand side of the caret survives as one more sibling so the
    // user never loses the tail of what they were typing.
    if !right.trim().is_empty() {
        let id = create_after(workspace, hlc, prev, Some(right.trim_start()))?;
        new_blocks.push(id);
    }

    Ok(PasteOutcome {
        new_blocks,
        host_text: Some(host_text),
        root_count: blocks.len(),
    })
}

/// Fallback used when the heuristic says the clipboard isn't an
/// outline. Behaviour depends on the anchor: caret-paste splices the
/// raw text; sibling/child variants drop it into a single new block.
pub(super) fn paste_plain_text(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    anchor: PasteAnchor,
    text: &str,
) -> Result<PasteOutcome, ActionError> {
    match anchor {
        PasteAnchor::AtCaret { block, caret } => {
            let original = host_text_for_caret(workspace, block)?;
            let (left, right) = split_at_char(&original, caret);
            let new_text = format!("{left}{text}{right}");
            edit_text(workspace, hlc, block, &new_text)?;
            Ok(PasteOutcome {
                new_blocks: Vec::new(),
                host_text: Some(new_text),
                root_count: 0,
            })
        }
        PasteAnchor::AfterBlock(after) => {
            if text.is_empty() {
                return Ok(PasteOutcome::default());
            }
            let id = create_after(workspace, hlc, after, Some(text))?;
            Ok(PasteOutcome {
                new_blocks: vec![id],
                host_text: None,
                // Plain-text path: see `PasteOutcome::root_count`. The
                // block exists because of the anchor, not because the
                // user pasted outline syntax — counter stays at 0 so
                // the status reads "pasted text".
                root_count: 0,
            })
        }
        PasteAnchor::AsLastChildOf(parent) => {
            if text.is_empty() {
                return Ok(PasteOutcome::default());
            }
            let id = create_under(workspace, hlc, parent, Some(text))?;
            Ok(PasteOutcome {
                new_blocks: vec![id],
                host_text: None,
                root_count: 0,
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read the host block's text for a caret paste.
///
/// A block that was just created empty — the `o` key or a new line
/// emits only `Op::Create`, no `Op::Edit` — exists in the tree but has
/// no materialized text, so `block_text` returns `None`. That is **not**
/// "missing from the tree": guarding existence on `block_text` made a
/// paste into a fresh empty block fail with `block <id> is not in the
/// tree`. Guard existence on the tree itself and default a missing text
/// to empty; only a genuinely absent node is `NotInTree`.
fn host_text_for_caret(workspace: &Workspace, block: NodeId) -> Result<String, ActionError> {
    if !workspace.tree().contains(block) {
        return Err(ActionError::NotInTree(block.to_string()));
    }
    Ok(workspace.block_text(block).unwrap_or_default())
}

fn to_spec(node: &ParsedNode) -> BlockTreeSpec {
    BlockTreeSpec {
        text: node.text.clone(),
        children: node.children.iter().map(to_spec).collect(),
    }
}

fn attach_children(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parsed: &ParsedNode,
    parent_id: NodeId,
    new_blocks: &mut Vec<NodeId>,
) -> Result<(), ActionError> {
    if parsed.children.is_empty() {
        return Ok(());
    }
    let child_specs: Vec<BlockTreeSpec> = parsed.children.iter().map(to_spec).collect();
    let child_outcomes = append_forest(workspace, hlc, parent_id, &child_specs)?;
    apply_properties_forest(workspace, hlc, &parsed.children, &child_outcomes)?;
    for o in &child_outcomes {
        push_ids(new_blocks, o);
    }
    Ok(())
}

fn apply_properties_forest(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parsed: &[ParsedNode],
    outcomes: &[BlockTreeOutcome],
) -> Result<(), ActionError> {
    debug_assert_eq!(
        parsed.len(),
        outcomes.len(),
        "parsed/outcome arity mismatch"
    );
    for (p, o) in parsed.iter().zip(outcomes.iter()) {
        apply_properties_node(workspace, hlc, p, o.id)?;
        apply_properties_forest(workspace, hlc, &p.children, &o.children)?;
    }
    Ok(())
}

fn apply_properties_node(
    workspace: &mut Workspace,
    hlc: &HlcGenerator,
    parsed: &ParsedNode,
    node: NodeId,
) -> Result<(), ActionError> {
    for (key, value) in &parsed.properties {
        set_property(
            workspace,
            hlc,
            node,
            key,
            Some(PropValue::Text(value.clone())),
        )?;
    }
    Ok(())
}

use crate::text::split_at_char;

fn join_caret_left_with(left: &str, addition: &str) -> String {
    if left.is_empty() {
        return addition.to_string();
    }
    let ends_ws = left
        .chars()
        .last()
        .map(|c| c.is_whitespace())
        .unwrap_or(true);
    let starts_ws = addition
        .chars()
        .next()
        .map(|c| c.is_whitespace())
        .unwrap_or(true);
    if ends_ws || starts_ws {
        format!("{left}{addition}")
    } else {
        format!("{left} {addition}")
    }
}

/// Flatten a `BlockTreeOutcome` forest into the ids it minted, in
/// DFS / sibling order.
fn collect_ids(outcomes: &[BlockTreeOutcome]) -> Vec<NodeId> {
    let mut out = Vec::new();
    for o in outcomes {
        push_ids(&mut out, o);
    }
    out
}

fn push_ids(out: &mut Vec<NodeId>, o: &BlockTreeOutcome) {
    out.push(o.id);
    for c in &o.children {
        push_ids(out, c);
    }
}
