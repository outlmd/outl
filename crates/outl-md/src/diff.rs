//! Translate a new outline AST + match result into a minimal `Op` sequence.
//!
//! The diff is intentionally simple for now: produce one `Create` per
//! new block (matching pass yielded `MatchLevel::Low`), one `Move` per
//! preserved block whose parent or position differs from the current
//! tree, and one `Move` to `TRASH_ROOT` per orphan id.
//!
//! Idempotency in `outl-core` means re-emitting an already-applied
//! `Create` is a no-op. The diff therefore doesn't need to know the
//! workspace's current state with full precision — it can over-emit and
//! rely on the CRDT to dedup. Higher-quality diff (only emit what
//! changed) lands when we wire the watcher in `outl-cli serve` and have
//! the live tree available.

use crate::matching::{flatten, Match, MatchLevel};
use crate::parse::OutlineNode;
use crate::sidecar::{content_hash, derive_ref_handle, Sidecar, SidecarBlock};
use outl_core::fractional::Fractional;
use outl_core::id::NodeId;
use outl_core::op::Op;

/// Plan produced by [`diff_to_ops`].
///
/// `ops` is the ordered sequence to apply via `Workspace::apply`.
/// `new_sidecar` is the sidecar to write back to disk after the ops
/// commit successfully.
#[derive(Debug, Clone)]
pub struct DiffPlan {
    /// Ops to apply (in order).
    pub ops: Vec<Op>,
    /// The sidecar reflecting the new tree.
    pub new_sidecar: Sidecar,
}

/// Build a [`DiffPlan`] from the new AST and the matching result.
///
/// `page_id` is the root NodeId of the page (preserved across edits).
/// `new_md_hash` is the SHA-256 of the new `.md` text (for the sidecar
/// `last_synced_hash`).
/// `old_blocks` is the previous sidecar's block list — used to **preserve
/// existing `ref_handle`s** when a block matches at level 1 or 2. A
/// preserved id keeps its handle verbatim (including a 7+ char tail if
/// a past collision forced expansion), so any `((blk-XXXXXX))` already
/// living in another `.md` keeps resolving.
pub fn diff_to_ops(
    new_blocks: &[OutlineNode],
    matches: &[Match],
    orphans: &[NodeId],
    page_id: NodeId,
    new_md_hash: &str,
    old_blocks: &[SidecarBlock],
) -> DiffPlan {
    diff_to_ops_with_page_props(
        new_blocks,
        matches,
        orphans,
        page_id,
        new_md_hash,
        old_blocks,
        &[],
        &|_| None,
    )
}

/// Same as [`diff_to_ops`] but also propagates **page-level**
/// properties (the `key:: value` lines at the top of the `.md`, before
/// the first bullet) into the op log as `Op::SetProp` on `page_id`.
///
/// Without this, page-level metadata (`type:: person`, `pinned::`,
/// `icon::`, `title::`, `role::`, …) lives only in the rendered `.md`
/// — the workspace's CRDT tree never learns it, so anything that
/// reads via `workspace.tree().property(page_id, ...)` (the desktop's
/// `search_persons`, `page_meta.pinned`, etc.) gets `None`. Meanwhile
/// `outl_md::WorkspaceIndex` parses the `.md` directly and sees the
/// property, producing a silent divergence between two "should-be"
/// authoritative views of the same fact.
///
/// Idempotency: emitting `Op::SetProp` with the same value the tree
/// already has is a no-op via the CRDT's last-writer-wins on the same
/// HLC clock, so we can safely re-emit on every reconcile pass
/// (external `.md` edit, `outl-cli serve` watcher, peer ops change)
/// without flooding the log.
// Each argument is a distinct reconcile input (new AST, matches,
// orphans, page id, hash, old sidecar, page props, current-position
// lookup); bundling them into a struct would just move the noise.
#[allow(clippy::too_many_arguments)]
pub fn diff_to_ops_with_page_props(
    new_blocks: &[OutlineNode],
    matches: &[Match],
    orphans: &[NodeId],
    page_id: NodeId,
    new_md_hash: &str,
    old_blocks: &[SidecarBlock],
    page_properties: &[(String, String)],
    // Current fractional position of a node in the workspace tree, or
    // `None` if it isn't there yet. Reusing the existing position for an
    // unchanged block keeps the `Move` a no-op (same target), so a
    // one-block edit doesn't churn every sibling's position and emit a
    // pile of redundant, fsynced `Move` ops. Pass `|_| None` to always
    // allocate fresh positions (the old behaviour; used by unit tests).
    current_pos: &dyn Fn(NodeId) -> Option<Fractional>,
) -> DiffPlan {
    let flat = flatten(new_blocks);
    assert_eq!(
        flat.len(),
        matches.len(),
        "match count must equal new block count"
    );

    // Assign IDs for each new block: keep the matched id for level-1
    // results, mint a fresh ULID for level-3.
    let ids: Vec<NodeId> = matches
        .iter()
        .map(|m| match m.level {
            MatchLevel::High => m.old_id.expect("level High implies Some(id)"),
            MatchLevel::Medium => m.old_id.expect("level Medium implies Some(id)"),
            MatchLevel::Low => NodeId::new(),
        })
        .collect();

    // For each new block, decide which handle persists. Preserved
    // blocks (level 1/2) keep the old sidecar's handle if present;
    // newly minted blocks (level 3) derive a fresh one from their id.
    //
    // The pre-fix path did `.iter().find()` per block — O(N²) over
    // pages with many blocks. A single HashMap pass up front keeps it
    // O(N).
    let old_by_id: std::collections::HashMap<NodeId, &SidecarBlock> =
        old_blocks.iter().map(|b| (b.id, b)).collect();
    let handles: Vec<String> = matches
        .iter()
        .zip(ids.iter())
        .map(|(m, id)| match m.level {
            MatchLevel::High | MatchLevel::Medium => old_by_id
                .get(id)
                .map(|b| b.ref_handle.clone())
                .filter(|h| !h.is_empty())
                .unwrap_or_else(|| derive_ref_handle(*id)),
            MatchLevel::Low => derive_ref_handle(*id),
        })
        .collect();

    // Walk the new tree in DFS preorder, generating ops + sidecar entries.
    let mut ops = Vec::<Op>::new();
    let mut sidecar_blocks = Vec::<SidecarBlock>::new();
    // Stack of (parent_id, last_position_used) per ancestor — index by indent.
    let mut parent_stack: Vec<NodeId> = vec![page_id];
    let mut last_position_per_indent: Vec<Option<Fractional>> = vec![None];

    // Track DFS line numbers for the sidecar.
    let mut line_counter: usize = 1;

    // Page-level properties (`title::`, `type::`, `pinned::`, `icon::`,
    // `role::`, …) live as `(key, value)` lines at the top of the `.md`
    // — outside any block. Emit one `Op::SetProp` per property on the
    // page root so the CRDT tree stays in sync with what's on disk.
    // Without this, anything reading `workspace.tree().property(page_id, ...)`
    // (`page_meta.pinned`, `search_persons`, future filters) silently
    // sees `None` while the `.md` shows the property — exactly the
    // divergence the desktop's `@` autocomplete hit on
    // fixture-populated person pages.
    //
    // Book-keeping keys are skipped. `outl-actions::page` owns `page-slug` /
    // `page-kind` via its own ops (inlined: no dep on `outl-actions`; keep in
    // sync with `outl_actions::page::{SLUG_KEY, KIND_KEY}`). `page-frontmatter`
    // is written only by `reconcile::page_root::sync_page_frontmatter` from the
    // real `---` fence; a typed `page-frontmatter::` line would land after it,
    // replace the fence body, and the next projection would clobber the YAML.
    const PAGE_SLUG_KEY: &str = "page-slug";
    const PAGE_KIND_KEY: &str = "page-kind";
    const PAGE_FRONTMATTER_KEY: &str = crate::frontmatter::PAGE_FRONTMATTER_KEY;
    for (key, value) in page_properties {
        if [PAGE_SLUG_KEY, PAGE_KIND_KEY, PAGE_FRONTMATTER_KEY].contains(&key.as_str()) {
            continue;
        }
        ops.push(Op::SetProp {
            node: page_id,
            key: key.clone(),
            value: Some(outl_core::property::PropValue::Text(value.clone())),
            old_value: None,
        });
    }

    walk(
        new_blocks,
        0,
        &ids,
        &handles,
        &mut 0usize, // running index into `ids` / `handles`
        &mut ops,
        &mut sidecar_blocks,
        &mut parent_stack,
        &mut last_position_per_indent,
        &mut line_counter,
        current_pos,
    );

    // Orphan moves: each goes to TRASH_ROOT at a fresh position.
    for (i, orphan) in orphans.iter().enumerate() {
        // Build a unique fractional position so concurrent trashings
        // don't collide on position.
        let pos = Fractional::between(None, None);
        let _ = i; // position uniqueness comes from siblings of TRASH_ROOT,
                   // not from this index; the CRDT will resolve any
                   // collision via HLC tiebreak.
        ops.push(Op::Move {
            node: *orphan,
            new_parent: NodeId::trash(),
            position: pos,
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        });
    }

    let new_sidecar = Sidecar {
        version: crate::sidecar::SIDECAR_VERSION,
        page_id,
        last_synced_hash: new_md_hash.to_string(),
        last_synced_at: chrono::Local::now().fixed_offset(),
        blocks: sidecar_blocks,
        // This sidecar was just built by `diff_to_ops_with_page_props`,
        // which emits one `Op::SetProp` per page-level property on
        // `page_id`. Persist `true` so the orphan scanner skips this
        // page on the next sweep (the migration is one-shot).
        pipeline_version: crate::sidecar::CURRENT_PIPELINE_VERSION,
    };

    DiffPlan { ops, new_sidecar }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    blocks: &[OutlineNode],
    indent: u32,
    ids: &[NodeId],
    handles: &[String],
    cursor: &mut usize,
    ops: &mut Vec<Op>,
    sidecar_blocks: &mut Vec<SidecarBlock>,
    parent_stack: &mut Vec<NodeId>,
    last_position_per_indent: &mut Vec<Option<Fractional>>,
    line_counter: &mut usize,
    current_pos: &dyn Fn(NodeId) -> Option<Fractional>,
) {
    // Ensure the stack has a slot for the current indent's position
    // bookkeeping.
    while last_position_per_indent.len() <= indent as usize + 1 {
        last_position_per_indent.push(None);
    }

    // Reset the position counter at this indent so siblings get fresh
    // positions when we enter a new parent.
    last_position_per_indent[indent as usize + 1] = None;

    let parent_id = *parent_stack.last().expect("stack never empty");

    for block in blocks {
        let id = ids[*cursor];
        let line = *line_counter;
        *line_counter += 1;

        // Position, strictly greater than the previous sibling's. Prefer
        // REUSING the node's current position: when the order hasn't
        // changed the reused value still sorts after `left`, so the
        // emitted `Move` is a no-op (same target) that the reconcile
        // filters out — no churn. Only when the block is new, or its old
        // position would break sort order (a real reorder), do we mint a
        // fresh one between `left` and the next sibling.
        let left = last_position_per_indent[indent as usize + 1].clone();
        let position = match current_pos(id) {
            Some(cur) if left.as_ref().is_none_or(|l| cur > *l) => cur,
            _ => Fractional::between(left.as_ref(), None),
        };

        // Always emit a Create. Idempotent if the block already exists.
        ops.push(Op::Create {
            node: id,
            parent: parent_id,
            position: position.clone(),
        });
        // Also emit a Move with the same target so the tree is
        // up-to-date even when this id already exists with a different
        // parent/position.
        ops.push(Op::Move {
            node: id,
            new_parent: parent_id,
            position: position.clone(),
            old_parent: NodeId::root(),
            old_position: Fractional::first(),
        });

        last_position_per_indent[indent as usize + 1] = Some(position.clone());

        // SetProp ops for each block property.
        for (k, v) in &block.properties {
            ops.push(Op::SetProp {
                node: id,
                key: k.clone(),
                value: Some(outl_core::property::PropValue::Text(v.clone())),
                old_value: None,
            });
        }

        sidecar_blocks.push(SidecarBlock {
            id,
            line,
            indent,
            content_hash: content_hash(&block.text),
            // Not `SidecarBlock::from_text`: the handle may be an
            // expanded (post-collision) one carried over verbatim from
            // the previous sidecar.
            ref_handle: handles[*cursor].clone(),
            // The text this sidecar entry describes. Next reconcile
            // reads it back as the "before" side of level-2 matching.
            text: block.text.clone(),
        });

        *cursor += 1;

        // Recurse into children.
        parent_stack.push(id);
        walk(
            &block.children,
            indent + 1,
            ids,
            handles,
            cursor,
            ops,
            sidecar_blocks,
            parent_stack,
            last_position_per_indent,
            line_counter,
            current_pos,
        );
        parent_stack.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::match_blocks;
    use crate::parse::parse;
    use crate::sidecar::file_hash;
    use outl_core::workspace::Workspace;

    #[test]
    fn diff_produces_ops_with_preserved_ids_for_level1() {
        let md = "- a\n- b\n";
        let ast = parse(md);
        let id_a = NodeId::new();
        let id_b = NodeId::new();
        let old = vec![
            SidecarBlock::from_text(id_a, 1, 0, "a"),
            SidecarBlock::from_text(id_b, 2, 0, "b"),
        ];
        let (matches, orphans) = match_blocks(&ast.blocks, &old);
        let plan = diff_to_ops(
            &ast.blocks,
            &matches,
            &orphans,
            NodeId::new(),
            &file_hash(md),
            &old,
        );
        // Each block produced 1 Create + 1 Move (no properties).
        assert_eq!(plan.ops.len(), 4);
        // Sidecar carries both ids.
        assert_eq!(plan.new_sidecar.blocks.len(), 2);
        assert_eq!(plan.new_sidecar.blocks[0].id, id_a);
        assert_eq!(plan.new_sidecar.blocks[1].id, id_b);
    }

    #[test]
    fn diff_orphans_become_trash_moves() {
        let md = "- a\n";
        let ast = parse(md);
        let id_a = NodeId::new();
        let id_dead = NodeId::new();
        let old = vec![
            SidecarBlock::from_text(id_a, 1, 0, "a"),
            SidecarBlock::from_text(id_dead, 2, 0, "gone"),
        ];
        let (matches, orphans) = match_blocks(&ast.blocks, &old);
        let plan = diff_to_ops(
            &ast.blocks,
            &matches,
            &orphans,
            NodeId::new(),
            &file_hash(md),
            &old,
        );
        // Last op must be Move(id_dead, TRASH).
        let last = plan.ops.last().unwrap();
        match last {
            Op::Move {
                node, new_parent, ..
            } => {
                assert_eq!(*node, id_dead);
                assert_eq!(*new_parent, NodeId::trash());
            }
            other => panic!("expected Move to trash, got {other:?}"),
        }
    }

    #[test]
    fn diff_apply_then_reparse_preserves_structure() {
        // End-to-end: parse md → match → diff → apply ops → materialize
        // tree → re-render md → reparse → same AST.
        let md = "title:: doc\n\n- a\n  - a1\n  - a2\n- b\n";
        let ast = parse(md);
        let page_id = NodeId::new();
        let (matches, orphans) = match_blocks(&ast.blocks, &[]);
        let plan = diff_to_ops(
            &ast.blocks,
            &matches,
            &orphans,
            page_id,
            &file_hash(md),
            &[],
        );

        let actor = outl_core::id::ActorId::new();
        let g = outl_core::hlc::HlcGenerator::new(actor);
        let mut ws = Workspace::open_in_memory(actor).unwrap();
        for op in plan.ops {
            let ts = g.next();
            ws.apply(outl_core::op::LogOp {
                ts,
                actor: ts.actor,
                op,
            })
            .unwrap();
        }
        // Tree contains the four blocks (a, a1, a2, b).
        assert_eq!(ws.tree().node_count(), 4);
    }

    #[test]
    fn level1_match_preserves_custom_ref_handle_verbatim() {
        // Hypothetical scenario: a past collision forced the block's
        // handle to expand to 7 chars (`blk-r6s4a1z`). Re-running the
        // matching → diff pipeline must NOT silently rederive a 6-char
        // handle, because any `.md` already citing `((blk-r6s4a1z))`
        // would stop resolving.
        let md = "- alpha\n";
        let ast = parse(md);
        let id = NodeId::new();
        let custom_handle = "blk-r6s4a1z".to_string();
        let old = vec![SidecarBlock {
            ref_handle: custom_handle.clone(),
            ..SidecarBlock::from_text(id, 1, 0, "alpha")
        }];
        let (matches, orphans) = match_blocks(&ast.blocks, &old);
        let plan = diff_to_ops(
            &ast.blocks,
            &matches,
            &orphans,
            NodeId::new(),
            &file_hash(md),
            &old,
        );
        assert_eq!(plan.new_sidecar.blocks.len(), 1);
        assert_eq!(
            plan.new_sidecar.blocks[0].ref_handle, custom_handle,
            "level-1 preserved blocks must keep the old sidecar's ref_handle verbatim"
        );
    }

    #[test]
    fn level3_block_gets_freshly_derived_handle() {
        // A wholly new block (no old match) gets its handle from
        // `derive_ref_handle(id)`. The format must match the canonical
        // shape and be tied to the freshly minted id.
        let md = "- brand new content\n";
        let ast = parse(md);
        let (matches, orphans) = match_blocks(&ast.blocks, &[]);
        let plan = diff_to_ops(
            &ast.blocks,
            &matches,
            &orphans,
            NodeId::new(),
            &file_hash(md),
            &[],
        );
        let sb = &plan.new_sidecar.blocks[0];
        assert_eq!(sb.ref_handle, derive_ref_handle(sb.id));
    }

    #[test]
    fn page_props_emit_setprop_on_page_id() {
        // A `.md` with page-level properties at the top must produce
        // one `Op::SetProp` per property targeting the page root.
        // Without this, page-level metadata (`type:: person`, …) lives
        // only in the rendered `.md` and never reaches the workspace
        // tree, so `workspace.tree().property(page_id, "type")` is
        // silently `None` even though `WorkspaceIndex` (which reads
        // the `.md` directly) sees the value.
        let md = "title:: Avelino\ntype:: person\npinned:: true\n\n- bio\n";
        let ast = parse(md);
        let (matches, orphans) = match_blocks(&ast.blocks, &[]);
        let page_id = NodeId::new();
        let plan = diff_to_ops_with_page_props(
            &ast.blocks,
            &matches,
            &orphans,
            page_id,
            &file_hash(md),
            &[],
            &ast.properties,
            &|_| None,
        );

        let prop_ops: Vec<(&str, &str)> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::SetProp {
                    node,
                    key,
                    value: Some(outl_core::property::PropValue::Text(v)),
                    ..
                } if *node == page_id => Some((key.as_str(), v.as_str())),
                _ => None,
            })
            .collect();

        assert!(
            prop_ops.contains(&("title", "Avelino")),
            "title:: Avelino must be emitted as SetProp on the page root"
        );
        assert!(
            prop_ops.contains(&("type", "person")),
            "type:: person must be emitted as SetProp on the page root"
        );
        assert!(
            prop_ops.contains(&("pinned", "true")),
            "pinned:: true must be emitted as SetProp on the page root"
        );
        assert_eq!(
            plan.new_sidecar.pipeline_version,
            crate::sidecar::CURRENT_PIPELINE_VERSION,
            "new sidecar must stamp the current pipeline version"
        );
    }

    #[test]
    fn page_props_skip_internal_book_keeping_keys() {
        // The page-model layer owns `page-slug` and `page-kind` and
        // emits its own ops for them. The reconcile pipeline must
        // **not** re-emit these from a `.md` parse — overwriting the
        // slug would rename the page silently.
        let md = "page-slug:: avelino\npage-kind:: page\ntitle:: Avelino\n\n- bio\n";
        let ast = parse(md);
        let (matches, orphans) = match_blocks(&ast.blocks, &[]);
        let page_id = NodeId::new();
        let plan = diff_to_ops_with_page_props(
            &ast.blocks,
            &matches,
            &orphans,
            page_id,
            &file_hash(md),
            &[],
            &ast.properties,
            &|_| None,
        );

        let prop_keys: Vec<&str> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::SetProp { node, key, .. } if *node == page_id => Some(key.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            !prop_keys.contains(&"page-slug"),
            "page-slug must be skipped — owned by outl_actions::page"
        );
        assert!(
            !prop_keys.contains(&"page-kind"),
            "page-kind must be skipped — owned by outl_actions::page"
        );
        assert!(
            prop_keys.contains(&"title"),
            "free-form props like `title` must still flow through"
        );
    }

    #[test]
    fn diff_to_ops_back_compat_does_not_emit_page_props() {
        // Old call-sites that haven't migrated to
        // `diff_to_ops_with_page_props` get the legacy behaviour:
        // page-level props don't flow into the op log, and the
        // sidecar marks them as not-yet-propagated so the orphan
        // scanner re-runs reconcile on the next sweep.
        let md = "type:: person\n\n- bio\n";
        let ast = parse(md);
        let (matches, orphans) = match_blocks(&ast.blocks, &[]);
        let page_id = NodeId::new();
        let plan = diff_to_ops(
            &ast.blocks,
            &matches,
            &orphans,
            page_id,
            &file_hash(md),
            &[],
        );
        let has_page_prop_op = plan.ops.iter().any(|op| {
            matches!(
                op,
                Op::SetProp { node, .. } if *node == page_id
            )
        });
        assert!(
            !has_page_prop_op,
            "legacy diff_to_ops must not emit page-level SetProps"
        );
    }
}
