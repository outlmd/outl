package app.outl.mobile_app

import android.os.Build
import android.system.Os
import android.system.OsConstants
import android.util.Log

/**
 * The one answer to "did `liboutl_mobile_lib.so` load in this process?".
 *
 * Every Kotlin entry point that reaches Rust asks here first instead of
 * finding out from an `UnsatisfiedLinkError` of its own. Tauri's generated
 * `Rust` object loads the same library from a static initializer, and a
 * static initializer that throws does not throw *there*: the JVM wraps it in
 * `ExceptionInInitializerError` at whatever line first touches the class,
 * and every later touch is a `NoClassDefFoundError`. For `MainActivity`
 * that line is `super.onCreate`, before any of our code has a say, so a
 * library the device refuses — 4 KB-aligned on a 16 KB-page phone, which is
 * what v0.12.0-beta.226 shipped, or an ABI the APK does not carry — used to
 * close the app on open with nothing on screen and one stack trace in
 * logcat.
 *
 * Asking first turns that into a decision the caller makes: `MainActivity`
 * shows [NativeLoadErrorActivity], the share target says nothing was saved,
 * the worker reports a failure. None of them touches a native symbol after
 * a `false` here.
 *
 * `System.loadLibrary` is idempotent per class loader, so a successful
 * probe makes the later loads in `Rust`, `NativeSetup` and `NativeSync`
 * no-ops rather than second attempts.
 */
internal object OutlNativeLibrary {
  private const val TAG = "outl"

  /** `liboutl_mobile_lib.so`, the `[lib] name` in `src-tauri/Cargo.toml`. */
  private const val NAME = "outl_mobile_lib"

  /** Where the error screen sends people. */
  const val ISSUES_URL = "https://github.com/outlmd/outl/issues"

  /**
   * `null` when the library is up, the reason otherwise.
   *
   * Loaded once and remembered, because a refused `dlopen` does not start
   * succeeding later in the same process. `by lazy` is synchronized, so the
   * share target and a worker racing in a cold process still load it once.
   *
   * `LinkageError` covers `UnsatisfiedLinkError` (the 16 KB and missing-ABI
   * cases); `RuntimeException` covers a `SecurityException` from a locked-down
   * ROM. Anything else — an `OutOfMemoryError`, say — is not a fact about the
   * library and is left to crash the way it would anywhere else.
   */
  val loadError: Throwable? by lazy {
    try {
      System.loadLibrary(NAME)
      null
    } catch (e: LinkageError) {
      report(e)
      e
    } catch (e: RuntimeException) {
      report(e)
      e
    }
  }

  val isLoaded: Boolean
    get() = loadError == null

  /** `Build.SUPPORTED_ABIS`, preferred first: what the device would accept. */
  fun abis(): String = Build.SUPPORTED_ABIS.joinToString(", ")

  /**
   * The kernel page size in bytes, or `null` if the device will not say.
   * 16384 next to a load error is the beta.226 failure exactly.
   */
  fun pageSizeBytes(): Long? = try {
    Os.sysconf(OsConstants._SC_PAGESIZE).takeIf { it > 0 }
  } catch (e: Exception) {
    null
  }

  private fun report(e: Throwable) {
    // One line with everything a bug report needs, so `adb logcat -s outl`
    // is enough even when nobody saw the screen.
    Log.e(
      TAG,
      "native library lib$NAME.so did not load: abis=[${abis()}] " +
        "pageSize=${pageSizeBytes() ?: "unknown"} sdk=${Build.VERSION.SDK_INT} " +
        "device=${Build.MANUFACTURER} ${Build.MODEL} app=${BuildConfig.VERSION_NAME}",
      e,
    )
  }
}
