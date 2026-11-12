//! `outl serve` — the background daemon: watch the workspace, and sync.
//!
//! Two halves, both optional, that between them are everything a machine needs
//! running in the background:
//!
//! - the **file watcher**, which reconciles external `.md` edits into the op
//!   log (`--no-watch` turns it off);
//! - the **sync supervisor**, which holds this device's iroh endpoint so
//!   paired peers converge continuously (`--no-sync` turns it off).
//!
//! The sync half used to be missing entirely, which meant a box running
//! `outl serve` under `launchd` synced with nobody and said nothing about it.
//! See [`crate::cmd::sync_supervisor`] for why it defers to a running GUI
//! rather than competing with it.
//!
//! A third, optional piece is the **embedded MCP server** (`--no-mcp` turns it
//! off): a Unix-socket listener whose sessions share this process's single
//! workspace, see [`crate::mcp`]'s `embedded` module.
//!
//! # Actor policy, and why `--no-watch` exists
//!
//! The watcher emits ops, so it needs the exclusive per-actor write lock. Any
//! process that loses the race for the device actor gets a **fresh ephemeral
//! actor** from [`outl_core::resolve_write_actor`] and its own
//! `ops-<ulid>.jsonl` — the documented multi-process contract. That is fine
//! for an occasional overlap and expensive for a daemon: `outl serve` running
//! permanently holds the device actor, so every later GUI or TUI launch mints
//! one more op-log file, forever.
//!
//! The sync half has no such problem. The transport buckets its writes by
//! `op.actor` under `OpsDirAppendLock`, never through the `JsonlStorage::append`
//! path that `ActorWriteLock` guards. So `--no-watch` resolves
//! the device actor **read-only** and takes no write lock at all — the mode to
//! run permanently next to a GUI you also use. The embedded MCP honours that:
//! it reads through a read open and takes the device actor only while an MCP
//! session is connected and has called a mutating tool, releasing it when the
//! last session closes. If a GUI holds the actor, that write fails with an
//! explicit error and no ephemeral actor is minted.

mod projection;

use crate::cmd::sync_supervisor;
use crate::mcp::{start_embedded, EmbeddedServer, SharedWs};
use crate::sync_engine::{reconcile_dir, reconcile_md, ReconcileReport};
use crate::workspace_layout::{ensure_ops_dir, is_workspace_md, read_config, Paths};
use anyhow::{bail, Context, Result};
use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use outl_actions::{FileSyncTransport, SyncTransport};
use outl_core::hlc::HlcGenerator;
use outl_core::workspace::Workspace;
use projection::{project_stale_pages, ProjectionReporter, PROJECTION_MIN_INTERVAL};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// How often the watcher loop wakes to re-check the shutdown flag when no
/// filesystem event arrives. Also the worst-case delay between SIGTERM and the
/// process exiting.
const TICK: Duration = Duration::from_secs(1);

/// Run the `serve` subcommand.
///
/// `once` reconciles every `.md` once and returns — useful for smoke tests and
/// scripting; it implies no watcher loop and no sync. `watch` and `sync` select
/// the two halves; turning both off is a usage error rather than a silent
/// no-op process.
///
/// Actor + lock policy mirrors `ws::open` and `outl-tui::open_workspace` when
/// the watcher runs: the device actor from
/// [`outl_ws::actor::resolve_device_actor`], a shared workspace lock, then a
/// per-actor write lock through [`outl_core::resolve_write_actor`]. Without the
/// watcher there is nothing to write, so the write lock is skipped — see the
/// module doc.
pub fn run(path: &Path, once: bool, watch: bool, sync: bool, mcp: bool) -> Result<()> {
    if !watch && !sync {
        bail!("--no-watch and --no-sync together leave nothing to do");
    }
    let paths = Paths::at(path.to_path_buf());
    let cfg =
        read_config(&paths).with_context(|| "workspace config missing — run `outl init` first")?;
    // The actor comes from this device's store, never from the
    // workspace — see `outl_ws::actor`.
    let device_actor = outl_ws::actor::resolve_device_actor(
        &paths,
        &cfg,
        &outl_core::device::DeviceStore::open_default(),
    )?;
    // Shared workspace lock — coexists with every other well-behaved
    // `outl` process.
    let _lock = outl_core::WorkspaceLock::acquire(&paths.root).with_context(|| {
        format!(
            "could not acquire workspace lock at {}",
            paths.root.display()
        )
    })?;
    ensure_ops_dir(&paths)?;

    if !watch {
        return run_sync_only(&paths, device_actor, mcp);
    }

    // One writing open, shared with the embedded MCP sessions (see
    // `crate::mcp::SharedWs` for why). It takes the exclusive per-actor write
    // lock, falling back to an ephemeral actor when another process owns the
    // device actor, and seeds the clock from the log.
    let wc = outl_ws::open_with(
        &paths.root,
        outl_ws::OpenOptions {
            write_snapshots: true,
            writes: true,
            device_actor_only: false,
        },
    )
    .with_context(|| format!("opening the workspace at {}", paths.root.display()))?;
    let actor = wc.actor;
    if wc.ephemeral_actor {
        info!(
            "another outl process owns the device actor {device_actor}; serve writes under ephemeral actor {actor}"
        );
    }
    let shared = SharedWs::with_open(wc);

    info!("starting outl serve at {}", paths.root.display());

    // Initial scan: reconcile every .md in pages/ and journals/.
    let initial = locked(&shared, |wc| {
        initial_scan(&mut wc.workspace, &wc.hlc, &paths)
    })??;
    summarize(&initial);
    // …then the other direction. `.md → tree` runs first on purpose: a
    // local hand edit becomes ops before the projection pass renders the
    // tree back out, so the two never fight over the same file.
    let mut reporter = ProjectionReporter::default();
    locked(&shared, |wc| {
        project_stale_pages(&wc.workspace, &paths.root, &mut reporter)
    })?;

    if once {
        return Ok(());
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    // Peer-op arrivals from the transport, folded into `shared.peer_dirty`, so
    // the watcher (and any MCP session) reloads before it reconciles.
    let (peer_tx, peer_rx) = channel::<()>();
    forward_peer_signals(peer_rx, Arc::clone(&shared));
    // Wakes the supervisor's own sleep so a signal doesn't have to wait out
    // the rest of a 30s lease retry.
    let (wake_tx, wake_rx) = channel::<()>();
    sync_supervisor::install_signal_handler(Arc::clone(&shutdown), wake_tx);

    let supervisor = if sync {
        let root = paths.root.clone();
        let shutdown = Arc::clone(&shutdown);
        let peer_tx = peer_tx.clone();
        Some(std::thread::spawn(move || {
            // `P2pDisabled` is survivable here, unlike under `--no-watch`:
            // the watcher below still has work to do.
            if sync_supervisor::run(&root, actor, peer_tx, wake_rx, shutdown)
                == sync_supervisor::SupervisorExit::P2pDisabled
            {
                warn!("P2P sync is off in config; this process only watches `.md` files");
            }
        }))
    } else {
        info!("P2P sync disabled (--no-sync); ops converge through the shared ops/ dir only");
        None
    };

    let embedded = if mcp {
        start_mcp(&paths.root, actor, &shared, &peer_tx, &shutdown, false)
    } else {
        info!("embedded MCP server disabled (--no-mcp)");
        None
    };

    // File watcher with 200ms debounce.
    let (tx, rx) = channel();
    let watcher = new_debouncer(Duration::from_millis(200), None, move |res| {
        let _ = tx.send(res);
    })
    .with_context(|| "creating file watcher")
    .and_then(|mut debouncer| {
        debouncer
            .watch(&paths.pages, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", paths.pages.display()))?;
        debouncer
            .watch(&paths.journals, RecursiveMode::NonRecursive)
            .with_context(|| format!("watching {}", paths.journals.display()))?;
        Ok(debouncer)
    });
    let debouncer = match watcher {
        Ok(d) => d,
        Err(e) => {
            // Same teardown as the loop's exit: the socket and the lease
            // must not outlive a failed start.
            stop_background(&shutdown, supervisor, embedded);
            return Err(e);
        }
    };

    info!("watching pages/ and journals/ (Ctrl-C or SIGTERM to stop)");

    // The one loop exit that is a failure. Without it the teardown below
    // cannot tell a dead watcher from a clean SIGTERM, so the process exits 0
    // and `Restart=on-failure` never brings the daemon back.
    let mut watcher_died = false;
    // Peer ops that landed and whose pages have not been projected yet.
    // A burst of batches sets it many times and costs one sweep.
    let mut projection_due = false;
    let mut last_projection = std::time::Instant::now();
    while !shutdown.load(Ordering::SeqCst) {
        // Keeps the snapshot current on an idle daemon; correctness is the
        // second call, just before the reconcile.
        projection_due |= reload_if_peer_ops(&shared, &paths.root);
        // The whole reason this daemon now has a `tree → .md` half: ops
        // that arrive by sync change the tree and nothing else, so the
        // `.md` stays wrong until a human opens the page. Throttled
        // because the sweep renders every page, and a sync burst would
        // otherwise render the graph once per batch.
        if projection_due && last_projection.elapsed() >= PROJECTION_MIN_INTERVAL {
            shared.run_locked(|wc| project_stale_pages(&wc.workspace, &paths.root, &mut reporter));
            projection_due = false;
            last_projection = std::time::Instant::now();
        }
        match rx.recv_timeout(TICK) {
            Ok(Ok(events)) => {
                let mut paths_to_sync: std::collections::BTreeSet<std::path::PathBuf> =
                    Default::default();
                for ev in events {
                    for p in &ev.event.paths {
                        if is_workspace_md(&paths, p) {
                            paths_to_sync.insert(p.clone());
                        }
                    }
                }
                // Again, right before reconciling: the call above ran before
                // a block of up to TICK, and peer ops that landed during it
                // would otherwise be diffed against the pre-peer tree.
                projection_due |= reload_if_peer_ops(&shared, &paths.root);
                shared.run_locked(|wc| {
                    for p in paths_to_sync {
                        reconcile_one(wc, &paths, &p);
                    }
                });
            }
            Ok(Err(errs)) => {
                for e in errs {
                    error!("watcher error: {e}");
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            // The debouncer's sender is gone, so the watcher half is dead.
            Err(RecvTimeoutError::Disconnected) => {
                error!("file watcher stopped delivering events; shutting down");
                watcher_died = true;
                break;
            }
        }
    }

    // Drop the watcher before joining: the supervisor's shutdown releases the
    // endpoint lease, and a lease left held locks every outl process on this
    // device out of an endpoint.
    drop(debouncer);
    stop_background(&shutdown, supervisor, embedded);
    if watcher_died {
        bail!("the file watcher stopped delivering events");
    }
    info!("outl serve stopped");
    Ok(())
}

/// Reconcile one edited `.md` into the shared workspace and log the outcome.
fn reconcile_one(wc: &mut outl_ws::WsCtx, paths: &Paths, p: &Path) {
    match reconcile_md(&mut wc.workspace, &wc.hlc, paths, p) {
        Ok(r) if r.ops_applied > 0 || r.orphans > 0 => {
            info!(
                "{} → {} ops, {} orphans, sidecar {}",
                r.md_path.display(),
                r.ops_applied,
                r.orphans,
                if r.created_sidecar {
                    "created"
                } else {
                    "updated"
                }
            );
        }
        Ok(_) => {}
        Err(e) => error!("reconcile failed for {}: {e:#}", p.display()),
    }
}

/// Run `f` on the shared workspace, or fail when the slot is empty (it never
/// is on the watcher path, which opens it before anything else).
fn locked<R>(shared: &SharedWs, f: impl FnOnce(&mut outl_ws::WsCtx) -> R) -> Result<R> {
    shared
        .run_locked(f)
        .ok_or_else(|| anyhow::anyhow!("the shared workspace is not open"))
}

/// Fold the supervisor's "peer ops landed" channel into the shared dirty flag.
/// Ends when every sender is gone.
fn forward_peer_signals(rx: std::sync::mpsc::Receiver<()>, shared: Arc<SharedWs>) {
    let _ = std::thread::Builder::new()
        .name("outl-serve-peer-signal".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                shared.peer_dirty.store(true, Ordering::Release);
            }
        });
}

/// What `start_mcp` brought up, torn down together.
struct EmbeddedMcp {
    server: EmbeddedServer,
    poller: FileSyncTransport,
}

/// Start the embedded MCP server plus the local file poller that flags
/// `peer_dirty` when another process on this machine writes to `ops/`.
/// `None` when the socket could not be set up; `serve` carries on without it.
///
/// No iroh endpoint is bound here: the sync supervisor owns it.
fn start_mcp(
    root: &Path,
    actor: outl_core::id::ActorId,
    shared: &Arc<SharedWs>,
    peer_tx: &std::sync::mpsc::Sender<()>,
    shutdown: &Arc<AtomicBool>,
    warm: bool,
) -> Option<EmbeddedMcp> {
    let server = start_embedded(root, Arc::clone(shared), Arc::clone(shutdown), warm)?;
    let poller = FileSyncTransport::default();
    poller.start(root.to_path_buf(), actor, peer_tx.clone());
    Some(EmbeddedMcp { server, poller })
}

/// Stop everything `serve` started in the background and wait for it.
///
/// Unconditional on the way out, not only on a signal: the main loop also
/// exits when the watcher dies, and the supervisor's other exits are a stop
/// signal and P2P being off. Without it `join()` waits for a signal that may
/// never come. Joining the MCP accept loop is what removes the socket file.
fn stop_background(
    shutdown: &AtomicBool,
    supervisor: Option<std::thread::JoinHandle<()>>,
    embedded: Option<EmbeddedMcp>,
) {
    shutdown.store(true, Ordering::SeqCst);
    if let Some(handle) = supervisor {
        if handle.join().is_err() {
            warn!("sync supervisor thread panicked on the way out");
        }
    }
    if let Some(mcp) = embedded {
        mcp.poller.shutdown();
        mcp.server.join();
    }
}

/// `--no-watch`: hold the endpoint and nothing else.
///
/// Takes no per-actor write lock (see the module doc), so it can run
/// permanently beside a GUI without pushing that GUI onto a fresh ephemeral
/// actor on every launch. The embedded MCP, when on, warms and serves reads
/// from a read open, and takes the device actor only while a session may be
/// writing (released when the last session closes). If another process holds
/// the actor, a mutating tool errors; nothing is minted.
fn run_sync_only(paths: &Paths, actor: outl_core::id::ActorId, mcp: bool) -> Result<()> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shared = SharedWs::on_demand();
    let (peer_tx, peer_rx) = channel::<()>();
    let (wake_tx, wake_rx) = channel::<()>();
    sync_supervisor::install_signal_handler(Arc::clone(&shutdown), wake_tx);
    info!(
        "starting outl serve at {} (sync only, no file watcher)",
        paths.root.display()
    );
    let embedded = if mcp {
        start_mcp(&paths.root, actor, &shared, &peer_tx, &shutdown, true)
    } else {
        info!("embedded MCP server disabled (--no-mcp)");
        None
    };
    if embedded.is_some() {
        forward_peer_signals(peer_rx, Arc::clone(&shared));
    } else {
        // Dropped, not bound: with no watcher and no MCP there is nothing to
        // reload, and a bound-but-never-drained receiver would queue one
        // message per ingested batch for as long as this runs — which is meant
        // to be weeks. Both send sites use `.ok()`, so dropping it is safe.
        drop(peer_rx);
    }
    // With no watcher, "P2P is off" leaves this process with nothing to do.
    // Returning `Ok` there would exit 0, and a process manager set to keep the
    // daemon alive would restart it into the same config forever, each restart
    // reporting success.
    let exit = sync_supervisor::run(&paths.root, actor, peer_tx, wake_rx, Arc::clone(&shutdown));
    stop_background(&shutdown, None, embedded);
    if exit == sync_supervisor::SupervisorExit::P2pDisabled {
        bail!(
            "`--no-watch` holds the P2P endpoint and nothing else, but `[sync] transport` \
             is \"file\". Set it to \"iroh\", or drop `--no-watch` to run the file watcher."
        );
    }
    info!("outl serve stopped");
    Ok(())
}

/// Reload the shared workspace when the op log changed, and report whether the
/// tree moved since the last call — which is what tells the caller a
/// `tree → .md` sweep now has something to do.
///
/// A session may have done the reload first; the shared `moved` fact keeps the
/// sweep from being skipped in that case. A failed reload is logged and left
/// alone: reconciling against the tree we already have beats not reconciling.
fn reload_if_peer_ops(shared: &SharedWs, root: &Path) -> bool {
    match shared.refresh(root) {
        Ok(_) => {
            let moved = shared.take_moved();
            if moved {
                info!("peer ops landed; workspace reloaded");
            }
            moved
        }
        Err(e) => {
            error!("reloading after peer ops failed: {}", e.message);
            false
        }
    }
}

fn initial_scan(
    ws: &mut Workspace,
    hlc: &HlcGenerator,
    paths: &Paths,
) -> Result<Vec<ReconcileReport>> {
    let mut all = Vec::new();
    for dir in [&paths.pages, &paths.journals] {
        let mut reports = reconcile_dir(ws, hlc, paths, dir)?;
        all.append(&mut reports);
    }
    Ok(all)
}

fn summarize(reports: &[ReconcileReport]) {
    let mut ops = 0usize;
    let mut orphans = 0usize;
    let mut created = 0usize;
    for r in reports {
        ops += r.ops_applied;
        orphans += r.orphans;
        if r.created_sidecar {
            created += 1;
        }
    }
    info!(
        "initial scan: {} files, {} ops applied, {} orphans, {} new sidecars",
        reports.len(),
        ops,
        orphans,
        created,
    );
}
