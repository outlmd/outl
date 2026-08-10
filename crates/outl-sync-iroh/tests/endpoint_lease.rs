//! Who gets to bind this device's one iroh endpoint, and when they give it back.
//!
//! The bug this guards (issue #220): the answer used to be hard-coded per
//! client — the GUI binds, the MCP server and the ephemeral CLI never do. On a
//! machine with no GUI (an agent driving `outl mcp serve`) that meant *nobody*
//! bound an endpoint, so the device was unreachable: its ops never left and no
//! peer's ops ever arrived, silently.
//!
//! The rule is "one live endpoint per identity", not "only the GUI", so the
//! answer is now first-process-in. These tests pin both halves — a lone process
//! gets the endpoint whatever kind of client it is, a second one is told to stay
//! off the wire — plus the half that turns a lease into issue #220 with a
//! padlock on it: a claim nobody ever releases.
//!
//! Single test file, single test: it sets `XDG_CONFIG_HOME` so
//! `outl_config::load()` reads a scratch config instead of the developer's, and
//! an env var is process-wide. The phases below deliberately run in one `#[test]`
//! rather than three, because two tests rewriting that variable in parallel
//! would each see the other's config.

use std::path::Path;

use outl_sync_iroh::{build_transport, EndpointLease, LeaseDenied, TransportOutcome};

/// Named for the bug it was written against (issue #220) and referenced by name
/// from the crate `CLAUDE.md` and the regression table in
/// `docs/iroh-internals.md`; the later phases ride inside it because
/// `XDG_CONFIG_HOME` is process-wide.
#[test]
fn one_process_binds_the_device_endpoint_and_the_next_one_is_told_to_stay_off_the_wire() {
    let home = tempfile::tempdir().expect("tempdir");
    let workspace = tempfile::tempdir().expect("tempdir");
    // Scratch config: the default `[sync] transport` is iroh, and an empty dir
    // resolves to the defaults. Without this the outcome would depend on the
    // developer's own `~/.config/outl/config.toml`.
    std::env::set_var("XDG_CONFIG_HOME", home.path());

    let identity = home.path().join("identity.key");

    first_process_in_wins(&identity, workspace.path());
    an_unstarted_transport_still_holds_it(&identity, workspace.path());
    opting_out_of_p2p_never_takes_the_lease(&identity, workspace.path(), home.path());
}

/// The kind of client is not what decides this — only who got there first.
fn first_process_in_wins(identity: &Path, workspace: &Path) {
    let first = expect_ready(identity, workspace);

    // The MCP server, a second GUI window, `outl sync`.
    assert!(
        matches!(
            build_transport(identity, workspace).expect("build"),
            TransportOutcome::EndpointBusy(LeaseDenied::HeldByAnotherProcess)
        ),
        "a second process must be refused the endpoint, not handed one that \
         would steal the first's relay route"
    );

    // The holder exits (stdin closed, window shut). The endpoint is now free —
    // this is what makes a headless MCP the device's peer when no GUI runs.
    drop(first);
    let _second = expect_ready(identity, workspace);
}

/// A transport that was built and then dropped without ever starting releases
/// the claim — and holds it for as long as it is alive.
///
/// This is the `outl sync` shape: build a transport, decide not to proceed,
/// exit. Nothing else on the device may bind while that decision is in flight,
/// and everything may bind once the process is gone.
///
/// **What this does NOT cover:** the started case, where `start()` moves the
/// lease onto the `outl-iroh-sync` thread so it is released when `run_iroh`
/// returns rather than when the client drops the transport. Reaching that path
/// means binding a real iroh endpoint (a relay round trip), which belongs with
/// the networked batteries in `tests/integration.rs`, not here. The two
/// consequences it guards are worth naming even so: a `bind()` failure must not
/// strand the claim for the life of the process (nothing on the device could
/// bind again, and the MCP server never re-asks), and a client dropping the
/// transport straight after `shutdown()` must not free the claim while the
/// thread is still inside `router.shutdown()` + `endpoint.close()`.
fn an_unstarted_transport_still_holds_it(identity: &Path, workspace: &Path) {
    let idle = expect_ready(identity, workspace);
    assert!(
        EndpointLease::try_acquire(identity).is_err(),
        "a built-but-unstarted transport is still deciding; the device's \
         endpoint is not free while it holds one"
    );

    drop(idle);
    assert!(
        EndpointLease::try_acquire(identity).is_ok(),
        "dropping the transport must release the device's endpoint claim"
    );
}

/// `[sync] transport = "file"` is the user opting out of P2P entirely, so the
/// process must not take a lease it is never going to use — that would lock out
/// a co-resident client whose own config still says iroh.
fn opting_out_of_p2p_never_takes_the_lease(identity: &Path, workspace: &Path, config_home: &Path) {
    let cfg_dir = config_home.join("outl");
    std::fs::create_dir_all(&cfg_dir).expect("config dir");
    std::fs::write(
        cfg_dir.join("config.toml"),
        "[sync]\ntransport = \"file\"\n",
    )
    .expect("write config");

    assert!(
        matches!(
            build_transport(identity, workspace).expect("build"),
            TransportOutcome::Disabled
        ),
        "`transport = \"file\"` is the explicit P2P opt-out"
    );
    assert!(
        EndpointLease::try_acquire(identity).is_ok(),
        "a process that will never bind must leave the device's endpoint free"
    );
}

fn expect_ready(identity: &Path, workspace: &Path) -> outl_sync_iroh::IrohSyncTransport {
    match build_transport(identity, workspace).expect("build") {
        TransportOutcome::Ready(t) => t,
        other => panic!("expected the endpoint to be free, got {other:?}"),
    }
}
