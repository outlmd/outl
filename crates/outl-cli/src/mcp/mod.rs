//! MCP (Model Context Protocol) server shim.
//!
//! Speaks JSON-RPC 2.0 over stdio implementing the MCP protocol surface
//! Claude Desktop expects:
//!
//! - `initialize` / `initialized`
//! - `tools/list`, `tools/call`
//! - `resources/list`, `resources/read`
//! - `prompts/list`, `prompts/get`
//!
//! Every tool delegates to the same handlers used by the CLI
//! subcommands, so business logic never duplicates.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::output::{codes, ApiError, Envelope};
use crate::ws::{self, WsCtx};
use outl_actions::SyncTransport;
use outl_md::index::WorkspaceIndex;
use outl_sync_iroh::TransportOutcome;

mod prompts;
mod protocol;
mod resources;
mod tools;

/// Protocol version this server speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// Server identification surfaced through `initialize`.
pub const SERVER_NAME: &str = "outl";

/// Server build version (mirrors the crate version).
pub const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Run the stdio MCP loop. Returns when the client closes stdin.
pub fn serve(workspace_path: PathBuf) -> anyhow::Result<()> {
    let ctx = Arc::new(ServerCtx::new(workspace_path));
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout = stdout.lock();
    let mut line = String::new();
    loop {
        line.clear();
        let n = stdin.read_line(&mut line)?;
        if n == 0 {
            // EOF — client closed the pipe.
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // A panic in one tool handler shouldn't kill the whole MCP session
        // (it'd drop the iroh transport and every cached workspace mid-chain).
        // Catch it, reply with a JSON-RPC internal error, and keep serving.
        let response = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle_line(trimmed, &ctx)
        })) {
            Ok(resp) => resp,
            Err(_) => {
                warn!("mcp: tool handler panicked; replied internal error, session stays up");
                Some(
                    r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"internal error"}}"#
                        .to_string(),
                )
            }
        };
        if let Some(resp) = response {
            writeln!(stdout, "{resp}")?;
            stdout.flush()?;
        }
    }
    // Client closed the pipe — tear the P2P transport down cleanly so its
    // endpoint releases the relay route for any other process on this device.
    ctx.shutdown_transport();
    Ok(())
}

/// Shared per-server context.
///
/// Holds the workspace open for the lifetime of the MCP session so we
/// don't re-replay the op log on every tool call, and caches the
/// `WorkspaceIndex` between read-only calls (invalidated whenever a
/// mutating tool runs).
pub(crate) struct ServerCtx {
    /// Workspace root the MCP server operates on.
    pub workspace_path: PathBuf,
    /// Lazy state guarded by a single mutex. We don't run tool calls
    /// concurrently today, so a `parking_lot::Mutex` is sufficient and
    /// cheap.
    state: Mutex<ServerState>,
    /// Set by the transport's peer-ready drain thread when a peer pushed
    /// new ops. The next workspace access drops the cache and reopens so
    /// the MCP serves the peer's edits, not a stale replay. An `AtomicBool`
    /// keeps the drain thread off the `state` mutex.
    peer_dirty: Arc<AtomicBool>,
}

#[derive(Default)]
struct ServerState {
    workspace: Option<WsCtx>,
    index: Option<WorkspaceIndex>,
    /// The iroh P2P transport, brought up on a workspace open once this
    /// process holds the device endpoint lease. `None` means this session is
    /// a passive writer — see [`ServerCtx::build_peer_transport`] for the four
    /// reasons, all of which are working states.
    transport: Option<Arc<dyn SyncTransport>>,
    /// Sender every transport we start signals peer arrivals on, kept alive
    /// here so later starts can clone it. `Some` also means the always-on file
    /// poller and its drain thread (which owns the matching receiver) are
    /// running — unlike the endpoint, they start exactly once.
    ///
    /// One channel for the whole session, not one per call: the receiver lives
    /// in the drain thread, so a channel built on a later call would hand the
    /// transport a sender nobody is listening to. That is precisely the call
    /// that matters — the endpoint is asked for again on every reopen, so the
    /// one path where the MCP wins the lease late is the one that would lose
    /// every signal, silently ([`ServerCtx::ensure_transport`]).
    peer_ready_tx: Option<Sender<()>>,
}

impl ServerCtx {
    fn new(workspace_path: PathBuf) -> Self {
        Self {
            workspace_path,
            state: Mutex::new(ServerState::default()),
            peer_dirty: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Run `f` against the cached workspace, opening it on first use.
    ///
    /// The lock is held for the whole call — fine because the MCP
    /// stdio loop is single-threaded today. If we ever serve concurrent
    /// requests this becomes the obvious throttling point.
    pub(crate) fn with_workspace<F, R>(self: &Arc<Self>, f: F) -> Result<R, ApiError>
    where
        F: FnOnce(&mut WsCtx) -> Result<R, ApiError>,
    {
        let mut state = self.state.lock();
        // A peer pushed ops since the last access — drop the cache so the
        // open below replays the freshly-arrived ops-*.jsonl.
        if self.peer_dirty.swap(false, Ordering::Acquire) {
            state.workspace = None;
            state.index = None;
        }
        if state.workspace.is_none() {
            let wc = ws::open(&self.workspace_path)?;
            // First open: bring the P2P transport up so this MCP session is a
            // first-class peer (pushes its ops, accepts inbound) without
            // depending on a GUI being open. Best-effort — a failure here
            // never blocks the tool call.
            self.ensure_transport(&mut state, &wc);
            state.workspace = Some(wc);
        }
        let wc = state.workspace.as_mut().ok_or_else(|| {
            ApiError::new(
                codes::INTERNAL,
                "workspace failed to materialise".to_string(),
            )
        })?;
        f(wc)
    }

    /// Bring the MCP's sync transports up on a workspace open.
    ///
    /// **The file poller always runs.** It notices ops that other processes on
    /// this machine (a co-resident GUI, the `outl` CLI) wrote to the shared
    /// `ops/` dir and flips `peer_dirty`, so the next tool call reopens and
    /// serves them. iroh only signals on its own wire receipts, so it does not
    /// subsume this — same reasoning as the TUI's always-on poller.
    ///
    /// **The iroh endpoint runs when this process wins the device lease.**
    /// iroh's relay routes one endpoint per node_id, and every outl process
    /// here shares `~/.outl/identity.key`, so a second endpoint would steal the
    /// route and break the holder's sync in both directions. The MCP used to be
    /// hard-coded as the loser of that contest, which works right up until
    /// there is nobody else: on a headless machine (an agent driving
    /// `outl mcp serve`, no GUI anywhere) nothing ever bound an endpoint, so
    /// the device's ops never left and no peer's ops ever arrived (issue #220).
    /// [`outl_sync_iroh::build_default_transport`] arbitrates instead — first
    /// process in wins, and a GUI that got there first still leaves the MCP exactly
    /// where it was, a passive writer converging through disk.
    fn ensure_transport(self: &Arc<Self>, state: &mut ServerState, wc: &WsCtx) {
        // The transports signal on this channel each time peer ops land; a tiny
        // drain thread flips `peer_dirty` so the next access reopens and
        // replays them. Built on the first call and kept in `state` from then
        // on, because the drain thread owns the only receiver — see
        // `ServerState::peer_ready_tx`.
        let tx = match &state.peer_ready_tx {
            Some(tx) => tx.clone(),
            None => {
                let (tx, rx) = std::sync::mpsc::channel::<()>();
                let dirty = self.peer_dirty.clone();
                std::thread::Builder::new()
                    .name("outl-mcp-peer-ready".into())
                    .spawn(move || {
                        while rx.recv().is_ok() {
                            dirty.store(true, Ordering::Release);
                        }
                    })
                    .ok();
                outl_actions::FileSyncTransport.start(wc.root.clone(), wc.actor, tx.clone());
                state.peer_ready_tx = Some(tx.clone());
                tx
            }
        };
        if state.transport.is_some() {
            return;
        }
        // Asked again on every workspace reopen while we have no endpoint,
        // because every reason to decline is a state the user can change from
        // outside this process: pairing a first device, or closing the GUI that
        // holds the lease. Answering once would strand an MCP session that
        // started before its device was paired.
        state.transport = self.build_peer_transport(wc).inspect(|transport| {
            transport.start(wc.root.clone(), wc.actor, tx);
        });
    }

    /// Ask for this device's iroh endpoint. `None` means the MCP stays a
    /// passive writer — P2P is off, someone else holds the endpoint, there is
    /// nothing paired to sync with, or the identity / peer store could not be
    /// read (all four are working states, never fatal to a tool call).
    fn build_peer_transport(self: &Arc<Self>, wc: &WsCtx) -> Option<Arc<dyn SyncTransport>> {
        match outl_sync_iroh::build_default_transport(&wc.root) {
            Ok(TransportOutcome::Ready(t)) if t.peers().is_empty() => {
                // Nothing paired: binding an endpoint (and a relay connection)
                // would buy nothing, and dropping `t` here hands the lease
                // straight back to a GUI that may want it for pairing.
                debug!("mcp: no paired devices; staying off the wire");
                None
            }
            Ok(TransportOutcome::Ready(t)) => {
                debug!("mcp: iroh endpoint bound (this process holds the device lease)");
                Some(Arc::new(t) as Arc<dyn SyncTransport>)
            }
            Ok(TransportOutcome::EndpointBusy(why)) => {
                // `why` distinguishes "another local process got here first"
                // (the ordinary election outcome) from "the lease file cannot
                // be opened at all", which no exiting process ever fixes. The
                // lease itself already warns on the second one.
                debug!("mcp: no iroh endpoint here ({why}); passive writer");
                None
            }
            Ok(TransportOutcome::Disabled) => None,
            Err(e) => {
                warn!("mcp: iroh unavailable ({e}); syncing through ops/ only");
                None
            }
        }
    }

    /// Tell peers a mutation landed, so they pull now instead of on their next
    /// catch-up tick. A no-op when this process is a passive writer.
    ///
    /// Latency only: correctness rides on every device's `MAINTENANCE_RESYNC`
    /// re-pull, which converges an unannounced write regardless.
    pub(crate) fn announce_local_ops(self: &Arc<Self>) {
        let mut state = self.state.lock();
        let Some(transport) = state.transport.clone() else {
            return;
        };
        let Some(wc) = state.workspace.as_mut() else {
            return;
        };
        // The first argument is a hint the gossip drain discards — it announces
        // under the canonical `WorkspaceId` it already holds, precisely because
        // clients kept passing something else (a page slug). Naming the source
        // is more honest than inventing a slug the MCP would have to look up.
        transport.announce_local_ops("mcp", wc.hlc.next());
    }

    /// Tear the transport down (called when the stdio pipe closes).
    pub(crate) fn shutdown_transport(self: &Arc<Self>) {
        if let Some(transport) = self.state.lock().transport.take() {
            transport.shutdown();
        }
    }

    /// Run `f` against the cached `WorkspaceIndex`, deriving it on
    /// first use. Mutating tools should call [`Self::invalidate_index`]
    /// after their `apply_page_md_with_sidecar` so the next read sees
    /// fresh blocks.
    ///
    /// Derived from the session's already-open workspace rather than
    /// walked off disk — the tree is the source of truth, and it is
    /// right here. Falls back to the disk build only if the workspace
    /// cannot be opened at all, so a read-only tool still answers
    /// something instead of failing on a state it could recover from.
    pub(crate) fn with_index<F, R>(self: &Arc<Self>, f: F) -> R
    where
        F: FnOnce(&WorkspaceIndex) -> R,
    {
        let mut state = self.state.lock();
        // A peer pushed ops since the last access — the cached index
        // describes the pre-push tree. Same invalidation
        // `with_workspace` performs, needed here because a read-only
        // tool may never reach that path.
        if self.peer_dirty.swap(false, Ordering::Acquire) {
            state.workspace = None;
            state.index = None;
        }
        if state.index.is_none() {
            if state.workspace.is_none() {
                if let Ok(wc) = ws::open(&self.workspace_path) {
                    self.ensure_transport(&mut state, &wc);
                    state.workspace = Some(wc);
                }
            }
            state.index = Some(match state.workspace.as_ref() {
                Some(wc) => outl_actions::index::derive(&wc.workspace, &wc.root),
                None => WorkspaceIndex::build(&self.workspace_path),
            });
        }
        f(state.index.as_ref().expect("index just populated"))
    }

    /// Drop the cached index. The next `with_index` re-derives it.
    pub(crate) fn invalidate_index(self: &Arc<Self>) {
        self.state.lock().index = None;
    }
}

fn handle_line(line: &str, ctx: &Arc<ServerCtx>) -> Option<String> {
    let request: protocol::JsonRpcRequest = match serde_json::from_str(line) {
        Ok(req) => req,
        Err(e) => {
            // Parse error — id may not be available; respond with null id.
            let resp = protocol::JsonRpcResponse::error(
                Value::Null,
                protocol::PARSE_ERROR,
                format!("invalid JSON: {e}"),
            );
            return serde_json::to_string(&resp).ok();
        }
    };

    // Notifications (no `id`) get no response.
    let is_notification = request.id.is_none();
    let id = request.id.clone().unwrap_or(Value::Null);
    let method = request.method.clone();
    let params = request.params.unwrap_or(Value::Null);

    let result = dispatch(&method, params, ctx);

    if is_notification {
        return None;
    }

    let response = match result {
        Ok(value) => protocol::JsonRpcResponse::success(id, value),
        Err(err) => protocol::JsonRpcResponse::error(id, err.code, err.message),
    };
    serde_json::to_string(&response).ok()
}

fn dispatch(
    method: &str,
    params: Value,
    ctx: &Arc<ServerCtx>,
) -> Result<Value, protocol::JsonRpcError> {
    match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": { "listChanged": false },
                "resources": { "subscribe": false, "listChanged": false },
                "prompts": { "listChanged": false },
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION,
            },
        })),
        "initialized" | "notifications/initialized" => Ok(Value::Null),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => tools::call(params, ctx),
        "resources/list" => Ok(json!({ "resources": resources::list() })),
        "resources/read" => resources::read(params, ctx),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": resources::templates() })),
        "prompts/list" => Ok(json!({ "prompts": prompts::list() })),
        "prompts/get" => prompts::get(params, ctx),
        other => Err(protocol::JsonRpcError::method_not_found(other)),
    }
}

/// Wrap an [`ApiError`] into MCP tool output. MCP tool errors flow
/// through the response shape `{ content: [...], isError: true }`
/// rather than as JSON-RPC errors, so the client gets a recoverable
/// signal instead of a protocol-level fault.
pub(crate) fn tool_error_payload(err: &ApiError) -> Value {
    json!({
        "content": [
            { "type": "text", "text": format!("{}: {}", err.code, err.message) }
        ],
        "isError": true,
    })
}

/// Wrap a successful tool result into the MCP tool-output envelope.
///
/// `tool_name` lets us pick a more useful `text` representation than
/// "pretty-printed JSON" for the tools where the user is asking for
/// raw markdown (`export_md`, `page_render`, etc.). The
/// `structuredContent` field always carries the full envelope so
/// callers that prefer machine shape still get it.
pub(crate) fn tool_success_payload(tool_name: &str, payload: &Value) -> Value {
    let text = preferred_text_for(tool_name, payload);
    let envelope = Envelope::success(payload.clone());
    json!({
        "content": [ { "type": "text", "text": text } ],
        "structuredContent": serde_json::to_value(&envelope).unwrap_or(Value::Null),
        "isError": false,
    })
}

/// Pick a text content best suited for `tool_name`.
///
/// Tools that produce a single big string (rendered markdown, summary
/// text) flatten the payload by reading its natural field. Everything
/// else stays as pretty-printed JSON so structured callers always see
/// the same shape.
fn preferred_text_for(tool_name: &str, payload: &Value) -> String {
    let take_field = |field: &str| -> Option<String> {
        payload
            .get(field)
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    match tool_name {
        // Pure-markdown surfaces: prefer the raw `md` field.
        "outl_export_md" | "outl_page_render" => take_field("md"),
        // Daily / page surfaces ship both `md` and a structured outline;
        // the host shows the markdown as the "natural" text content.
        "outl_daily_today" | "outl_daily_get" => take_field("md"),
        _ => None,
    }
    .unwrap_or_else(|| {
        serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A transport that does nothing, so the test can pre-fill
    /// `ServerState::transport` and keep `ensure_transport` off the network.
    struct NoopTransport;

    impl SyncTransport for NoopTransport {
        fn start(&self, _: PathBuf, _: outl_core::id::ActorId, _: Sender<()>) {}
        fn announce_local_ops(&self, _: &str, _: outl_core::hlc::Hlc) {}
        fn shutdown(&self) {}
    }

    /// The peer-ready channel must survive a second `ensure_transport`.
    ///
    /// It runs again on every workspace reopen — the only path by which an MCP
    /// session that started before its device was paired (or before the GUI
    /// holding the lease closed) ever wins the endpoint. Building a fresh
    /// channel per call left the drain thread holding the *first* receiver, so
    /// from the second call on every `peer_ready_tx.send(())` landed in a
    /// closed channel without a single log line: the reopen that finally got
    /// an endpoint was exactly the one that could not signal through it.
    #[test]
    fn the_peer_ready_channel_survives_a_second_ensure_transport() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        crate::cmd::init::run(dir.path(), "global").expect("init workspace");
        let wc = ws::open(dir.path()).expect("open workspace");
        let ctx = Arc::new(ServerCtx::new(dir.path().to_path_buf()));

        // Pre-filled so `ensure_transport` returns before asking iroh for an
        // endpoint; the channel wiring under test runs before that point.
        let mut state = ServerState {
            transport: Some(Arc::new(NoopTransport)),
            ..Default::default()
        };
        ctx.ensure_transport(&mut state, &wc);
        ctx.ensure_transport(&mut state, &wc);

        let tx = state
            .peer_ready_tx
            .clone()
            .expect("the channel is built on the first call");
        tx.send(()).expect("its receiver must still be alive");

        let deadline = Instant::now() + Duration::from_secs(5);
        while !ctx.peer_dirty.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            ctx.peer_dirty.load(Ordering::Acquire),
            "a signal sent after the second call must reach the drain thread"
        );
    }
}
