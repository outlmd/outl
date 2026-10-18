package app.outl.mobile_app

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.graphics.Typeface
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.util.TypedValue
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

/**
 * What the user sees when `liboutl_mobile_lib.so` will not load on their
 * device, instead of the app closing on open.
 *
 * Deliberately not a `TauriActivity`: everything wry and Tauri do in their
 * lifecycle hooks calls into the library that just failed, so this is a
 * plain activity with views built in code, and it never touches a native
 * symbol. It says what happened, names the two facts that decide most of
 * these (ABI and page size), and points at the issue tracker. The text is
 * selectable so it can be pasted into a report as-is.
 *
 * Started by `MainActivity` after [OutlNativeLibrary] reports a load error.
 * Not exported: nothing outside the app has a reason to open it.
 */
class NativeLoadErrorActivity : AppCompatActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val error = intent.getStringExtra(EXTRA_ERROR) ?: getString(R.string.native_load_error_unknown)
    setContentView(content(error))
  }

  private fun content(error: String): ScrollView {
    val pad = dp(24)
    val column = LinearLayout(this).apply {
      orientation = LinearLayout.VERTICAL
      setPadding(pad, pad, pad, pad)
    }

    column.addView(text(getString(R.string.native_load_error_title), sizeSp = 22f, bold = true))
    column.addView(text(getString(R.string.native_load_error_body)))
    column.addView(
      text(details(error), sizeSp = 14f, monospace = true).apply { setTextIsSelectable(true) },
    )
    column.addView(text(getString(R.string.native_load_error_report, ISSUES_LABEL)))
    column.addView(
      Button(this).apply {
        setText(R.string.native_load_error_open_issues)
        setOnClickListener { openIssues() }
      },
      LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(16) },
    )

    val scroll = ScrollView(this).apply { addView(column) }
    // targetSdk 35+ draws edge to edge whether we ask or not, so keep the
    // text out from under the status and navigation bars.
    ViewCompat.setOnApplyWindowInsetsListener(scroll) { view, insets ->
      val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
      view.setPadding(bars.left, bars.top, bars.right, bars.bottom)
      insets
    }
    return scroll
  }

  private fun details(error: String): String {
    val page = OutlNativeLibrary.pageSizeBytes()
    return getString(
      R.string.native_load_error_details,
      OutlNativeLibrary.abis(),
      if (page == null) getString(R.string.native_load_error_unknown) else "$page bytes (${page / 1024} KB)",
      "${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})",
      "${Build.MANUFACTURER} ${Build.MODEL}",
      BuildConfig.VERSION_NAME,
      error,
    )
  }

  private fun openIssues() {
    try {
      startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(OutlNativeLibrary.ISSUES_URL)))
    } catch (e: ActivityNotFoundException) {
      // No browser. The address is on screen and selectable anyway.
    }
  }

  private fun text(
    value: String,
    sizeSp: Float = 16f,
    bold: Boolean = false,
    monospace: Boolean = false,
  ) = TextView(this).apply {
    text = value
    setTextSize(TypedValue.COMPLEX_UNIT_SP, sizeSp)
    typeface = when {
      bold -> Typeface.DEFAULT_BOLD
      monospace -> Typeface.MONOSPACE
      else -> Typeface.DEFAULT
    }
    layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = dp(12) }
  }

  private fun dp(value: Int): Int =
    TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value.toFloat(), resources.displayMetrics).toInt()

  companion object {
    private const val EXTRA_ERROR = "app.outl.mobile_app.NATIVE_LOAD_ERROR"

    /** The tracker address as people would type it, without the scheme. */
    private const val ISSUES_LABEL = "github.com/outlmd/outl/issues"

    fun intent(context: Context, error: Throwable): Intent =
      Intent(context, NativeLoadErrorActivity::class.java)
        .putExtra(EXTRA_ERROR, error.toString())
  }
}
