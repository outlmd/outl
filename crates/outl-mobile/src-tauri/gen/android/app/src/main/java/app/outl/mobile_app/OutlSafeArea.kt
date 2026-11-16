package app.outl.mobile_app

import android.app.Activity
import android.view.View
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import java.lang.ref.WeakReference

/**
 * Publishes the real system insets (status bar, navigation bar, display
 * cutout) to the web layer.
 *
 * **Why this exists:** `MainActivity` calls `enableEdgeToEdge()`, so the
 * WebView is drawn under the system bars, and Android's WebView does not
 * report the status bar through `env(safe-area-inset-top)`. The header's
 * `max(env(safe-area-inset-top), 12px)` then resolved to its 12px floor and
 * the Journal chrome sat under the clock. We read the insets the system
 * actually dispatched and the page applies them as `--safe-area-inset-*` on
 * `:root` (`src/lib/safe-area.ts`). Nothing here knows a bar height: it is
 * whatever this device reports, and it follows rotation, gesture vs
 * three-button navigation and cutout changes through the insets listener.
 *
 * **Load order:** the page can boot before or after the first push, so both
 * directions are covered. A push runs against the document that exists
 * now, and the [Bridge] (exposed as `window.OutlSafeArea`) lets a document
 * that loads later read the current value synchronously at boot.
 *
 * The bridge is registered from [attach], which `MainActivity` calls from
 * wry's `onWebViewCreate`, before the first `loadUrl`. That ordering is what
 * closes the gap between the two directions: an `addJavascriptInterface`
 * only reaches documents created after it, and a push evaluated before the
 * first navigation commits lands in the outgoing blank document and is lost.
 * With the bridge already in place, the first real document always finds
 * it, and [latest] is written before every push, so whichever of "push" and
 * "page reads `get()`" comes second sees the current value. The push
 * deduplication below therefore never strands a document.
 *
 * The keyboard (IME) is deliberately not part of this: the web layer
 * already tracks it through `visualViewport` (`lib/viewport.ts`).
 */
object OutlSafeArea {
  private const val EMPTY = """{"top":0,"right":0,"bottom":0,"left":0}"""

  @Volatile private var latest: String = EMPTY
  private var pushedJson: String? = null
  private var pushedTo: WeakReference<WebView>? = null
  private val bridge = Bridge()

  /** Read by the page on its own thread, hence [latest] is volatile. */
  class Bridge {
    @JavascriptInterface fun get(): String = latest
  }

  /**
   * Exposes `window.OutlSafeArea` on [web] before it navigates anywhere.
   * Must run before the first `loadUrl`; see the load-order note above.
   */
  fun attach(web: WebView) {
    if (pushedTo?.get() === web) return
    web.addJavascriptInterface(bridge, "OutlSafeArea")
    pushedTo = WeakReference(web)
    pushedJson = null
  }

  fun install(activity: Activity) {
    val decor = activity.window.decorView
    ViewCompat.setOnApplyWindowInsetsListener(decor) { view, insets ->
      val bars = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or WindowInsetsCompat.Type.displayCutout(),
      )
      // Pixels to CSS pixels: the WebView's devicePixelRatio is the display density.
      val d = view.resources.displayMetrics.density
      latest = json(bars.top / d, bars.right / d, bars.bottom / d, bars.left / d)
      push(view)
      // This listener only observes: returning `insets` unchanged consumes
      // nothing. Calling ViewCompat.onApplyWindowInsets(view, ...) here would
      // re-enter this same listener, since it replaces the view's own handler.
      insets
    }
    // The WebView is created after this runs; the first layout that contains
    // it is the first chance to hand it the value.
    decor.viewTreeObserver.addOnGlobalLayoutListener { push(decor) }
    ViewCompat.requestApplyInsets(decor)
  }

  private fun json(top: Float, right: Float, bottom: Float, left: Float): String =
    String.format(
      java.util.Locale.ROOT,
      """{"top":%.2f,"right":%.2f,"bottom":%.2f,"left":%.2f}""",
      top, right, bottom, left,
    )

  private fun push(root: View) {
    val known = pushedTo?.get()
    if (known != null && known.isAttachedToWindow && pushedJson == latest) return

    val web = known?.takeIf { it.isAttachedToWindow } ?: OutlCapture.findWebView(root) ?: return
    // Fallback for a WebView [attach] never saw: visible to documents loaded
    // after this call, and the evaluate below covers the one already loaded.
    if (web !== known) attach(web)
    pushedJson = latest
    web.evaluateJavascript(
      "window.__outlSafeArea=$latest;" +
        "window.__outlApplySafeArea&&window.__outlApplySafeArea(window.__outlSafeArea);",
      null,
    )
  }
}
