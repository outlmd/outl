//! A `sync-transport` plugin is a transport, not a trusted clock.
//!
//! `PluginHost::sync_pull` is the second path in the workspace that ingests ops
//! written by someone else's clock (`outl-sync-iroh`'s `ingest_received_ops` is
//! the first). It inherited the `LogOp` shape, the `hlc.observe` call and the
//! `Workspace::apply` call, and not the skew gate — which lived on the other
//! side of a crate boundary ([issue #283](https://github.com/outlmd/outl/issues/283)).
//!
//! The cost is not "one bad op". `HlcGenerator::observe` is monotonic, so a
//! timestamp from the year 584 million raises this device's clock and *keeps* it
//! there; every op the device writes afterwards is beyond its peers' own gate
//! and gets dropped by them. The device keeps working locally and silently
//! stops syncing anything it writes, with no user-facing way back.

#![cfg(feature = "js")]

use outl_core::fractional::Fractional;
use outl_core::hlc::{self, Hlc, HlcGenerator, MAX_CLOCK_SKEW_MS};
use outl_core::id::{ActorId, NodeId};
use outl_core::op::{LogOp, Op};
use outl_core::workspace::Workspace;
use outl_plugins::{Capability, PermissionSet, PluginHost, PluginManifest};

/// A loopback sync transport whose `pull` hands back exactly `lines`.
///
/// Stands in for "a backend answered", which is the whole trust boundary: the
/// bytes are whatever the plugin says they are.
fn host_pulling(lines: &[String]) -> PluginHost {
    let quoted: Vec<String> = lines.iter().map(|l| format!("'{l}'")).collect();
    let bundle = format!(
        r#"globalThis.__outl_register({{ activate(ctx) {{
            ctx.sync.register({{ push: () => {{}}, pull: () => [{}].join('\n') }});
        }} }});"#,
        quoted.join(",")
    );
    let manifest = PluginManifest::parse(
        br#"{"id":"run.x.sync","name":"Sync","version":"1.0.0","api":"^1.0","main":"i.js",
             "capabilities":["sync-transport"]}"#,
    )
    .expect("valid manifest");
    let mut host = PluginHost::new([Capability::SyncTransport].into_iter().collect());
    host.load_plugin(
        manifest,
        &bundle,
        PermissionSet::new(vec![]),
        serde_json::Value::Null,
    )
    .expect("plugin loads");
    host
}

fn create_at(node: NodeId, actor: ActorId, physical_ms: u64) -> String {
    let op = LogOp {
        ts: Hlc::new(physical_ms, 0, actor),
        actor,
        op: Op::Create {
            node,
            parent: NodeId::root(),
            position: Fractional::first(),
        },
    };
    serde_json::to_string(&op).expect("LogOp serializes")
}

#[test]
fn sync_pull_drops_an_op_from_the_future_without_advancing_the_local_clock() {
    let them = ActorId::new();
    let now = hlc::wall_clock_ms_checked().expect("a local clock at or after the epoch");
    // A minute past the window rather than the exact `+ 1` boundary: the wall
    // clock moves between this read and the pull, which would pull a `+ 1` op
    // back *inside* the window and pass the test for the wrong reason. The
    // boundary itself is pinned where it can be pinned exactly, against a fixed
    // `now`, in `outl_core::hlc`'s own tests.
    let future_ms = now + MAX_CLOCK_SKEW_MS + 60_000;

    let in_window = NodeId::new();
    let at_the_edge = NodeId::new();
    let poisoned = NodeId::new();
    let lines = vec![
        create_at(in_window, them, now),
        // A device a full day ahead is inside the window and must still sync —
        // a gate that drops everything passes every deny test ever written.
        create_at(at_the_edge, them, now + MAX_CLOCK_SKEW_MS),
        create_at(poisoned, them, future_ms),
    ];

    let mut host = host_pulling(&lines);
    let me = ActorId::new();
    let mut ws = Workspace::open_in_memory(me).expect("in-memory workspace");
    let clock = HlcGenerator::new(me);

    let applied = host.sync_pull(&mut ws, &clock).expect("pull runs");

    assert_eq!(
        applied, 2,
        "exactly the two ops inside the window may apply"
    );
    assert!(
        ws.tree().contains(in_window),
        "an ordinary remote op was dropped"
    );
    assert!(
        ws.tree().contains(at_the_edge),
        "an op at the edge of the window was dropped; the gate is too tight"
    );
    assert!(
        !ws.tree().contains(poisoned),
        "an op past the skew window reached the tree"
    );

    let next = clock.next();
    assert!(
        next.physical_ms < future_ms,
        "an untrusted op raised the local clock to {next:?}; every op this device \
         writes from now on sorts past its peers' own skew gate and is dropped by them"
    );
}

#[test]
fn a_malformed_line_is_still_skipped_rather_than_trusted() {
    // The guard added for the future-HLC case must not become the only guard:
    // a line that is not a `LogOp` at all still has to be skipped, and the
    // valid lines around it still have to apply.
    let them = ActorId::new();
    let now = hlc::wall_clock_ms_checked().expect("a local clock at or after the epoch");
    let good = NodeId::new();
    let lines = vec![
        "not json at all".to_string(),
        create_at(good, them, now),
        "{\"ts\":\"wrong shape\"}".to_string(),
    ];

    let mut host = host_pulling(&lines);
    let me = ActorId::new();
    let mut ws = Workspace::open_in_memory(me).expect("in-memory workspace");
    let clock = HlcGenerator::new(me);

    assert_eq!(host.sync_pull(&mut ws, &clock).expect("pull runs"), 1);
    assert!(ws.tree().contains(good));
}
