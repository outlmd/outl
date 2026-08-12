//! Background-sync bridge for the mobile clients' background windows.
//!
//! Both mobile OSes stop running the app shortly after it leaves the
//! foreground — iOS by suspending the process and its sockets, Android by
//! freezing the cached process — so neither has continuous background P2P.
//! What each *does* offer is a bounded window in which the app may run, and
//! in that window somebody has to **actively** drive a sync pass instead of
//! waiting for a catch-up tick that will not arrive.
//!
//! That "somebody" is this module. The live transport (plus the workspace
//! root) registers itself here at boot via [`register`], and each platform
//! gets a thin exported wrapper over the same two operations:
//!
//! | Operation | iOS symbol (C ABI) | Android symbol (JNI) |
//! |---|---|---|
//! | forced pass, ≤ caller's budget | `outl_ios_background_sync_capped` | `NativeSync.backgroundSync` (≤ [`SYNC_WINDOW`], see below) |
//! | paired-peer count | `outl_ios_peer_count` | `NativeSync.peerCount` |
//!
//! The peer count exists so each scheduler can gate *scheduling itself* on
//! having someone to sync with — a device with zero paired peers must never
//! burn a background window (iOS) or expedited-job quota (Android) on a wake
//! that has nothing to do.
//!
//! **iOS takes its cap from the caller; Android cannot, so it doesn't.**
//! Every iOS window knows a different ceiling and the flush's is only knowable
//! at runtime (`UIApplication.backgroundTimeRemaining` is a report, not a
//! promise), so Swift passes the number and Rust clamps it. Android exposes no
//! equivalent: a `Worker` learns it is out of time through `isStopped`, never
//! as a countdown it could hand down. A capped JNI symbol would have no honest
//! argument to receive, so the Android side keeps [`SYNC_WINDOW`].
//!
//! Instead of sleeping a fixed worst-case window, [`drive_sync`] enqueues a
//! **sequenced** forced pass ([`IrohSyncTransport::sync_now_seq`]) and polls
//! every [`POLL_INTERVAL`] until that pass — not merely *some* pass — has
//! completed and no dial is left on the wire, then hands the unused window
//! back to the OS (both schedulers reward short tasks with more frequent
//! grants). The cap is only the fallback for a pass that outlives the window.
//!
//! **Why the sequence number matters.** The obvious version of this ("snap
//! the completed counter, fire, wait for it to move") is wrong, and was
//! shipped: the counter is global and the mobile frontend fires `syncNow()`
//! on a 3s foreground timer, so a background flush would watch the in-flight
//! *foreground* pass complete ~250ms later, declare victory, and release the
//! OS window with its own request still queued. The mechanism built to
//! finish the sync was the thing ending it. Waiting on
//! [`IrohSyncTransport::inbound_serves`] too closes the companion hole: a
//! forced pass skips a peer that already has a dial running, so its
//! completion says nothing about that peer.
//!
//! Everything here is deliberately panic-free (no `unwrap`/`expect`,
//! `parking_lot` locks don't poison), because a panic must never unwind
//! across the C ABI into Swift, nor across the JNI boundary into Kotlin —
//! both abort the process. The Android wrappers additionally route through
//! `catch_unwind` (see below), belt and braces.
//!
//! **Why the exported symbols are `cfg`-gated but the bodies are not.**
//! Neither platform should compile the other's surface: the C symbols are
//! dead weight in an Android `.so`, and the JNI symbols cannot even be
//! *declared* without the `jni` crate, which is an Android-only dependency.
//! The logic, however, is identical on both, so it stays unconditional —
//! which is also what keeps it covered by the host test suite below, where
//! neither platform's exports exist.

// …and because neither platform's exports exist on a host build, the shared
// bodies they wrap have no non-test caller there. That is the design, not an
// oversight, so silence `dead_code` for this module on host targets only —
// both real targets still report it.
#![cfg_attr(not(any(target_os = "ios", target_os = "android")), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

// No `SyncTransport` import: this module deliberately drives the *concrete*
// `IrohSyncTransport` rather than the trait. `sync_now_seq` / `inbound_serves`
// are inherent methods, and the trait's fire-and-forget `sync_now` is exactly
// the thing that cannot be waited on correctly.
use outl_sync_iroh::{workspace_peers_path, IrohSyncTransport, PeersStore};
use parking_lot::Mutex;
use tracing::{info, warn};

/// Upper bound for an Android forced pass (the expedited job, guaranteed at
/// least a minute of runtime).
///
/// Cross-network iroh connects can take ~20s (multipath) and this stays under
/// that budget. With early-exit it is a *cap*, not the typical duration — a
/// same-LAN pass returns in a couple of seconds.
///
/// iOS has no equivalent constant: each of its windows passes its own ceiling
/// through the capped symbol.
#[cfg_attr(target_os = "ios", allow(dead_code))]
const SYNC_WINDOW: Duration = Duration::from_secs(20);

/// How often a forced pass re-checks the completed-pass counter while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Bounds a caller-supplied cap ([`clamp_cap`]).
///
/// The floor keeps a `0` (or a Swift arithmetic slip on an already-expired
/// budget) from degrading the wait into a bare poll that reports every pass
/// as unfinished. The ceiling is the real safety property: this runs on a
/// thread the OS is about to reclaim, and no background budget on either
/// platform is worth blocking for a minute, so a wrong argument can waste at
/// most that.
///
/// Unconditional even though only iOS supplies a budget to clamp, because it
/// is core, not an export — see the module doc. Gating it `not(android)` read
/// as tidy and broke `--all-targets` for `aarch64-linux-android`: the `lib
/// test` target compiles for the *platform*, so the test below vanished its
/// own subject. Per-target dead code is a warning; a missing symbol is a
/// build failure.
#[cfg_attr(target_os = "android", allow(dead_code))]
const CAP_BOUNDS: std::ops::RangeInclusive<u64> = 1..=60;

/// What the background-window entry points need from the live app: the
/// transport handle (to fire + observe a forced sync) and the workspace root
/// (to read the paired peer list off `peers.json` — the on-disk file is the
/// source of truth; the transport's in-memory store is a boot-time snapshot
/// that pairing after boot does not refresh).
#[derive(Clone)]
struct Registration {
    transport: IrohSyncTransport,
    workspace_root: PathBuf,
}

/// The live registration, refreshed every boot. A re-settable slot (not a
/// bare `OnceLock<Registration>`) so a relaunch / workspace reopen replaces a
/// stale handle instead of keeping the first one forever.
fn slot() -> &'static Mutex<Option<Registration>> {
    static SLOT: OnceLock<Mutex<Option<Registration>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Stash a clone of the live transport + the workspace root so the
/// background-window entry points can reach them. Called from
/// `iroh_sync::wire_iroh_transport` after the transport starts.
/// `IrohSyncTransport` is `Clone` (its handles are `Arc`-backed), so the
/// clone drives the same endpoint as `AppState.iroh`.
pub(crate) fn register(transport: &IrohSyncTransport, workspace_root: PathBuf) {
    *slot().lock() = Some(Registration {
        transport: transport.clone(),
        workspace_root,
    });
}

/// Number of paired peers for the registered workspace, or `0` when nothing
/// is registered (transport off / boot not finished) or the peer list can't
/// be read.
///
/// Read fresh from `<workspace>/.outl/peers.json` on every call — pairing
/// and peer removal both write that file, so the count is current even for
/// peers added after the transport booted. Both schedulers gate on `> 0`:
/// with zero peers a background wake costs a process launch (Android) or a
/// whole app boot (iOS) just to discover there is nobody to talk to.
fn registered_peer_count() -> u32 {
    let root = match slot().lock().as_ref() {
        Some(reg) => reg.workspace_root.clone(),
        None => return 0,
    };
    peer_count_at(&root)
}

// ── iOS exports (C ABI, bound with `@_silgen_name` in Swift) ─────────────

/// Drive one forced sync pass from an iOS background window, waiting at most
/// `seconds` (clamped to [`CAP_BOUNDS`]).
///
/// Returns `true` when a transport was wired and a sync was fired, `false`
/// when iroh is off (no transport) so the Swift side can mark its task
/// accordingly.
///
/// **One symbol, not one per budget.** Each of the three windows knows its own
/// ceiling — the `BGProcessingTask`'s minutes, the `BGAppRefreshTask`'s ~30s,
/// and the flush's whatever-`backgroundTimeRemaining`-reports — and only the
/// last of those is even knowable at compile time. Fixed-cap symbols were the
/// original shape, on the reasoning that no argument marshalling is trivially
/// safer than some; that argument stopped applying the moment a parameterised
/// symbol had to exist anyway for the flush, leaving two constants duplicated
/// on both sides of the FFI.
///
/// The clamp is what makes an untrusted `u32` safe: a `0` (an expired budget,
/// or a Swift arithmetic slip) or an absurd value cannot hang this thread past
/// a minute.
///
/// Swift binding: `@_silgen_name("outl_ios_background_sync_capped")`.
#[cfg(target_os = "ios")]
#[no_mangle]
pub extern "C" fn outl_ios_background_sync_capped(seconds: u32) -> bool {
    drive_sync(clamp_cap(seconds))
}

/// Paired-peer count for the Swift scheduler ([`registered_peer_count`]).
///
/// Plain C ABI, no arguments, no pointers — safe to call from Swift via
/// `@_silgen_name("outl_ios_peer_count")`.
#[cfg(target_os = "ios")]
#[no_mangle]
pub extern "C" fn outl_ios_peer_count() -> u32 {
    registered_peer_count()
}

// ── Android exports (JNI, bound by `NativeSync` in Kotlin) ───────────────
//
// Same three operations, same bodies. The style follows `android_jni.rs`
// exactly: `EnvUnowned` + `with_env` (which wraps the body in
// `catch_unwind`) + [`jni::errors::LogErrorAndDefault`] (which logs, never
// throws). A thrown exception would surface inside `Worker.doWork()` and
// fail the job; a panic unwinding into the JVM would abort the process.
// Neither is an acceptable outcome for "we could not sync right now", so
// both degrade to the `Default` — `JNI_FALSE` / `0` — which every caller
// already reads as "no transport, nothing done".

/// Drive one forced sync pass, capped at [`SYNC_WINDOW`]. Called from
/// `SyncFlushWorker` (the went-to-background handover) and
/// `PeriodicSyncWorker`.
///
/// `JNI_FALSE` means no transport is registered **in this process**. That is
/// the normal answer when WorkManager started a *cold* process for the job:
/// it launches no Activity, so Tauri never booted and iroh was never wired.
/// The Kotlin side reads it as "nothing to do", never as a failure worth
/// retrying — a retry would land in the same cold process.
///
/// # Safety
///
/// Called by the JVM with a valid JNI env and class.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_app_outl_mobile_1app_NativeSync_backgroundSync<'local>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
) -> jni::sys::jboolean {
    unowned_env
        .with_env(|_env: &mut jni::Env| Ok::<_, jni::errors::Error>(jbool(drive_sync(SYNC_WINDOW))))
        .resolve::<jni::errors::LogErrorAndDefault>()
}

/// Paired-peer count for the Kotlin scheduler ([`registered_peer_count`]).
///
/// Returned as `jint` because Kotlin has no unsigned `Int`; the count
/// saturates rather than wrapping, so an implausibly large peer list still
/// reads as "many" and never as a negative number that would fail a `> 0`
/// gate and silently stop scheduling.
///
/// # Safety
///
/// Called by the JVM with a valid JNI env and class.
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_app_outl_mobile_1app_NativeSync_peerCount<'local>(
    mut unowned_env: jni::EnvUnowned<'local>,
    _class: jni::objects::JClass<'local>,
) -> jni::sys::jint {
    unowned_env
        .with_env(|_env: &mut jni::Env| {
            Ok::<_, jni::errors::Error>(
                jni::sys::jint::try_from(registered_peer_count()).unwrap_or(jni::sys::jint::MAX),
            )
        })
        .resolve::<jni::errors::LogErrorAndDefault>()
}

/// `bool` → `jboolean`. JNI's boolean is a `u8` whose only valid values are
/// the two constants; casting the `bool` directly happens to work today,
/// which is exactly the kind of thing that stops working quietly.
#[cfg(target_os = "android")]
fn jbool(value: bool) -> jni::sys::jboolean {
    if value {
        jni::sys::JNI_TRUE
    } else {
        jni::sys::JNI_FALSE
    }
}

/// Turn a caller-supplied budget in seconds into a wait cap, bounded by
/// [`CAP_BOUNDS`]. Saturating, never panicking — the argument crosses an FFI
/// from Swift, so it is untrusted input, not a compile-time constant.
///
/// Unconditional for the reason in [`CAP_BOUNDS`]: it is pure core logic, and
/// its test has to find it on every target the test target is built for.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn clamp_cap(seconds: u32) -> Duration {
    Duration::from_secs(u64::from(seconds).clamp(*CAP_BOUNDS.start(), *CAP_BOUNDS.end()))
}

/// Fire a forced sync pass and wait for **that pass** to finish (early-exit)
/// or the `cap` to elapse (fallback), whichever comes first. `false` iff no
/// transport is registered.
///
/// `true` with no waiting when the transport is registered but its runtime is
/// down: `sync_now_seq` returned `0`, meaning nothing was enqueued and so
/// nothing will ever complete. Holding the OS window open for a request that
/// does not exist would burn the whole cap to learn nothing.
fn drive_sync(cap: Duration) -> bool {
    // Clone the handle out so we don't hold the lock across the wait below.
    let Some(reg) = slot().lock().clone() else {
        return false;
    };
    let transport = reg.transport;
    // Sequenced, NOT "snapshot the counter and wait for it to move": the
    // counter is global, and a concurrent foreground `syncNow()` would
    // satisfy that weaker predicate while our own request was still queued.
    let seq = transport.sync_now_seq();
    if seq == 0 {
        info!("bg-sync: nothing queued (transport runtime down), returning now");
        return true;
    }
    let settled = wait_until(cap, POLL_INTERVAL, || {
        // Our own request drained, AND nobody is mid-push through us.
        //
        // `inbound_serves()` counts ONLY responder-side exchanges. Waiting on
        // our outbound dials as well made the predicate unreachable exactly
        // when the device was worst off: a dial to an
        // unreachable peer costs 15s (5s direct + 10s relay) while the
        // catch-up loop starts another every 8s, so with one peer offline the
        // outbound set is never empty. On device that read as
        // `window elapsed before pass #107 settled` — the full cap burned on
        // a condition that could not become true, which iOS repays by
        // granting fewer windows.
        //
        // Our dials do not need the wait: `completed_sync_passes() >= seq`
        // already covers the pass we asked for, and one hung on a dead peer
        // will not land inside any window. An inbound serve is the half worth
        // holding the OS open for — someone else's ops, seconds from the
        // durable-ingest confirmation that stops them being re-sent.
        transport.completed_sync_passes() >= seq && transport.inbound_serves() == 0
    });
    if settled {
        info!("bg-sync: forced pass #{seq} completed, returning window early");
    } else {
        info!("bg-sync: window elapsed before pass #{seq} settled");
    }
    true
}

/// Poll `probe` every `poll` until it returns `true` or `cap` elapses.
/// Returns whether the probe fired (`false` = timed out).
///
/// Kept separate from [`drive_sync`] so the early-exit/timeout contract is
/// testable without a live transport.
fn wait_until(cap: Duration, poll: Duration, probe: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + cap;
    loop {
        if probe() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        // Never oversleep the deadline on the last iteration.
        std::thread::sleep(poll.min(deadline - now));
    }
}

/// Count the paired peers recorded in `<root>/.outl/peers.json`. `0` on a
/// missing or unreadable file — the conservative answer for the scheduling
/// gate (better to skip a window than to wake for a corrupt list).
fn peer_count_at(root: &Path) -> u32 {
    match PeersStore::load_or_default(&workspace_peers_path(root)) {
        Ok(store) => u32::try_from(store.list().len()).unwrap_or(u32::MAX),
        Err(e) => {
            warn!("bg-sync: peer count read failed: {e}");
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Early-exit contract: the wait returns `true` as soon as the probe
    /// fires, well before the cap.
    #[test]
    fn wait_until_returns_early_when_probe_fires() {
        let calls = AtomicU32::new(0);
        let started = Instant::now();
        let fired = wait_until(Duration::from_secs(10), Duration::from_millis(5), || {
            calls.fetch_add(1, Ordering::Relaxed) >= 3
        });
        assert!(fired);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "early-exit should not wait anywhere near the cap"
        );
    }

    /// Timeout contract: a probe that never fires bounds the wait at the cap.
    #[test]
    fn wait_until_times_out_at_the_cap() {
        let started = Instant::now();
        let fired = wait_until(Duration::from_millis(60), Duration::from_millis(10), || {
            false
        });
        assert!(!fired);
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    /// The wait predicate keys on the caller's **own** sequence number, so a
    /// counter advance caused by somebody else's pass does not release it.
    ///
    /// This is the regression net for the defect that shipped: with a
    /// "did the counter move" predicate, a foreground `syncNow()` completing
    /// mid-wait ended a background flush whose request was still queued.
    /// Modelled on `wait_until` directly (no transport needed) — that is
    /// exactly the property `wait_until` is factored out to preserve.
    #[test]
    fn the_wait_ignores_a_pass_completed_below_the_callers_sequence() {
        let my_seq = 7u64;
        // Somebody else's pass lands: the global counter moves, but not to
        // our request's number.
        let completed = AtomicU32::new(6);
        let fired = wait_until(Duration::from_millis(60), Duration::from_millis(10), || {
            u64::from(completed.load(Ordering::Relaxed)) >= my_seq
        });
        assert!(!fired, "another actor's completed pass must not release us");

        // Now ours drains.
        completed.store(7, Ordering::Relaxed);
        let fired = wait_until(Duration::from_secs(10), Duration::from_millis(5), || {
            u64::from(completed.load(Ordering::Relaxed)) >= my_seq
        });
        assert!(fired);
    }

    /// A caller-supplied cap is untrusted input from Swift: `0` and absurd
    /// values are pulled into [`CAP_BOUNDS`] rather than degrading the wait
    /// into a bare poll or blocking a doomed thread for hours.
    #[test]
    fn clamp_cap_bounds_an_untrusted_budget() {
        assert_eq!(clamp_cap(0), Duration::from_secs(1));
        assert_eq!(clamp_cap(12), Duration::from_secs(12));
        assert_eq!(clamp_cap(u32::MAX), Duration::from_secs(60));
    }

    /// `peer_count_at` reads the on-disk peers.json fresh: absent file → 0,
    /// entries present → their count.
    #[test]
    fn peer_count_reads_peers_json() {
        let tmp = tempfile::tempdir().expect("tempdir");
        assert_eq!(peer_count_at(tmp.path()), 0, "no peers.json yet");

        let path = workspace_peers_path(tmp.path());
        let mut store = PeersStore::load_or_default(&path).expect("empty store");
        for i in 0..2u8 {
            store
                .add(outl_sync_iroh::PeerEntry {
                    node_id: format!("test-node-{i}"),
                    alias: None,
                    relay_url: None,
                    endpoint_addr: None,
                    added_at: "2026-01-01T00:00:00Z".to_string(),
                })
                .expect("add peer");
        }
        assert_eq!(peer_count_at(tmp.path()), 2);
    }

    /// The exported surface follows the registration slot: everything reports
    /// "off" before `register`, and the peer count reflects the registered
    /// workspace afterwards. One sequential test because the slot is a
    /// process-wide global.
    ///
    /// It calls [`drive_sync`] / [`registered_peer_count`] rather than
    /// `outl_ios_*` / `Java_..._NativeSync_*` because neither set of exports
    /// exists on a host build — each is `cfg`-gated to its own target, and
    /// the JNI ones cannot be invoked from Rust at all (there is no
    /// `EnvUnowned` without a JVM). Every export is a one-line wrapper over
    /// exactly these two calls, so this is the same coverage reached one
    /// layer down, not a weaker assertion.
    #[test]
    fn ffi_surface_follows_registration() {
        // Before any registration: sync reports "no transport", count is 0.
        assert!(!drive_sync(SYNC_WINDOW));
        assert!(!drive_sync(clamp_cap(12)));
        assert_eq!(registered_peer_count(), 0);

        let tmp = tempfile::tempdir().expect("tempdir");
        let identity =
            outl_sync_iroh::IrohIdentity::load_or_generate(&tmp.path().join("identity.key"))
                .expect("identity");
        let peers_path = workspace_peers_path(tmp.path());
        let mut peers = PeersStore::load_or_default(&peers_path).expect("empty store");
        peers
            .add(outl_sync_iroh::PeerEntry {
                node_id: "test-node".to_string(),
                alias: Some("test".to_string()),
                relay_url: None,
                endpoint_addr: None,
                added_at: "2026-01-01T00:00:00Z".to_string(),
            })
            .expect("add peer");
        // Unstarted transport is fine: peer count never touches the runtime.
        let transport = IrohSyncTransport::new(identity, peers, None);

        register(&transport, tmp.path().to_path_buf());
        assert_eq!(registered_peer_count(), 1);

        // A registered-but-unstarted transport has no runtime, so
        // `sync_now_seq` enqueues nothing and returns 0. That must come back
        // as "done" IMMEDIATELY, not as a 20s wait for a request that will
        // never be drained — the OS window is the scarce resource here.
        let started = Instant::now();
        assert!(
            drive_sync(SYNC_WINDOW),
            "a registered transport reports true"
        );
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a zero sequence must not wait on the cap"
        );

        // Leave the slot empty for any test run after this one.
        *slot().lock() = None;
        assert_eq!(registered_peer_count(), 0);
    }
}
