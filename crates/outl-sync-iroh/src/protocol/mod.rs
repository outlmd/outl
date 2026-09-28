//! Wire protocol for the outl sync ALPN.
//!
//! ALPN: `b"outl-sync/3"`
//!
//! ## Sync request (JSON, 4-byte length prefix)
//!
//! Sent by the side that wants to pull:
//! ```json
//! {
//!   "workspace_id": "my-workspace",
//!   "vector_clock": {
//!     "<actor-ulid>": {
//!       "max": { "physical_ms": 1234567890123, "logical": 5, "actor": "<ulid>" },
//!       "count": 347
//!     }
//!   }
//! }
//! ```
//!
//! ## Response (JSON, 4-byte length prefix)
//!
//! Sent by the responder right after it decodes the request, carrying the
//! responder's own vector clock so the initiator can compute the reverse
//! delta. Same `{ actor → ActorClock }` shape as the request's `vector_clock`.
//!
//! ## Ops blob (JSONL, 4-byte length prefix)
//!
//! A length-prefixed batch of newline-separated `LogOp` JSON lines. Used in
//! both directions so a single bi stream can carry two independent op batches
//! without EOF framing ambiguity.
//!
//! ## Bidirectional exchange (single bi stream)
//!
//! 1. initiator → responder: [`SyncRequest`] (vector clock A).
//! 2. responder → initiator: [`SyncResponse`] (vector clock B).
//! 3. responder → initiator: ops blob — ops missing under clock A (per-actor:
//!    everything above `A[actor].max`, or the actor's FULL log when a gap
//!    below `A[actor].max` is detected — see `engine_sync::ops_missing_for`).
//! 4. initiator → responder: ops blob — same rule under clock B, then
//!    `finish()`.
//! 5. responder → initiator: [`ACK_DURABLE`], written only after the batch is
//!    fsynced, then `finish()`.
//!
//! Every step is length-prefixed, so both directions fully reconcile on one
//! stream — and step 5 is why the CONNECTION survives it. Confirming by
//! closing (v2) meant a fresh QUIC connect per sync; see [`crate::peer_conn`].
//!
//! ## Which file owns what
//!
//! - `alpn` — which conversations this endpoint accepts, and at which version.
//! - `close` — how a connection ended, and what the user is told about it.
//! - `frame` — the 4-byte length prefix, with no opinion on the body.
//! - `delta` — the sync body above: vector clocks, op batches, the durable ack.
//! - `asset` — the asset manifest, the one payload that is neither.
//!
//! The submodules are private and everything is re-exported here, so every
//! caller keeps writing `crate::protocol::<item>` and no wire byte depends on
//! where the code lives.

mod alpn;
mod asset;
mod close;
mod delta;
mod frame;

pub use alpn::{ASSET_ALPN, PAIRING_ALPN, SNAPSHOT_ALPN, SYNC_ALPN};
pub(crate) use asset::{decode_asset_manifest, encode_asset_manifest};
pub(crate) use close::{
    classify_close, close_refusal_reason, CloseVerdict, CLOSE_NORMAL, CLOSE_UNKNOWN_PEER,
    CLOSE_WORKSPACE_MISMATCH,
};
pub(crate) use delta::{
    decode_ops_blob, decode_request, decode_response, encode_op, encode_ops_blob, encode_request,
    encode_response, ActorClock, SyncRequest, SyncResponse, ACK_DURABLE,
};
pub(crate) use frame::encode_blob_frame;
