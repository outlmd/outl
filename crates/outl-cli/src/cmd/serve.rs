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
//! run permanently next to a GUI you also use.

use crate::cmd::sync_supervisor;
use crate::sync_engine::{reconcile_dir, reconcile_md, ReconcileReport};
use crate::workspace_layout::{ensure_ops_dir, is_workspace_md, read_config, Paths};
use anyhow::{bail, Context, Result};
use notify::RecursiveMode;
use notify_debouncer_full::new_debouncer;
use outl_actions::SyncEngine;
use outl_core::hlc::HlcGenerator;
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;
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
pub fn run(path: &Path, once: bool, watch: bool, sync: bool) -> Result<()> {
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
        return run_sync_only(&paths, device_actor);
    }

    // Exclusive per-actor write lock. Falls back to ephemeral when
    // another process already owns the config actor.
    let (_actor_lock, actor) = outl_core::resolve_write_actor(&paths.ops, device_actor)
        .with_context(|| format!("acquiring per-actor write lock at {}", paths.ops.display()))?;
    if actor != device_actor {
        info!(
            "another outl process owns the device actor {device_actor}; serve writes under ephemeral actor {actor}"
        );
    }
    let storage = JsonlStorage::open(paths.ops.clone(), actor)?;
    let mut ws = Workspace::open_with_storage(actor, Box::new(storage), Some(paths.root.clone()))?;
    let hlc = HlcGenerator::new(actor);

    info!("starting outl serve at {}", paths.root.display());

    // Initial scan: reconcile every .md in pages/ and journals/.
    let initial = initial_scan(&mut ws, &hlc, &paths)?;
    summarize(&initial);

    if once {
        return Ok(());
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    // Peer-op arrivals from the transport, so the watcher reloads before it
    // reconciles.
    let (peer_tx, peer_rx) = channel::<()>();
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

    // File watcher with 200ms debounce.
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(Duration::from_millis(200), None, move |res| {
        let _ = tx.send(res);
    })
    .with_context(|| "creating file watcher")?;

    debouncer
        .watch(&paths.pages, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", paths.pages.display()))?;
    debouncer
        .watch(&paths.journals, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", paths.journals.display()))?;

    info!("watching pages/ and journals/ (Ctrl-C or SIGTERM to stop)");

    let engine = SyncEngine::new(paths.root.clone(), actor);
    while !shutdown.load(Ordering::SeqCst) {
        // Keeps the snapshot current on an idle daemon; correctness is the
        // second call, just before the reconcile.
        reload_if_peer_ops(&peer_rx, &engine, &mut ws);
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
                reload_if_peer_ops(&peer_rx, &engine, &mut ws);
                for p in paths_to_sync {
                    match reconcile_md(&mut ws, &hlc, &paths, &p) {
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
                break;
            }
        }
    }

    // Unconditional, not only on the signal path: the loop also exits when the
    // watcher dies, and the supervisor's other exits are a stop signal and P2P
    // being off. Without this, `join()` waits for a signal that may never come
    // and the process sits there looking alive with a dead watcher.
    shutdown.store(true, Ordering::SeqCst);
    // Drop the watcher before joining: the supervisor's shutdown releases the
    // endpoint lease, and a lease left held locks every outl process on this
    // device out of an endpoint.
    drop(debouncer);
    if let Some(handle) = supervisor {
        if handle.join().is_err() {
            warn!("sync supervisor thread panicked on the way out");
        }
    }
    info!("outl serve stopped");
    Ok(())
}

/// `--no-watch`: hold the endpoint and nothing else.
///
/// Takes no per-actor write lock (see the module doc), so it can run
/// permanently beside a GUI without pushing that GUI onto a fresh ephemeral
/// actor on every launch.
fn run_sync_only(paths: &Paths, actor: outl_core::id::ActorId) -> Result<()> {
    let shutdown = Arc::new(AtomicBool::new(false));
    // The receiver is dropped, not bound: with no watcher there is nothing to
    // reload, and a bound-but-never-drained receiver would queue one message
    // per ingested batch for as long as this runs — which is meant to be
    // weeks. Both send sites use `.ok()`, so dropping it is safe.
    let (peer_tx, _) = channel::<()>();
    let (wake_tx, wake_rx) = channel::<()>();
    sync_supervisor::install_signal_handler(Arc::clone(&shutdown), wake_tx);
    info!(
        "starting outl serve at {} (sync only, no file watcher)",
        paths.root.display()
    );
    // With no watcher, "P2P is off" leaves this process with nothing to do.
    // Returning `Ok` there would exit 0, and a process manager set to keep the
    // daemon alive would restart it into the same config forever, each restart
    // reporting success.
    if sync_supervisor::run(&paths.root, actor, peer_tx, wake_rx, shutdown)
        == sync_supervisor::SupervisorExit::P2pDisabled
    {
        bail!(
            "`--no-watch` holds the P2P endpoint and nothing else, but `[sync] transport` \
             is \"file\". Set it to \"iroh\", or drop `--no-watch` to run the file watcher."
        );
    }
    info!("outl serve stopped");
    Ok(())
}

/// Reload `ws` from the op log when the transport signals peer ops.
///
/// Drains the whole burst first: one reload covers every signal in it.
/// A failed reload is logged and left alone — the next signal retries, and
/// reconciling against the tree we already have beats not reconciling at all.
fn reload_if_peer_ops(
    peer_rx: &std::sync::mpsc::Receiver<()>,
    engine: &SyncEngine,
    ws: &mut Workspace,
) {
    if peer_rx.try_iter().count() == 0 {
        return;
    }
    match engine.reload_workspace() {
        Ok(fresh) => {
            *ws = fresh;
            info!("peer ops landed; workspace reloaded");
        }
        Err(e) => error!("reloading after peer ops failed: {e:#}"),
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
