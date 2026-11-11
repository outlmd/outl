//! Placing a block after another one, over and over.
//!
//! The property the proptest in `outl-core` cannot see: it pins
//! [`outl_core::fractional::Fractional::between`] alone, and `between`
//! behaves exactly as its contract says — on an empty gap the lower
//! bound wins and the result sorts past `right`. The misuse was in the
//! callers, which took that key and placed a block with it, so the
//! block landed **after** the neighbour it was supposed to precede
//! ([issue #476](https://github.com/outlmd/outl/issues/476)).
//!
//! So these are assertions over the actions a keystroke runs —
//! `outdent`, `create_after`, `move_after` — never over `between`.
//! Two halves are under test and both reach the same misplacement:
//!
//! - the **mint rule**: a new key never ends in the floor byte, so the
//!   empty gap stops being produced;
//! - the **check** in `position_after`, for the op logs that already
//!   hold floor-tailed keys and the ties from #282. Those tests place
//!   their siblings at hand-picked positions, because that is the only
//!   way to reconstruct a workspace an older build wrote.

use outl_actions::block::{append_block, create_after, move_after, outdent};
use outl_actions::page::{open_or_create, PageKind};
use outl_actions::tree::children_of;
use outl_core::fractional::Fractional;
use outl_core::hlc::HlcGenerator;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

fn new_workspace() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).expect("in-memory workspace"),
        HlcGenerator::new(actor),
    )
}

/// Put `node` under `parent` at `pos`, straight through the op log.
///
/// Reconstructs a workspace an older build left behind: the positions
/// are the subject here, and `append_block` does not let a test choose
/// them.
fn place(ws: &mut Workspace, hlc: &HlcGenerator, node: NodeId, parent: NodeId, pos: &str) {
    let ts = hlc.next();
    ws.apply(LogOp {
        ts,
        actor: ts.actor,
        op: Op::Create {
            node,
            parent,
            position: Fractional::parse(pos).expect("valid position"),
        },
    })
    .expect("create applies");
}

/// Block texts under `parent`, in the order every device renders them.
fn order(ws: &Workspace, parent: NodeId) -> Vec<String> {
    children_of(ws, parent)
        .into_iter()
        .map(|(id, _)| ws.block_text(id).unwrap_or_default())
        .collect()
}

fn position_of(ws: &Workspace, node: NodeId) -> String {
    ws.tree()
        .position(node)
        .map(|p| p.as_str().to_string())
        .expect("node is in the tree")
}

/// Lifting a list out bottom-up, which is how it keeps its order.
///
/// Each outdent asks for the slot right after `P`, so the gap between
/// `P` and whatever was placed last keeps halving towards `P`. Around
/// the seventh step the old mint rule produced `P`'s key plus a single
/// floor byte, the step after that fell into the empty gap above it, and
/// the block landed behind the one it should have preceded: the issue's
/// `P c4 c3 c5 …`.
#[test]
fn outdenting_bottom_up_preserves_visual_order() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let parent = append_block(&mut ws, &hlc, Some(page), Some("P")).expect("P");
    let children: Vec<NodeId> = (1..=10)
        .map(|i| append_block(&mut ws, &hlc, Some(parent), Some(&format!("c{i}"))).expect("child"))
        .collect();
    append_block(&mut ws, &hlc, Some(page), Some("Q")).expect("Q");

    for child in children.iter().rev() {
        outdent(&mut ws, &hlc, *child).expect("outdent");
    }

    let expected: Vec<String> = std::iter::once("P".to_string())
        .chain((1..=10).map(|i| format!("c{i}")))
        .chain(std::iter::once("Q".to_string()))
        .collect();
    assert_eq!(order(&ws, page), expected);
}

/// Enter on the same block, nine times.
///
/// Every new block goes immediately after `X`, so the newest is the
/// closest to it and the run reads back in reverse. Same halving gap as
/// the outdent walk, same wall at the seventh key.
#[test]
fn repeated_inserts_after_one_block_stay_in_order() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let x = append_block(&mut ws, &hlc, Some(page), Some("X")).expect("X");
    append_block(&mut ws, &hlc, Some(page), Some("Y")).expect("Y");

    for i in 1..=9 {
        create_after(&mut ws, &hlc, x, Some(&format!("n{i}"))).expect("create after X");
    }

    let expected: Vec<String> = std::iter::once("X".to_string())
        .chain((1..=9).rev().map(|i| format!("n{i}")))
        .chain(std::iter::once("Y".to_string()))
        .collect();
    assert_eq!(order(&ws, page), expected);
}

/// The third caller of the same slot, through the cut-and-paste
/// primitive rather than a create.
#[test]
fn repeated_moves_after_one_block_stay_in_order() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let x = append_block(&mut ws, &hlc, Some(page), Some("X")).expect("X");
    append_block(&mut ws, &hlc, Some(page), Some("Y")).expect("Y");
    let pool: Vec<NodeId> = (1..=9)
        .map(|i| append_block(&mut ws, &hlc, Some(page), Some(&format!("m{i}"))).expect("block"))
        .collect();

    for node in &pool {
        move_after(&mut ws, &hlc, *node, x).expect("move after X");
    }

    let expected: Vec<String> = std::iter::once("X".to_string())
        .chain((1..=9).rev().map(|i| format!("m{i}")))
        .chain(std::iter::once("Y".to_string()))
        .collect();
    assert_eq!(order(&ws, page), expected);
}

/// The half the mint rule cannot reach: a workspace whose op log
/// **already** holds `left + "a"`.
///
/// The issue's own trace — `b9` at `b`, `b14` right after it at `ba`.
/// Positions are replayed, never recomputed, so no change to `between`
/// repairs this pair; the slot has to be opened by moving the right
/// neighbour up, the way `create_before` shifts its anchor.
#[test]
fn creating_after_a_floor_tailed_neighbour_opens_a_slot() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let left = NodeId::new();
    let right = NodeId::new();
    place(&mut ws, &hlc, left, page, "b");
    place(&mut ws, &hlc, right, page, "ba");
    outl_actions::block::edit_text(&mut ws, &hlc, left, "b9").expect("text");
    outl_actions::block::edit_text(&mut ws, &hlc, right, "b14").expect("text");

    create_after(&mut ws, &hlc, left, Some("fresh")).expect("create after b9");

    assert_eq!(order(&ws, page), vec!["b9", "fresh", "b14"]);
}

/// Same gap, reached by the outdent path.
#[test]
fn outdenting_past_a_floor_tailed_neighbour_opens_a_slot() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let parent = NodeId::new();
    let right = NodeId::new();
    place(&mut ws, &hlc, parent, page, "b");
    place(&mut ws, &hlc, right, page, "ba");
    outl_actions::block::edit_text(&mut ws, &hlc, parent, "b9").expect("text");
    outl_actions::block::edit_text(&mut ws, &hlc, right, "b14").expect("text");
    let child = append_block(&mut ws, &hlc, Some(parent), Some("b12")).expect("child");

    outdent(&mut ws, &hlc, child).expect("outdent");

    assert_eq!(order(&ws, page), vec!["b9", "b12", "b14"]);
}

/// Same gap, reached by the cut-and-paste primitive.
#[test]
fn moving_after_a_floor_tailed_neighbour_opens_a_slot() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let left = NodeId::new();
    let right = NodeId::new();
    place(&mut ws, &hlc, left, page, "b");
    place(&mut ws, &hlc, right, page, "ba");
    outl_actions::block::edit_text(&mut ws, &hlc, left, "b9").expect("text");
    outl_actions::block::edit_text(&mut ws, &hlc, right, "b14").expect("text");
    let wanderer = append_block(&mut ws, &hlc, Some(page), Some("b12")).expect("block");

    move_after(&mut ws, &hlc, wanderer, left).expect("move after b9");

    assert_eq!(order(&ws, page), vec!["b9", "b12", "b14"]);
}

/// The slot cannot always be opened, and then no existing sibling may
/// move.
///
/// `"baa"` is `"ba"` plus a floor byte, so the gap above the right
/// neighbour is empty too. Shifting it there would send it below a block
/// the user never touched — the rule `create_before` already follows.
/// What the new block gives up in exchange is placement relative to that
/// neighbour: it shares the key, and `sort_siblings`' `NodeId` tiebreak
/// decides which of the two comes first. Landing after the anchor is
/// guaranteed, and that is what the user asked for.
#[test]
fn a_slot_that_cannot_be_opened_moves_nobody() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let left = NodeId::new();
    let mid = NodeId::new();
    let right = NodeId::new();
    place(&mut ws, &hlc, left, page, "b");
    place(&mut ws, &hlc, mid, page, "ba");
    place(&mut ws, &hlc, right, page, "baa");
    outl_actions::block::edit_text(&mut ws, &hlc, left, "left").expect("text");
    outl_actions::block::edit_text(&mut ws, &hlc, mid, "mid").expect("text");
    outl_actions::block::edit_text(&mut ws, &hlc, right, "right").expect("text");

    let fresh = create_after(&mut ws, &hlc, left, Some("fresh")).expect("create after left");

    assert_eq!(position_of(&ws, left), "b", "the anchor may not move");
    assert_eq!(position_of(&ws, mid), "ba", "no sibling may move");
    assert_eq!(position_of(&ws, right), "baa", "no sibling may move");
    let rendered = order(&ws, page);
    assert_eq!(rendered.len(), 4, "nothing dropped: {rendered:?}");
    let at = |text: &str| {
        rendered
            .iter()
            .position(|t| t == text)
            .expect("still a sibling")
    };
    assert_eq!(at("left"), 0, "the new block lands after the anchor");
    assert_eq!(at("right"), 3, "the run keeps its order: {rendered:?}");
    assert_eq!(ws.block_text(fresh).as_deref(), Some("fresh"));
}

/// A tie is the other empty gap, and it is the one shape where moving
/// the right neighbour buys nothing.
///
/// Two offline devices creating the first child of one parent mint the
/// same key (#282), so `left` and `mid` hold one position and no key
/// sorts between them. Freeing `mid`'s key would hand the new block a
/// tie with `left`, where the `NodeId` tiebreak could put it *above* the
/// block the user pressed Enter on. So the tie keeps the honest answer:
/// a key above both, one slot further than asked, never behind the
/// anchor.
#[test]
fn an_insert_after_a_tied_sibling_lands_after_the_tie() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let mut ids = [NodeId::new(), NodeId::new()];
    ids.sort();
    for (i, id) in ids.iter().enumerate() {
        place(&mut ws, &hlc, *id, page, "a");
        outl_actions::block::edit_text(&mut ws, &hlc, *id, &format!("tied{i}")).expect("text");
    }

    create_after(&mut ws, &hlc, ids[0], Some("fresh")).expect("create after the first");

    assert_eq!(position_of(&ws, ids[0]), "a", "no sibling may move");
    assert_eq!(position_of(&ws, ids[1]), "a", "no sibling may move");
    assert_eq!(order(&ws, page), vec!["tied0", "tied1", "fresh"]);
}

/// Convergence, because every repair here is a `Move` in the op log and
/// nothing local: a peer replaying the same ops in the opposite order
/// must render the same sibling order.
#[test]
fn a_peer_replaying_the_repair_renders_the_same_order() {
    let (mut ws, hlc) = new_workspace();
    let page = open_or_create(&mut ws, &hlc, "p", "P", PageKind::Page).expect("page");
    let x = append_block(&mut ws, &hlc, Some(page), Some("X")).expect("X");
    append_block(&mut ws, &hlc, Some(page), Some("Y")).expect("Y");
    let left = NodeId::new();
    let right = NodeId::new();
    place(&mut ws, &hlc, left, page, "b");
    place(&mut ws, &hlc, right, page, "ba");
    for i in 1..=9 {
        create_after(&mut ws, &hlc, x, Some(&format!("n{i}"))).expect("create after X");
    }
    create_after(&mut ws, &hlc, left, Some("fresh")).expect("create after the floor-tailed pair");

    let actor = ActorId::new();
    let mut peer = Workspace::open_in_memory(actor).expect("peer workspace");
    let mut ops: Vec<LogOp> = ws.log().iter().cloned().collect();
    ops.reverse();
    for op in ops {
        peer.apply(op).expect("replay");
    }

    let page_id = outl_actions::page::find_by_slug(&peer, "p").expect("page replayed");
    assert_eq!(
        order(&ws, page),
        order(&peer, page_id),
        "two replicas must render one order"
    );
}
