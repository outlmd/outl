//! Cross-client sync engine.
//!
//! Encapsulates the reload-workspace + reproject-page dance that both
//! mobile and TUI need when a peer (another device, another local
//! process) writes new ops into `<root>/ops/`. Clients still own:
//!
//! - **Detection.** TUI uses a worker thread that stats the jsonl
//!   files every ~2 s; mobile registers an `NSMetadataQuery` on the
//!   iCloud ubiquity container. Both call [`SyncEngine::snapshot`]
//!   to know what changed.
//! - **Policy.** TUI must defer the reload while the user is in
//!   Insert mode (the in-flight `ParsedPage` would be clobbered);
//!   mobile commits each mutation atomically via Tauri commands and
//!   can always apply immediately.
//!
//! What lives **here**, shared between every client:
//!
//! - Opening a fresh [`Workspace`] from the on-disk op log
//!   ([`SyncEngine::reload_workspace`]).
//! - Re-projecting a page's `.md` + sidecar from the materialised
//!   workspace ([`SyncEngine::reproject_page`]) so the on-disk view
//!   always reflects the merged op log.
//! - The shorthand that does both in sequence
//!   ([`SyncEngine::refresh_page`]) for the typical "peer fired,
//!   pull the new state in" path.
//!
//! Adding a new client means writing a detector + policy and calling
//! these three functions — never re-implementing them.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use outl_core::hlc::{Hlc, HlcGenerator};
use outl_core::id::{ActorId, NodeId};
use outl_core::storage::JsonlStorage;
use outl_core::workspace::Workspace;

use crate::error::ActionError;
use crate::journal::apply_page_md_with_sidecar_guarded;

/// Reachability snapshot for one known peer, derived from the
/// transport's own dial outcomes (never a fresh probe endpoint).
///
/// A GUI status indicator reads this from the running [`SyncTransport`]
/// (`peer_health`) instead of binding a second iroh endpoint with the
/// device identity — two endpoints sharing one `node_id` make the relay
/// route the inbound sync to the wrong one (see
/// `outl-sync-iroh/CLAUDE.md` → "One endpoint per identity, elected not
/// assigned").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerHealthSnapshot {
    /// Peer's node id (string form), as stored in `peers.json`.
    pub node_id: String,
    /// `true` if the most recent dial (boot connect, catch-up, or an
    /// inbound serve) to this peer succeeded.
    pub reachable: bool,
    /// Round-trip-ish duration of the last successful dial, in
    /// milliseconds. `None` when the peer has never been reached this
    /// session.
    pub last_rtt_ms: Option<u64>,
}

/// A sync-progress update the transport pushes to the UI while a pass runs.
///
/// **Purely informational — distinct from the `start` reload `tx`.** That
/// unit signal stays the load-bearing "peer ops landed, reload the workspace"
/// trigger; correctness depends on it. This enum only drives a progress
/// indicator (the pairing screen's "downloading snapshot 8/15 MB…" feed), so
/// a dropped update is cosmetic, never a lost op.
///
/// Every variant carries `peer` as the peer's **short** node id
/// (`EndpointId::fmt_short`); the UI resolves it to a friendly alias against
/// the peer list it already holds. The only honest percentage is
/// [`Self::Snapshot`] — its `total` comes from the frame's length prefix,
/// known before the body arrives; op counts are known only once a batch
/// finishes, so they surface as a live count, not a bar.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "phase", rename_all = "kebab-case")]
pub enum SyncProgress {
    /// Dialing / handshaking a peer, before any bytes flow.
    Connecting {
        /// Peer's short node id.
        peer: String,
    },
    /// Pulling a peer's materialized snapshot. `received` / `total` in bytes.
    Snapshot {
        /// Peer's short node id.
        peer: String,
        /// Bytes of the snapshot frame received so far.
        received: u64,
        /// Total snapshot frame size, from the length prefix.
        total: u64,
    },
    /// Pulling a peer's binary asset (an uploaded file: PDF, image). Fired per
    /// asset the initiator is missing; `received` / `total` are the bytes of the
    /// asset currently transferring, from that file's frame length prefix — the
    /// same honest-percentage shape as [`Self::Snapshot`]. Asset bytes are
    /// content-addressed blobs replicated outside the op log (see
    /// `outl_actions::asset`); over iroh they travel on `outl-asset/1`.
    Asset {
        /// Peer's short node id.
        peer: String,
        /// Bytes of the current asset frame received so far.
        received: u64,
        /// Total size of the current asset frame, from the length prefix.
        total: u64,
    },
    /// Ingested `count` ops the peer had that this device lacked. `nodes`
    /// carries the (capped) distinct block ids touched so the UI can name the
    /// pages that changed — empty on a bulk pass (the initial pair), where
    /// naming every page is meaningless.
    ReceivedOps {
        /// Peer's short node id.
        peer: String,
        /// Number of ops applied in this batch.
        count: u64,
        /// Distinct block ids touched (capped; empty on a bulk pass).
        nodes: Vec<String>,
    },
    /// Pushed `count` ops the peer lacked.
    PushedOps {
        /// Peer's short node id.
        peer: String,
        /// Number of ops pushed to the peer.
        count: u64,
    },
    /// A sync pass with this peer finished cleanly.
    Synced {
        /// Peer's short node id.
        peer: String,
    },
    /// The exchange was cut off before the peer could confirm, and this device
    /// will retry on its next pass.
    ///
    /// Distinct from [`Self::Failed`] because the *cause* is different and so
    /// is the honest thing to show a user: a phone that locked its screen, a
    /// laptop that slept, a carrier NAT that dropped the flow. Nothing is
    /// wrong, nothing is lost, and the next catch-up tick re-pushes.
    ///
    /// It exists because the alternative reads as breakage. A responder
    /// confirms durable ingest by closing with code 0, so a peer suspended
    /// mid-exchange produces a failed pass **every time** — a user who locks
    /// their phone watched their desktop paint a red row for a sync that was
    /// working exactly as designed. Do NOT "fix" that by treating an
    /// unconfirmed push as success; the confirmation is what makes the
    /// re-push safe to skip (see `delta_sync`'s trailing `conn.closed()`).
    /// Only the colour was ever wrong.
    Interrupted {
        /// Peer's short node id.
        peer: String,
        /// Human-readable description of how the connection ended.
        reason: String,
    },
    /// A sync pass with this peer failed.
    Failed {
        /// Peer's short node id.
        peer: String,
        /// Human-readable failure reason.
        error: String,
    },
}

/// Transport abstraction — how ops travel between devices.
///
/// iCloud/filesystem: detects file changes via polling.
/// iroh: receives ops over QUIC streams, writes them to local FS, fires signal.
///
/// Both transports result in `ops-<peer>.jsonl` files landing on disk.
/// `SyncEngine::reload_workspace` picks them up identically regardless of transport.
pub trait SyncTransport: Send + Sync + 'static {
    /// Start the transport.
    ///
    /// Spawns whatever background tasks are needed (polling thread, iroh runtime, …).
    /// Sends on `tx` whenever peer ops have been written to the local `ops/`
    /// directory and the workspace is ready to reload.
    fn start(
        &self,
        workspace_root: std::path::PathBuf,
        actor: outl_core::id::ActorId,
        tx: std::sync::mpsc::Sender<()>,
    );

    /// Called after this device commits local ops to the op log.
    ///
    /// FileSyncTransport: no-op (iCloud/Syncthing carries the file).
    /// IrohSyncTransport: gossip-announces the new HLC to connected peers.
    fn announce_local_ops(&self, workspace_id: &str, hlc: Hlc);

    /// Graceful shutdown. Transport must stop background tasks.
    fn shutdown(&self);

    /// Register a channel the transport pushes [`SyncProgress`] updates to.
    ///
    /// Optional and purely informational: a UI progress indicator reads it,
    /// correctness never does (the reload trigger is the `start` `tx`). Call
    /// this **before** [`Self::start`] so the transport captures the sink as
    /// it wires up its tasks. The default no-op means [`FileSyncTransport`]
    /// and any transport without granular progress simply never report.
    fn set_progress_sink(&self, _tx: std::sync::mpsc::Sender<SyncProgress>) {}

    /// Force an immediate sync pass against every known peer, instead of
    /// waiting for the transport's own periodic catch-up tick.
    ///
    /// Drives the "pull to refresh" / "sync now" affordance in the GUI: the
    /// user wants the freshest state right now, so re-dial every peer (even
    /// healthy ones the catch-up loop would otherwise leave to gossip) and run
    /// the same delta sync. A no-op when the transport is down.
    ///
    /// The default does nothing — only transports that actually dial peers
    /// (iroh) have anything to force. [`FileSyncTransport`] relies on the OS
    /// file watcher / its own polling and has no peer to dial.
    fn sync_now(&self) {}

    /// Reachability snapshot for every known peer, derived from the
    /// transport's own dial outcomes.
    ///
    /// GUI status indicators call this instead of standing up a probe
    /// endpoint. The default returns an empty vector — only transports
    /// that actually dial peers (iroh) have anything to report;
    /// [`FileSyncTransport`] has no peer concept.
    fn peer_health(&self) -> Vec<PeerHealthSnapshot> {
        Vec::new()
    }
}

/// Filesystem / iCloud transport — the v0 implementation.
///
/// Detection: polls `ops/` every 2 s for peer file changes.
/// Delivery: no-op — iCloud Drive / Syncthing / shared FS carries the bytes.
#[derive(Debug, Clone, Default)]
pub struct FileSyncTransport;

impl SyncTransport for FileSyncTransport {
    fn start(
        &self,
        workspace_root: std::path::PathBuf,
        actor: outl_core::id::ActorId,
        tx: std::sync::mpsc::Sender<()>,
    ) {
        // Build a temporary engine just for snapshot polling.
        let engine = SyncEngine::new(workspace_root, actor);
        std::thread::Builder::new()
            .name("outl-file-sync".into())
            .spawn(move || {
                let mut last = engine.snapshot_peers();
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let current = engine.snapshot_peers();
                    if current != last {
                        last = current;
                        if tx.send(()).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("failed to spawn outl-file-sync thread");
    }

    fn announce_local_ops(&self, _workspace_id: &str, _hlc: Hlc) {
        // File transport: the file is already on disk; the peer's poller will
        // notice it on the next 2 s tick. Nothing to announce explicitly.
    }

    fn shutdown(&self) {
        // The polling thread exits when the mpsc Sender is dropped by the caller.
    }
}

/// Snapshot of one `ops-<actor>.jsonl` file. Detectors compare these
/// across polls to decide whether to fire a reload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpsFileSnapshot {
    /// Filename inside `<root>/ops/`.
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Last modification time as reported by the filesystem.
    pub mtime: SystemTime,
}

/// Owns the workspace root + actor identity for one running client.
///
/// Stateless beyond those two fields — every method opens what it
/// needs and returns it. Multiple instances pointing at the same root
/// are safe (the underlying op log is append-only per actor).
#[derive(Clone)]
pub struct SyncEngine {
    workspace_root: PathBuf,
    actor: ActorId,
    transport: Option<std::sync::Arc<dyn SyncTransport>>,
}

impl std::fmt::Debug for SyncEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncEngine")
            .field("workspace_root", &self.workspace_root)
            .field("actor", &self.actor)
            .field(
                "transport",
                &self.transport.as_ref().map(|_| "<dyn SyncTransport>"),
            )
            .finish()
    }
}

/// Path of a workspace's orphan log (`<root>/.outl/orphans.log`).
///
/// **Every caller of `outl_md::reconcile_md` must pass this.** A block
/// that fails to match an existing id drops to matching level 3, which
/// moves it to the trash — and the crate's hard rule is that it appears
/// in this log *before* that happens. Passing `None` is what turned a
/// GUI boot reconcile into a silent delete: the ids are gone from the
/// tree, the `.md` no longer shows them, and nothing on disk says why.
///
/// This lives here rather than in `outl-ws` because the GUI clients and
/// `outl-actions` itself reconcile without depending on that crate; one
/// owner beats each client joining `.outl` to a filename by hand.
pub fn orphans_log_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".outl").join("orphans.log")
}

impl SyncEngine {
    /// Path of this workspace's orphan log — see [`orphans_log_path`].
    pub fn orphans_log(&self) -> PathBuf {
        orphans_log_path(&self.workspace_root)
    }

    /// Bind to a workspace root + actor.
    pub fn new(workspace_root: PathBuf, actor: ActorId) -> Self {
        Self {
            workspace_root,
            actor,
            transport: None,
        }
    }

    /// Bind to a workspace root + actor with an explicit transport.
    ///
    /// `transport.start()` is NOT called here; call `start_transport` once the
    /// caller's notification channel is ready.
    pub fn with_transport(
        workspace_root: PathBuf,
        actor: ActorId,
        transport: Box<dyn SyncTransport>,
    ) -> Self {
        Self {
            workspace_root,
            actor,
            transport: Some(std::sync::Arc::from(transport)),
        }
    }

    /// Start the transport's background tasks.
    ///
    /// Calls `transport.start(workspace_root, actor, tx)` if a transport is set.
    /// No-op when no transport was configured (callers manage detection themselves).
    pub fn start_transport(&self, peer_ready_tx: std::sync::mpsc::Sender<()>) {
        if let Some(t) = &self.transport {
            t.start(self.workspace_root.clone(), self.actor, peer_ready_tx);
        }
    }

    /// Announce new local ops to connected peers.
    ///
    /// Calls `transport.announce_local_ops` if a transport is set.
    /// No-op when no transport was configured.
    pub fn announce_local_ops(&self, workspace_id: &str, hlc: Hlc) {
        if let Some(t) = &self.transport {
            t.announce_local_ops(workspace_id, hlc);
        }
    }

    /// Workspace root this engine talks to.
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Actor id this engine writes as.
    pub fn actor(&self) -> ActorId {
        self.actor
    }

    /// Open a fresh `Workspace` from disk. The caller swaps it in
    /// place of whatever stale workspace they were holding.
    ///
    /// Reads every `ops-*.jsonl` in `<root>/ops/`, merges them by
    /// HLC, and replays the resulting ordered sequence into the
    /// materialised tree.
    ///
    /// `hlc` is the caller's live generator — the same one its next local
    /// edit will stamp from — and it is raised here past everything the
    /// merged log holds ([`Workspace::seed_clock`]). Boot seeding alone
    /// does not cover this path: a peer that is ahead of our wall clock
    /// lands its ops *after* boot, so without this the next local edit is
    /// stamped below them and pays the paper's undo/redo window over every
    /// newer entry. That is a **cost**, not a convergence bug — `apply_op`
    /// reorders to the same tree either way — but it is paid synchronously
    /// on a foreground keystroke, and it recurs on every sync tick.
    pub fn reload_workspace(&self, hlc: &HlcGenerator) -> Result<Workspace, ActionError> {
        let ops_dir = self.workspace_root.join("ops");
        let storage = JsonlStorage::open(ops_dir, self.actor)
            .map_err(|e| ActionError::Io(std::io::Error::other(format!("jsonl open: {e}"))))?;
        let mut workspace = Workspace::open_with_storage(
            self.actor,
            Box::new(storage),
            Some(self.workspace_root.clone()),
        )?;
        // Propagated, exactly as the boot sites do. `seed_clock` reads the
        // per-actor maxima off the index the `open_with_storage` above just
        // built, so a failure here means storage stopped being readable
        // between two statements — and then the workspace we would hand back
        // is no more trustworthy than the seeding. Every reload caller
        // already has a safe answer for `Err`: keep the workspace you have
        // and retry on the next signal. Swallowing it would leave the clock
        // unseeded with no signal anywhere, which is this bug returning
        // invisibly.
        workspace.seed_clock(hlc)?;

        // Write-through snapshot after a big cold replay.
        //
        // Peer-sync ingest writes ops straight to `ops-*.jsonl` (never
        // through `Workspace::apply`), so the background snapshot writer —
        // which only fires from `apply` crossing the op threshold — never
        // runs on a receive-only device. Every reload then full-replays the
        // ENTIRE log, and the GUI reloads every few seconds (5s poll +
        // `workspace-ready`), pinning the CPU on a freshly-synced 200k-op
        // workspace: the journal paints but the UI keeps stuttering.
        //
        // Re-persist a fresh snapshot whenever this reload FULL-REPLAYED
        // (snapshot absent, stale, or rejected by the convergence guard) so
        // the next boot adopts one instead of replaying. `save_snapshot` is
        // O(log) — the block-text index makes `force_materialize_pending`
        // cheap, not the old O(blocks × log) that made this catastrophic —
        // and it no-ops on an empty log, so there's no size floor worth
        // gating on. A gate here used to require `log().len() >= 10_000`
        // before re-persisting, which meant any workspace under that size
        // whose snapshot got rejected once by the convergence guard (the
        // routine case for two actively-syncing actors — see
        // `late_low_hlc_op_from_unseen_actor_survives_snapshot_boot`) never
        // got a fresh cutoff and full-replayed on *every* subsequent
        // incremental reload, forever.
        if !workspace.booted_from_snapshot() {
            if let Err(e) = workspace.save_snapshot() {
                tracing::warn!("reload: could not persist boot snapshot: {e}");
            }
        }
        Ok(workspace)
    }

    /// Re-project a single page's `.md` + sidecar from `workspace`,
    /// after merging a peer's ops.
    ///
    /// Call this after [`Self::reload_workspace`] so the on-disk `.md`
    /// picks up the merged state (own ops + peer ops). Other pages get
    /// re-projected lazily when the user navigates to them.
    ///
    /// **Can refuse.** This re-renders the page from the tree, which is
    /// the direction root `CLAUDE.md` invariant 8 forbids doing
    /// unconditionally: a peer's merge is exactly the case where the
    /// user never touched this page, so a `.md` that has drifted ahead
    /// of the op log here is the worst version of the bug — content
    /// vanishing with no local edit to blame. `Err(ActionError::PageMarkdownAheadOfLog)`
    /// means the write was withheld, not that the merge failed: `workspace`
    /// already holds the peer's ops regardless of what this call returns.
    ///
    /// **Every caller must treat this as one page's problem, not the
    /// whole sync pass's.** Bubbling the error out of a loop that
    /// reprojects many pages would let one frozen page stop every other
    /// page from converging — worse than the bug this guards against.
    /// Both current callers already reproject a single, specific page
    /// (the TUI's focused page, the GUI's "today" journal) and log a
    /// refusal rather than letting it abort the reload; see
    /// `outl-tui/src/actions/lifecycle/peer_sync.rs::reload_workspace_from_disk`
    /// and the mobile/desktop `reload_workspace` Tauri commands.
    pub fn reproject_page(
        &self,
        workspace: &Workspace,
        page_id: NodeId,
    ) -> Result<(), ActionError> {
        apply_page_md_with_sidecar_guarded(workspace, &self.workspace_root, page_id)?;
        Ok(())
    }

    /// Reload the workspace **and** re-project the focused page in
    /// one go. Returns the new workspace.
    ///
    /// This is the typical entry point for the "peer fired, pull
    /// new state" path. Clients call this from their detector once
    /// they have decided it's safe (e.g. user is not mid-edit).
    ///
    /// Propagates [`Self::reproject_page`]'s refusal as-is. Unlike the
    /// TUI and GUI reload paths, this helper isn't currently called from
    /// production code (see call sites above) — if a future caller
    /// reaches for it, it inherits the same rule: a refusal here means
    /// this one page's `.md` didn't update, not that the reload failed,
    /// and it must not be allowed to abort a larger sync pass.
    pub fn refresh_page(
        &self,
        hlc: &HlcGenerator,
        page_id: NodeId,
    ) -> Result<Workspace, ActionError> {
        let ws = self.reload_workspace(hlc)?;
        self.reproject_page(&ws, page_id)?;
        Ok(ws)
    }

    /// List every `ops-*.jsonl` file in the workspace with size and
    /// mtime. Used by polling detectors (TUI) to decide whether a
    /// peer wrote since the last check.
    ///
    /// Returns an empty vec when `<root>/ops/` is absent (workspace
    /// is using the SQLite backend, or hasn't been initialised yet).
    pub fn snapshot(&self) -> Vec<OpsFileSnapshot> {
        let ops_dir = self.workspace_root.join("ops");
        let Ok(entries) = std::fs::read_dir(&ops_dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("ops-") || !name.ends_with(".jsonl") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let Ok(mtime) = meta.modified() else { continue };
            out.push(OpsFileSnapshot {
                name,
                size: meta.len(),
                mtime,
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Like [`Self::snapshot`] but **excludes this engine's own
    /// `ops-<actor>.jsonl` file**.
    ///
    /// This is what polling detectors should call: a TUI that
    /// reacted to its *own* `.jsonl` growing would reload the
    /// workspace, re-project the page, and overwrite the `.md` the
    /// user just edited — closing a destructive loop where each save
    /// triggers a reload that races the user's next save. Reacting
    /// only to peer files avoids that entirely.
    pub fn snapshot_peers(&self) -> Vec<OpsFileSnapshot> {
        let own = format!("ops-{}.jsonl", self.actor);
        self.snapshot()
            .into_iter()
            .filter(|f| f.name != own)
            .collect()
    }

    /// Find every `.md` under `journals/` and `pages/` that the op
    /// log doesn't reflect yet.
    ///
    /// Two reasons a `.md` ends up orphaned:
    ///
    /// 1. **Bootstrap.** The file was just dropped in by an importer
    ///    (Roam → outl, copy from a Logseq graph), by a peer that
    ///    only ships the projection, or by an external editor like
    ///    vim. No sidecar exists yet.
    /// 2. **External edit.** The user opened the `.md` outside the
    ///    TUI / mobile (vim, VS Code, Finder Quick Look) and saved.
    ///    The sidecar still references the old contents, so
    ///    `last_synced_hash` no longer matches.
    ///
    /// Both look identical to a peer reading via `read_page_view` —
    /// the outline comes out empty or stale. Running `reconcile_md`
    /// on the file resolves both: it emits Create / Move / Edit ops
    /// for whatever the file actually contains and rewrites the
    /// sidecar.
    ///
    /// This call is **read-only** and cheap (one `file_hash` per
    /// `.md`, one sidecar JSON parse). Safe to run on a background
    /// thread; clients call `outl_md::reconcile::reconcile_md` on
    /// the main thread once they have the workspace handle available.
    pub fn scan_for_orphans(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for sub in ["journals", "pages"] {
            scan_dir(&self.workspace_root.join(sub), &mut out);
        }
        out
    }

    /// Find every `.md` whose sidecar is hash-in-sync with the file
    /// but references ids the materialised tree has never seen — the
    /// "projection ran ahead of the op log" state
    /// [`Self::scan_for_orphans`] is structurally blind to (the hash
    /// gate says "in sync" forever, so the blocks stay invisible to
    /// the CRDT and to every peer).
    ///
    /// Needs the materialised tree, hence the `&Workspace` parameter
    /// the hash-only scan doesn't take. Pages this flags go through
    /// [`crate::desync::recover_desynced_projection`], not
    /// `reconcile_md`.
    pub fn scan_for_desynced_projections(&self, ws: &Workspace) -> Vec<PathBuf> {
        crate::desync::scan_for_desynced_projections(ws, &self.workspace_root)
    }
}

fn scan_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_dir(&path, out);
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        if needs_reconcile(&path) {
            out.push(path);
        }
    }
}

/// `true` when the `.md` at `md_path` is not reflected in its
/// sidecar: either the sidecar is missing, its `last_synced_hash`
/// doesn't match the file's current hash, or page-level properties
/// (`type::`, `pinned::`, `icon::`, …) haven't been propagated into
/// the op log yet (legacy sidecars predating
/// `diff_to_ops_with_page_props`).
///
/// The version check is a forward-compatible migration trigger.
/// Every bump of [`outl_md::sidecar::CURRENT_PIPELINE_VERSION`]
/// forces every legacy sidecar (with a lower value, including the
/// default `0` from `#[serde(default)]` on payloads written before
/// the field existed) through `reconcile_md` once.
/// The reconcile emits the missing ops on the page root and stamps
/// the new version in the rewritten sidecar.
/// Subsequent scans skip the page until the next pipeline bump.
/// Without this, pages authored via fixtures, imports, or external
/// editors keep their `type:: person` only in the rendered `.md`,
/// and the desktop's `@` autocomplete (which reads from the CRDT
/// tree) silently disagrees with the TUI's (which reads
/// `WorkspaceIndex`'s parse of the same `.md`).
///
/// **The hash gate here is safe in the only direction it runs, and it is
/// also this codebase's oldest blind spot.**
/// Safe: everything downstream of a `true` moves `.md → tree`
/// (`reconcile_md`), so a false positive costs a redundant reconcile and
/// can never overwrite a `.md` from the tree.
/// Blind: a hash-faithful `.md` holding content that exists in **no op**
/// reads as "in sync" and is never queued — which is why RFC 0210's 233
/// pages went unnoticed for months while every integrity surface agreed
/// they were healthy. Answering that question needs the sidecar's blocks
/// (`outl_md::unlogged::content_lines_missing_from`), not its hash.
/// It deliberately stays out of this scan: the fix is to emit ops for
/// content the log has never seen, which is a write, not a repair, so it
/// lives behind the opt-in `outl reconcile --ahead-of-log` and is
/// *reported* by `outl doctor`. Widening this predicate would make every
/// boot silently author ops on the user's behalf.
fn needs_reconcile(md_path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(md_path) else {
        return false;
    };
    let current = outl_md::sidecar::file_hash(&text);
    let sidecar_path = outl_md::resolve_sidecar_path(md_path);
    match outl_md::sidecar::read(&sidecar_path) {
        Ok(sc) => {
            sc.last_synced_hash != current
                || sc.pipeline_version < outl_md::sidecar::CURRENT_PIPELINE_VERSION
        }
        Err(_) => true, // sidecar missing or unreadable → orphan
    }
}

#[cfg(test)]
mod tests;
