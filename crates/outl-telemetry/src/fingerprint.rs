//! Groups panics by their redacted message instead of their stack trace.
//!
//! The CLI and TUI ship without debug symbols attached to most installs, so
//! `sentry-panic`'s stack trace is a column of bare addresses. Sentry's
//! default grouping hashes that trace, and an unsymbolicated trace hashes the
//! same across unrelated panics — OUTL-4 was two different `unwrap()` sites
//! filed as one issue because of this.
//!
//! The panic message survives [`redact`] and is specific to the call site
//! (`called \`Option::unwrap()\` on a \`None\` value`, an index-out-of-bounds
//! message with its bounds, …), so it is a better grouping key than the
//! trace. This runs after [`scrub_event`] has already redacted the exception
//! value, so the fingerprint only ever carries what the scrubber already
//! allowed through.
//!
//! [`redact`]: crate::redact
//! [`scrub_event`]: crate::scrub_event

use sentry::protocol::Event;

/// Sets `event.fingerprint` to `["outl-panic", <redacted first exception
/// value>]`, so distinct panic messages group into distinct Sentry issues.
///
/// Leaves the fingerprint alone when there is no exception, or its value is
/// empty: an empty fingerprint serializes as `[]`, which tells the server to
/// group every crash into one issue, the opposite of what this exists to do.
pub fn set_panic_fingerprint(event: &mut Event<'static>) {
    let Some(value) = event
        .exception
        .values
        .first()
        .and_then(|exception| exception.value.as_deref())
    else {
        return;
    };
    if value.is_empty() {
        return;
    }
    event.fingerprint = vec!["outl-panic".into(), value.to_string().into()].into();
}

#[cfg(test)]
mod tests {
    use sentry::protocol::Exception;

    use super::*;

    fn event_with_exception_value(value: Option<&str>) -> Event<'static> {
        Event {
            exception: vec![Exception {
                ty: "panic".into(),
                value: value.map(str::to_string),
                ..Default::default()
            }]
            .into(),
            ..Default::default()
        }
    }

    #[test]
    fn two_distinct_panic_messages_get_distinct_fingerprints() {
        let mut a =
            event_with_exception_value(Some("called `Option::unwrap()` on a `None` value"));
        let mut b = event_with_exception_value(Some(
            "index out of bounds: the len is 3 but the index is 5",
        ));

        set_panic_fingerprint(&mut a);
        set_panic_fingerprint(&mut b);

        assert_ne!(a.fingerprint, b.fingerprint);
        assert_eq!(
            fingerprint_strs(&a),
            vec!["outl-panic", "called `Option::unwrap()` on a `None` value"]
        );
    }

    /// The value here is what a redacted leak looks like after `scrub_event`
    /// has already run over it — the fixture stands in for that, rather than
    /// re-running `redact` itself, since this function never sees unredacted
    /// text in production.
    #[test]
    fn a_redacted_value_reaches_the_fingerprint_unchanged() {
        let mut event =
            event_with_exception_value(Some("could not open <path>: no such file or directory"));

        set_panic_fingerprint(&mut event);

        assert_eq!(
            fingerprint_strs(&event),
            vec!["outl-panic", "could not open <path>: no such file or directory"]
        );
    }

    fn fingerprint_strs(event: &Event<'static>) -> Vec<&str> {
        event.fingerprint.iter().map(|s| s.as_ref()).collect()
    }

    #[test]
    fn no_exception_keeps_the_protocol_default_fingerprint() {
        let mut event = Event::default();
        let default_fingerprint = event.fingerprint.clone();

        set_panic_fingerprint(&mut event);

        assert_eq!(event.fingerprint, default_fingerprint);
    }

    #[test]
    fn an_empty_exception_value_keeps_the_protocol_default_fingerprint() {
        let mut event = event_with_exception_value(Some(""));
        let default_fingerprint = event.fingerprint.clone();

        set_panic_fingerprint(&mut event);

        assert_eq!(event.fingerprint, default_fingerprint);
    }

    #[test]
    fn a_missing_exception_value_keeps_the_protocol_default_fingerprint() {
        let mut event = event_with_exception_value(None);
        let default_fingerprint = event.fingerprint.clone();

        set_panic_fingerprint(&mut event);

        assert_eq!(event.fingerprint, default_fingerprint);
    }
}
