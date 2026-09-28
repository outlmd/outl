//! How a connection ended, and what the user should be told about it.
//!
//! Two vocabularies meet here. The close *codes* are what a responder writes on
//! the wire when it refuses a dialer before any payload, and the *verdict* is
//! what the initiator makes of an ended connection afterwards.
//!
//! Both are values rather than inline `match`es on purpose: every arm is silent
//! when it is wrong (a misclassified close still returns the same error and
//! still re-pushes, so nothing fails and the user is merely told the wrong
//! thing), so the decision table has to be something a test can enumerate
//! without standing up real QUIC.

/// Close code for "I am ending this connection normally" — a shutdown, a
/// finished snapshot or asset transfer, a completed pairing, a status probe.
///
/// It used to mean "durably ingested your push", which is why it was named
/// `CLOSE_DONE`. That moved onto the stream as [`ACK_DURABLE`] so the
/// connection could survive the exchange, and a close code that no longer
/// confirms anything should not keep a name that says it does.
///
/// [`ACK_DURABLE`]: super::ACK_DURABLE
pub(crate) const CLOSE_NORMAL: u32 = 0;

/// Close code: the dialer belongs to a different workspace (its
/// `SyncRequest.workspace_id` does not match ours). Sent before any payload.
pub(crate) const CLOSE_WORKSPACE_MISMATCH: u32 = 3;

/// Close code: the dialer is not in our `peers.json` — unpaired, or revoked
/// on this side. Sent before any payload.
pub(crate) const CLOSE_UNKNOWN_PEER: u32 = 4;

/// What a peer's close means for the pass that just ran.
///
/// The distinction is the difference between "your sync is broken" and "your
/// phone locked", and only one of those is worth a red row in the UI. There is
/// no success variant: success is reading [`ACK_DURABLE`] off the stream, and
/// by the time anyone asks this question that read has already failed.
///
/// [`ACK_DURABLE`]: super::ACK_DURABLE
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloseVerdict {
    /// The peer stopped answering rather than refusing — OS suspension, sleep,
    /// a dropped carrier-NAT flow, or a clean shutdown on its side. Expected,
    /// transient, retried next tick.
    Interrupted,
    /// The peer answered and said no, or the transport itself failed. A real
    /// problem the user may have to act on.
    Failed,
}

/// Classify how a connection ended.
///
/// Split out of `delta_sync` so the table is a value one test can enumerate
/// rather than a `match` reachable only over real QUIC. Getting a variant into
/// the wrong bucket is silent by construction: both verdicts return the same
/// error and re-push, so nothing fails, the user just sees the wrong colour.
pub(crate) fn classify_close(err: &iroh::endpoint::ConnectionError) -> CloseVerdict {
    use iroh::endpoint::ConnectionError;
    match err {
        ConnectionError::TimedOut
        | ConnectionError::Reset
        | ConnectionError::LocallyClosed
        | ConnectionError::ConnectionClosed(_) => CloseVerdict::Interrupted,
        // A peer shutting down cleanly is going away, not refusing us.
        ConnectionError::ApplicationClosed(ac) if ac.error_code == CLOSE_NORMAL.into() => {
            CloseVerdict::Interrupted
        }
        _ => CloseVerdict::Failed,
    }
}

/// Human-readable reason a peer refused this connection, or `None` when it
/// ended some other way — still open, timed out, reset, or closed with a code
/// that is not a refusal.
///
/// A refusal reaches the initiator as a failed *read*, because the responder
/// closes before writing a byte. Without translating the code, the one failure
/// a user genuinely has to act on — this device is no longer paired — arrives
/// as an unexplained dead peer.
pub(crate) fn close_refusal_reason(conn: &iroh::endpoint::Connection) -> Option<&'static str> {
    let iroh::endpoint::ConnectionError::ApplicationClosed(ac) = conn.close_reason()? else {
        return None;
    };
    match u32::try_from(u64::from(ac.error_code)).ok()? {
        CLOSE_WORKSPACE_MISMATCH => Some("peer refused: different workspace"),
        CLOSE_UNKNOWN_PEER => Some("peer refused: this device is not paired with it"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::endpoint::ConnectionError;

    fn app_close(code: u32) -> ConnectionError {
        ConnectionError::ApplicationClosed(iroh::endpoint::ApplicationClose {
            error_code: code.into(),
            reason: Vec::new().into(),
        })
    }

    /// The whole decision table, enumerated.
    ///
    /// Every arm here is silent when it is wrong: a misclassified close still
    /// returns the same error and still re-pushes, so nothing fails and no
    /// test over real QUIC would notice. The only symptom is the user being
    /// told the wrong thing — which is exactly the bug this table was added to
    /// fix, so the table is the thing worth pinning.
    #[test]
    fn close_classification_covers_every_variant() {
        // Code 0 is a normal close, NOT a confirmation: v3 moved durable
        // ingest onto the stream as `ACK_DURABLE`, so a peer closing with 0
        // is just going away cleanly — amber, retried, never a red row.
        assert_eq!(
            classify_close(&app_close(CLOSE_NORMAL)),
            CloseVerdict::Interrupted
        );

        // The peer went away mid-exchange. A locked phone, a sleeping laptop,
        // a dropped carrier-NAT flow. Amber, retried, not the user's problem.
        for err in [
            ConnectionError::TimedOut,
            ConnectionError::Reset,
            ConnectionError::LocallyClosed,
        ] {
            assert_eq!(
                classify_close(&err),
                CloseVerdict::Interrupted,
                "{err:?} is a peer going away, not a peer refusing"
            );
        }

        // The peer answered and said no, or the two builds cannot talk. Red.
        assert_eq!(
            classify_close(&app_close(CLOSE_WORKSPACE_MISMATCH)),
            CloseVerdict::Failed
        );
        assert_eq!(
            classify_close(&app_close(CLOSE_UNKNOWN_PEER)),
            CloseVerdict::Failed
        );
        assert_eq!(
            classify_close(&ConnectionError::VersionMismatch),
            CloseVerdict::Failed
        );

        // An application code we do not recognise is Failed. The peer answered
        // with something this version has no meaning for, and guessing is how
        // the desktop→mobile "synced ok but nothing arrived" bug happened.
        assert_eq!(classify_close(&app_close(9)), CloseVerdict::Failed);
    }
}
