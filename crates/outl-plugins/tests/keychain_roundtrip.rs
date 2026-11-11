//! The only test that talks to the real OS keychain.
//!
//! `KeyringStore` had no coverage at all: every other test in this crate swaps
//! in `MemorySecretStore`, so the production path was exercised by nothing.
//! That is survivable until a `keyring` major bump, and then it is not —
//! keyring 4 moved every backend into its own crate and registers the store
//! behind the `v1` feature, so a wrong feature set still **compiles** and still
//! passes the whole suite while `ctx.secrets` returns an error at runtime. The
//! plugins that read a token (Excalidraw, Oura) would be the ones to find out.
//!
//! `#[ignore]`d because it writes to whoever runs it. Run it by hand after any
//! change to the `keyring` dependency or its features:
//!
//! ```sh
//! cargo test -p outl-plugins --test keychain_roundtrip -- --ignored
//! ```
//!
//! The service name carries the pid and a timestamp so a run cannot collide
//! with a real secret or with another run, and the test deletes what it wrote
//! even on the assertion path.

use std::time::{SystemTime, UNIX_EPOCH};

use outl_plugins::{KeyringStore, SecretStore};

/// A service name that belongs to this run and nothing else.
fn scratch_service() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "outl-keychain-roundtrip-test-{}-{}",
        std::process::id(),
        nanos
    )
}

#[test]
#[ignore = "writes to the real OS keychain; run by hand after a keyring bump"]
fn the_real_keychain_round_trips_a_secret() {
    let store = KeyringStore::new();
    let service = scratch_service();
    let key = "token";

    // Absent before anything is written. This is also the assertion that
    // catches a missing store registration: keyring reports that as an `Err`,
    // not as `Ok(None)`.
    let before = store
        .get(&service, key)
        .expect("reading an absent secret must succeed, not error");
    assert_eq!(
        before, None,
        "a service nobody has written to must read empty"
    );

    store
        .set(&service, key, "uk-not-a-real-token")
        .expect("writing a secret to the keychain");

    let read_back = store.get(&service, key).expect("reading it back");

    // Delete before asserting the value, so a mismatch does not leave the
    // entry behind in the developer's keychain.
    store.delete(&service, key).expect("deleting the secret");

    assert_eq!(
        read_back.as_deref(),
        Some("uk-not-a-real-token"),
        "the keychain must hand back exactly what was stored"
    );

    let after = store.get(&service, key).expect("reading after the delete");
    assert_eq!(after, None, "a deleted secret must read empty");

    // The `SecretStore` contract says deleting what is not there is a no-op
    // success, and `KeyringStore` implements that by mapping `NoEntry` to
    // `Ok(())`. A renamed error variant would surface right here.
    store
        .delete(&service, key)
        .expect("deleting an absent secret is idempotent, not an error");
}
