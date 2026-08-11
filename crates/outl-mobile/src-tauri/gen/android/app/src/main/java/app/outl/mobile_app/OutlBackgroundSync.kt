package app.outl.mobile_app

import android.content.Context
import android.os.Build
import android.util.Log
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.workDataOf
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Background work for P2P sync on Android — the counterpart of iOS's
 * `OutlBackgroundRefresh.swift`.
 *
 * Android does not suspend a backgrounded app's sockets the way iOS does, so
 * the failure looks different but lands in the same place. Once the app has
 * no visible component the process becomes *cached*, and a cached process is
 * **frozen** (cgroup freezer, shipped in API 30, on by default from API 31,
 * ~10s after caching on API 34+ — an OEM-tunable default, not a contract).
 * A frozen process runs no timers and drains no socket buffer, so an iroh
 * delta sync mid-flight simply stops: the peer never sees the close-code-0
 * that confirms durable ingest and logs
 * `peer did not confirm durable ingest (closed: timed out)`, showing a red
 * row in the desktop Sync panel every time the user pockets their phone.
 *
 * Two mechanisms cover that, and they are deliberately not a 1:1 copy of
 * iOS's three:
 *
 * **1. The handover — finishing the pass already running.** On
 * [DefaultLifecycleObserver.onStop] we enqueue an **expedited**
 * `OneTimeWorkRequest`. A running job keeps the process out of the cached
 * state, and therefore out of the freezer, for the duration of that job —
 * which is the whole mechanism. This is the honest analogue of iOS's
 * `beginBackgroundTask` assertion; see [enqueueFlush] for what it does *not*
 * promise.
 *
 * **2. The deferred catch-up.** A 15-minute `PeriodicWorkRequest` (the
 * platform floor) constrained on connectivity. This is one schedule where
 * iOS has two (`BGAppRefreshTask` + `BGProcessingTask`) because WorkManager
 * has no equivalent split — its periodic work is already the "when the
 * scheduler feels like it, honouring Doze and App Standby" window that both
 * iOS task types approximate. Adding a second identical schedule would only
 * double the quota spend.
 *
 * **Scheduling is gated on having at least one paired peer**
 * ([NativeSync.peerCount] `> 0`), exactly like iOS: an unpaired install must
 * never wake for nothing. Losing the last peer cancels the periodic schedule
 * on the next backgrounding rather than leaving it armed forever.
 *
 * ### The one thing that is *not* like iOS
 *
 * When iOS grants a `BGProcessingTask` it **launches the app**, so the Tauri
 * setup hook runs and iroh is up before the handler fires. WorkManager does
 * not: it starts the process and `Application.onCreate`, but no Activity, so
 * Tauri never boots and no transport is ever registered. A periodic run in a
 * process that has since been killed therefore syncs nothing and says so
 * (`SyncWorker` logs "no transport in this process"). It is real work only
 * while the process is still alive — which on Android is the common case,
 * since a cached process usually outlives the foreground session by a long
 * way. Closing that gap needs a headless transport bootstrap that does not
 * exist yet; until it does, this is a catch-up for a *frozen* app, not for a
 * *killed* one, and it is not documented as more than that.
 */
object OutlBackgroundSync {

  private const val TAG = "outl"

  /** Unique-work name for the handover flush ([enqueueFlush]). */
  private const val FLUSH_WORK = "outl.sync.flush"

  /** Unique-work name for the deferred catch-up ([enqueuePeriodic]). */
  private const val PERIODIC_WORK = "outl.sync.periodic"

  /**
   * `PeriodicWorkRequest.MIN_PERIODIC_INTERVAL_MILLIS`. A floor, never a
   * schedule — Doze and the app's standby bucket both stretch it, and in the
   * Restricted bucket it is closer to once a day.
   */
  private const val PERIODIC_MINUTES = 15L

  /**
   * `install` can run again on activity recreation (a background kill, a
   * theme change that outlives the 700ms debounce). Registering the observer
   * twice would fire two flushes per backgrounding, doubling the quota spend
   * for one pass. Mirrors `INSTALLED` in `android_jni.rs`.
   */
  private val installed = AtomicBoolean(false)

  /**
   * Arm the background-sync schedules. Called from `MainActivity.onCreate`
   * (main thread — `addObserver` requires it), alongside `NativeSetup`.
   *
   * Nothing is enqueued here: at launch the workspace has not opened yet, so
   * [NativeSync.peerCount] still reads 0 and the gate below would refuse
   * anyway. Everything arms on the first backgrounding instead, which also
   * covers "the user paired their first device with the app open" without
   * needing a Rust→Kotlin signal.
   */
  @JvmStatic
  fun install(context: Context) {
    if (!installed.compareAndSet(false, true)) return
    val appContext = context.applicationContext

    // ProcessLifecycleOwner, not Activity.onStop: this is an app-level
    // question ("did outl go to the background?"), and its 700ms debounce is
    // exactly what stops a rotation or an activity swap — which destroy and
    // recreate the Activity — from being reported as a backgrounding. An
    // onStop-based hook would fire a sync pass every time the user rotates
    // the phone.
    ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
      override fun onStop(owner: LifecycleOwner) {
        onEnteredBackground(appContext)
      }
    })
  }

  /**
   * The app just went to the background: flush what is in flight, and make
   * sure a later catch-up is armed.
   */
  private fun onEnteredBackground(context: Context) {
    val peers = runCatching { NativeSync.peerCount() }.getOrElse { e ->
      Log.w(TAG, "bg-sync: peer count unavailable, not scheduling", e)
      0
    }
    val work = WorkManager.getInstance(context)
    if (peers <= 0) {
      // Nothing to sync with. Cancel rather than leave a schedule running:
      // periodic work survives process death and reboot, so an install that
      // once had a peer would otherwise keep waking forever after the user
      // unpaired their last device.
      work.cancelUniqueWork(PERIODIC_WORK)
      Log.i(TAG, "bg-sync: no paired peers, schedules cancelled")
      return
    }
    enqueueFlush(work)
    enqueuePeriodic(work)
  }

  /**
   * Finish the pass that is in flight right now, before the freezer stops it.
   *
   * **What this buys.** A job that is *running* puts the process above the
   * cached state, and only cached processes are frozen. Expedited work is the
   * sanctioned way to get such a job started immediately from the background
   * without a foreground service: on API 31+ WorkManager maps it to a
   * JobScheduler expedited job, which is guaranteed at least a minute of
   * runtime — comfortably more than the ~20s cap the Rust side applies.
   *
   * **What this does NOT guarantee, and iOS's `beginBackgroundTask` does.**
   *
   * - **It is not immediate.** `beginBackgroundTask` takes effect on the
   *   calling line. This is a request that goes through Room and the
   *   scheduler first. Between `onStop` (itself 700ms after the last
   *   Activity stopped) and the job actually starting, the process can
   *   already have been cached and frozen. When that happens the pass is not
   *   finished — it is *restarted* after the job unfreezes the process, and
   *   the peer still sees one timed-out exchange. This shrinks the window; it
   *   does not close it.
   * - **Expedited quota is finite** — roughly 30 min per rolling 24h in the
   *   Active standby bucket, down to 5 min in Restricted. Past that,
   *   [OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST] downgrades this to
   *   ordinary deferred work, which may not run for minutes or hours. Chosen
   *   over `DROP_WORK_REQUEST` because a late sync beats no sync.
   * - **A user force-stop, or an aggressive OEM task killer, drops it.** No
   *   WorkManager schedule survives those.
   * - **Below API 31 it is not expedited at all.** WorkManager implements
   *   expedited work there via a foreground service, which would require a
   *   `getForegroundInfo()` override and put a user-visible notification on
   *   screen for a 20-second sync. The freezer this exists to escape only
   *   arrived in API 30, so on older devices plain work is both sufficient
   *   and quieter — the request is enqueued without `setExpedited`.
   *
   * A foreground service was the other candidate and was rejected: it needs
   * `FOREGROUND_SERVICE_DATA_SYNC` on API 34+, a mandatory notification for
   * what is a few seconds of work, and starting one from the background is
   * itself restricted since API 31 (`ForegroundServiceStartNotAllowedException`).
   * Paying that for a handover is a bad trade.
   *
   * `KEEP` on the unique name: a second backgrounding while a flush is still
   * pending keeps the first, mirroring the iOS rule that re-entering the
   * assertion would leak the earlier one.
   */
  private fun enqueueFlush(work: WorkManager) {
    val request = OneTimeWorkRequestBuilder<SyncWorker>()
      .setConstraints(networkConstraints())
      .setInputData(workDataOf(SyncWorker.KEY_REASON to "flush"))
      .apply {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
          setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
        }
      }
      .build()
    work.enqueueUniqueWork(FLUSH_WORK, ExistingWorkPolicy.KEEP, request)
  }

  /**
   * Arm (or leave armed) the deferred catch-up.
   *
   * `KEEP` so re-arming on every backgrounding does not reset the interval —
   * `REPLACE` would push the next run 15 minutes out each time the user
   * closed the app, which for a phone means it would essentially never fire.
   */
  private fun enqueuePeriodic(work: WorkManager) {
    val request = PeriodicWorkRequestBuilder<SyncWorker>(PERIODIC_MINUTES, TimeUnit.MINUTES)
      .setConstraints(networkConstraints())
      .setInputData(workDataOf(SyncWorker.KEY_REASON to "periodic"))
      .build()
    work.enqueueUniquePeriodicWork(PERIODIC_WORK, ExistingPeriodicWorkPolicy.KEEP, request)
  }

  /**
   * Only run with a network. Waiting for connectivity delays the flush, but
   * a sync pass with no radio is a guaranteed 20-second no-op that spends
   * expedited quota to accomplish nothing.
   */
  private fun networkConstraints(): Constraints =
    Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
}
