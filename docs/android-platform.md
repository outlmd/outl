# Android platform integration

What the Android build needs that no other client does, and what it deliberately does not promise.

The user-facing background-sync behaviour is [sync.md → Background sync on Android](sync.md#background-sync-on-android); the iOS counterpart of this document is [ios-platform.md](ios-platform.md).

---

## Where the Android-specific code lives

Fourteen files, none of which holds business logic — the same rule as every other client.

| Piece | Path | Job |
|---|---|---|
| JNI bootstrap | `crates/outl-mobile/src-tauri/src/android_jni.rs` | Prime the JVM globals iroh's TLS + DNS need |
| Capture JNI | `crates/outl-mobile/src-tauri/src/capture_jni.rs` | Write one capture through `outl_actions::capture` |
| Capture plumbing | `gen/android/…/OutlCapture.kt` | Inbox folder, the env var, the webview nudge |
| Share target | `gen/android/…/OutlShareActivity.kt` | `ACTION_SEND` → one capture, a toast, finish |
| Widget | `gen/android/…/OutlCaptureWidget.kt` | One tap opens `outl://capture` |
| Quick settings tile | `gen/android/…/OutlCaptureTile.kt` | Same, from the pull-down |
| Background sync (Rust) | `crates/outl-mobile/src-tauri/src/bg_sync.rs` | Two JNI entry points over a shared, platform-agnostic core |
| Activity | `gen/android/app/src/main/java/app/outl/mobile_app/MainActivity.kt` | Checks the native library loaded, then calls `NativeSetup.install` + `OutlBackgroundSync.install` before Tauri boots |
| Library probe | `gen/android/…/OutlNativeLibrary.kt` | The one "did `liboutl_mobile_lib.so` load?" answer, logged with ABI and page size |
| Load error screen | `gen/android/…/NativeLoadErrorActivity.kt` | Plain activity shown instead of a crash when the library will not load |
| Native binding | `gen/android/…/NativeSync.kt` | `external fun` declarations matching the JNI symbols |
| Scheduler | `gen/android/…/OutlBackgroundSync.kt` | Lifecycle observer + the two WorkManager schedules |
| Worker | `gen/android/…/SyncWorker.kt` | Drives one forced pass, then finishes |
| Multicast lock | `gen/android/…/OutlMulticast.kt` | Lifts the Wi-Fi multicast filter so mDNS peer discovery can hear an answer |

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

## When the native library does not load

A device can refuse `liboutl_mobile_lib.so` outright: a 4 KB-aligned build on a 16 KB-page phone (v0.12.0-beta.226, see [below](#the-so-has-to-be-linked-for-16-kb-pages)), or an ABI the APK does not carry.
Before this was handled, that refusal surfaced inside Tauri's generated `Rust` object, whose static initializer calls `System.loadLibrary`.
A throwing static initializer reaches the caller as `ExceptionInInitializerError` at the first line that touches the class — `super.onCreate` in `MainActivity` — so the app closed on open with nothing on screen.

`OutlNativeLibrary` now owns the question.
It loads the library once, remembers the error, and logs one line under the `outl` tag with what a report needs: supported ABIs, the kernel page size (`Os.sysconf(_SC_PAGESIZE)`), API level, device and app version.

```bash
adb logcat -s outl
# E outl: native library liboutl_mobile_lib.so did not load: abis=[arm64-v8a] pageSize=16384 sdk=35 ...
```

Every Kotlin path that reaches Rust asks it first:

| Entry point | When the library did not load |
|---|---|
| `MainActivity` | Starts `NativeLoadErrorActivity` and finishes; `NativeSetup`, the capture env var and `OutlBackgroundSync` are never reached |
| `OutlShareActivity` (via `OutlCapture.save`) | "Nothing to save." toast, no write |
| `SyncWorker` | `Result.failure()` without touching `NativeSync` |

`NativeLoadErrorActivity` is a plain `AppCompatActivity`, not a `TauriActivity`, because wry and Tauri call into the library from every lifecycle hook.
It says the app could not load its native library on this device, shows the ABI and page size as selectable text, and links to the issue tracker.

**`MainActivity` still calls `super.onCreate`, and catches what it throws.**
Android requires the call, and the part it checks (`Activity.onCreate`) runs first in the chain; wry's `Rust.onCreate` comes after and is what throws.
The activity then finishes from `onCreate`, which sends it straight to `onDestroy` and skips `onStart`, `onResume`, `onPause` and `onStop` — every other hook where wry calls `Rust`.
`onDestroy` catches the same `LinkageError` for the same reason.
This depends on wry calling `super` before `Rust` in those two hooks, which is true of wry 0.57; re-read `WryActivity.kt` when bumping it.

---

## Capture from outside the app

Three entry points, the same shapes iOS has (RFC 0363): a **share target**
(`ACTION_SEND`, `text/plain`), a **home screen widget**, and a **quick
settings tile** — the closest Android has to iOS's Lock Screen widget.

The widget and the tile write nothing. They open `outl://capture` and the
app's capture sheet does the work, which is why RFC 0363 ships that shape
first. On Android that needs the `outl` scheme registered on
`MainActivity` in `AndroidManifest.xml`; without the intent filter both
open nothing at all, which is why it landed with them.

### Android owns the format in Rust, iOS owns it in Swift

The one deliberate difference from iOS, and the reason is the platform,
not taste.

An iOS widget and share extension are separate processes with their own
sandbox and a memory ceiling far below what loading this library needs, so
a small Swift writer (`OutlKit/Capture/CaptureInbox.swift`) owns the
inbox file format there.

Android has no extension model. The share target is an ordinary activity
in **this** app's process, with `liboutl_mobile_lib.so` already loaded.
So it writes through `Java_app_outl_mobile_1app_OutlCapture_write`, and
`outl_actions::capture::write_capture` + `shared_line` stay the owners of
the file format and of "a share becomes one line". Kotlin holds the
folder and the user-facing strings; nothing else.

That also means the Android path is covered by `cargo test`, where the
Swift one needs `swift test` and the Kotlin side needs an APK build.

### No "a capture arrived" notification, and that is not an omission

iOS posts a Darwin notification after an extension's write, because an
iOS extension can write **while the app is on screen** — a share sheet
hosted over outl itself, a widget tap on the Lock Screen.

Android has no such case. A share sheet belongs to the app the user is
*in*, so outl is never foreground when the share target writes, and a
widget or tile tap opens the app rather than writing. Coming back is
therefore the only moment a capture can be new, which is why
`MainActivity.onResume` nudges the webview
(`OutlCapture.notifyWebView`) instead.

The webview's own `visibilitychange` usually fires on resume too, and the
drain is idempotent — the block id is the capture's ULID — so the two
together cost one no-op call and remove the dependency on which one the
platform actually delivers.

### Where a capture waits

`filesDir/captures`, exported to Rust as `OUTL_CAPTURE_INBOX` from
`MainActivity.onCreate` **before** `super.onCreate` boots Tauri — the
same ordering `NativeSetup.install` needs, so the variable is set before
the first `drain_capture_inbox` asks.

Not the workspace's own `.outl/captures`: the workspace root is
user-picked and a share target has not opened it. `filesDir` is the
Android equivalent of the iOS App Group container — device-local,
reachable without resolving anything.

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

`bg_sync.rs` and `capture_jni.rs` expose the JNI entry points; `NativeSync.kt` and `OutlCapture.kt` declare the matching `external fun`s.

| Kotlin | JNI symbol | Returns |
|---|---|---|
| `NativeSync.backgroundSync()` | `Java_app_outl_mobile_1app_NativeSync_backgroundSync` | `false` = no transport in this process |
| `NativeSync.peerCount()` | `Java_app_outl_mobile_1app_NativeSync_peerCount` | paired devices, read fresh from `peers.json` |
| `OutlCapture.write(dir, text, url, source, ms)` | `Java_app_outl_mobile_1app_OutlCapture_write` | `false` = nothing saved (blank input or a failed write) |

Rules that keep this from crashing:

1. **Nothing throws and nothing panics across the boundary.**
   Each entry point goes through `EnvUnowned::with_env` (which wraps the body in `catch_unwind`) and resolves with `LogErrorAndDefault`, which logs instead of throwing — the same style as `android_jni.rs`.
   A thrown exception would fail the job; an unwind into the JVM would abort the process.
   Both degrade instead to `JNI_FALSE` / `0`, which every caller already reads as "no transport, nothing done".
2. **The Kotlin side wraps the *first* call anyway.**
   A `Worker` can be the first thing to touch the native library in a cold process, so `UnsatisfiedLinkError` and class-init failures surface there and nowhere else.
   `SyncWorker` catches them, logs, and returns `Result.failure()`; a missed sync must not become a crash report, but a defect that recurs on every run for this install must not read as success either.
   Failure never retries one-time work, the periodic schedule stays in place, and the breakage is visible in WorkManager's bookkeeping instead of only in logcat.
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
It does need one for **LAN peer discovery** — see below — and four for capture: the `outl` scheme on `MainActivity`, the share activity, the widget receiver and the tile service.
Those four were verified the same way, by reading them back out of the built APK (`aapt2 dump xmltree --file AndroidManifest.xml`) rather than from the source manifest, because a merge can drop what a source file declares.
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

## mDNS peer discovery (the permission is not the whole story)

Same-LAN peer discovery ([issue #149](https://github.com/outlmd/outl/issues/149)) is wired once in `outl-sync-iroh`'s `bind::attach_mdns`, so Android gets it from the shared Rust path with no Android-specific code — **and it would have found nobody.**

Android's Wi-Fi driver drops multicast and broadcast frames not addressed to this device, as a battery optimisation.
That filter sits **below** the socket API, so every call `swarm-discovery` makes succeeds: the `IP_ADD_MEMBERSHIP` join returns `Ok`, queries go out on the wire, and answers are discarded before the socket ever sees them.
Nothing errors. Discovery just returns nothing, which is indistinguishable from "there are no peers on this LAN" — the exact silent-degradation shape this repo keeps paying for.

Two pieces lift it, and both are needed:

- `android.permission.CHANGE_WIFI_MULTICAST_STATE` in `AndroidManifest.xml`. It is a normal permission (no runtime prompt).
- A held `WifiManager.MulticastLock`, in `OutlMulticast.kt`.

**The lock follows `onResume` / `onPause`, not the activity's lifetime.**
Holding it is a documented battery drain — the radio stops filtering and the CPU processes every multicast frame on the network.
It is worth that only while a foreground session wants to find a peer *now*; background sync runs off known peers and the relay, which need no multicast at all.
`setReferenceCounted(false)` so one `release()` is enough however many times `acquire()` ran.

Failures are logged and swallowed, matching `attach_mdns`: a device with no `WifiManager`, or an OEM that refuses the lock, still syncs over the relay.

> Wired multicast (Android TV on Ethernet) does not go through the Wi-Fi filter and needs no lock. `getSystemService(WIFI_SERVICE)` still returns a `WifiManager` on any device with Wi-Fi hardware regardless of the active transport, so the lock is taken and simply does nothing there.

---

## Release

`release.yml`'s `build_android` job builds an optimized (release) arm64 APK on every release and attaches it to the GitHub Release.
It is **debug-signed** — the build is a release build, only the signature is a throwaway key; a real upload keystore and a Play track are tracked in [issue #171](https://github.com/outlmd/outl/issues/171).
The job is best-effort: `publish_release` waits for it but does not require its success, so a broken Android build never blocks the desktop/CLI release.

**What best-effort costs, which took three releases to notice.**
The draft flips to published with or without the APK, and a published release is immutable, so "Re-run failed jobs" on the Android job arrives too late to attach anything.
It used to answer with a bare `HTTP 422: Cannot upload assets to an immutable release`, which reads like a token problem; the upload step now checks `isDraft` first and says what it actually means.
The only route to an APK on the release is a new beta, and since a push to `main` cuts one, that is usually the next commit.

Worse, a job's implicit `success()` reads the whole ancestor tree rather than its direct `needs`, so a failed Android build also skipped `update_tap` — through a `publish_release` that had succeeded.
v0.12.0-beta.207 shipped that way and left `Formula/outl-beta.rb` on `.206`, so `brew upgrade outl-beta` served the previous binary and nothing went red.
`update_tap` carries `!cancelled()` now, the same guard `publish_release` already had for the same reason.

`cleanup-tags.yml` is the third thing standing on that decision and is deliberately left alone.
It gates on `workflow_run.conclusion == 'success'`, so an Android failure skips the prune too, but its prune recomputes "keep the 2 newest betas per base version" from scratch, so the next green release catches up.
A tap bump does not recompute: it patches the version of the release that triggered it, and a release it sat out stays skipped.

The NDK is pinned to `27.1.12297006` in CI, matching what the project builds against locally.
The runner image does not ship that revision, so every run downloads ~700 MB, and a truncated transfer is routine.
beta.207 died at 33% on `Error reading Zip content from a SeekableByteChannel`, which `sdkmanager` logs as a *Warning* before exiting 1.
The install retries three times, deleting the half-unzipped package and the download scratch between attempts.
It gates on the `clang` binary `cargo-ndk` will invoke, not on the exit code: a partial unzip still leaves `source.properties` behind and claims to be installed.
Version comes from the workspace `Cargo.toml`, injected via `cargo tauri android build --config`.
Tauri's mobile path does not fall back to `Cargo.toml` on its own — the same trap as iOS, see [`crates/outl-mobile/CLAUDE.md`](../crates/outl-mobile/CLAUDE.md).

### Play Console: Data Safety

The Data Safety form is a Play Console answer, not a file in the repo, so nothing fails if it drifts from the app.
Before the first Play submission, and again whenever what the app sends changes, fill it in to match [`privacy.md`](privacy.md) → Crash reports:

| Question | Answer |
|---|---|
| Does the app collect or share user data? | Collects, does not share |
| Data type | App info and performance → **Crash logs** |
| Shared with third parties | **No** (Sentry processes the report for us, and a service provider acting on our behalf is not "sharing" under Play's definition) |
| Linked to the user | **No**: no account, no device id, no persistent identifier |
| Collection is optional | **Yes**: the user can turn it off in Settings, Privacy |
| Purpose | App functionality |
| Data encrypted in transit | Yes |
| Way for users to request deletion | Not applicable, nothing is linked to a user; say so in the form |

Crash reporting covers Rust panics only, not Kotlin exceptions ([RFC 0377](rfcs/0377-crash-reporting.md)), so the form must not claim broader coverage than the app has.

### The `.so` has to be linked for 16 KB pages

Some Android devices page at 16 KB rather than 4 KB, and there `dlopen` refuses a library whose LOAD segments are aligned to 4 KB.
The Pixel 8 and 9 families expose the mode as a developer option, newer hardware ships with it on, and since November 2025 Google Play has required support from every app update targeting Android 15+.
The app dies on its first `System.loadLibrary`, which is `NativeSetup`'s initializer in `MainActivity.onCreate` — so the whole app crashes on open, before any of our code runs, on a device where nothing else is wrong.
It now shows an error screen instead of closing ([When the native library does not load](#when-the-native-library-does-not-load)), but that only makes the failure legible; the APK is still unusable on that device until the alignment below holds.

v0.12.0-beta.226 shipped exactly that.
Its three native libraries, read out of the published APK:

| library | `p_align` |
|---|---|
| `libimage_processing_util_jni.so` (vendored AAR) | `0x4000` |
| `libsurface_util_jni.so` (vendored AAR) | `0x4000` |
| `liboutl_mobile_lib.so` (ours) | `0x1000` |

Nothing in the toolchain was going to fix that for us.
The NDK does set `max-page-size` for 64-bit targets, but it sets it in its CMake and ndk-build toolchain files, and cargo uses neither: it drives the NDK's `clang` as a bare linker.
rustc 1.95 does not pass it either.
So the flag lives in `.cargo/config.toml`, per Android 64-bit target, and the two vendored `.aar` libraries were only correct because whoever built them passed it.

Two alignments have to hold, and they come from different steps:

- **The ELF**, from the linker flag above. Governs every device, because it is a property of the library itself.
- **The zip offset**, from `zipalign -P 16` in `release.yml`. Only governs an *uncompressed* entry, which is all of them: `extractNativeLibs` defaults to false, so the libraries are mapped straight out of the APK rather than unpacked on install.

Neither says anything when it stops working — the build is green, the APK installs, the signature verifies, and the app dies on the device.
So `build_android` runs [`android-apk-page-alignment.py`](../crates/outl-mobile/scripts/android-apk-page-alignment.py) over the signed APK and fails the job.
Checking the finished artifact covers both steps at once, plus any library arriving through a dependency we do not build.

To check an APK by hand:

```bash
python3 crates/outl-mobile/scripts/android-apk-page-alignment.py outl-android-arm64.apk
```

A `RUSTFLAGS` exported into the environment **replaces** the `rustflags` in `.cargo/config.toml` rather than merging with it, so a workflow that sets one has to carry the link flag itself.
`build_android` deliberately sets none.

---

## What can only be checked on a device

The whole background path.
Host and CI can prove the pieces fit — the `.so` exports exactly the two `Java_app_outl_mobile_1app_NativeSync_*` symbols `NativeSync.kt` declares, the Kotlin compiles, the manifest merges — but none of that exercises a single scheduled job.

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

### Capture, and what an emulator *can* prove

More than the background path, because the risky seam is one call rather than a scheduler.
On an emulator, without touching the UI:

```bash
# The share target, shaped the way Chrome shares a page (subject + link).
adb shell "am start -a android.intent.action.SEND -t text/plain \
  --es android.intent.extra.SUBJECT 'a title' \
  --es android.intent.extra.TEXT 'https://outl.app' \
  -n app.outl.mobile_app/.OutlShareActivity"
# What landed in the inbox, and whether `shared_line` composed it.
adb shell run-as app.outl.mobile_app ls files/captures/
# The widget, the tile and the scheme are registered.
adb shell dumpsys appwidget | grep outl
adb shell "cmd package query-services -a android.service.quicksettings.action.QS_TILE" | grep outl
adb shell "cmd package resolve-activity -a android.intent.action.VIEW -d outl://capture"
```

That covers Kotlin → JNI → `write_capture` → a file with the right field
names, which is the part no test in this repo reaches.

What still needs a real device is the rest of the gesture: adding the
widget to a home screen and the tile to the pull-down, and the drain
landing the captures in a journal — the last one needs a workspace open,
so it needs the app actually running, not an `am start`.
