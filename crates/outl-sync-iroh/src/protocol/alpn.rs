//! Which conversations this endpoint will accept, and at which version.
//!
//! One iroh endpoint per device answers four ALPNs, and an ALPN string is the
//! only negotiation this protocol has: there is no capability handshake and no
//! compat shim, so a version bump is how two builds that must not talk are made
//! to fail at connect instead of half-conversing. The rationale for each bump
//! lives on the constant it bumped.
//!
//! The four are re-exported from the crate root, so they are the one part of
//! `protocol` that is public API.

/// ALPN for the op-sync protocol.
///
/// v2 bumped the vector clock from a bare max-HLC per actor to
/// `ActorClock` (max + count) so the sender can detect gaps below the
/// receiver's watermark. v1 and v2 clocks are wire-incompatible; the ALPN
/// bump makes an old↔new dial fail cleanly at connect instead of
/// half-conversing.
/// v3 moves the durable-ingest confirmation from the connection close code
/// onto the stream (`ACK_DURABLE`), so a connection outlives the exchange
/// and can be pooled. A v2 peer confirms by closing and a v3 peer waits for a
/// frame that never arrives, so the two must not talk: the ALPN bump makes
/// that a clean connect failure instead of a 30s hang on every sync.
pub const SYNC_ALPN: &[u8] = b"outl-sync/3";

/// ALPN for device pairing.
pub const PAIRING_ALPN: &[u8] = b"outl-sync/pair/1";

/// ALPN for peer snapshot transfer (Phase 2 snapshot sync).
///
/// A freshly-paired device pulls a peer's materialized snapshot
/// (`snap-<actor>.bin`) over this ALPN so it can boot from settled state
/// instead of receiving + replaying the full op log. Carried on the SAME sync
/// endpoint's router (one endpoint per identity). See `crate::engine_snapshot`.
pub const SNAPSHOT_ALPN: &[u8] = b"outl-snapshot/1";

/// ALPN for peer binary-asset transfer (uploaded files: PDFs, images).
///
/// Asset bytes are content-addressed blobs stored at `<root>/assets/<hash>.<ext>`
/// and NEVER enter the op log (a multi-MB PDF replayed through the CRDT would
/// bloat every device's log). The `file` transport (iCloud / Syncthing) carries
/// them for free; over iroh they must be transferred explicitly. Unlike a
/// snapshot (one blob), assets are N files, so this ALPN negotiates a manifest
/// first (the peer's `assets/` basenames), then the initiator pulls only the
/// files it lacks. Carried on the SAME sync endpoint's router (one endpoint per
/// identity). See `crate::engine_assets`.
pub const ASSET_ALPN: &[u8] = b"outl-asset/1";
