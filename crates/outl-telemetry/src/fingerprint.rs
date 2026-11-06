//! Splits distinct panic messages into distinct Sentry issues.
//!
//! Sentry's default grouping hashes the stack trace, and a release the
//! symbol server has never seen (a build whose debug files did not upload,
//! or a platform `sentry-cli` cannot read) has no usable stack: every frame
//! is a bare address, so the default collapses onto release + culprit alone.
//! `OUTL-4` is the case in point — a WebView2 startup message and an
//! unrelated `index out of bounds` panic landed in one issue because neither
//! symbolicated.
//!
//! Split out of `scrub.rs` because it is one self-contained decision (what
//! the fingerprint is) rather than part of the allowlist `scrub_event`
//! enforces field by field.

use std::borrow::Cow;

use sentry::protocol::Event;

/// Sets a panic-specific fingerprint on `event`, replacing whatever
/// [`crate::scrub::scrub_event`] reset it to.
///
/// The fingerprint is a dedicated `("outl-panic", <value>)` pair rather than
/// `["{{ default }}", value]`: the default component still hashes whatever
/// stack *is* present, so appending to it would keep issues split by build
/// metadata that has nothing to do with which bug this is, and split one bug
/// across releases when it does symbolicate. The exception's `value` is the
/// one field that names the panic, and the caller must have already
/// redacted it: this function reads whatever is there and does not call
/// [`crate::redact::redact`] itself, so the fingerprint carries the scrubbed
/// string, never the raw one a leaking panic message might hold.
///
/// Events with no exception, or one whose value redacted to empty, keep
/// whatever fingerprint the caller set before this ran — grouping those by
/// release + culprit is the behavior this function has nothing to add to.
pub(crate) fn set_panic_fingerprint(event: &mut Event<'static>) {
    let Some(value) = event
        .exception
        .values
        .first()
        .and_then(|exception| exception.value.as_deref())
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    event.fingerprint = Cow::Owned(vec![
        Cow::Borrowed("outl-panic"),
        Cow::Owned(value.to_string()),
    ]);
}
