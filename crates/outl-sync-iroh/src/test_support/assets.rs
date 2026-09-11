//! Test hooks for the binary-asset protocol (`ASSET_ALPN`).
//!
//! Split out of the parent module so both stay under the file-size guard, and
//! because they share a question the rest of `test_support` does not: the asset
//! transfer is the only protocol on the endpoint that serves **many** payloads
//! over one connection, so it is the only one where "may this dialer be served?"
//! has to be re-asked mid-stream. [`run_staged_asset_pull`] exists for exactly
//! that, and lives next to the helpers it shares a workspace layout with.

use std::path::Path;

use anyhow::{Context, Result};
use iroh::protocol::Router;

use super::authorize_peer;
use crate::engine_sync::read_frame;
use crate::protocol::{encode_blob_frame, ASSET_ALPN};

/// Mount the production `AssetProtocolHandler` on a `Router` and return it.
///
/// The responder serves `<workspace_root>/assets/` (the manifest + per-name
/// bytes) on [`crate::ASSET_ALPN`] to an **approved** dialer — an empty manifest
/// when the dir is absent. `authorized_peers` is seeded into the responder's
/// `peers.json` before the handler goes up, exactly like
/// [`super::spawn_responder`]; pass an empty slice to stand up a responder that trusts
/// nobody, which is how the disclosure tests prove a revoked device gets
/// nothing.
///
/// Keep the returned `Router` alive for as long as it must accept connections.
/// Lets a loopback test exercise the real binary-asset transfer (server side)
/// over real QUIC.
pub fn spawn_asset_responder(
    endpoint: iroh::Endpoint,
    workspace_root: &Path,
    authorized_peers: &[iroh::EndpointId],
) -> Router {
    for peer in authorized_peers {
        authorize_peer(workspace_root, *peer);
    }
    let workspace_root = workspace_root.to_path_buf();
    Router::builder(endpoint)
        .accept(
            crate::protocol::ASSET_ALPN,
            crate::engine_assets::AssetProtocolHandler { workspace_root },
        )
        .spawn()
}

/// Write a content-addressed asset (`<root>/assets/<hash>.<ext>`) exactly like
/// `outl_actions::import_asset` would, and return its basename.
///
/// Lets a loopback test seed a peer's `assets/` without pulling `outl-actions` /
/// `outl-md` into the test crate's own dependency set — the filename IS the
/// sha-256 of `bytes`, so the puller's content-hash check passes.
pub fn write_test_asset(workspace_root: &Path, bytes: &[u8], ext: &str) -> String {
    let dir = outl_actions::assets_dir(workspace_root);
    std::fs::create_dir_all(&dir).expect("create assets dir");
    let name = format!("{}.{ext}", outl_md::asset::hash_bytes(bytes));
    std::fs::write(dir.join(&name), bytes).expect("write test asset");
    name
}

/// Run the production asset pull (initiator side) against `peer` — the exact call
/// `drain_pair_completions` / the catch-up loop make after the delta-sync.
///
/// Dials `peer` on [`crate::ASSET_ALPN`], negotiates the manifest, and writes
/// every asset the peer holds that `workspace_root/assets/` lacks (atomically,
/// content-hash-verified). Returns how many assets were written.
pub async fn run_asset_pull(
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    workspace_root: &Path,
) -> Result<usize> {
    crate::engine_assets::pull_assets_from_peer(
        endpoint,
        peer.into(),
        workspace_root,
        &crate::progress::ProgressSink::default(),
    )
    .await
}

/// Drive the asset protocol by hand, pausing after every blob.
///
/// [`run_asset_pull`] sends its requests back to back, so it cannot express
/// "the user ran `outl peer remove` while the transfer was still going". This
/// can: it requests the manifest, then one blob per entry in `names`, running
/// `between(i)` after blob `i` lands so a test can rewrite the responder's
/// `peers.json` mid-connection.
///
/// Returns one body per name the responder actually answered, and STOPS at the
/// first unanswered request. A short vector therefore means the responder hung
/// up; a full one with an empty entry means it answered "I don't have that".
/// The two are different verdicts and a test that cannot tell them apart would
/// pass against a responder that leaks every blob and calls the last one empty.
pub async fn run_staged_asset_pull(
    endpoint: &iroh::Endpoint,
    peer: impl Into<iroh::EndpointAddr>,
    names: &[String],
    mut between: impl FnMut(usize),
) -> Result<Vec<Vec<u8>>> {
    let conn = endpoint
        .connect(peer.into(), ASSET_ALPN)
        .await
        .context("staged asset connect")?;
    let (mut send, mut recv) = conn.open_bi().await.context("open asset bi stream")?;
    send.write_all(&encode_blob_frame(&[])?)
        .await
        .context("send asset manifest request")?;
    read_frame(&mut recv).await.context("read asset manifest")?;

    let mut bodies = Vec::new();
    for (i, name) in names.iter().enumerate() {
        if send
            .write_all(&encode_blob_frame(name.as_bytes())?)
            .await
            .is_err()
        {
            break;
        }
        match read_frame(&mut recv).await {
            Ok(frame) => bodies.push(frame[4..].to_vec()),
            Err(_) => break,
        }
        between(i);
    }
    Ok(bodies)
}
