//! The two findings for an op the materialized tree does not reflect,
//! which are two different defects with opposite remediations.
//!
//! A node the log never `Create`d was not built here: the op that
//! would have placed it did not arrive, the peer that wrote the op we
//! *do* hold may still have it, and naming that actor is the whole
//! remediation. A node the log did `Create` and the tree still lacks
//! is the replay rejecting it, which no sync pass fixes.
//!
//! Split out of `tree.rs` when that file crossed the 600-line ratchet.
//! The seam is the subject: these are the only tests here that write a
//! raw `LogOp` behind `Workspace`'s back, because the state under test
//! is one no API can reach.

use super::*;

/// Append a raw `LogOp` to this device's op log, bypassing `Workspace`.
///
/// That is the point: the state under test is an op naming a node the
/// log never created, and no API can produce it — it arrives from a peer
/// whose `Create` did not. The reference workspace in [issue #301] holds
/// five, all `SetCollapsed`, one op each, from one actor, inside a
/// minute.
///
/// [issue #301]: https://github.com/outlmd/outl/issues/301
fn append_raw_op(ops: &Path, actor: outl_core::id::ActorId, op: outl_core::op::Op) {
    use std::io::Write;
    let log = ops.join(format!("ops-{actor}.jsonl"));
    let ts = outl_core::hlc::Hlc {
        physical_ms: 1_782_844_005_086,
        logical: 0,
        actor,
    };
    let line =
        serde_json::to_string(&outl_core::op::LogOp { ts, actor, op }).expect("a LogOp serializes");
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .expect("open the actor log");
    writeln!(f, "{line}").expect("append the raw op");
    // The index sidecars now describe a shorter file than the one on
    // disk. They are a boot cache and a stale one rebuilds, which is
    // exactly what the next open must do to see this op at all.
    for ext in [".idx", ".nodes.idx"] {
        let _ = std::fs::remove_file(ops.join(format!(".ops-{actor}{ext}")));
    }
}

/// The finding used to say "5 node id(s)" and stop, so the user could
/// not tell harmless residue from a subtree that never arrived. It must
/// name the actor and the op, because those are what make the next step
/// possible: ask *that* peer for the ops it never shipped.
#[test]
fn an_unmaterialized_node_names_the_actor_and_the_op_that_touched_it() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["a block"]);

    let actor = {
        let ctx = crate::ws::open(&root).expect("open");
        ctx.actor
    };
    let ghost = NodeId::new();
    append_raw_op(
        &paths.ops,
        actor,
        outl_core::op::Op::SetCollapsed {
            node: ghost,
            value: true,
            old_value: false,
        },
    );

    let report = collect(&root, false).expect("doctor runs");
    let msgs = messages(&report);
    assert!(
        has(&report, &ghost.to_string()),
        "the node id must still be named, got: {msgs:#?}"
    );
    assert!(
        has(&report, "SetCollapsed"),
        "the op that touched it must be named, got: {msgs:#?}"
    );
    assert!(
        has(&report, &actor.to_string()),
        "the actor whose log carries it must be named, got: {msgs:#?}"
    );
}

/// A `Move` is not evidence that the log placed a node.
///
/// `Tree::do_op` on a `Move` whose node it has never seen is a no-op on
/// the tree, so a node carrying a `Move` and no `Create` is absent for
/// exactly the reason this split exists to report: the `Create` never
/// arrived. Counting `Move` as placement sent that node down the
/// "please report it" branch with no actor named — the finding the
/// split was built to replace.
#[test]
fn a_move_without_its_create_is_an_unarrived_op_not_a_rejected_one() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["a block"]);

    let actor = {
        let ctx = crate::ws::open(&root).expect("open");
        ctx.actor
    };
    let ghost = NodeId::new();
    append_raw_op(
        &paths.ops,
        actor,
        outl_core::op::Op::Move {
            node: ghost,
            new_parent: outl_core::id::NodeId::root(),
            position: outl_core::fractional::Fractional::between(None, None),
            old_parent: outl_core::id::NodeId::root(),
            old_position: outl_core::fractional::Fractional::between(None, None),
        },
    );

    let report = collect(&root, false).expect("doctor runs");
    let msgs = messages(&report);
    assert!(
        has(&report, "never arrived"),
        "a Move with no Create is an unarrived op, got: {msgs:#?}"
    );
    assert!(
        has(&report, &actor.to_string()),
        "the actor to ask for the missing Create must be named, got: {msgs:#?}"
    );
    assert!(
        !has(&report, "please report it"),
        "this is not a rejected replay, got: {msgs:#?}"
    );
    assert!(
        has(&report, &ghost.to_string()),
        "the node id must be named, got: {msgs:#?}"
    );
}

/// The two states are not the same defect and must not read alike.
///
/// A node with no `Create` anywhere in the merged log is
/// evidence that the ops which built it never arrived — the peer may
/// still have them, so the line says to go and get them. A node the log
/// *did* place and the tree still lacks is a different, worse thing, and
/// conflating them is why the old single line could not be acted on.
#[test]
fn a_node_the_log_never_placed_is_reported_as_ops_that_did_not_arrive() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["a block"]);

    let actor = {
        let ctx = crate::ws::open(&root).expect("open");
        ctx.actor
    };
    append_raw_op(
        &paths.ops,
        actor,
        outl_core::op::Op::SetCollapsed {
            node: NodeId::new(),
            value: true,
            old_value: false,
        },
    );

    let report = collect(&root, false).expect("doctor runs");
    let msgs = messages(&report);
    assert!(
        has(&report, "never arrived"),
        "a node the log never placed must say the creating ops are missing, got: {msgs:#?}"
    );
    assert!(
        has(&report, "outl sync"),
        "the line must say what to do about it, got: {msgs:#?}"
    );
}

/// `--repair` writes projections and caches, **never** the op log
/// (`outl-cli/CLAUDE.md` invariant 2). Issue #301 floats "drop the
/// orphaned op" as a possible answer; it is not one here, because the
/// log is append-only and the op is the only surviving evidence that
/// something is missing. So the repair pass must leave `ops/` untouched
/// and the finding must stay a finding.
#[test]
fn repair_never_drops_an_orphaned_op() {
    let (_dir, root, paths) = fresh();
    seed_page(&root, "notes", &["a block"]);

    let actor = {
        let ctx = crate::ws::open(&root).expect("open");
        ctx.actor
    };
    append_raw_op(
        &paths.ops,
        actor,
        outl_core::op::Op::SetCollapsed {
            node: NodeId::new(),
            value: true,
            old_value: false,
        },
    );
    let log = paths.ops.join(format!("ops-{actor}.jsonl"));
    let before = std::fs::read(&log).expect("read the log");

    let report = collect(&root, true).expect("doctor repairs");
    assert_eq!(
        std::fs::read(&log).expect("read the log"),
        before,
        "--repair must not rewrite the op log"
    );
    assert!(
        has(&report, "never arrived"),
        "the finding survives a repair pass, got: {:#?}",
        messages(&report)
    );
}
