//! Delivering one set of ops in many orders.
//!
//! Shared by the four `trash_restore_*` suites, which each own one of
//! the CRDT invariants restore has to keep. They share this because the
//! harness is the part that must not differ between them: two suites
//! with their own "replay the union" would be two definitions of what
//! convergence means, and the one that drifts is the one that stops
//! permuting.
//!
//! `revive_convergence.rs` is the sibling suite for `page::revive` and
//! takes this module too. It used to carry its own copy, whose
//! `fingerprint` compared `(id, parent, text)` where [`shape`] compared
//! `(parent, position)` — so adopting this one would have **dropped**
//! cross-order text convergence from that file. [`shape`] carries the
//! text now, which is why the swap is safe and why the other four
//! suites got stronger for free.
#![allow(dead_code)]

use std::collections::BTreeMap;

use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::ActorId;
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;

/// A fresh in-memory replica with its own actor.
pub fn replica() -> (Workspace, HlcGenerator) {
    let actor = ActorId::new();
    (
        Workspace::open_in_memory(actor).expect("in-memory workspace"),
        HlcGenerator::new(actor),
    )
}

/// Every op in a workspace's log, in log order.
pub fn ops(ws: &Workspace) -> Vec<LogOp> {
    ws.log().iter().cloned().collect()
}

/// A fresh replica with `order` delivered one op at a time.
pub fn replay(order: &[LogOp]) -> Workspace {
    let actor = ActorId::new();
    let mut ws = Workspace::open_in_memory(actor).expect("in-memory workspace");
    for op in order {
        ws.apply(op.clone()).expect("apply");
    }
    ws
}

/// Replay `prefix` onto a fresh replica, run `f` on it, and hand back
/// **only** the ops `f` produced.
///
/// The shape every concurrency scenario needs: a causal prefix both
/// devices saw, then one device's independent decision. The generator
/// observes the prefix's newest HLC first, so the new ops sort after it
/// — a branch whose clock started at zero would be ordered *before* its
/// own cause and would test reordering rather than concurrency.
pub fn branch(prefix: &[LogOp], f: impl FnOnce(&mut Workspace, &HlcGenerator)) -> Vec<LogOp> {
    let mut ws = replay(prefix);
    let hlc = HlcGenerator::new(ActorId::new());
    if let Some(last) = prefix.last() {
        hlc.observe(last.ts);
    }
    f(&mut ws, &hlc);
    ops(&ws)[prefix.len()..].to_vec()
}

/// The whole materialized tree as `node -> (parent, position, text)`,
/// in a comparable shape.
///
/// **Positions** are included deliberately: two replicas that agree
/// about parents and disagree about order render different `.md`, so a
/// parent-only comparison would pass on a real divergence.
///
/// **Text** is included for the same reason one level down. It is not
/// tree state — Yrs merges a block's string, the tree does not — but a
/// suite that permutes `Op::Edit` deliveries and compares only structure
/// cannot see a text divergence at all. `revive_convergence.rs`'s own
/// harness compared it before that file adopted this module, so leaving
/// it out here would have silently deleted that coverage; q4 / q4b in
/// that file are exactly the scenarios that need it.
pub type Shape = BTreeMap<String, (String, String, Option<String>)>;

pub fn shape(ws: &Workspace) -> Shape {
    ws.tree()
        .iter_nodes()
        .map(|(node, parent, pos)| {
            (
                node.to_string(),
                (parent.to_string(), format!("{pos:?}"), ws.block_text(node)),
            )
        })
        .collect()
}

/// Every op in the log, keyed by HLC and sorted, so "is this op still
/// there" is a question about identity rather than about order.
/// Invariant 5 is asked with this.
pub fn log_ids(ws: &Workspace) -> Vec<Hlc> {
    let mut ids: Vec<Hlc> = ws.log().iter().map(|o| o.ts).collect();
    ids.sort();
    ids
}

pub fn permutations<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let head = rest.remove(i);
        for mut tail in permutations(&rest) {
            let mut one = vec![head.clone()];
            one.append(&mut tail);
            out.push(one);
        }
    }
    out
}

/// How many shuffled orders [`orders`] adds past seven ops.
const SHUFFLES: usize = 300;

/// Delivery orders to try: the full factorial up to seven ops, and past
/// that the identity, the reverse, every rotation and [`SHUFFLES`]
/// deterministic shuffles.
///
/// The cap is not a confidence statement — 8! replays is already slower
/// than the whole suite's budget. The shuffles are seeded, so a failure
/// reproduces.
pub fn orders<T: Clone>(items: &[T]) -> Vec<Vec<T>> {
    if items.len() <= 7 {
        return permutations(items);
    }
    let mut out: Vec<Vec<T>> = vec![items.to_vec()];
    let mut rev = items.to_vec();
    rev.reverse();
    out.push(rev);
    for k in 1..items.len() {
        let mut rot = items[k..].to_vec();
        rot.extend_from_slice(&items[..k]);
        out.push(rot);
    }
    let mut state: u64 = 0x2545_F491_4F6C_DD1D;
    for _ in 0..SHUFFLES {
        let mut one = items.to_vec();
        for i in (1..one.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let j = (state % (i as u64 + 1)) as usize;
            one.swap(i, j);
        }
        out.push(one);
    }
    out
}

/// Replay `union` in every order [`orders`] produces and assert one
/// materialized tree and one op set.
///
/// Returns the agreed shape, so a caller can go on to assert *what*
/// they converged on rather than only that they agreed.
pub fn assert_converges(label: &str, union: &[LogOp]) -> Shape {
    let all = orders(union);
    let baseline_ws = replay(&all[0]);
    let baseline = shape(&baseline_ws);
    let baseline_log = log_ids(&baseline_ws);
    for (i, order) in all.iter().enumerate() {
        let ws = replay(order);
        assert_eq!(
            shape(&ws),
            baseline,
            "{label}: order #{i} diverged\n  order = {:?}",
            order.iter().map(describe).collect::<Vec<_>>()
        );
        assert_eq!(
            log_ids(&ws),
            baseline_log,
            "{label}: order #{i} lost or duplicated an op"
        );
    }
    eprintln!(
        "{label}: {} orders, one tree ({} nodes, {} ops)",
        all.len(),
        baseline.len(),
        baseline_log.len()
    );
    baseline
}

/// One op, short enough to read in an assertion message.
pub fn describe(op: &LogOp) -> String {
    match &op.op {
        Op::Create { node, parent, .. } => format!("Create({node}->{parent})"),
        Op::Move {
            node, new_parent, ..
        } => format!("Move({node}->{new_parent})"),
        Op::Edit { node, .. } => format!("Edit({node})"),
        Op::SetProp { node, key, .. } => format!("SetProp({node},{key})"),
        other => format!("{other:?}"),
    }
}
