//! Conditions 3 and 4 of RFC 0377, the two the environment can close: an
//! `outl` invocation inside somebody else's pipeline reports nothing, and so
//! does one the user silenced with `OUTL_TELEMETRY=0`.
//!
//! Single test file, single test, by the precedent of
//! `crates/outl-sync-iroh/tests/endpoint_lease.rs`: an environment variable is
//! process-wide, so two tests setting `CI` in parallel would each see the
//! other's value. The phases below run inside one `#[test]` rather than four
//! for that reason, not for brevity.
//!
//! The variables are restored at the end so a developer running
//! `cargo test -p outl-telemetry` on a CI machine does not lose them for the
//! rest of the binary.

use outl_telemetry::{silenced_by_env, suppressed_by_ci};

const VARS: [&str; 3] = ["CI", "GITHUB_ACTIONS", "OUTL_TELEMETRY"];

#[test]
fn the_environment_closes_the_gate() {
    let saved: Vec<_> = VARS.iter().map(|k| (*k, std::env::var_os(k))).collect();
    for (key, _) in &saved {
        std::env::remove_var(key);
    }

    a_bare_machine_is_not_ci();
    the_generic_ci_variable_suppresses();
    the_github_actions_variable_suppresses();
    an_empty_value_is_not_a_signal();

    an_unset_variable_does_not_silence();
    zero_silences();
    one_does_not_silence();
    an_empty_opt_out_value_is_not_a_signal();
    an_unparseable_opt_out_value_does_not_silence();

    for (key, value) in saved {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
}

/// The baseline: without either variable, reporting is not suppressed. Without
/// this phase the three below would pass against a function that always
/// returns `true`.
fn a_bare_machine_is_not_ci() {
    assert!(
        !suppressed_by_ci(),
        "neither CI nor GITHUB_ACTIONS is set, so nothing should be suppressed"
    );
}

fn the_generic_ci_variable_suppresses() {
    std::env::set_var("CI", "true");
    assert!(suppressed_by_ci());
    std::env::remove_var("CI");
}

/// Checked separately from `CI`: GitHub Actions sets both, but another runner
/// may set only one, and a single `||` typed as `&&` would pass the test above.
fn the_github_actions_variable_suppresses() {
    std::env::set_var("GITHUB_ACTIONS", "true");
    assert!(suppressed_by_ci());
    std::env::remove_var("GITHUB_ACTIONS");
}

/// `CI=` is not a claim to be in CI. A shell that exports an empty variable
/// would otherwise silence a real user's reports forever, invisibly.
fn an_empty_value_is_not_a_signal() {
    std::env::set_var("CI", "");
    assert!(!suppressed_by_ci());
    std::env::remove_var("CI");
}

/// The baseline for condition 3's second source, for the same reason as
/// `a_bare_machine_is_not_ci`: without it, the phases below would pass against
/// a function that always returns `true`.
fn an_unset_variable_does_not_silence() {
    assert!(
        !silenced_by_env(),
        "OUTL_TELEMETRY is unset, so nothing should be silenced"
    );
}

fn zero_silences() {
    std::env::set_var("OUTL_TELEMETRY", "0");
    assert!(silenced_by_env());
    std::env::remove_var("OUTL_TELEMETRY");
}

/// The asymmetry with `suppressed_by_ci` is the point: there, any non-empty
/// value is a signal, because a lost report is the cheap way to be wrong. Here
/// only `0` counts, because `OUTL_TELEMETRY=1` has to keep meaning *on*. A
/// variable that is off when set to `1` contradicts its own name, and the user
/// who typed it would never find out.
fn one_does_not_silence() {
    std::env::set_var("OUTL_TELEMETRY", "1");
    assert!(!silenced_by_env());
    std::env::remove_var("OUTL_TELEMETRY");
}

fn an_empty_opt_out_value_is_not_a_signal() {
    std::env::set_var("OUTL_TELEMETRY", "");
    assert!(!silenced_by_env());
    std::env::remove_var("OUTL_TELEMETRY");
}

/// `OUTL_TELEMETRY=false` reads like an opt-out to a person and is not one
/// here. Pinned rather than fixed: the documented value is `0`, and widening
/// the match to every word that looks negative is how `off`, `no`, `nope` and
/// `disabled` each become a bug report. If this is ever widened, it is a
/// decision with a test to change, not an accident.
fn an_unparseable_opt_out_value_does_not_silence() {
    std::env::set_var("OUTL_TELEMETRY", "false");
    assert!(!silenced_by_env());
    std::env::remove_var("OUTL_TELEMETRY");
}
