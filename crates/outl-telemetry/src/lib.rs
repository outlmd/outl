//! Crash reporting: the gate that decides whether anything is sent, and the
//! scrubber that decides what a sent event may contain.
//!
//! This crate exists so those two decisions have **one owner and one test
//! suite**, for the same reason `outl_md::unlogged::sidecar_can_vouch_for` and
//! `Filter::Not` do. Two copies of a safety predicate drift, and the direction
//! a scrubber drifts is always toward sending more.
//!
//! See [RFC 0377](https://github.com/outlmd/outl/blob/main/docs/rfcs/0377-crash-reporting.md).
//!
//! # Four conditions, all necessary
//!
//! | # | Condition | Enforced by |
//! |---|---|---|
//! | 1 | the `sentry` feature, off by default | the compiler: with it off, [`init`] links no SDK and returns `None` |
//! | 2 | `OUTL_SENTRY_DSN` baked in at compile time | the private `DSN` constant is `None` in every build outside our release pipeline |
//! | 3 | the user has not opted out | the `enabled` argument (the caller reads the config) **and** [`silenced_by_env`] |
//! | 4 | `CI` / `GITHUB_ACTIONS` unset | [`suppressed_by_ci`] |
//!
//! Conditions 1 and 2 are independent on purpose: either one alone fails
//! closed.
//!
//! Condition 3 has two sources because the user has two ways to say no: a
//! persistent `[telemetry] enabled = false`, which a client writes, and
//! `OUTL_TELEMETRY=0`, which silences a single invocation. Both live behind
//! this one condition because they are the same answer, arriving by different
//! routes.
//!
//! # With no feature, with no DSN
//!
//! With the `sentry` feature off, [`Guard`] is an empty struct and [`init`]
//! compiles to a `None`. [`redact`] still works — it is feature-independent so
//! its proptests run on every `cargo test`, not only on the build that ships.

#![forbid(unsafe_code)]

mod redact;
mod stdio;

pub use redact::{redact, FILE_MARKER, ID_MARKER, PATH_MARKER, TEXT_MARKER};
pub use stdio::{install_panic_hook, lossy_stderr, LossyStderr};

#[cfg(feature = "sentry")]
mod scrub;
#[cfg(feature = "sentry")]
pub use scrub::{scrub_event, ALLOWED_EVENT_KEYS};

/// The compile-time DSN. `None` in any build that did not set
/// `OUTL_SENTRY_DSN`, which is every build outside our release pipeline.
///
/// Compile-time rather than a runtime environment variable on purpose: a
/// runtime DSN is repointable by anyone running the binary. A DSN is write-only
/// by design and is not a credential, so this is hygiene rather than secrecy —
/// but a constant baked at link time is also the only form that cannot be set
/// by accident in a shell profile.
const DSN: Option<&str> = option_env!("OUTL_SENTRY_DSN");

/// Keeps the reporter alive, and flushes pending events when dropped.
///
/// Hold it for as long as the process should report. Dropping it early stops
/// reporting, which is why a caller binds it rather than discarding it with
/// `let _ =`.
#[cfg(feature = "sentry")]
pub struct Guard(#[allow(dead_code)] sentry::ClientInitGuard);

/// With the `sentry` feature off there is nothing to guard, and [`init`] never
/// returns one.
#[cfg(not(feature = "sentry"))]
pub struct Guard;

/// Starts crash reporting, or returns `None` when any of the four conditions
/// above is unmet.
///
/// `client` names the binary (`"cli"`, `"tui"`, `"desktop"`, `"mobile"`) and is
/// expected to be a literal in the caller's source. It reaches Sentry as the
/// release identifier, `<client>@<version>`, which is Sentry's own format.
///
/// Returning `None` is the ordinary case, not an error: it is what a developer
/// build, a third party's build, an opted-out user, an `OUTL_TELEMETRY=0`
/// invocation and a CI run all look like.
pub fn init(client: &str, enabled: bool) -> Option<Guard> {
    // Before the gate, and therefore before `sentry::init` on the path that
    // reaches it. The panic integration chains to whatever hook is installed
    // now. Installing afterwards would chain to the default `eprintln!` hook,
    // which is the panic this hook exists to stop. A closed gate still needs
    // it: a developer build panics the same way, and reports nothing.
    install_panic_hook();
    let suppressed = suppressed_by_ci() || silenced_by_env();
    let dsn = report_target(DSN, enabled, suppressed)?;
    start(dsn, client)
}

/// Whether this process is running inside somebody's CI, in which case it
/// reports nothing (condition 4).
///
/// A CLI runs in other people's automation. Reporting from there spends quota
/// on failures we cannot act on and carries paths out of an environment that is
/// not ours to read.
///
/// A non-empty value is enough; the value itself is not parsed. That is the
/// cheap direction to be wrong in: a false positive costs one lost report,
/// while a false negative reports out of somebody else's pipeline.
///
/// Separate from [`silenced_by_env`], which is a person deliberately silencing
/// one run on their own machine. Neither replaces the other: this one is
/// automatic and about somebody else's pipeline.
pub fn suppressed_by_ci() -> bool {
    ["CI", "GITHUB_ACTIONS"]
        .iter()
        .filter_map(std::env::var_os)
        .any(|value| !value.is_empty())
}

/// Whether this invocation was silenced with `OUTL_TELEMETRY=0` (condition 3's
/// second source).
///
/// This lives here rather than in each client because `docs/privacy.md` does
/// not restrict the variable to the CLI, and four clients applying it
/// themselves is four chances to forget. The direction that mistake breaks in
/// is the one that matters: the user exported the variable and the app reported
/// anyway.
///
/// Only `0` silences, unlike [`suppressed_by_ci`] where any non-empty value is
/// enough. The cheap direction to be wrong in is the opposite here: if any
/// value silenced, then `OUTL_TELEMETRY=1` would mean off, and the variable
/// would contradict itself.
pub fn silenced_by_env() -> bool {
    std::env::var_os("OUTL_TELEMETRY").is_some_and(|value| value == "0")
}

/// Conditions 2, 3 and 4 as one pure function, so each has a test that can
/// fail. Condition 1 is not here because it is not a runtime value: the
/// compiler enforces it through the two `start` definitions below.
fn report_target(dsn: Option<&str>, enabled: bool, suppressed: bool) -> Option<&str> {
    if !enabled || suppressed {
        return None;
    }
    dsn
}

#[cfg(feature = "sentry")]
fn start(dsn: &str, client: &str) -> Option<Guard> {
    use std::borrow::Cow;
    use std::sync::Arc;
    use std::time::Duration;

    // `ClientOptions` is `#[non_exhaustive]`, so it cannot be built from a
    // struct literal. `new()` rather than `default()` only to keep clippy's
    // `field_reassign_with_default` quiet; they are the same call.
    let mut options = sentry::ClientOptions::new();

    // `.ok()?`, never `.expect()` and never `ClientOptions::from(dsn)` (which
    // expects internally): a malformed DSN injected into a build must not
    // become a panic during boot. The worst it can do is turn reporting off.
    options.dsn = Some(dsn.parse().ok()?);
    options.release = Some(Cow::Owned(format!(
        "{client}@{}",
        env!("CARGO_PKG_VERSION")
    )));
    options.environment = Some(Cow::Borrowed("production"));
    options.send_default_pii = false;
    // Pinned, because `apply_defaults` reads `SSL_VERIFY` from the
    // environment **unconditionally** — unlike `SENTRY_DSN`, `SENTRY_RELEASE`
    // and `SENTRY_ENVIRONMENT`, which it only consults when the option is
    // `None` and which the lines above therefore already close. Without this,
    // `SSL_VERIFY=false` in a shell profile would ship the crash envelope
    // over a connection with no certificate validation.
    options.accept_invalid_certs = false;
    options.server_name = None;
    options.max_breadcrumbs = 0;
    options.before_breadcrumb = Some(Arc::new(|_| None));
    options.before_send = Some(Arc::new(scrub::scrub_event));
    // Short on purpose. The panic integration calls `flush` *synchronously*
    // inside the panic handler, so the 2s default is two seconds of frozen
    // screen before the abort. 600ms is the compromise.
    options.shutdown_timeout = Duration::from_millis(600);

    Some(Guard(sentry::init(options)))
}

/// Without the `sentry` feature there is no SDK to start, and this is the whole
/// of condition 1: not a runtime check that could be mis-ordered, but an
/// absence of code.
#[cfg(not(feature = "sentry"))]
fn start(_dsn: &str, _client: &str) -> Option<Guard> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEV_DSN: &str = "https://key@o0.ingest.sentry.io/1";

    /// Condition 1. Fails if the feature joins `default` in this manifest.
    ///
    /// Read from the manifest rather than asserted through `cfg`, because a
    /// `cfg`-based test passes exactly when the mistake has been made.
    #[test]
    fn the_sentry_feature_is_off_by_default() {
        let manifest = include_str!("../Cargo.toml");
        let default = manifest
            .lines()
            .find(|line| line.starts_with("default ="))
            .expect("[features] must declare an explicit `default`");
        assert_eq!(
            default, "default = []",
            "the `sentry` feature must stay off by default (RFC 0377 condition 1)"
        );
    }

    /// Condition 2. `DSN` is `None` in a test build, so the public call is the
    /// no-op; the gate is then checked over the whole matrix, because a test
    /// that can only observe `None` cannot tell a closed gate from a missing
    /// one.
    #[test]
    fn init_without_a_dsn_is_a_no_op() {
        assert!(init("cli", true).is_none());

        assert_eq!(report_target(None, true, false), None);
        assert_eq!(report_target(Some(DEV_DSN), true, false), Some(DEV_DSN));
    }

    /// Condition 3: the user's opt-out closes the gate even with a DSN baked in.
    #[test]
    fn init_respects_the_config_opt_out() {
        assert!(init("cli", false).is_none());

        assert_eq!(report_target(Some(DEV_DSN), false, false), None);
    }

    /// Condition 4, as the gate sees it. That `suppressed_by_ci` actually reads
    /// those two variables is pinned in `tests/ci_env.rs`, which owns the
    /// process-wide environment.
    #[test]
    fn a_suppressed_environment_closes_the_gate() {
        assert_eq!(report_target(Some(DEV_DSN), true, true), None);
    }

    /// Every condition is sufficient on its own, so no single one can be
    /// relaxed without the gate opening.
    #[test]
    fn each_condition_closes_the_gate_alone() {
        for (dsn, enabled, suppressed) in [
            (None, true, false),
            (Some(DEV_DSN), false, false),
            (Some(DEV_DSN), true, true),
        ] {
            assert_eq!(report_target(dsn, enabled, suppressed), None);
        }
    }
}
