package app.outl.mobile_app

import android.content.Context
import android.net.wifi.WifiManager
import android.util.Log

/**
 * Holds the Wi-Fi `MulticastLock` that mDNS peer discovery needs (issue #149).
 *
 * ## Why this file has to exist
 *
 * Android's Wi-Fi stack drops multicast and broadcast frames that are not
 * addressed to this device, as a battery optimisation. That filter sits in the
 * driver, below the socket API — so `swarm-discovery` (behind
 * `iroh-mdns-address-lookup`) joins `224.0.0.251`, gets no error, sends its
 * queries fine, and simply never receives an answer. Discovery finds nobody,
 * and nothing in the Rust layer can tell that apart from "there are no peers on
 * this LAN". Wiring mDNS without this would have shipped Android a feature that
 * looks present in the logs and does nothing.
 *
 * `WifiManager.MulticastLock` is the only way to lift that filter, and it needs
 * `CHANGE_WIFI_MULTICAST_STATE` in the manifest.
 *
 * ## Why it follows resume/pause, not the activity's lifetime
 *
 * The lock costs battery — a documented drain, because the radio stops
 * filtering and the CPU processes every multicast frame on the network. It buys
 * something only while a foreground session wants to find a peer *now*.
 * Background sync runs off known peers and the relay, neither of which needs
 * multicast, so holding it while backgrounded would be pure cost.
 *
 * Not reference counted, so the pause path releases in one call however many
 * times resume ran.
 *
 * ## Best-effort, like the Rust side
 *
 * Every failure is logged and swallowed, matching `bind::attach_mdns`: a device
 * with no Wi-Fi service, or an OEM that refuses the lock, still syncs over the
 * relay exactly as before. Losing LAN discovery is a degradation; refusing to
 * run is not an acceptable trade for it.
 */
object OutlMulticast {
  private const val TAG = "OutlMulticast"
  private const val LOCK_TAG = "outl-mdns"

  private var lock: WifiManager.MulticastLock? = null

  /** Stop the Wi-Fi driver filtering multicast, so mDNS answers reach us. */
  @Synchronized
  fun acquire(context: Context) {
    if (lock?.isHeld == true) return
    try {
      val wifi = context.applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
      if (wifi == null) {
        Log.w(TAG, "no WifiManager; mDNS peer discovery stays off, relay sync unaffected")
        return
      }
      lock =
          wifi.createMulticastLock(LOCK_TAG).apply {
            setReferenceCounted(false)
            acquire()
          }
      Log.d(TAG, "multicast lock held; mDNS peer discovery active")
    } catch (e: Exception) {
      Log.w(TAG, "could not hold the multicast lock; relay sync unaffected", e)
    }
  }

  /** Hand the battery back. Safe to call when nothing is held. */
  @Synchronized
  fun release() {
    try {
      lock?.takeIf { it.isHeld }?.release()
    } catch (e: Exception) {
      Log.w(TAG, "could not release the multicast lock", e)
    } finally {
      lock = null
    }
  }
}
