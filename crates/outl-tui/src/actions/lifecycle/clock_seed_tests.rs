//! `App::new` raises the editor's clock above the op log it was handed.
//!
//! The TUI builds its `HlcGenerator` inside `App`'s own struct
//! initializer, so this is the earliest point the generator and the
//! workspace exist together — and the only place the seeding can happen
//! for this client.
//!
//! Without it, a generator that starts from the wall clock stamps ops
//! *below* what the log already holds whenever the clock moved backwards
//! between two runs (NTP correction, stale VM, restored backup). The CRDT
//! converges either way; the price is that each late op forces the paper's
//! undo/redo window over every newer entry, on the keystroke that made it.
//!
//! A log stamped into the far future is the reproducible stand-in for the
//! backwards clock — the generator cannot tell the two apart, and only one
//! of them is testable without touching the system clock.

use crate::state::App;
use outl_core::fractional::Fractional;
use outl_core::hlc::Hlc;
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;
use tempfile::TempDir;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after epoch")
        .as_millis() as u64
}

/// An in-memory workspace whose log already holds one op stamped `ms`.
fn workspace_logged_at(actor: ActorId, ms: u64, seed: &str) -> Workspace {
    let mut ws = Workspace::open_in_memory(actor).expect("open_in_memory");
    ws.apply(LogOp {
        ts: Hlc::new(ms, 0, actor),
        actor,
        op: Op::Create {
            node: NodeId::from_seed(b"tui-clock-seed:", seed),
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    })
    .expect("apply");
    ws
}

fn app_over(ws: Workspace, actor: ActorId) -> (App, TempDir) {
    let dir = TempDir::new().expect("tempdir");
    let app = App::new(
        dir.path().to_path_buf(),
        ws,
        actor,
        crate::theme::default_theme(),
        false,
    )
    .expect("App::new");
    (app, dir)
}

/// The headline case: the editor's generator comes back above a log
/// written ahead of this machine's wall clock.
#[test]
fn app_new_seeds_the_clock_from_the_op_log() {
    let actor = ActorId::new();
    let ahead = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS / 2;
    let (app, _dir) = app_over(workspace_logged_at(actor, ahead, "ahead"), actor);

    let issued = app.hlc.next();
    assert!(
        issued.physical_ms >= ahead,
        "the TUI open issued {issued:?}, below the {ahead} already in the log"
    );
}

/// Every op this editor writes must sort **after** everything the log
/// already held. That is the property the seeding exists for, stated the
/// way the cost is actually paid: a late op is one `apply_op` has to
/// re-order against every newer entry.
///
/// Asserted against the ops this write added, not against the log's tail:
/// the tail is sorted either way, because the reorder is exactly what an
/// unseeded stamp triggers.
#[test]
fn every_op_the_editor_writes_sorts_above_what_the_log_already_held() {
    let actor = ActorId::new();
    let ahead = now_ms() + outl_core::hlc::MAX_CLOCK_SKEW_MS;
    let (mut app, _dir) = app_over(workspace_logged_at(actor, ahead, "ahead"), actor);

    let before: std::collections::HashSet<Hlc> =
        app.workspace.log().iter().map(|op| op.ts).collect();

    let hlc = app.hlc.clone();
    let page = outl_actions::open_today(&mut app.workspace, &hlc).expect("open today");
    outl_actions::append_block(&mut app.workspace, &hlc, Some(page), Some("first"))
        .expect("append block");

    let written: Vec<Hlc> = app
        .workspace
        .log()
        .iter()
        .map(|op| op.ts)
        .filter(|ts| !before.contains(ts))
        .collect();
    assert!(
        !written.is_empty(),
        "the fixture must actually write ops, or this pins nothing"
    );
    for ts in written {
        assert!(
            ts.physical_ms >= ahead,
            "the editor stamped {ts:?}, below the {ahead} already in the log — every op like \
             this forces an undo/redo window over the whole newer log"
        );
    }
}
