//! Sibling **order** and visit **sequence** of the two whole-subtree
//! walks: `outl_actions::walk_subtree` and
//! `outl_actions::project_outline`.
//!
//! Both used to resolve children through `tree::children_of`, one full
//! `iter_nodes()` rescan per visited node. Replacing that with a
//! pre-built index is a pure performance change, and the thing a
//! performance change here breaks is order — which reaches the user as
//! reordered blocks in their `.md`, and (root `CLAUDE.md` invariant 7)
//! as two devices rendering one op log two ways.
//!
//! So these tests pin the observable output rather than the mechanism:
//! DFS pre-order, siblings by `(position, NodeId)`, ties included,
//! early-stop prefix included, trash excluded. They are deliberately
//! written against the public API so they hold whichever way children
//! are resolved underneath.

use outl_actions::{children_of, project_outline, walk_subtree, OutlineNode};
use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

struct Fixture {
    ws: Workspace,
    hlc: HlcGenerator,
}

impl Fixture {
    fn new() -> Self {
        let actor = ActorId::new();
        Fixture {
            ws: Workspace::open_in_memory(actor).expect("in-memory workspace"),
            hlc: HlcGenerator::new(actor),
        }
    }

    fn create_at(&mut self, parent: NodeId, position: &str) -> NodeId {
        let node = NodeId::new();
        let ts = self.hlc.next();
        self.ws
            .apply(LogOp {
                ts,
                actor: ts.actor,
                op: Op::Create {
                    node,
                    parent,
                    position: Fractional::parse(position).expect("valid position"),
                },
            })
            .expect("create applies");
        node
    }

    fn move_to(&mut self, node: NodeId, new_parent: NodeId, position: &str) {
        let ts = self.hlc.next();
        self.ws
            .apply(LogOp {
                ts,
                actor: ts.actor,
                op: Op::Move {
                    node,
                    new_parent,
                    position: Fractional::parse(position).expect("valid position"),
                    old_parent: NodeId::root(),
                    old_position: Fractional::first(),
                },
            })
            .expect("move applies");
    }

    fn visit(&self, from: NodeId) -> Vec<NodeId> {
        let mut seen = Vec::new();
        walk_subtree(&self.ws, from, |id| {
            seen.push(id);
            true
        });
        seen
    }

    fn outline_ids(&self, from: NodeId) -> Vec<NodeId> {
        fn flatten(nodes: &[OutlineNode], out: &mut Vec<NodeId>) {
            for node in nodes {
                let ulid = ulid::Ulid::from_string(&node.id).expect("outline id is a ULID");
                out.push(NodeId(ulid));
                flatten(&node.children, out);
            }
        }
        let mut out = Vec::new();
        flatten(&project_outline(&self.ws, from), &mut out);
        out
    }
}

/// Siblings sharing one `Fractional` are ordered by `NodeId`, at every
/// depth — not just at the top level the `children_of` unit test covers.
///
/// `Tree::iter_nodes` walks a `HashMap` seeded per process, so a walk
/// that loses the tiebreak renders the same op log differently on every
/// boot. Two levels of ties is the case an index build gets wrong when
/// it sorts the top level and forgets the rest.
#[test]
fn tied_positions_are_broken_by_node_id_at_every_depth() {
    let mut f = Fixture::new();
    let page = f.create_at(NodeId::root(), "m");

    let mut parents: Vec<NodeId> = (0..6).map(|_| f.create_at(page, "a")).collect();
    let mut kids_by_parent: Vec<(NodeId, Vec<NodeId>)> = parents
        .iter()
        .map(|&p| {
            let mut kids: Vec<NodeId> = (0..4).map(|_| f.create_at(p, "a")).collect();
            kids.sort();
            (p, kids)
        })
        .collect();
    parents.sort();
    kids_by_parent.sort_by_key(|(p, _)| *p);

    let mut expected = Vec::new();
    for (parent, kids) in &kids_by_parent {
        expected.push(*parent);
        expected.extend(kids.iter().copied());
    }

    assert_eq!(
        f.visit(page),
        expected,
        "walk_subtree must order tied siblings by NodeId at every depth"
    );
    assert_eq!(
        f.outline_ids(page),
        expected,
        "project_outline must agree with walk_subtree, tie for tie"
    );
}

/// Position wins over `NodeId` — the tiebreak is the *second* key, not
/// the first. An index that sorted by id alone would pass the tie test
/// above and still reorder every real page.
#[test]
fn position_orders_siblings_before_the_node_id_tiebreak() {
    let mut f = Fixture::new();
    let page = f.create_at(NodeId::root(), "m");

    // Created in reverse position order, so creation order (and the
    // ULID order that follows it) disagrees with the intended order.
    let last = f.create_at(page, "z");
    let middle = f.create_at(page, "n");
    let first = f.create_at(page, "b");

    assert_eq!(f.visit(page), vec![first, middle, last]);
    assert_eq!(f.outline_ids(page), vec![first, middle, last]);
    assert_eq!(
        children_of(&f.ws, page)
            .into_iter()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        vec![first, middle, last],
        "the walks must agree with the single-lookup primitive"
    );
}

/// DFS pre-order: a node is visited before its descendants, and a whole
/// subtree is finished before the next sibling starts. Breadth-first
/// would pass a "same set" assertion and reorder every nested page.
#[test]
fn nested_siblings_are_visited_depth_first_pre_order() {
    let mut f = Fixture::new();
    let page = f.create_at(NodeId::root(), "m");

    let a = f.create_at(page, "b");
    let a1 = f.create_at(a, "b");
    let a1x = f.create_at(a1, "b");
    let a2 = f.create_at(a, "n");
    let b = f.create_at(page, "n");
    let b1 = f.create_at(b, "b");

    assert_eq!(f.visit(page), vec![a, a1, a1x, a2, b, b1]);
    assert_eq!(f.outline_ids(page), vec![a, a1, a1x, a2, b, b1]);
}

/// A single-child chain is the degenerate shape a level-batched index
/// build handles worst — one node per level, `depth` passes. Pin that
/// it still produces the chain in order.
#[test]
fn a_deep_single_child_chain_walks_top_down() {
    let mut f = Fixture::new();
    let page = f.create_at(NodeId::root(), "m");

    let mut chain = Vec::new();
    let mut current = page;
    for _ in 0..40 {
        current = f.create_at(current, "m");
        chain.push(current);
    }

    assert_eq!(f.visit(page), chain);
    assert_eq!(f.outline_ids(page), chain);
}

/// A leaf invokes the closure zero times and projects an empty outline.
#[test]
fn a_leaf_subtree_visits_nothing() {
    let mut f = Fixture::new();
    let page = f.create_at(NodeId::root(), "m");
    let leaf = f.create_at(page, "m");

    assert!(f.visit(leaf).is_empty());
    assert!(project_outline(&f.ws, leaf).is_empty());
}

/// Deleting is `Move(node, TRASH_ROOT)` (invariant 6), so trashed blocks
/// are still in `iter_nodes()`. A walk of the page must not see them,
/// and a walk of the trash must — an index built by scanning every node
/// gets the second half wrong if it scopes itself to the live tree.
#[test]
fn trashed_blocks_leave_the_page_walk_and_stay_reachable_from_the_trash() {
    let mut f = Fixture::new();
    let page = f.create_at(NodeId::root(), "m");
    let kept = f.create_at(page, "b");
    let doomed = f.create_at(page, "n");
    let doomed_kid = f.create_at(doomed, "m");
    let tail = f.create_at(page, "y");

    f.move_to(doomed, NodeId::trash(), "m");

    assert_eq!(f.visit(page), vec![kept, tail]);
    assert_eq!(f.outline_ids(page), vec![kept, tail]);
    assert_eq!(
        f.visit(NodeId::trash()),
        vec![doomed, doomed_kid],
        "the deleted subtree keeps its shape under the trash root"
    );
}

/// Returning `false` stops the walk *immediately* — the visited prefix
/// is exactly the DFS pre-order up to and including the refusing node,
/// with no sibling of any ancestor visited afterwards. A rewrite that
/// materialises the subtree first and filters later would visit more.
#[test]
fn returning_false_stops_the_walk_at_that_node() {
    let mut f = Fixture::new();
    let page = f.create_at(NodeId::root(), "m");

    let a = f.create_at(page, "b");
    let a1 = f.create_at(a, "b");
    let _a1x = f.create_at(a1, "b");
    let _a2 = f.create_at(a, "n");
    let _b = f.create_at(page, "n");

    let mut seen = Vec::new();
    walk_subtree(&f.ws, page, |id| {
        seen.push(id);
        id != a1
    });

    assert_eq!(
        seen,
        vec![a, a1],
        "nothing below or after the refusing node"
    );
}

/// A block moved out of a page leaves that page's walk and joins the
/// other's, at the position the move gave it. The walks read the
/// materialised tree, never creation order.
#[test]
fn a_moved_block_walks_with_its_new_parent() {
    let mut f = Fixture::new();
    let left = f.create_at(NodeId::root(), "b");
    let right = f.create_at(NodeId::root(), "n");
    let first = f.create_at(right, "b");
    let wanderer = f.create_at(left, "m");
    let last = f.create_at(right, "y");

    f.move_to(wanderer, right, "n");

    assert!(f.visit(left).is_empty());
    assert_eq!(f.visit(right), vec![first, wanderer, last]);
    assert_eq!(f.outline_ids(right), vec![first, wanderer, last]);
}

/// The root walk is the whole live workspace in page order, and it does
/// not wander into the trash — `NodeId::root()` and `NodeId::trash()`
/// are siblings, not parent and child.
#[test]
fn the_root_walk_covers_every_page_in_order_and_skips_the_trash() {
    let mut f = Fixture::new();
    let second = f.create_at(NodeId::root(), "n");
    let first = f.create_at(NodeId::root(), "b");
    let first_kid = f.create_at(first, "m");
    let gone = f.create_at(second, "m");
    f.move_to(gone, NodeId::trash(), "m");

    assert_eq!(f.visit(NodeId::root()), vec![first, first_kid, second]);
    assert_eq!(
        f.outline_ids(NodeId::root()),
        vec![first, first_kid, second]
    );
}

/// The whole-subtree walks agree, node for node, with a reference walk
/// that resolves every level through [`children_of`] — the single-lookup
/// primitive, which is what they both used to do.
///
/// This is the equivalence the index build has to preserve and the one
/// no hand-written example can cover: a tree wide and deep enough, with
/// enough tied positions, that a wrong tiebreak or a level confusion
/// shows up somewhere. `children_of` is deliberately *not* being changed,
/// so it stays a fair referee.
#[test]
fn the_indexed_walks_match_a_children_of_reference_walk() {
    fn reference(ws: &Workspace, parent: NodeId, out: &mut Vec<NodeId>) {
        for (id, _) in children_of(ws, parent) {
            out.push(id);
            reference(ws, id, out);
        }
    }

    let mut f = Fixture::new();
    // Deterministic, so a failure is reproducible. Positions are drawn
    // from a small alphabet on purpose: ties are the interesting case
    // and a wide alphabet would hide them.
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let positions = ["a", "b", "c", "am", "bm"];

    let mut frontier = vec![NodeId::root()];
    let mut all = Vec::new();
    for _ in 0..5 {
        let mut born = Vec::new();
        for &parent in &frontier {
            for _ in 0..(1 + next() % 4) {
                let pos = positions[(next() % positions.len() as u64) as usize];
                born.push(f.create_at(parent, pos));
            }
        }
        all.extend(born.iter().copied());
        frontier = born;
    }
    // Trash a scattering of them, so the walk has to stop at subtrees
    // that are still in `iter_nodes()`.
    for (i, &node) in all.iter().enumerate() {
        if i % 11 == 0 {
            f.move_to(node, NodeId::trash(), positions[i % positions.len()]);
        }
    }
    assert!(all.len() > 50, "tree too small to be a real test");

    for &from in [NodeId::root(), NodeId::trash()].iter().chain(all.iter()) {
        let mut expected = Vec::new();
        reference(&f.ws, from, &mut expected);
        assert_eq!(
            f.visit(from),
            expected,
            "walk_subtree diverged below {from}"
        );
        assert_eq!(
            f.outline_ids(from),
            expected,
            "project_outline diverged below {from}"
        );
    }
}

/// The **whole-workspace** index builder produces the same order as
/// [`children_of`], for every parent in the tree — trash included.
///
/// `backlinks_index::build_children_index` is the second caller of
/// `tree::sort_siblings`, and the reason this test exists is that it
/// used to carry its own copy of the comparator. The copy got the
/// `NodeId` tiebreak late and never got a test, while its twin
/// `children_of` got both. A shared helper that nothing exercises
/// through the second call site is a shared helper in name only, so
/// this walks that entrance specifically.
#[test]
fn the_whole_workspace_index_matches_children_of_for_every_parent() {
    let mut f = Fixture::new();

    // Ties everywhere, at three depths, plus a trashed subtree — the
    // whole-workspace builder indexes the trash too, and that half has
    // no other test.
    let mut interesting = vec![NodeId::root(), NodeId::trash()];
    for p in 0..4 {
        let page = f.create_at(NodeId::root(), "a");
        interesting.push(page);
        for b in 0..5 {
            let block = f.create_at(page, if b % 2 == 0 { "a" } else { "b" });
            interesting.push(block);
            for _ in 0..3 {
                interesting.push(f.create_at(block, "a"));
            }
        }
        if p == 3 {
            f.move_to(page, NodeId::trash(), "a");
        }
    }

    let index = outl_actions::backlinks_index::build_children_index(&f.ws);
    for &parent in &interesting {
        let expected: Vec<NodeId> = children_of(&f.ws, parent)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        let actual = index.get(&parent).cloned().unwrap_or_default();
        assert_eq!(
            actual, expected,
            "build_children_index disagreed with children_of below {parent}"
        );
    }
}
