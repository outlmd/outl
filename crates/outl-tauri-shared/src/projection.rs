//! Background `.md` + sidecar projection writer.
//!
//! The op log is the source of truth; the `.md` + `.outl` sidecar are
//! projections of it. Re-rendering a page and re-hashing its sidecar on
//! every keystroke-commit is real CPU work (SHA-256 per block + a
//! whole-page render), and doing it on the Tauri IPC thread made the
//! commit block the next keystroke. This moves it off that thread.
//!
//! **Async-by-default (see the outl async-writes principle).** A commit
//! now only mutates the op log (the truth), builds the reply view from
//! the tree, and *queues* the page here; the actual disk write lands a
//! beat later on this worker.
//!
//! ## Why it can't corrupt the `.md`↔sidecar pair
//!
//! A torn projection (a `.md` from write A next to a sidecar from write
//! B) breaks the 3-level matching algorithm and desyncs peers. Two rules
//! prevent it:
//!
//! 1. **One worker, serial.** A single thread drains the queue, so two
//!    projections never run concurrently.
//! 2. **Written under the workspace lock.** Every projection path in the
//!    app (`finish_in_page`, templates, exec, …) writes the `.md` +
//!    sidecar while holding the workspace `Mutex`. This worker takes the
//!    same lock, so its write is mutually exclusive with any synchronous
//!    projection too — the pair is always rendered + written from one
//!    consistent tree snapshot.
//!
//! ## Coalescing
//!
//! `apply_page_md_with_sidecar` re-renders from the *current* tree, so a
//! burst of edits to one page collapses to a single write of the final
//! state: the worker drains everything already queued into a dedup set
//! before writing, and a later edit that arrives mid-write just queues
//! another pass.
//!
//! ## Durability
//!
//! A crash with writes still queued leaves the `.md` briefly behind the
//! op log — never data loss, because the op log *is* the truth and the
//! next boot re-projects stale pages (`apply_page_md_with_sidecar_if_stale`
//! on open, plus the orphan scanner). Peers sync ops over iroh, not the
//! `.md`, so a lagging projection never ships a wrong tree.

use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;

use outl_actions::apply_page_md_with_sidecar_guarded;
use outl_core::id::NodeId;
use outl_core::workspace::Workspace;
use parking_lot::Mutex;
use tracing::warn;

use crate::host::StorageRootProvider;
use crate::state::ProjectionWriteFailed;

/// One message on the worker's channel: a page to project, or a
/// synchronization barrier (see [`ProjectionWriter::flush`]).
enum Msg {
    Page(NodeId),
    /// Acked once every `Page` queued *before* this `Flush` has been
    /// written. `std::sync::mpsc` preserves per-sender FIFO order, so a
    /// `Flush` queued after a `Page` is guaranteed to drain behind it.
    Flush(Sender<Result<(), String>>),
}

/// Handle to the background projection worker. Cheap to hold in
/// `AppState`; cloning the `Sender` is how a command queues a write.
pub struct ProjectionWriter {
    tx: Sender<Msg>,
    report_failure: Arc<dyn Fn(ProjectionWriteFailed) + Send + Sync>,
}

impl ProjectionWriter {
    /// Spawn the worker. It owns clones of the same workspace slot every
    /// command locks, plus the client's storage-root provider (a fixed
    /// `PathBuf` on mobile, a swap-capable `Arc<Mutex<Option<PathBuf>>>`
    /// on desktop — both implement [`StorageRootProvider`]). `report_failure`
    /// runs after a refused write so the host can emit it to the webview.
    pub fn spawn<R, F>(workspace: Arc<Mutex<Option<Workspace>>>, root: R, report_failure: F) -> Self
    where
        R: StorageRootProvider,
        F: Fn(ProjectionWriteFailed) + Send + Sync + 'static,
    {
        let (tx, rx) = mpsc::channel::<Msg>();
        let report_failure: Arc<dyn Fn(ProjectionWriteFailed) + Send + Sync> =
            Arc::new(report_failure);
        let worker_report_failure = report_failure.clone();
        thread::Builder::new()
            .name("outl-projection".into())
            .spawn(move || {
                let mut failure_since_flush: Option<String> = None;
                // Block until something is queued, then coalesce every
                // other pending page into one batch. A `Flush` seen while
                // draining ends the batch there (so everything queued
                // ahead of it gets written first) and is acked once that
                // batch's writes land, before the loop goes back to
                // blocking on `recv`.
                while let Ok(first) = rx.recv() {
                    let mut dirty: HashSet<NodeId> = HashSet::new();
                    let mut pending_acks: Vec<Sender<Result<(), String>>> = Vec::new();
                    let drain_until_flush = match first {
                        Msg::Page(id) => {
                            dirty.insert(id);
                            true
                        }
                        Msg::Flush(ack) => {
                            pending_acks.push(ack);
                            false
                        }
                    };
                    if drain_until_flush {
                        while let Ok(more) = rx.try_recv() {
                            match more {
                                Msg::Page(id) => {
                                    dirty.insert(id);
                                }
                                Msg::Flush(ack) => {
                                    pending_acks.push(ack);
                                    break;
                                }
                            }
                        }
                    }
                    if let Some(root) = root.current() {
                        for page in dirty {
                            // Lock per page and drop between writes so a
                            // synchronous command isn't starved during a
                            // large batch. The write (render + sidecar) is
                            // atomic per file and happens under this lock, so
                            // it can't interleave with another projection.
                            let guard = workspace.lock();
                            let Some(ws) = guard.as_ref() else {
                                failure_since_flush =
                                    Some("projection skipped: workspace is loading".into());
                                break;
                            };
                            // Guarded, not the plain write. This worker runs
                            // after a real mutation, so it must write — but
                            // "must write" is not "may delete". A page whose
                            // `.md` carries content the op log never saw is
                            // exactly what `apply_page_md_with_sidecar_if_stale`
                            // refuses on the open path, and writing it here
                            // deleted the same bytes one keystroke later.
                            // The user's edit is safe either way: it went
                            // through `Workspace::apply` and is in the log.
                            // Only the on-disk projection lags, which is the
                            // recoverable direction, and the banner the open
                            // path raises already tells the user why.
                            if let Err(e) = apply_page_md_with_sidecar_guarded(ws, &root, page) {
                                warn!("background projection skipped for {page}: {e}");
                                if failure_since_flush.is_none() {
                                    failure_since_flush = Some(e.to_string());
                                }
                                worker_report_failure(ProjectionWriteFailed::from_error(page, &e));
                            }
                        }
                    } else if !dirty.is_empty() {
                        failure_since_flush = Some(
                            "projection skipped: no workspace storage root is available".into(),
                        );
                    }
                    // Ack every flush collected in this batch now that its
                    // pages (if any) are written. A dropped receiver (the
                    // caller stopped waiting) makes the send a silent no-op.
                    if !pending_acks.is_empty() {
                        let result = failure_since_flush.take().map_or(Ok(()), Err);
                        for ack in pending_acks {
                            let _ = ack.send(result.clone());
                        }
                    }
                }
            })
            .expect("spawning the projection writer thread should not fail");
        Self { tx, report_failure }
    }

    /// Surface a synchronous projection failure through the same bridge the
    /// worker uses, without changing the already-committed command result.
    pub fn report_failure(&self, page: NodeId, error: &outl_actions::ActionError) {
        (self.report_failure)(ProjectionWriteFailed::from_error(page, error));
    }

    /// Queue a page for background `.md` + sidecar projection.
    ///
    /// Coalesced: repeated queues of the same page before the worker
    /// catches up collapse into one write of the current tree. A dropped
    /// receiver (worker gone at shutdown) is a silent no-op — the next
    /// boot re-projects from the op log.
    pub fn queue(&self, page: NodeId) {
        let _ = self.tx.send(Msg::Page(page));
    }

    /// Block until every page queued **before** this call has been
    /// written to disk.
    ///
    /// Exists because a caller that needs the on-disk `.md` + sidecar to
    /// reflect the current tree — `outl_actions::restore_page_md`'s
    /// reconcile-from-disk step, which `undo_page` / `redo_page` run, is
    /// the concrete case — cannot assume the background worker has
    /// caught up just because `queue()` returned; `queue()` only proves
    /// the page was *accepted*, not written. Returns the first queued
    /// projection failure, and returns an error if the worker has stopped.
    pub fn flush(&self) -> Result<(), String> {
        let (ack_tx, ack_rx) = mpsc::channel::<Result<(), String>>();
        self.tx
            .send(Msg::Flush(ack_tx))
            .map_err(|_| "projection writer stopped before flush".to_string())?;
        ack_rx
            .recv()
            .map_err(|_| "projection writer stopped during flush".to_string())?
    }
}
