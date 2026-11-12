//! The MCP server embedded in `outl serve`.
//!
//! `outl serve` already holds the workspace, so a second `outl mcp serve`
//! process only cost a second replay of the op log and, when it lost the race
//! for the device actor, an `ops-<ulid>.jsonl` and a lock file left behind
//! forever. Here the daemon serves MCP itself over a Unix socket: every
//! connection is one session (see [`super::serve_session`]), all of them
//! sharing one [`super::ServerCtx`], and `outl mcp serve` becomes a bridge to
//! that socket (see [`bridge`]).
//!
//! # One writer per actor
//!
//! Two `Workspace` values appending under one actor would run independent HLC
//! generators and risk duplicate op ids. So the process holds **one**
//! [`SharedWs`] behind one mutex, and everything that writes goes through it:
//! the MCP sessions, and the file watcher's reconcile and reload when it runs.
//! This is the smallest correct refactor: the watcher keeps its loop and
//! swaps three local variables for `SharedWs::run_locked` calls, instead of
//! the MCP growing a second workspace plus a protocol to keep it coherent
//! with the watcher's. Without the watcher (`--no-watch`), the slot starts
//! empty and the first access opens it, writing under the device actor.
//!
//! # `--no-watch`: the writer is taken on demand
//!
//! `--no-watch` promises no per-actor write lock, so a daemon can run
//! permanently next to a GUI without pushing it onto an ephemeral actor. The
//! embedded MCP keeps that promise: the slot is a **read open** (warm-up and
//! every read tool), and a tool in `MUTATING` first upgrades it to a writing
//! open of the device actor only ([`SharedWs::upgrade_for_write`]). If another
//! process holds that actor the tool fails with an explicit error; nothing is
//! minted and there is no silent fallback. The lock is released again when the
//! **last connected session closes** ([`SessionGuard`]), so a GUI started
//! afterwards keeps the device actor. While any session is connected the
//! writer stays, which keeps a burst of calls from reopening twice each.
//! Switching reopens the workspace, because the lock lives inside the loaded
//! context. Everything stays serialized on the one mutex.
//!
//! The embedded server never mints an actor: if another process holds the
//! device actor it falls back to a read-only open (see
//! [`crate::ws::open_resident`]) and retries on the next peer signal.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use parking_lot::Mutex;
use tracing::{info, warn};

use super::socket;
use super::ServerCtx;
use crate::output::{codes, ApiError};
use crate::ws::{self, WsCtx};

/// The workspace `outl serve` and its embedded MCP sessions share.
pub(crate) struct SharedWs {
    ws: Mutex<Option<WsCtx>>,
    /// Raised by the sync supervisor's peer channel and the file poller:
    /// the op log changed under us, reload before the next access.
    pub(crate) peer_dirty: Arc<AtomicBool>,
    /// Raised, under the workspace lock, whenever the tree changed; the
    /// sessions' cached `WorkspaceIndex` is stale until it is cleared.
    pub(super) index_stale: AtomicBool,
    /// The slot holds a read-only open (the device actor was taken).
    read_only: AtomicBool,
    /// The tree moved (open or reload) and nobody has projected it to `.md`
    /// yet. A session can consume the reload before the watcher sees the
    /// signal, so the watcher reads the fact here rather than from who reloaded.
    moved: AtomicBool,
    /// `--no-watch`: hold the writer only while a session may be writing.
    on_demand: bool,
    /// Connected socket sessions, so the last one out can release the writer.
    sessions: AtomicUsize,
}

impl SharedWs {
    /// An empty slot: the first access opens the workspace (`--no-watch`).
    pub(crate) fn empty() -> Arc<Self> {
        Self::build(false)
    }

    /// An empty slot for `--no-watch`: reads open read-only, and the device
    /// actor is taken only for a mutating tool (see the module doc).
    pub(crate) fn on_demand() -> Arc<Self> {
        Self::build(true)
    }

    fn build(on_demand: bool) -> Arc<Self> {
        Arc::new(Self {
            ws: Mutex::new(None),
            peer_dirty: Arc::new(AtomicBool::new(false)),
            index_stale: AtomicBool::new(false),
            read_only: AtomicBool::new(false),
            moved: AtomicBool::new(false),
            on_demand,
            sessions: AtomicUsize::new(0),
        })
    }

    /// A slot holding the workspace `outl serve`'s watcher already opened.
    pub(crate) fn with_open(wc: WsCtx) -> Arc<Self> {
        let shared = Self::empty();
        *shared.ws.lock() = Some(wc);
        shared
    }

    /// Bring the slot up to date: open it when empty, reload it when a peer
    /// (or another local process) changed the log. Returns whether the tree
    /// moved. Called with the lock held.
    fn refresh_locked(&self, slot: &mut Option<WsCtx>, root: &Path) -> Result<bool, ApiError> {
        let dirty = self.peer_dirty.swap(false, Ordering::AcqRel);
        // On demand, a read open is the resting state, not a lost race.
        let retry_write = dirty && !self.on_demand && self.read_only.load(Ordering::Acquire);
        if slot.is_none() || retry_write {
            // Release the old read-only open first: it holds the shared
            // workspace lock and we are about to ask for the write lock.
            *slot = None;
            let (wc, read_only) = if self.on_demand {
                (ws::open_read(root)?, true)
            } else {
                ws::open_resident(root)?
            };
            self.read_only.store(read_only, Ordering::Release);
            *slot = Some(wc);
            self.index_stale.store(true, Ordering::Release);
            self.moved.store(true, Ordering::Release);
            return Ok(true);
        }
        if !dirty {
            return Ok(false);
        }
        let Some(wc) = slot.as_mut() else {
            return Ok(false);
        };
        let engine = outl_actions::SyncEngine::new(wc.root.clone(), wc.actor);
        match engine.reload_workspace(&wc.hlc) {
            Ok(fresh) => {
                wc.workspace = fresh;
                self.index_stale.store(true, Ordering::Release);
                self.moved.store(true, Ordering::Release);
                Ok(true)
            }
            // The signal is spent either way; the next one retries.
            Err(e) => {
                warn!("reloading after peer ops failed: {e:#}");
                Ok(false)
            }
        }
    }

    /// Reload now if the log changed. The watcher calls this ahead of a
    /// reconcile so it diffs against the merged tree.
    pub(crate) fn refresh(&self, root: &Path) -> Result<bool, ApiError> {
        let mut slot = self.ws.lock();
        self.refresh_locked(&mut slot, root)
    }

    /// Whether the tree moved since the last call, clearing the fact.
    pub(crate) fn take_moved(&self) -> bool {
        self.moved.swap(false, Ordering::AcqRel)
    }

    /// Run `f` on the workspace for a caller that is not an MCP session
    /// (the watcher), marking the sessions' index stale afterwards. `None`
    /// when the slot is empty.
    pub(crate) fn run_locked<R>(&self, f: impl FnOnce(&mut WsCtx) -> R) -> Option<R> {
        let mut slot = self.ws.lock();
        let out = slot.as_mut().map(f);
        // Still under the lock: a session that derives its index afterwards
        // sees the change, one that derived before sees the flag.
        self.index_stale.store(true, Ordering::Release);
        out
    }

    /// Make sure the slot holds the writer before a mutating tool runs.
    /// A no-op unless `--no-watch`; fails (keeping the read open) when
    /// another process holds the device actor.
    pub(super) fn upgrade_for_write(&self, root: &Path) -> Result<(), ApiError> {
        if !self.on_demand {
            return Ok(());
        }
        let mut slot = self.ws.lock();
        self.refresh_locked(&mut slot, root)?;
        if !self.read_only.load(Ordering::Acquire) {
            return Ok(());
        }
        let wc = ws::open_device_writer(root)?;
        *slot = Some(wc);
        self.read_only.store(false, Ordering::Release);
        self.index_stale.store(true, Ordering::Release);
        Ok(())
    }

    /// A session ended; when it was the last one, give the device actor back.
    fn session_closed(&self, root: &Path) {
        if self.sessions.fetch_sub(1, Ordering::AcqRel) != 1 || !self.on_demand {
            return;
        }
        let mut slot = self.ws.lock();
        // A session may have connected while we waited for the lock.
        if self.sessions.load(Ordering::Acquire) != 0
            || slot.is_none()
            || self.read_only.load(Ordering::Acquire)
        {
            return;
        }
        // Drop the writer first so its lock is gone even if the read open fails;
        // the next access then reopens.
        *slot = None;
        match ws::open_read(root) {
            Ok(wc) => {
                *slot = Some(wc);
                self.read_only.store(true, Ordering::Release);
            }
            Err(e) => warn!(
                "mcp: reopening read-only after the last session: {}",
                e.message
            ),
        }
        self.index_stale.store(true, Ordering::Release);
    }

    /// Run `f` as an MCP session: refresh, then act, under one lock hold.
    pub(super) fn with_ws<F, R>(&self, root: &Path, f: F) -> Result<R, ApiError>
    where
        F: FnOnce(&mut WsCtx) -> Result<R, ApiError>,
    {
        let mut slot = self.ws.lock();
        self.refresh_locked(&mut slot, root)?;
        let wc = slot.as_mut().ok_or_else(|| {
            ApiError::new(
                codes::INTERNAL,
                "workspace failed to materialise".to_string(),
            )
        })?;
        f(wc)
    }
}

/// Counts one connected session; the last to drop releases the writer.
struct SessionGuard {
    shared: Arc<SharedWs>,
    root: std::path::PathBuf,
}

impl SessionGuard {
    fn new(shared: &Arc<SharedWs>, root: &Path) -> Self {
        shared.sessions.fetch_add(1, Ordering::AcqRel);
        Self {
            shared: Arc::clone(shared),
            root: root.to_path_buf(),
        }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        self.shared.session_closed(&self.root);
    }
}

/// Handle on the running socket server.
pub(crate) struct EmbeddedServer {
    thread: JoinHandle<()>,
}

impl EmbeddedServer {
    /// Wait for the accept loop to notice the shutdown flag. Dropping the
    /// listener it owns is what removes the socket file, so call this on every
    /// way out of `outl serve`.
    pub(crate) fn join(self) {
        if self.thread.join().is_err() {
            warn!("mcp: the socket server thread panicked on the way out");
        }
    }
}

/// Start serving MCP on this workspace's socket. Never fails startup:
/// anything that stops it is logged and `outl serve` carries on without MCP.
///
/// `warm` opens the workspace in the background, so the first session does
/// not pay the replay, without delaying anything `serve` does itself.
#[cfg(unix)]
pub(crate) fn start_embedded(
    root: &Path,
    shared: Arc<SharedWs>,
    shutdown: Arc<AtomicBool>,
    warm: bool,
) -> Option<EmbeddedServer> {
    let path = socket::socket_path(root);
    let listener = match socket::claim(&path) {
        socket::Claim::Bound(l) => l,
        socket::Claim::AlreadyServed => {
            warn!(
                "another `outl serve` already serves MCP for this workspace at {}; \
                 this one runs without MCP",
                path.display()
            );
            return None;
        }
        socket::Claim::Unavailable(why) => {
            warn!("embedded MCP unavailable ({why}); continuing without it");
            return None;
        }
    };
    let ctx = Arc::new(ServerCtx::new_embedded(root.to_path_buf(), shared));
    if warm {
        let ctx = Arc::clone(&ctx);
        let _ = std::thread::Builder::new()
            .name("outl-mcp-warm".into())
            .spawn(move || {
                if let Err(e) = ctx.with_workspace(|_| Ok(())) {
                    warn!("mcp: warming the workspace failed: {}", e.message);
                }
            });
    }
    info!("embedded MCP server listening on {}", path.display());
    let thread = std::thread::Builder::new()
        .name("outl-mcp-accept".into())
        .spawn(move || accept_loop(listener, ctx, shutdown))
        .map_err(|e| warn!("embedded MCP unavailable (cannot spawn accept thread: {e})"))
        .ok()?;
    Some(EmbeddedServer { thread })
}

/// Sockets do not exist here; `outl serve` runs without MCP.
#[cfg(not(unix))]
pub(crate) fn start_embedded(
    _root: &Path,
    _shared: Arc<SharedWs>,
    _shutdown: Arc<AtomicBool>,
    _warm: bool,
) -> Option<EmbeddedServer> {
    info!("embedded MCP needs Unix domain sockets; this platform runs without it");
    None
}

#[cfg(unix)]
fn accept_loop(listener: socket::McpListener, ctx: Arc<ServerCtx>, shutdown: Arc<AtomicBool>) {
    use std::time::Duration;
    if let Err(e) = listener.listener.set_nonblocking(true) {
        warn!("embedded MCP stopped: cannot make the listener non-blocking: {e}");
        return;
    }
    while !shutdown.load(Ordering::SeqCst) {
        match listener.listener.accept() {
            Ok((stream, _)) => {
                // Some platforms hand the accepted stream the listener's mode.
                let _ = stream.set_nonblocking(false);
                let ctx = Arc::clone(&ctx);
                // Counted before the thread starts so the last-session check
                // cannot miss a connection that is still being set up.
                let guard = ctx
                    .shared()
                    .map(|s| SessionGuard::new(s, &ctx.workspace_path));
                let _ = std::thread::Builder::new()
                    .name("outl-mcp-session".into())
                    .spawn(move || {
                        let _guard = guard;
                        let reader = match stream.try_clone() {
                            Ok(r) => std::io::BufReader::new(r),
                            Err(e) => {
                                warn!("mcp: cannot clone the session socket: {e}");
                                return;
                            }
                        };
                        if let Err(e) = super::serve_session(reader, stream, &ctx) {
                            tracing::debug!("mcp: session ended: {e}");
                        }
                    });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                warn!("mcp: accept failed: {e}");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    // `listener` drops here and removes the socket file.
}

/// Pipe stdin/stdout to a live embedded server for `workspace`, if there is
/// one. `Ok(true)` means the session ran as a bridge and is over;
/// `Ok(false)` means there was nothing to bridge to.
#[cfg(unix)]
pub(super) fn bridge(workspace: &Path) -> anyhow::Result<bool> {
    use std::io::Write;
    let path = socket::socket_path(workspace);
    let Ok(stream) = socket::connect(&path) else {
        return Ok(false);
    };
    info!(
        "mcp: bridging to the running `outl serve` at {}",
        path.display()
    );
    let mut to_server = stream.try_clone()?;
    // Detached on purpose: when the server goes away first, this thread may be
    // parked in a stdin read that never returns, and must not hold the exit.
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut to_server);
        let _ = to_server.shutdown(std::net::Shutdown::Write);
    });
    let mut from_server = stream;
    let mut stdout = std::io::stdout();
    std::io::copy(&mut from_server, &mut stdout)?;
    stdout.flush()?;
    Ok(true)
}

#[cfg(not(unix))]
pub(super) fn bridge(_workspace: &Path) -> anyhow::Result<bool> {
    Ok(false)
}
