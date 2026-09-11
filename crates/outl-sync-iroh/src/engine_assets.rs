//! Asset sync — peer-to-peer binary-asset (uploaded file) transfer.
//!
//! Uploaded files (a PDF, an image) are copied into `<root>/assets/<hash>.<ext>`
//! and referenced from markdown as `[name](assets/<hash>.<ext>)`. Their bytes
//! NEVER enter the op log (a multi-MB blob replayed through the CRDT would bloat
//! every device's log irreversibly — see `outl_actions::asset`). They are plain
//! content-addressed blobs, replicated like the `.md` projections: the `file`
//! transport (iCloud / Syncthing) carries them for free, but over the default
//! iroh (p2p) transport they must be transferred explicitly. This module is that
//! transport, mirroring [`crate::engine_snapshot`] for a *set* of blobs.
//!
//! Because a device holds N assets (not one, like a snapshot), the protocol
//! negotiates a **manifest** first:
//!
//! 1. The initiator opens a bi stream on [`ASSET_ALPN`] and sends an empty
//!    manifest-request marker frame.
//! 2. The responder ([`AssetProtocolHandler`]) replies with an
//!    [`AssetManifest`](crate::protocol::encode_asset_manifest) — the basenames
//!    in its `assets/` dir. Names ARE content hashes, so two devices name the
//!    same content identically; a name the initiator already holds needs no
//!    transfer.
//! 3. The initiator diffs the manifest against its own `assets/` and, for each
//!    missing file, sends an [`AssetRequest`](crate::protocol::encode_blob_frame)
//!    (the name) and reads back the bytes as a blob frame, writing each
//!    atomically (tmp + rename). After the last request it finishes the stream.
//! 4. The responder re-authorizes the dialer, then serves each requested name
//!    (validated: plain basename, no `/` or `..`) until the initiator finishes,
//!    then closes. The re-check is per REQUEST, not per connection — this is
//!    the only protocol on the endpoint that hands over an unbounded number of
//!    payloads over one connection, so anything coarser lets a peer approved at
//!    connect time keep reading long after `outl peer remove`.
//!
//! Like the snapshot transfer, neither side holds the workspace lock — assets are
//! immutable content-addressed cache files read straight off disk — and every
//! failure is best-effort: an absent, unreadable, or hash-mismatched asset is
//! skipped, never fatal (the op log stays source of truth; a link to a
//! not-yet-transferred asset just renders as a dead link until the next pull).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use tracing::{debug, info, warn};

use outl_actions::{assets_dir, SyncProgress};
use outl_md::asset::is_safe_asset_name;

use crate::engine_sync::{read_frame, read_frame_reporting};
use crate::protocol::{
    decode_asset_manifest, encode_asset_manifest, encode_blob_frame, ASSET_ALPN,
};

/// Bound on a single asset-transfer connect attempt. Mirrors
/// [`crate::engine_snapshot`]'s `SNAPSHOT_CONNECT_TIMEOUT`: iroh 1.0.0 multipath
/// can stall ~30s on a dead direct addr, so each attempt is capped and the
/// bare-id (relay/discovery) fallback takes over.
const ASSET_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Monotonic sequence for temp-file names, so two concurrent pulls of the same
/// asset (a pair + a catch-up tick landing together) never share a tmp path.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The content hash a file named `<hash>.<ext>` (or bare `<hash>`) claims, as the
/// stem before the first `.`. A sha-256 hex hash carries no dot, so this always
/// isolates it.
fn claimed_hash(name: &str) -> &str {
    name.split_once('.').map(|(h, _)| h).unwrap_or(name)
}

/// List the safe, transferable asset basenames in `<root>/assets/`.
///
/// Skips the dir entirely when absent (a workspace with no uploads yet → empty
/// manifest). Skips non-files, dotfiles, and `*.tmp` (in-flight writes from
/// `import_asset` / a concurrent pull), and any name that fails the
/// anti-traversal guard.
async fn list_asset_names(workspace_root: &Path) -> Vec<String> {
    let dir = assets_dir(workspace_root);
    let mut names = Vec::new();
    let mut rd = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(_) => return names, // absent dir → empty manifest
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        if let Ok(ft) = entry.file_type().await {
            if !ft.is_file() {
                continue;
            }
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') || name.ends_with(".tmp") {
            continue;
        }
        if is_safe_asset_name(&name) {
            names.push(name);
        }
    }
    names
}

/// Write `bytes` to `<dir>/<name>` atomically (unique tmp + rename), idempotent.
///
/// A pre-existing file is left untouched (content-addressed: the bytes are
/// identical). A rename race — a concurrent pull landed the same file first — is
/// treated as success, not an error.
///
/// The scratch file is owned by an [`outl_md::atomic::TempFile`] guard rather
/// than by hand-written `remove_file` calls. The hand-written version cleaned up
/// only inside the `rename` arm, which left the scratch behind on a failed write
/// **and** on task cancellation between the write and the rename — a routine
/// outcome here, not an edge case: a pull loop is dropped whenever the transport
/// shuts down or the peer connection times out mid-transfer. A guard is the only
/// thing that covers cancellation, because a dropped future runs `Drop` and
/// nothing else.
///
/// The temp is `fsync`ed before the rename. Without it a power loss can publish
/// a truncated file under the *destination* name, and since the caller skips any
/// name already present, that corrupt asset is never re-pulled. The parent
/// directory is deliberately **not** `fsync`ed: a lost rename is self-healing
/// (the next manifest diff sees the name missing and requests it again), and
/// this runs once per asset inside a bulk pull loop.
async fn write_asset_atomic(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    use tokio::io::AsyncWriteExt as _;

    let dest = dir.join(name);
    if dest.exists() {
        return Ok(());
    }
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let guard = outl_md::atomic::TempFile::new(
        dir.join(format!(".{name}.{}.{seq}.pull.tmp", std::process::id())),
    );
    let tmp = guard.path().to_path_buf();

    let mut file = tokio::fs::File::create(&tmp)
        .await
        .with_context(|| format!("create asset tmp {}", tmp.display()))?;
    file.write_all(bytes)
        .await
        .with_context(|| format!("write asset tmp {}", tmp.display()))?;
    file.sync_all()
        .await
        .with_context(|| format!("fsync asset tmp {}", tmp.display()))?;
    drop(file);

    if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
        // The guard unlinks the scratch on the way out. If another concurrent
        // pull already landed the file (identical content-addressed bytes)
        // that's success, not failure.
        if dest.exists() {
            return Ok(());
        }
        return Err(e).with_context(|| format!("rename asset into {}", dest.display()));
    }
    guard.keep();
    Ok(())
}

/// Router handler that serves this device's `assets/` to an **authorized**
/// dialing peer.
///
/// Content-addressed and device-agnostic — unlike the snapshot handler, there is
/// no per-actor scoping: a device serves whatever assets it holds, to anyone in
/// this workspace's `peers.json` (checked by [`crate::authz`] before the
/// manifest goes out, and again before each blob). Holds no workspace lock
/// (assets are immutable cache files read straight off disk).
#[derive(Clone)]
pub(crate) struct AssetProtocolHandler {
    /// Local workspace root, so the handler can resolve `<root>/assets/`.
    pub(crate) workspace_root: PathBuf,
}

impl std::fmt::Debug for AssetProtocolHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssetProtocolHandler")
            .field("workspace_root", &self.workspace_root)
            .finish()
    }
}

impl ProtocolHandler for AssetProtocolHandler {
    async fn accept(&self, conn: Connection) -> Result<(), AcceptError> {
        if let Err(e) = self.serve(conn).await {
            warn!("asset serve failed: {e:#}");
            return Err(AcceptError::from_boxed(e.into()));
        }
        Ok(())
    }
}

impl AssetProtocolHandler {
    /// Send the manifest, then serve per-name byte requests until the initiator
    /// finishes its send stream.
    async fn serve(&self, conn: Connection) -> Result<()> {
        // The manifest names every uploaded file this device holds and the loop
        // below hands over their bytes. Both went to ANY dialer speaking the
        // ALPN, so a device the user revoked kept pulling every PDF and image in
        // the workspace long after `outl peer remove` said otherwise. Authorize
        // FIRST — before the manifest, which is itself a disclosure (it leaks
        // how many assets exist and their content hashes). Same owner as the
        // sync handler's check — see `crate::authz`.
        if !crate::authz::authorize_or_close(&conn, &self.workspace_root).await {
            return Ok(());
        }

        let (mut send, mut recv) = conn.accept_bi().await.context("accept asset bi stream")?;

        // 1. Drain the manifest-request marker frame (its content is irrelevant —
        //    the ALPN itself means "tell me what assets you have").
        let _marker = read_frame(&mut recv)
            .await
            .context("read asset manifest request")?;

        // 2. Ship the manifest (basenames we hold; empty when we have none).
        let names = list_asset_names(&self.workspace_root).await;
        send.write_all(&encode_asset_manifest(&names)?)
            .await
            .context("send asset manifest")?;

        // 3. Serve each requested name. The initiator sends one name frame per
        //    asset it lacks and reads back one blob frame; when it finishes the
        //    stream, `read_frame` errors (clean EOF) and we stop. We ALWAYS reply
        //    with exactly one frame per request so the ping-pong stays aligned —
        //    an unknown / unsafe / unreadable name yields an empty frame the
        //    initiator skips.
        let dir = assets_dir(&self.workspace_root);
        loop {
            let frame = match read_frame(&mut recv).await {
                Ok(f) => f,
                // Clean EOF (initiator finished) or the peer went away — either
                // ends the exchange.
                Err(_) => break,
            };
            // Re-ask, per request, BEFORE reading the blob off disk.
            //
            // One connection serves an unbounded number of blobs at the
            // initiator's pace, so a check that ran only at connect time gave a
            // peer approved a second before `outl peer remove` an open-ended
            // read of `assets/` — the command returned, the user believed it,
            // and the device carried on draining. `SYNC_ALPN` never had this
            // shape: its check lives inside `serve_exchange`, i.e. per exchange
            // on a pooled connection, so a removal lands on the next one. This
            // is that same granularity for a protocol whose "next exchange" is
            // the next frame.
            //
            // The cost is one small `peers.json` read per requested asset,
            // against a QUIC round trip and a blob of up to `[assets] max_bytes`
            // (100 MiB) — and `authz` re-reads the file rather than caching it
            // precisely so a revocation does not wait for a restart.
            if !crate::authz::authorize_or_close(&conn, &self.workspace_root).await {
                return Ok(());
            }

            let requested = std::str::from_utf8(&frame[4..]).ok().map(str::to_string);
            let bytes = match requested {
                Some(name) if is_safe_asset_name(&name) => {
                    // Absent / unreadable → empty reply (the initiator skips it).
                    tokio::fs::read(dir.join(&name)).await.unwrap_or_default()
                }
                Some(name) => {
                    warn!("asset serve: refusing unsafe requested name {name:?}");
                    Vec::new()
                }
                None => {
                    warn!("asset serve: non-UTF-8 asset request; replying empty");
                    Vec::new()
                }
            };
            send.write_all(&encode_blob_frame(&bytes)?)
                .await
                .context("send asset bytes")?;
        }

        send.finish().context("finish asset send")?;
        // Wait for the initiator to close before the endpoint tears the
        // connection down (mirrors the snapshot handler).
        conn.closed().await;
        Ok(())
    }
}

/// Connect to `peer` on [`ASSET_ALPN`], resilient to a stale direct address.
///
/// Mirrors `engine_snapshot::connect_snapshot`: try the full addr (fast on-LAN),
/// fall back to the bare node id via relay / discovery if that stalls or fails
/// and the addr carried a (possibly dead) direct addr.
async fn connect_asset(
    endpoint: &iroh::Endpoint,
    peer_addr: iroh::EndpointAddr,
) -> Result<Connection> {
    let node_id = peer_addr.id;
    let had_direct = peer_addr.ip_addrs().next().is_some();

    match tokio::time::timeout(
        ASSET_CONNECT_TIMEOUT,
        endpoint.connect(peer_addr, ASSET_ALPN),
    )
    .await
    {
        Ok(Ok(conn)) => return Ok(conn),
        Ok(Err(e)) if !had_direct => return Err(e).context("asset connect"),
        Err(_) if !had_direct => return Err(anyhow::anyhow!("asset connect timed out")),
        Ok(Err(e)) => debug!(
            "direct asset connect to {} failed ({e}); retrying via relay/discovery",
            node_id.fmt_short()
        ),
        Err(_) => debug!(
            "direct asset connect to {} timed out; retrying via relay/discovery",
            node_id.fmt_short()
        ),
    }

    tokio::time::timeout(ASSET_CONNECT_TIMEOUT, endpoint.connect(node_id, ASSET_ALPN))
        .await
        .context("asset relay/discovery connect timed out")?
        .context("asset connect (relay/discovery)")
}

/// Pull every asset `peer` holds that this device lacks, writing each atomically
/// into `<root>/assets/`. Best-effort and idempotent.
///
/// Returns the number of assets actually written. A peer with no assets (empty
/// manifest) or a diff that finds nothing missing returns `Ok(0)`. A single
/// asset that arrives empty (peer lacks it) or whose bytes don't hash to their
/// name (a corrupt / malicious peer) is skipped, never written — the rest still
/// transfer.
pub(crate) async fn pull_assets_from_peer(
    endpoint: &iroh::Endpoint,
    peer: iroh::EndpointAddr,
    workspace_root: &Path,
    progress: &crate::progress::ProgressSink,
) -> Result<usize> {
    let peer_node_id = peer.id;
    let peer_short = peer_node_id.fmt_short().to_string();
    let conn = connect_asset(endpoint, peer).await?;
    let (mut send, mut recv) = conn.open_bi().await.context("open asset bi stream")?;

    // 1. Request the manifest (empty marker frame establishes the stream).
    send.write_all(&encode_blob_frame(&[])?)
        .await
        .context("send asset manifest request")?;
    // 2. Read the manifest.
    let manifest = read_frame(&mut recv).await.context("read asset manifest")?;
    let names = decode_asset_manifest(&manifest)?;

    // 3. Diff against local `assets/`: keep only safe names we don't already
    //    hold (content-addressed → a name match means we have the bytes).
    let dir = assets_dir(workspace_root);
    let wanted: Vec<String> = names
        .into_iter()
        .filter(|name| is_safe_asset_name(name) && !dir.join(name).exists())
        .collect();

    if wanted.is_empty() {
        send.finish().context("finish asset request stream")?;
        conn.close(0u32.into(), b"done");
        return Ok(0);
    }

    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("create assets dir {}", dir.display()))?;

    let mut written = 0usize;
    for name in &wanted {
        // Request this asset's bytes.
        send.write_all(&encode_blob_frame(name.as_bytes())?)
            .await
            .context("send asset request")?;
        // Read the bytes. An asset can be up to the `[assets] max_bytes` cap
        // (100 MiB default), so report byte progress throttled to ~256 KiB steps
        // — the same honest-percentage feed the snapshot pull emits.
        let mut last_emitted = 0u64;
        let frame = read_frame_reporting(&mut recv, |received, total| {
            if total > 0
                && (received == 0 || received >= total || received - last_emitted >= 256 * 1024)
            {
                last_emitted = received;
                progress.emit(SyncProgress::Asset {
                    peer: peer_short.clone(),
                    received,
                    total,
                });
            }
        })
        .await
        .context("read asset bytes")?;

        let body = &frame[4..];
        if body.is_empty() {
            debug!("asset pull: peer {peer_short} lacks {name}");
            continue;
        }
        // Defense in depth against a corrupt / malicious peer: the filename IS
        // the content's sha-256, so recompute and compare before landing it. A
        // mismatch means the bytes are not what the name claims — discard.
        if outl_md::asset::hash_bytes(body) != claimed_hash(name) {
            warn!("asset pull: {name} from {peer_short} failed content-hash check; discarding");
            continue;
        }
        write_asset_atomic(&dir, name, body).await?;
        written += 1;
    }

    send.finish().context("finish asset request stream")?;
    conn.close(0u32.into(), b"done");
    if written > 0 {
        info!(
            "asset pull: wrote {written}/{} assets from {peer_short}",
            wanted.len()
        );
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    // `is_safe_asset_name` is owned + tested in `outl_md::asset`; this crate
    // consumes it as the single anti-traversal validator.

    /// Every `*.tmp` sibling in `dir`, whatever it is called.
    fn leftover_temps(dir: &Path) -> Vec<PathBuf> {
        let mut found: Vec<_> = std::fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "tmp"))
            .collect();
        found.sort();
        found
    }

    #[tokio::test]
    async fn a_successful_pull_leaves_no_scratch_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_asset_atomic(dir.path(), "abc.bin", b"payload")
            .await
            .expect("write");
        assert_eq!(
            std::fs::read(dir.path().join("abc.bin")).expect("read back"),
            b"payload"
        );
        assert_eq!(leftover_temps(dir.path()), Vec::<PathBuf>::new());
    }

    /// The path a plain error test misses: the pull future is **dropped**
    /// between creating the scratch file and renaming it, which is what
    /// happens every time the transport shuts down or a peer connection
    /// times out mid-transfer.
    ///
    /// Deterministic, not timing-based: the future is polled by hand until
    /// the scratch file is observably on disk, and only then dropped. Before
    /// the [`outl_md::atomic::TempFile`] guard, nothing ran on that path at
    /// all — `Drop` is the only thing a cancelled future executes.
    #[tokio::test]
    async fn a_cancelled_pull_leaves_no_scratch_file() {
        use std::future::Future as _;

        let dir = tempfile::tempdir().expect("tempdir");
        // Large enough that `write_all` + `sync_all` stay pending for a while
        // after `File::create` has already put the scratch file on disk.
        let bytes = vec![0xABu8; 8 * 1024 * 1024];

        let mut fut = Box::pin(write_asset_atomic(dir.path(), "abc.bin", &bytes));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());

        let mut saw_scratch = false;
        for _ in 0..5_000 {
            if fut.as_mut().poll(&mut cx).is_ready() {
                break;
            }
            if !leftover_temps(dir.path()).is_empty() {
                saw_scratch = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            saw_scratch,
            "the scratch file must become observable before the future completes, \
             otherwise this test is not exercising cancellation"
        );

        drop(fut);

        assert_eq!(
            leftover_temps(dir.path()),
            Vec::<PathBuf>::new(),
            "a cancelled pull must not leak its scratch file"
        );
        assert!(
            !dir.path().join("abc.bin").exists(),
            "a cancelled pull must not publish a partial asset"
        );
    }

    #[test]
    fn claimed_hash_isolates_the_stem() {
        assert_eq!(claimed_hash("abc123.pdf"), "abc123");
        assert_eq!(claimed_hash("deadbeef"), "deadbeef");
        // Real names are `<sha256hex>.<ext>` — no dot in the hash itself.
        assert_eq!(claimed_hash("00ff.tar.gz"), "00ff");
    }
}
