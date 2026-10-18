package app.outl.mobile_app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.system.ErrnoException
import android.system.Os
import android.util.Log
import android.view.View
import android.view.ViewGroup
import android.webkit.WebView
import java.io.File

/**
 * Android's half of the capture inbox (RFC 0363).
 *
 * The share target, the widget and the quick settings tile are
 * components of *this* app, in this process, with the Rust library
 * already loaded — so nothing here knows the inbox file format or how a
 * share becomes one line. Both live in `outl_actions::capture`, reached
 * through [write]. That is the one place Android deliberately differs
 * from iOS, where a widget and a share extension are separate processes
 * under a memory ceiling the library would not fit, and a small Swift
 * writer (`OutlKit/Capture/CaptureInbox.swift`) owns the format instead.
 *
 * No component here touches the workspace. A capture waits as one file
 * until `drain_capture_inbox` turns it into ops, which is what keeps
 * root invariant 7: a second writer must never append to the op log.
 */
internal object OutlCapture {
  private const val TAG = "OutlCapture"

  /**
   * Asked on first use, never loaded from `init`.
   *
   * A failing `System.loadLibrary` in an object initializer does not
   * reach the `catch` inside `save`: the JVM wraps it in
   * `ExceptionInInitializerError` and throws that at the point the class
   * is first *touched*, which is `OutlCapture.save(...)` in
   * `OutlShareActivity` — outside the try. The load itself, and the log
   * line with the device's ABI and page size, belong to
   * [OutlNativeLibrary], which `MainActivity` asks the same question of.
   */
  private val libraryLoaded: Boolean
    get() = OutlNativeLibrary.isLoaded.also { loaded ->
      if (!loaded) Log.w(TAG, "native library did not load; capture not saved")
    }

  /** Folder inside `filesDir`, matching `outl_actions::capture::CAPTURE_DIR`. */
  private const val FOLDER = "captures"

  /** Env var the Rust side reads; `outl_actions::capture::CAPTURE_INBOX_ENV`. */
  private const val ENV_VAR = "OUTL_CAPTURE_INBOX"

  /** `outl://capture` — opens the app's capture sheet (`DeepLinkTarget::Capture`). */
  private const val CAPTURE_URL = "outl://capture"

  /**
   * The inbox folder, created if it is not there yet.
   *
   * `filesDir`, not the workspace: the workspace root is user-picked and
   * a share target has not opened it, so this is the Android equivalent
   * of the iOS App Group container — device-local, reachable by every
   * component without resolving anything.
   */
  fun inboxDir(context: Context): File =
    File(context.filesDir, FOLDER).apply { mkdirs() }

  /**
   * Point the Rust side at the inbox.
   *
   * Called from `MainActivity.onCreate` **before** `super.onCreate`
   * boots Tauri, the same ordering `NativeSetup.install` needs, so the
   * variable is already set the first time `drain_capture_inbox` asks.
   * A failure here costs the drain, not the app: the captures keep
   * waiting and `onResume` tries again on the next launch.
   */
  fun exportInbox(context: Context) {
    try {
      Os.setenv(ENV_VAR, inboxDir(context).absolutePath, true)
    } catch (e: ErrnoException) {
      Log.w(TAG, "could not export $ENV_VAR; captures will wait", e)
    }
  }

  /**
   * Write one capture, returning false when nothing was saved.
   *
   * `text` and `url` are the two things an `ACTION_SEND` carries; either
   * may be empty. False means blank input or a failed write, and the
   * caller says so rather than claiming a line it does not have — a
   * capture that silently disappears is the failure RFC 0363 exists to
   * avoid.
   */
  fun save(context: Context, text: String?, url: String?, source: String): Boolean =
    if (!libraryLoaded) {
      false
    } else try {
      write(
        inboxDir(context).absolutePath,
        text ?: "",
        url ?: "",
        source,
        System.currentTimeMillis(),
      )
    } catch (e: UnsatisfiedLinkError) {
      // A cold process reaching the native library for the first time,
      // the same case `SyncWorker` guards. Nothing is saved and the user
      // is told, rather than the share sheet crashing.
      Log.w(TAG, "native library unavailable; capture not saved", e)
      false
    }

  /** An intent that opens the app on the capture sheet. */
  fun captureIntent(): Intent =
    Intent(Intent.ACTION_VIEW, Uri.parse(CAPTURE_URL))
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

  /**
   * Ask the webview to drain, if it is up.
   *
   * The iOS app needs a Darwin notification for this because its
   * extensions can write while the app is on screen. Android has no such
   * case — a share sheet belongs to the app the user is *in*, so outl is
   * never foreground when a capture lands — which is why this hangs off
   * `onResume` instead: coming back is the moment a capture can be new.
   * `visibilitychange` in the webview usually fires then too, and the
   * drain is idempotent (the block id is the capture's ULID), so the two
   * together cost one extra no-op call and remove the dependency on
   * which one the platform actually delivers.
   */
  fun notifyWebView(root: View?) {
    val web = findWebView(root) ?: return
    web.evaluateJavascript(
      "window.__outlCapturesArrived && window.__outlCapturesArrived()",
      null,
    )
  }

  private fun findWebView(view: View?): WebView? = when (view) {
    null -> null
    is WebView -> view
    is ViewGroup -> (0 until view.childCount).firstNotNullOfOrNull { findWebView(view.getChildAt(it)) }
    else -> null
  }

  /**
   * `Java_app_outl_mobile_1app_OutlCapture_write` in
   * `src-tauri/src/capture_jni.rs`. Not private: the symbol name has to
   * stay exactly what the Rust side exports, and the same
   * `@JvmStatic external fun` shape as `NativeSync` is what keeps it
   * greppable from both sides.
   */
  @JvmStatic
  external fun write(
    dir: String,
    text: String,
    url: String,
    source: String,
    capturedAtMs: Long,
  ): Boolean
}
