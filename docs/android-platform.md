# Android platform integration

What the Android build needs that no other client does, and what it deliberately does not promise.

The user-facing background-sync behaviour is [sync.md → Background sync on Android](sync.md#background-sync-on-android); the iOS counterpart of this document is [ios-platform.md](ios-platform.md).

---

## Where the Android-specific code lives

Six files, none of which holds business logic — the same rule as every other client.

| Piece | Path | Job |
|---|---|---|
| JNI bootstrap | `crates/outl-mobile/src-tauri/src/android_jni.rs` | Prime the JVM globals iroh's TLS + DNS need |
| Background sync (Rust) | `crates/outl-mobile/src-tauri/src/bg_sync.rs` | Three JNI entry points over a shared, platform-agnostic core |
| Activity | `gen/android/app/src/main/java/app/outl/mobile_app/MainActivity.kt` | Calls `NativeSetup.install` + `OutlBackgroundSync.install` before Tauri boots |
| Native binding | `gen/android/…/NativeSync.kt` | `external fun` declarations matching the JNI symbols |
| Scheduler | `gen/android/…/OutlBackgroundSync.kt` | Lifecycle observer + the two WorkManager schedules |
| Worker | `gen/android/…/SyncWorker.kt` | Drives one forced pass, then finishes |

Everything under `gen/android/app/build/` is **build output** — never edit it.

---

## JNI bootstrap (why the app used to `SIGABRT` on launch)

iroh's relay client verifies certificates through `rustls-platform-verifier` and reads system DNS through `iroh-dns` / `hickory`.
Both call into the JVM — the Android KeyStore and `ConnectivityManager` — and both read a process-wide JNI context that neither Tauri/wry nor `tao`'s bundled `ndk_glue` installs.
So the first QUIC connection panicked, the panic poisoned a `quinn` mutex, and the next task's `.unwrap()` on the `PoisonError` aborted the process: a `SIGABRT` on a `tokio-rt-worker` thread seconds after boot.

`MainActivity.onCreate` calls `NativeSetup.install(applicationContext)` **before** `super.onCreate` boots Tauri, priming both globals exactly once.
The `rustls-platform-verifier` Kotlin component is not on Maven ([rustls/rustls-platform-verifier#115](https://github.com/rustls/rustls-platform-verifier/issues/115)).
Its `.aar` is therefore vendored under `gen/android/app/libs/`, and must be re-copied whenever the crate version bumps.

---

## Background sync (Android)

Android does not suspend a backgrounded app's sockets the way iOS does, so the failure looks different but lands in the same place.

Once the app has no visible component its process becomes **cached**, and a cached process is **frozen** by the kernel cgroup freezer.
The freezer shipped in API 30, is on by default from API 31 where the kernel supports it, and on API 34+ the default debounce is ~10 seconds after the process is cached — a `DeviceConfig` value OEMs tune, not a contract.
A frozen process runs no timers and drains no socket buffer, so an iroh delta sync in flight simply stops.
The peer never sees the close-code-0 that confirms durable ingest, logs `peer did not confirm durable ingest (closed: timed out)`, and re-pushes on its next tick.
That is *safe* — no data is lost — but it is **visible**: the desktop Sync panel shows a red row every time the user pockets their phone.

> AOSP documents that a fully frozen app has its active **TCP** sockets terminated.
> It says nothing about UDP, and iroh is QUIC over UDP.
> The observable behaviour is the stall described above; do not write down a guarantee about QUIC under the freezer that no source supports.

### Two mechanisms, not iOS's three

**1. The handover — finishing the pass already running.**
On `ProcessLifecycleOwner`'s `ON_STOP`, `OutlBackgroundSync` enqueues an **expedited** `OneTimeWorkRequest`.
A job that is *running* holds the process above the cached state, and only cached processes are frozen — that is the entire mechanism.
On API 31+ WorkManager maps expedited work to a JobScheduler expedited job, guaranteed at least a minute of runtime, comfortably more than the ~20s ceiling the Rust side applies.

**2. The deferred catch-up.**
A 15-minute `PeriodicWorkRequest` (the platform floor) constrained on connectivity.
This is one schedule where iOS has two, because WorkManager has no `BGAppRefreshTask` / `BGProcessingTask` split.
Its periodic work already *is* the "whenever the scheduler feels like it, honouring Doze and App Standby" window that both iOS task types approximate.
A second identical schedule would only double the quota spend.

Both are gated on `NativeSync.peerCount() > 0`, exactly like iOS: an unpaired install must never wake for nothing.
Losing the last paired device cancels the periodic schedule on the next backgrounding rather than leaving it armed forever — periodic work survives process death and reboot, so "stop scheduling" has to be an explicit cancel.

Nothing is enqueued at launch.
At `MainActivity.onCreate` the workspace has not opened yet, so the peer count still reads 0 and the gate would refuse anyway.
Everything arms on the first backgrounding instead.
That also covers "the user paired their first device with the app open", with no Rust→Kotlin signal needed.

### Why `ProcessLifecycleOwner`, not `Activity.onStop`

"Did outl go to the background?" is an app-level question, and `ProcessLifecycleOwner`'s 700 ms debounce is precisely what keeps a configuration change — which destroys and recreates the Activity — from being reported as one.
An `onStop`-based hook would fire a sync pass every time the user rotated the phone.
`install()` is idempotent (an `AtomicBoolean`, mirroring `INSTALLED` in `android_jni.rs`) because `onCreate` runs again after an activity recreation, and a second observer would double every flush.

### What the handover does NOT guarantee

This is the honest analogue of iOS's `beginBackgroundTask` assertion, not an equivalent of it.

- **It is not immediate.**
  `beginBackgroundTask` takes effect on the calling line.
  This is a request that goes through Room and the scheduler first.
  Between `ON_STOP` (itself 700 ms after the last Activity stopped) and the job actually starting, the process can already have been cached and frozen.
  When that happens the in-flight pass is not *finished*, it is **restarted** once the job unfreezes the process — and the peer still logged one timed-out exchange.
  This shrinks the window; it does not close it.
- **Expedited quota is finite.**
  Roughly 30 minutes per rolling 24 h in the Active standby bucket, down to 5 minutes in Restricted.
  Past that, `OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST` downgrades the request to ordinary deferred work, which may not run for minutes or hours.
  Chosen over `DROP_WORK_REQUEST` because a late sync beats no sync.
- **Below API 31 it is not expedited at all.**
  WorkManager implements expedited work there via a foreground service, which would require a `getForegroundInfo()` override and put a user-visible notification on screen for a twenty-second sync.
  The freezer this exists to escape only arrived in API 30, so on older devices plain work is both sufficient and quieter, and `setExpedited` is simply not called.
- **A user force-stop, or an aggressive OEM task killer, drops it.**
  No WorkManager schedule survives those.

A **foreground service** was the other candidate and was rejected.
It needs `FOREGROUND_SERVICE_DATA_SYNC` on API 34+, a mandatory user-visible notification for what is a few seconds of work, and a 6-hour-per-24h runtime cap on API 35+.
Starting one from the background is itself restricted since API 31 (`ForegroundServiceStartNotAllowedException`).
Paying all of that for a handover is a bad trade.

### The one thing that is genuinely unlike iOS

When iOS grants a `BGProcessingTask` it **launches the app**, so the Tauri setup hook runs and iroh is up before the handler fires.

WorkManager does not.
It starts the process and `Application.onCreate` (plus the `androidx.startup` content providers), but **no Activity** — so Tauri never boots, no `IrohSyncTransport` is ever created, and `bg_sync::register` is never called.
A periodic run in a process that has since been killed therefore syncs nothing, and says so: `SyncWorker` logs `bg-sync: periodic pass skipped, no transport in this process` and returns `Result.success()`.
It does **not** retry — the retry would land in the same cold process, and a retry is never expedited, so it would spend quota to achieve nothing.

So the periodic schedule is a catch-up for a **frozen** app, not for a **killed** one.
On Android that is usually the case that matters, since a cached process routinely outlives the foreground session by hours.
It is still a real gap.
Closing it needs a headless transport bootstrap — open the workspace and start iroh without Tauri — which does not exist yet.

---

## The Rust ↔ Kotlin contract

`bg_sync.rs` exposes three JNI entry points; `NativeSync.kt` declares the matching `external fun`s.

| Kotlin | JNI symbol | Returns |
|---|---|---|
| `NativeSync.backgroundSync()` | `Java_app_outl_mobile_1app_NativeSync_backgroundSync` | `false` = no transport in this process |
| `NativeSync.peerCount()` | `Java_app_outl_mobile_1app_NativeSync_peerCount` | paired devices, read fresh from `peers.json` |

Rules that keep this from crashing:

1. **Nothing throws and nothing panics across the boundary.**
   Each entry point goes through `EnvUnowned::with_env` (which wraps the body in `catch_unwind`) and resolves with `LogErrorAndDefault`, which logs instead of throwing — the same style as `android_jni.rs`.
   A thrown exception would fail the job; an unwind into the JVM would abort the process.
   Both degrade instead to `JNI_FALSE` / `0`, which every caller already reads as "no transport, nothing done".
2. **The Kotlin side wraps the *first* call anyway.**
   A `Worker` can be the first thing to touch the native library in a cold process, so `UnsatisfiedLinkError` and class-init failures surface there and nowhere else.
   `SyncWorker` catches them and logs; a missed sync must not become a crash report.
3. **There is no capped variant on Android, on purpose.**
   iOS has `outl_ios_background_sync_capped(seconds)` because Swift must clamp its wait against `UIApplication.backgroundTimeRemaining`, whose budget is not contractual.
   Android exposes no equivalent number — a `Worker` learns it is out of time through `isStopped`, never as a countdown it could pass down — so a capped JNI symbol would have no honest argument to receive.
4. **`SyncWorker` is `public`, not `internal`.**
   WorkManager instantiates it reflectively from the persisted class name, and work-runtime's consumer ProGuard rule (`-keep public class * extends ListenableWorker`) only matches public classes.
   A minified release build with the class hidden fails at runtime, long after the build went green.

The Rust bodies these wrap (`drive_sync`, `registered_peer_count`, `wait_until`) are **not** `cfg`-gated — only the exported symbols are.
That is what keeps them covered by the host test suite, where neither platform's exports are compiled.

---

## Gradle and manifest

`AndroidManifest.xml` needs **no change** for background sync, and this was verified against the merged manifest rather than assumed.
`androidx.work:work-runtime` merges in `WAKE_LOCK`, `ACCESS_NETWORK_STATE`, `RECEIVE_BOOT_COMPLETED` and `FOREGROUND_SERVICE`, plus the `androidx.startup` `WorkManagerInitializer` that boots WorkManager with no `Configuration.Provider` of our own.

Two dependencies in `gen/android/app/build.gradle.kts`:

- `androidx.lifecycle:lifecycle-process` — `ProcessLifecycleOwner`.
- `androidx.work:work-runtime` — **not** `work-runtime-ktx`, which has been an empty artifact since 2.9.0 and survives only for compatibility.

> **Version ceiling, and it is load-bearing.**
> The module compiles with Kotlin 1.9.25 (`kotlin-gradle-plugin` in the root `build.gradle.kts`).
> `androidx.work:work-runtime:2.11.x` pulls `kotlin-stdlib:2.1.20`, whose metadata version 2.1.0 the 1.9 compiler cannot read.
> The failure is not confined to the new code — it breaks every file in the module, including Tauri's generated `WryActivity.kt`.
> `2.10.5` is the newest version that builds today.
> Raising it means bumping the Kotlin Gradle plugin first.

---

## Release

`release.yml`'s `build_android` job builds an optimized (release) arm64 APK on every release and attaches it to the GitHub Release.
It is **debug-signed** — the build is a release build, only the signature is a throwaway key; a real upload keystore and a Play track are tracked in [issue #171](https://github.com/avelino/outl/issues/171).
The job is best-effort: `publish_release` waits for it but does not require its success, so a broken Android build never blocks the desktop/CLI release.

The NDK is pinned to `27.1.12297006` in CI, matching what the project builds against locally.
Version comes from the workspace `Cargo.toml`, injected via `cargo tauri android build --config`.
Tauri's mobile path does not fall back to `Cargo.toml` on its own — the same trap as iOS, see [`crates/outl-mobile/CLAUDE.md`](../crates/outl-mobile/CLAUDE.md).

---

## What can only be checked on a device

The whole background path.
Host and CI can prove the pieces fit — the `.so` exports exactly the three `Java_app_outl_mobile_1app_NativeSync_*` symbols `NativeSync.kt` declares, the Kotlin compiles, the manifest merges — but none of that exercises a single scheduled job.

To validate for real, on a device or emulator:

```bash
# Force the periodic/expedited job to run now.
adb shell cmd jobscheduler run -f app.outl.mobile_app <jobId>
# What WorkManager thinks is scheduled.
adb shell dumpsys jobscheduler | grep -A 20 app.outl.mobile_app
# The app's own log lines (Rust `bg-sync:` + Kotlin, tag `outl`).
adb logcat -s outl:V RustStdoutStderr:V
```

Then confirm the peer no longer reports `peer did not confirm durable ingest` when the phone is locked mid-sync.
That is the actual acceptance test, and nothing short of two paired devices produces it.
