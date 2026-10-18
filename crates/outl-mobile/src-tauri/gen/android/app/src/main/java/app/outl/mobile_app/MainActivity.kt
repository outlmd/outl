package app.outl.mobile_app

import android.content.Context
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  /**
   * The native library did not load, so this activity only exists long
   * enough to hand over to [NativeLoadErrorActivity] and finish.
   */
  private var nativeUnavailable = false

  override fun onCreate(savedInstanceState: Bundle?) {
    // Ask before anything below touches Rust. Every line after this block,
    // and Tauri's own `super.onCreate`, assumes the library is up; finding
    // out from inside one of them is an uncaught `ExceptionInInitializerError`
    // and an app that closes on open. See OutlNativeLibrary.
    val loadError = OutlNativeLibrary.loadError
    if (loadError != null) {
      nativeUnavailable = true
      startActivity(NativeLoadErrorActivity.intent(this, loadError))
      // `super.onCreate` is mandatory, and the part of it that matters to
      // Android (`Activity.onCreate`) runs first. wry's `WryActivity.onCreate`
      // then calls `Rust.onCreate`, whose initializer reloads the library and
      // throws. Catching that is safe because nothing after it in wry or
      // Tauri is needed: the activity finishes on the next line, and a
      // finish from `onCreate` goes straight to `onDestroy`, skipping
      // onStart / onResume / onPause / onStop — every other lifecycle hook
      // that would call into `Rust`.
      try {
        super.onCreate(savedInstanceState)
      } catch (expected: LinkageError) {
        // The same load failure, already logged by OutlNativeLibrary.
      }
      finish()
      return
    }

    enableEdgeToEdge()
    // Prime the JVM-backed globals iroh's TLS verifier + system-DNS reader
    // need on Android, BEFORE super.onCreate() boots Tauri (and the iroh
    // transport). Without this the first QUIC/TLS connection panics
    // (uninitialized rustls-platform-verifier / ndk_context) and the process
    // aborts with SIGABRT on a tokio worker thread. See android_jni.rs.
    NativeSetup.install(applicationContext)
    // Point the Rust side at the capture inbox before Tauri boots, so
    // `drain_capture_inbox` finds it on the webview's very first drain.
    // Same ordering requirement as NativeSetup above, for the same
    // reason: after `super.onCreate` the first command may already have
    // run. See OutlCapture.
    OutlCapture.exportInbox(applicationContext)
    // Arm the background-sync schedules (WorkManager + the ProcessLifecycle
    // observer that fires the handover flush). Nothing is enqueued here — the
    // workspace hasn't opened yet, so there are no peers to gate on; this only
    // installs the observer, and it is idempotent across activity recreation.
    OutlBackgroundSync.install(applicationContext)
    super.onCreate(savedInstanceState)
  }

  // The Wi-Fi driver filters multicast frames not addressed to this device, so
  // mDNS peer discovery (issue #149) sends queries fine and never hears an
  // answer until the lock is held. Tied to resume/pause rather than to the
  // activity: the lock is a real battery drain, and background sync runs off
  // known peers and the relay, which need no multicast. See OutlMulticast.
  override fun onResume() {
    super.onResume()
    OutlMulticast.acquire(applicationContext)
    // Coming back is the only moment a capture can be new: a share sheet
    // belongs to the app the user is *in*, so outl is never foreground
    // when the share target writes. That is why Android needs no
    // equivalent of the iOS Darwin notification. See OutlCapture.
    OutlCapture.notifyWebView(window?.decorView)
  }

  override fun onPause() {
    OutlMulticast.release()
    super.onPause()
  }

  override fun onDestroy() {
    if (!nativeUnavailable) {
      super.onDestroy()
      return
    }
    // `Activity.onDestroy` runs first inside the chain, then wry calls
    // `Rust.onDestroy`, which is a `NoClassDefFoundError` after the failed
    // initializer in `onCreate`. Same reasoning as there.
    try {
      super.onDestroy()
    } catch (expected: LinkageError) {
    }
  }
}

/// Loads the Rust lib and bridges to `Java_app_outl_mobile_1app_NativeSetup_install`.
/// Separate from Tauri's generated `Rust` object so the JNI symbol name is stable.
private object NativeSetup {
  init { System.loadLibrary("outl_mobile_lib") }

  @JvmStatic external fun install(context: Context)
}
