package app.outl.mobile_app

/**
 * Rust JNI for background sync, implemented in `src-tauri/src/bg_sync.rs`.
 *
 * Separate from Tauri's generated `Rust` object and from `NativeSetup` so the
 * JNI symbol names are stable and greppable from both sides:
 * `Java_app_outl_mobile_1app_NativeSync_<method>`.
 *
 * Both calls are safe to make from **any** thread and from a process
 * that never started an Activity. `backgroundSync` blocks for as long as the
 * pass takes (bounded at ~20s on the Rust side), so it must never run on the
 * main thread — every caller here is a `Worker`, which WorkManager already
 * runs on its own executor.
 *
 * The Rust side never throws and never panics across this boundary; a failure
 * to reach the transport comes back as `false` / `0`, not as an exception.
 * What *can* still fail is this object's own initialisation: a Worker may be
 * the first thing to touch the native library in a cold process, so callers
 * wrap the first call (see `SyncWorker`).
 */
internal object NativeSync {
  init { System.loadLibrary("outl_mobile_lib") }

  /**
   * Force one iroh sync pass against every paired peer and block until it
   * completes or the Rust-side cap (~20s) elapses.
   *
   * `false` means no transport is registered in this process — the normal
   * answer when WorkManager started a cold process, since it launches no
   * Activity and therefore never boots Tauri. Not an error, and not worth
   * retrying: the retry would land in the same cold process.
   */
  @JvmStatic external fun backgroundSync(): Boolean

  /**
   * Paired devices recorded in `<workspace>/.outl/peers.json`, read fresh on
   * every call so a device paired after boot counts. `0` when the workspace
   * has not opened yet, when iroh is off, or when the list is unreadable.
   */
  @JvmStatic external fun peerCount(): Int
}
