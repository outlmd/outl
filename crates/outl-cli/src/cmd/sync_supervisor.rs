//! The P2P half of `outl serve`: hold this device's iroh endpoint, deferentially.
//!
//! Continuous sync already exists. Once **any** process holds this device's
//! iroh endpoint, [`outl_sync_iroh`]'s catch-up loop and gossip converge with
//! every paired peer on their own. What was missing is a headless process
//! willing to hold that endpoint, so a machine with no GUI open still syncs.
//! `outl serve` was already the long-lived background process — it just never
//! called [`outl_sync_iroh::build_default_transport`], so a box running it
//! under `launchd` synced with nobody, silently.
//!
//! # Why it defers instead of winning
//!
//! One endpoint per device identity, elected not assigned. A daemon that
//! simply took the lease would hold it against the desktop GUI and the TUI for
//! as long as it ran, permanently pushing both into the degraded mode where
//! the sync indicator never turns green and Refresh cannot force a pass.
//!
//! So the supervisor **retries** rather than competes: it asks for the lease
//! every [`LEASE_RETRY`], and a refusal is a normal state, not a failure. An
//! open GUI keeps the endpoint it already has; the supervisor takes over the
//! moment that GUI exits, and hands it back the next time one wins the race.
//! Nobody who was relying on "the GUI holds the endpoint" loses it.
//!
//! It also stands down when no devices are paired: holding the endpoint to
//! sync with nobody only denies it to a GUI that could be using it to pair.
//!
//! # Why it re-reads `peers.json`
//!
//! [`outl_sync_iroh::PeersStore`] is read once, at transport build. A device
//! paired from the GUI after the supervisor started would otherwise never be
//! synced with, while the daemon reported itself perfectly healthy. So the run
//! loop watches the file's mtime and cycles the transport when it moves.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use outl_actions::SyncTransport;
use outl_core::id::ActorId;
use outl_sync_iroh::{LeaseDenied, TransportOutcome};
use tracing::{debug, error, info, warn};

/// How long to wait before asking for the endpoint lease again.
///
/// Only ever paid while something else legitimately holds the endpoint (a GUI,
/// a TUI, `outl mcp serve`) or while there is nothing to sync with, so the cost
/// of a longer wait is "the supervisor takes up to this long to notice the GUI
/// closed". 30s keeps that unnoticeable without polling the lease file hard.
const LEASE_RETRY: Duration = Duration::from_secs(30);

/// How often the run loop wakes to re-check shutdown and the peer store, when
/// no peer traffic arrives to wake it sooner.
const TICK: Duration = Duration::from_secs(1);

/// Retry interval while reclaiming the endpoint we just released ourselves.
///
/// [`SyncTransport::shutdown`] only *signals*: the endpoint closes, and the
/// lease with it, on the transport's own thread afterwards. So the re-acquire
/// that follows our own teardown races that teardown, and losing that race is
/// us losing to us — worth 500ms and another try, not 30s of standing down.
const RECLAIM_RETRY: Duration = Duration::from_millis(500);

/// How long a refusal still counts as "that is probably our own teardown".
///
/// Past this, a GUI really did take the endpoint during the gap and the normal
/// stand-down applies. Bounded on purpose: without it, a genuine handover would
/// keep this loop polling at [`RECLAIM_RETRY`] forever.
const RECLAIM_WINDOW: Duration = Duration::from_secs(10);

/// Run the deferential lease loop until `shutdown` is set.
///
/// `peer_ready` is the channel the transport signals peer-op arrivals on; the
/// caller owns it so the watcher half of `outl serve` can react to the same
/// signal. `shutdown` is shared with the caller's signal handler.
///
/// Never returns an error: a supervisor that exits on a transient failure is a
/// supervisor a process manager restarts into the same failure. Every problem
/// it can hit is logged and retried, except "P2P is off in config", which it
/// reports once and then stops trying.
pub fn run(
    workspace_root: &Path,
    actor: ActorId,
    peer_ready: Sender<()>,
    wake: Receiver<()>,
    shutdown: Arc<AtomicBool>,
) -> SupervisorExit {
    let peers_file = workspace_root.join(".outl").join("peers.json");
    // Say each stand-down reason once, not every LEASE_RETRY: a supervisor
    // that logs the same line twice a minute for a week is a log nobody reads.
    let mut announced = None;
    // Set when we tear the transport down ourselves; until it expires, a
    // refusal is read as our own teardown rather than as another process.
    let mut reclaim_until: Option<Instant> = None;

    while !shutdown.load(Ordering::SeqCst) {
        // Snapshot BEFORE the build, because the build is what reads
        // `peers.json`. Taken after it, a write landing between the two would
        // become the new baseline and never be noticed again.
        let peers_baseline = peers_mtime(&peers_file);
        let retry_in = match outl_sync_iroh::build_default_transport(workspace_root) {
            Ok(TransportOutcome::Disabled) => {
                info!(
                    "`[sync] transport` is \"file\"; no P2P endpoint to hold. \
                     Ops converge through the shared ops/ dir instead."
                );
                return SupervisorExit::P2pDisabled;
            }
            Ok(TransportOutcome::Ready(transport)) if transport.peers().is_empty() => {
                announce_once(&mut announced, StandDown::NoPeers, || {
                    info!(
                        "no paired devices yet; standing down so a GUI can use the endpoint \
                         to pair. Run `outl peer pair` to add one."
                    );
                });
                // Drop before sleeping — holding the lease here would deny it
                // to the very pairing flow that fixes this state.
                drop(transport);
                reclaim_until = None;
                LEASE_RETRY
            }
            Ok(TransportOutcome::Ready(transport)) => {
                // No `reclaim_until` reset here: this arm either returns or
                // sets its own deadline below, so clearing it would be dead.
                announced = None;
                info!(
                    "holding the device endpoint; syncing with {} paired device(s)",
                    transport.peers().len()
                );
                transport.start(workspace_root.to_path_buf(), actor, peer_ready.clone());
                let reason = run_until_interrupted(&wake, &shutdown, &peers_file, peers_baseline);
                transport.shutdown();
                match reason {
                    StopReason::Shutdown => return SupervisorExit::Stopped,
                    StopReason::PeersChanged => {
                        info!("peers.json changed; rebuilding the transport around it");
                        // `shutdown()` signals; the endpoint and its lease go
                        // down on the transport's own thread after this
                        // returns. Give that a moment before asking again.
                        reclaim_until = Some(Instant::now() + RECLAIM_WINDOW);
                        RECLAIM_RETRY
                    }
                }
            }
            Ok(TransportOutcome::EndpointBusy(LeaseDenied::HeldByAnotherProcess))
                if reclaim_until.is_some_and(|deadline| Instant::now() < deadline) =>
            {
                // Us, still letting go. Saying "another outl process holds the
                // endpoint" here would accuse a process that does not exist.
                debug!("endpoint still held by our own teardown; retrying shortly");
                RECLAIM_RETRY
            }
            Ok(TransportOutcome::EndpointBusy(LeaseDenied::HeldByAnotherProcess)) => {
                reclaim_until = None;
                announce_once(&mut announced, StandDown::Held, || {
                    info!(
                        "another outl process on this device holds the endpoint (a GUI, a TUI, \
                         `outl mcp serve`, or another `outl serve`) and is syncing. \
                         Waiting for it to exit."
                    );
                });
                LEASE_RETRY
            }
            Ok(TransportOutcome::EndpointBusy(denied)) => {
                reclaim_until = None;
                announce_once(&mut announced, StandDown::Unusable, || {
                    warn!("cannot arbitrate the endpoint on this device: {denied}. Retrying.");
                });
                LEASE_RETRY
            }
            // A transient failure (an unreadable peers.json caught mid-write, a
            // momentarily absent device dir) must not kill the supervisor.
            Err(e) => {
                reclaim_until = None;
                announce_once(&mut announced, StandDown::BuildFailed, || {
                    warn!("could not build the sync transport: {e:#}. Retrying.");
                });
                LEASE_RETRY
            }
        };
        if sleep_until_shutdown(&wake, &shutdown, retry_in) {
            return SupervisorExit::Stopped;
        }
    }
    SupervisorExit::Stopped
}

/// Why [`run`] returned.
#[derive(Debug, PartialEq, Eq)]
pub enum SupervisorExit {
    /// A stop signal arrived, or the caller's channels went away.
    Stopped,
    /// `[sync] transport` is not `iroh`, so there is no endpoint to hold and
    /// never will be while this config stands. The caller decides whether that
    /// is fine (the watcher still has work) or fatal (`--no-watch` has none).
    P2pDisabled,
}

/// Why [`run_until_interrupted`] returned.
enum StopReason {
    /// A signal arrived; the supervisor should exit.
    Shutdown,
    /// `peers.json` changed on disk, so the running transport's peer list is
    /// stale and the transport has to be rebuilt around the new one.
    PeersChanged,
}

/// Which stand-down message has already been logged, so the retry loop says
/// each one once rather than on every pass.
#[derive(PartialEq, Eq)]
enum StandDown {
    NoPeers,
    Held,
    Unusable,
    BuildFailed,
}

fn announce_once(state: &mut Option<StandDown>, reason: StandDown, log: impl FnOnce()) {
    if state.as_ref() != Some(&reason) {
        log();
        *state = Some(reason);
    }
}

/// Block while the transport syncs, returning when a signal arrives or the
/// peer store changes underneath us.
fn run_until_interrupted(
    wake: &Receiver<()>,
    shutdown: &Arc<AtomicBool>,
    peers_file: &Path,
    baseline: Option<SystemTime>,
) -> StopReason {
    loop {
        match wake.recv_timeout(TICK) {
            Ok(()) => debug!("sync supervisor woken"),
            Err(RecvTimeoutError::Timeout) => {}
            // Every sender is gone, which means the caller is tearing down.
            Err(RecvTimeoutError::Disconnected) => return StopReason::Shutdown,
        }
        if shutdown.load(Ordering::SeqCst) {
            return StopReason::Shutdown;
        }
        if peers_mtime(peers_file) != baseline {
            return StopReason::PeersChanged;
        }
    }
}

/// Sleep up to `total`, returning `true` if a shutdown signal cut it short.
///
/// No polling: [`install_signal_handler`] sends on `wake` right after setting
/// the flag, so one `recv_timeout` covers the whole wait. The flag is still
/// checked up front, because a caller that arrives *after* that send already
/// missed it and must not sit out the full retry.
fn sleep_until_shutdown(wake: &Receiver<()>, shutdown: &Arc<AtomicBool>, total: Duration) -> bool {
    if shutdown.load(Ordering::SeqCst) {
        return true;
    }
    // `Disconnected` returns INSTANTLY and keeps doing so. Swallowing it turns
    // the caller's retry loop into a busy-loop that re-opens and re-locks files
    // as fast as the CPU allows. Reachable whenever the waker's owner goes away
    // without setting the flag — `install_signal_handler` bailing on a runtime
    // it could not build, for one.
    if let Err(RecvTimeoutError::Disconnected) = wake.recv_timeout(total) {
        return true;
    }
    shutdown.load(Ordering::SeqCst)
}

/// `peers.json`'s mtime, or `None` when it is absent or unreadable.
///
/// Absent and unreadable deliberately collapse to the same value: neither is a
/// *change*, and treating a transient read error as one would cycle the
/// transport for nothing.
fn peers_mtime(peers_file: &Path) -> Option<SystemTime> {
    std::fs::metadata(peers_file).ok()?.modified().ok()
}

/// Flip `shutdown` and wake both loops when SIGTERM or SIGINT arrives.
///
/// SIGTERM is the one that matters: it is what `launchd`, `systemd` and
/// `docker stop` send, and the process must release the endpoint lease on the
/// way out. A lease left held by a killed process locks **every** outl process
/// on this device out of an endpoint until it is cleared.
///
/// Runs on its own thread with its own current-thread runtime, because the
/// sync transport owns a separate multi-thread runtime on a thread of its own
/// and neither should be nested inside the other.
pub fn install_signal_handler(shutdown: Arc<AtomicBool>, waker: Sender<()>) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            // Without this the daemon still runs, it just cannot be stopped
            // politely — worth a loud line rather than a silent degradation.
            Err(e) => {
                error!("signal handler unavailable ({e}); SIGTERM will not shut down cleanly");
                return;
            }
        };
        rt.block_on(wait_for_stop_signal());
        info!("stop signal received; shutting down");
        shutdown.store(true, Ordering::SeqCst);
        let _ = waker.send(());
    });
}

#[cfg(unix)]
async fn wait_for_stop_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            error!("cannot listen for SIGTERM ({e}); falling back to Ctrl-C only");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_stop_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    /// A peer store that appears (first pairing) or disappears must cycle the
    /// transport. `PeersStore` is read once at build, so a device paired from
    /// the GUI after the daemon started is otherwise never synced with — while
    /// the daemon goes on reporting itself perfectly healthy.
    ///
    /// Create/delete rather than a rewrite on purpose: it moves the mtime from
    /// `None` to `Some` and back, which no filesystem timestamp resolution can
    /// blur into "unchanged".
    #[test]
    fn a_peer_store_appearing_cycles_the_transport() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        let (tx, rx) = channel();
        let shutdown = Arc::new(AtomicBool::new(false));

        // Baseline taken while absent, exactly as `run` takes it before the
        // build. Create the file, then poke the loop so it evaluates on its
        // first pass instead of waiting out a TICK.
        let baseline = peers_mtime(&peers);
        std::fs::write(&peers, "[]").unwrap();
        tx.send(()).unwrap();

        assert!(matches!(
            run_until_interrupted(&rx, &shutdown, &peers, baseline),
            StopReason::PeersChanged
        ));
    }

    #[test]
    fn a_peer_store_vanishing_cycles_the_transport() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        std::fs::write(&peers, "[]").unwrap();
        let (tx, rx) = channel();
        let shutdown = Arc::new(AtomicBool::new(false));

        let baseline = peers_mtime(&peers);
        std::fs::remove_file(&peers).unwrap();
        tx.send(()).unwrap();

        assert!(matches!(
            run_until_interrupted(&rx, &shutdown, &peers, baseline),
            StopReason::PeersChanged
        ));
    }

    /// A stop signal has to win over a steady peer store, or SIGTERM never
    /// reaches `transport.shutdown()` and the endpoint lease outlives the
    /// process that held it — locking every outl process on this device out of
    /// an endpoint.
    #[test]
    fn a_stop_signal_ends_the_run_loop() {
        let dir = tempfile::tempdir().unwrap();
        let peers = dir.path().join("peers.json");
        std::fs::write(&peers, "[]").unwrap();
        let (tx, rx) = channel();
        let shutdown = Arc::new(AtomicBool::new(true));
        let baseline = peers_mtime(&peers);
        tx.send(()).unwrap();

        assert!(matches!(
            run_until_interrupted(&rx, &shutdown, &peers, baseline),
            StopReason::Shutdown
        ));
    }

    /// The case the old polling loop covered by accident: the flag is already
    /// set when the sleep starts, so the waker's send has come and gone. With
    /// a bare `recv_timeout` and no up-front check this waits the full retry.
    ///
    /// `_tx` stays bound on purpose — dropping it would make the channel
    /// disconnect and the test pass for the wrong reason.
    #[test]
    fn a_shutdown_already_set_is_not_slept_through() {
        let (_tx, rx) = channel::<()>();
        let shutdown = Arc::new(AtomicBool::new(true));

        let started = Instant::now();
        assert!(sleep_until_shutdown(
            &rx,
            &shutdown,
            Duration::from_secs(30)
        ));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "an already-set flag must return at once, took {:?}",
            started.elapsed()
        );
    }

    /// A waker whose sender is gone must STOP the retry loop, not spin it.
    ///
    /// `recv_timeout` on a disconnected channel returns instantly and keeps
    /// doing so, so a `sleep` that reports "no shutdown" here turns `run`'s
    /// retry loop into a busy-loop re-opening and re-locking files at full
    /// CPU. Reachable when `install_signal_handler` cannot build its runtime:
    /// it logs, returns, and drops the sender without ever setting the flag.
    #[test]
    fn a_dead_waker_stops_the_loop_instead_of_spinning_it() {
        let (tx, rx) = channel::<()>();
        let shutdown = Arc::new(AtomicBool::new(false));
        drop(tx);

        assert!(
            sleep_until_shutdown(&rx, &shutdown, Duration::from_secs(30)),
            "a disconnected waker must end the retry loop, not report 'keep going'"
        );
    }

    /// The lease retry must not sit out its full sleep after a signal: a
    /// daemon that takes 30s to answer SIGTERM is a daemon `launchd` escalates
    /// to SIGKILL, which is exactly the un-released lease above.
    #[test]
    fn the_lease_retry_gives_up_early_on_a_signal() {
        let (tx, rx) = channel();
        let shutdown = Arc::new(AtomicBool::new(true));
        tx.send(()).unwrap();

        let started = Instant::now();
        assert!(sleep_until_shutdown(
            &rx,
            &shutdown,
            Duration::from_secs(30)
        ));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a signal must cut the retry sleep short, took {:?}",
            started.elapsed()
        );
    }

    /// Losing the endpoint to a GUI is a normal, possibly week-long state. It
    /// gets one line, not one line per retry — a log that repeats itself twice
    /// a minute is a log nobody reads, and the real events drown in it.
    #[test]
    fn each_stand_down_reason_is_announced_once() {
        let mut state = None;
        let mut logged = 0;

        for _ in 0..5 {
            announce_once(&mut state, StandDown::Held, || logged += 1);
        }
        assert_eq!(logged, 1, "the same reason must not repeat");

        announce_once(&mut state, StandDown::NoPeers, || logged += 1);
        assert_eq!(logged, 2, "a different reason must be announced");

        announce_once(&mut state, StandDown::Held, || logged += 1);
        assert_eq!(logged, 3, "returning to an earlier reason is news again");
    }

    /// Absent and unreadable collapse to the same answer on purpose: neither
    /// is a *change*, and treating a transient read error as one would cycle
    /// the transport — dropping every live peer connection — for nothing.
    #[test]
    fn an_absent_peer_store_reads_as_no_mtime() {
        let dir = tempfile::tempdir().unwrap();
        assert!(peers_mtime(&dir.path().join("peers.json")).is_none());
    }
}
