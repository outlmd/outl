//! Condition 1 of RFC 0377, checked where it can actually fail: the clients.
//!
//! `the_sentry_feature_is_off_by_default` guards this crate's own manifest, and
//! that is not enough. Each client declares its own `sentry` feature, and a
//! client that puts it in `default` — or that pins `features = ["sentry"]` on
//! the dependency — links the reporter into every build on every machine,
//! including the build of somebody who cloned the repo to read it.
//!
//! That is the one requirement this whole feature was conditioned on: download
//! the code, run it, and there is no Sentry. Nothing else in the suite fails if
//! a client breaks it, because a client that reports is a *working* client —
//! the defect is invisible from inside the process.
//!
//! The manifests are parsed, not grepped. A textual check for `default = []`
//! passes against `default = [ "sentry" ]` written with other spacing, and
//! against a `default` declared under `[target.'cfg(...)'.features]`.

use std::path::{Path, PathBuf};

/// Every binary a user can run. A new client belongs here on the day it is
/// created, which is the day it would otherwise start reporting unguarded.
const CLIENTS: [&str; 4] = [
    "outl-cli",
    "outl-tui",
    "outl-desktop/src-tauri",
    "outl-mobile/src-tauri",
];

fn manifest(client: &str) -> (PathBuf, toml::Table) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join(client)
        .join("Cargo.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} is not readable: {e}", path.display()));
    // `toml::Table`, not `toml::Value`: parsing into `Value` reads the text as
    // a single TOML *value*, so `[package]` comes back as an array and the
    // rest of the file is "unexpected content".
    let parsed = text
        .parse::<toml::Table>()
        .unwrap_or_else(|e| panic!("{} is not valid TOML: {e}", path.display()));
    (path, parsed)
}

/// No client may ship the reporter in a default build.
#[test]
fn no_client_enables_crash_reporting_by_default() {
    for client in CLIENTS {
        let (path, parsed) = manifest(client);
        let features = parsed
            .get("features")
            .unwrap_or_else(|| panic!("{} declares no [features]", path.display()));

        let default = features.get("default").unwrap_or_else(|| {
            panic!(
                "{} must declare an explicit `default`: an absent one is not \
                 the same promise, because the next person to add a feature \
                 inherits no rule",
                path.display()
            )
        });
        let default = default
            .as_array()
            .unwrap_or_else(|| panic!("{}: `default` must be an array", path.display()));
        assert!(
            default.is_empty(),
            "{} has a non-empty `default` ({default:?}). The reporter must be \
             absent from a build nobody asked for it in — see RFC 0377 §2.",
            path.display()
        );

        let sentry = features.get("sentry").unwrap_or_else(|| {
            panic!(
                "{} has no `sentry` feature, so the release pipeline cannot \
                 switch reporting on for it",
                path.display()
            )
        });
        let sentry = sentry
            .as_array()
            .unwrap_or_else(|| panic!("{}: `sentry` must be an array", path.display()));
        assert!(
            sentry
                .iter()
                .any(|entry| entry.as_str() == Some("outl-telemetry/sentry")),
            "{}: its `sentry` feature must forward to `outl-telemetry/sentry`, \
             or the flag is inert and the build silently reports nothing",
            path.display()
        );
    }
}

/// The other way to defeat condition 1, and the quieter one: leave `default`
/// empty and pin the feature on the dependency instead. The client's own
/// feature then looks correct while the SDK is linked unconditionally.
#[test]
fn no_client_pins_the_sentry_feature_on_the_dependency() {
    for client in CLIENTS {
        let (path, parsed) = manifest(client);
        let dep = parsed
            .get("dependencies")
            .and_then(|deps| deps.get("outl-telemetry"))
            .unwrap_or_else(|| {
                panic!(
                    "{} does not depend on outl-telemetry under [dependencies] \
                     — a dev-dependency would make the reporter test-only",
                    path.display()
                )
            });

        let pinned = dep
            .get("features")
            .and_then(|f| f.as_array())
            .is_some_and(|features| {
                features
                    .iter()
                    .any(|entry| entry.as_str() == Some("sentry"))
            });
        assert!(
            !pinned,
            "{} pins `features = [\"sentry\"]` on the outl-telemetry \
             dependency, which links the SDK in every build regardless of \
             what `default` says",
            path.display()
        );
    }
}
