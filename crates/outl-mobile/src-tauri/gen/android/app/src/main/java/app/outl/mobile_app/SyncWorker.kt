package app.outl.mobile_app

import android.content.Context
import android.util.Log
import androidx.work.Worker
import androidx.work.WorkerParameters

/**
 * Drives exactly one forced iroh sync pass, then finishes.
 *
 * One class serves both schedules ([OutlBackgroundSync] enqueues it as the
 * expedited handover flush and as the 15-minute periodic catch-up) because
 * the work is identical: call into Rust, block until the pass lands, report
 * done. Only the log line differs, so the caller passes its reason as input
 * data rather than the codebase carrying two near-identical Worker classes
 * that will drift.
 *
 * `Worker` (not `CoroutineWorker`) on purpose: [NativeSync.backgroundSync] is
 * a blocking FFI call, and `Worker.doWork()` already runs on WorkManager's
 * background executor. Wrapping a blocking call in a coroutine would add a
 * dispatcher and change nothing.
 *
 * **Always returns success, including when nothing was synced.** The two
 * failure modes here are "no transport in this process" (a cold process — a
 * retry lands in the same one) and "the pass did not finish inside its cap"
 * (the transport re-pushes on its own next tick). Neither is fixed by
 * `Result.retry()`, and a retry is never expedited, so retrying would spend
 * quota to achieve nothing.
 *
 * Public, not `internal`, because WorkManager instantiates it reflectively
 * from the class name it persisted — and because work-runtime's consumer
 * ProGuard rule that keeps it (`-keep public class * extends
 * ListenableWorker`) only matches public classes. A minified release build
 * with this class hidden would fail to start the job at runtime, long after
 * the build passed.
 */
class SyncWorker(context: Context, params: WorkerParameters) : Worker(context, params) {

  override fun doWork(): Result {
    val reason = inputData.getString(KEY_REASON) ?: "unknown"

    // A Worker can be the first thing to touch the native library in a cold
    // process, so this is where an `UnsatisfiedLinkError` (missing .so for
    // this ABI) or a class-init failure would surface. Crashing the job would
    // turn a missed sync into a crash report; log it and move on.
    val synced = runCatching { NativeSync.backgroundSync() }.getOrElse { e ->
      Log.w(TAG, "bg-sync: $reason pass could not reach the native library", e)
      return Result.success()
    }

    if (synced) {
      Log.i(TAG, "bg-sync: $reason pass finished")
    } else {
      Log.i(TAG, "bg-sync: $reason pass skipped, no transport in this process")
    }
    return Result.success()
  }

  companion object {
    const val KEY_REASON = "outl.sync.reason"
    private const val TAG = "outl"
  }
}
