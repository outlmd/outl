import BackgroundTasks
import Foundation
import UIKit

/// Rust FFI, defined in `src/bg_sync.rs` and linked into the app's static
/// library.
///
/// Fires a forced iroh sync pass (`sync_now`) against every paired peer and
/// blocks until **that** pass completes or `seconds` elapses, early-exiting
/// the moment it lands; returns `false` when iroh isn't wired.
///
/// One symbol for all three windows. Each caller knows its own ceiling — the
/// `BGProcessingTask`'s minutes, the `BGAppRefreshTask`'s ~30s, and the
/// flush's whatever `backgroundTimeRemaining` reports — and only the first two
/// are constants. Rust clamps the argument, so an expired budget or an
/// arithmetic slip here cannot hang the thread.
@_silgen_name("outl_ios_background_sync_capped")
private func outlIosBackgroundSync(_ seconds: UInt32) -> Bool

/// Number of paired peers, read fresh from `<workspace>/.outl/peers.json`.
/// `0` when the Rust side hasn't registered the transport yet (early in
/// launch, or iroh disabled). Gates BG-task submission below.
@_silgen_name("outl_ios_peer_count")
private func outlIosPeerCount() -> UInt32

/// Background work for P2P sync.
///
/// iOS gives apps two kinds of opportunistic background windows; we use
/// both. Each identifier MUST be listed in Info.plist's
/// `BGTaskSchedulerPermittedIdentifiers`, and the matching `UIBackgroundModes`
/// (`fetch` / `processing`) must be declared, or `register`/`submit` fail.
///
/// - `app.outl.mobile-app.refresh` — a short `BGAppRefreshTask`. A handful
///   of windows a day, ~30s each. Drives a forced sync pass capped at
///   [`refreshWindowSeconds`] so even the cheap windows pull fresh ops
///   instead of being wasted on a bare reschedule.
/// - `app.outl.mobile-app.sync` — a longer `BGProcessingTask` that requires
///   network connectivity. iOS grants it when the device is on Wi-Fi (often
///   charging) and it can run for minutes — enough for an iroh pull/push to
///   complete.
///
/// There is **no continuous background P2P on iOS**: the system suspends the
/// app's sockets shortly after it leaves the foreground. These tasks are the
/// only sanctioned way to sync while the app is closed, and the OS — not us —
/// decides when they fire.
///
/// They are also the wrong tool for the *handover*. Neither one covers the
/// exchange already in flight when the user locks the screen — both are
/// requests for a window minutes or hours from now. That gap is
/// [`flushOnBackground`], a `beginBackgroundTask` assertion that keeps the
/// process (and its sockets) alive for one last forced pass. Three pieces,
/// three jobs: flush what is running, refresh cheaply and often, sync deeply
/// when the device is idle on Wi-Fi.
///
/// How the sync actually happens: when iOS launches/resumes the app for one
/// of these tasks, the Tauri `setup` hook brings `IrohSyncTransport` up (the
/// same path as a normal launch), and its catch-up loop runs a delta sync
/// against every paired peer. The handlers below keep the task alive only
/// until the forced pass reports completion (or a bounded ceiling), then
/// report completion so iOS keeps granting future windows — returning unused
/// window early is what keeps the grants coming.
///
/// Scheduling is gated on having at least one paired peer
/// (`outl_ios_peer_count() > 0`): with zero peers a background wake boots
/// the whole app for nothing. The handlers are ALWAYS registered (mandatory
/// before the end of launch); only the `submit` is conditional. Because the
/// launch-time submit usually runs before the Rust side has registered the
/// transport (peer count reads 0), the schedule is re-armed on every
/// `didEnterBackground` — which also arms it right after the user pairs
/// their first peer with the app open, no Rust→Swift bridge needed.
@objc(OutlBackgroundRefresh)
public final class OutlBackgroundRefresh: NSObject {

    private static let refreshIdentifier = "app.outl.mobile-app.refresh"
    private static let syncIdentifier = "app.outl.mobile-app.sync"

    /// Floor, not a guarantee — iOS schedules when it can. Kept modest (15 min,
    /// not 1 h) so the scheduler has more latitude to grant a window; a larger
    /// floor is a self-imposed ceiling on how soon a background sync can run.
    private static let interval: TimeInterval = 15 * 60

    /// Cap for the `BGAppRefreshTask` pass. Its whole window is ~30s, so this
    /// leaves headroom for the handler to report completion before iOS expires
    /// the task — an expired task is one iOS grants less often afterwards.
    private static let refreshWindowSeconds: UInt32 = 12

    /// Cap for the `BGProcessingTask` pass, which iOS grants minutes for.
    /// Cross-network iroh connects can take ~20s (multipath); with early-exit
    /// this is a ceiling, not the typical duration.
    private static let processingWindowSeconds: UInt32 = 20

    /// Single entry point called by `OutlBootstrap.+load` in `main.mm`.
    @objc public static func install() {
        guard #available(iOS 13.0, *) else { return }
        // Register the launch handlers SYNCHRONOUSLY, right here. Apple requires
        // every `BGTaskScheduler` handler to be registered before the app
        // finishes launching — otherwise a COLD background launch for that task
        // can't find its handler and iOS silently drops the window (the likely
        // cause of "doesn't sync while closed"). `register` needs no UIKit state
        // (only `submit`/scheduling touch app state), and `install()` is already
        // hopped onto the main queue by `main.mm`'s `+load`, so the previous
        // extra `DispatchQueue.main.async` here only pushed registration a
        // runloop turn too late. Keep only the observer registration inline too.
        registerTasks()
        // Re-arm on every backgrounding: at launch `registerTasks` runs BEFORE
        // the Rust side registers the transport (peer count reads 0, so nothing
        // is submitted), and pairing the first peer happens with the app open.
        // Both cases arm here, the moment the app actually goes to background.
        // Re-submitting an already-pending identifier just replaces the request,
        // so this is idempotent.
        //
        // `queue: nil` is deliberate and load-bearing: it delivers the block
        // SYNCHRONOUSLY on the posting thread. `queue: .main` enqueues it
        // instead, which was harmless while this only submitted BGTask
        // requests, and is not once it takes a background assertion — a
        // runloop turn is exactly the window in which iOS starts suspending us.
        NotificationCenter.default.addObserver(
            forName: UIApplication.didEnterBackgroundNotification,
            object: nil,
            queue: nil
        ) { _ in
            // Assertion FIRST: everything else here can afford a delay, and
            // this is the only part that stops mattering once we are suspended.
            flushOnBackground()
            scheduleRefresh()
            scheduleSync()
        }
        // Coming back to the foreground ends the flush early. Without this, a
        // background → foreground → background bounce (a glance at a
        // notification, a failed Face ID, a peek in the app switcher) leaves
        // the first flush still holding `flushTaskId`, and the re-entry guard
        // then skips the SECOND backgrounding — the real one — entirely.
        NotificationCenter.default.addObserver(
            forName: UIApplication.willEnterForegroundNotification,
            object: nil,
            queue: nil
        ) { _ in
            endFlush()
        }
    }

    // ── Finish the in-flight pass before suspension ──────────────────────

    /// The background-task assertion held while [`flushOnBackground`] runs.
    /// `.invalid` when none is held. Guarded by `flushLock`.
    private static var flushTaskId: UIBackgroundTaskIdentifier = .invalid
    private static let flushLock = NSLock()

    /// Seconds held back from `backgroundTimeRemaining` so the FFI returns and
    /// the assertion is released *before* the expiration handler is the one
    /// doing it. Ending on our own terms keeps the connection teardown
    /// graceful; ending on iOS's is the tear-down we are trying to avoid.
    private static let handlerReserve: Double = 5

    /// Ceiling regardless of a generous budget. Past this the honest answer is
    /// "this peer isn't reachable right now" — that is what the two
    /// `BGTaskScheduler` windows are for.
    private static let maxFlushSeconds: UInt32 = 20

    /// Floor regardless of a stingy budget. Below this there is no point
    /// starting: a same-LAN pass needs a couple of seconds, and returning
    /// instantly would leave the exchange exactly as torn down as doing
    /// nothing.
    private static let minFlushSeconds: UInt32 = 3

    /// Hold a runtime assertion across one last forced sync pass when the app
    /// backgrounds (screen lock, app switcher, home).
    ///
    /// **This is not the same thing as the two `BGTaskScheduler` windows
    /// above, and it is the piece they cannot cover.** `submit` asks iOS for a
    /// window *later* — minutes to hours, at the scheduler's discretion. It
    /// does nothing for the exchange happening at the instant the screen
    /// locks, and iOS suspends the process (and its sockets) within seconds of
    /// `didEnterBackground`. A delta sync torn down at that point is not a
    /// silent partial success: the responder confirms durable ingest by
    /// closing with code 0, so the peer reports
    /// `peer did not confirm durable ingest (closed: timed out)` and re-pushes
    /// on its next tick — correct, but the user sees a red row in the desktop
    /// Sync panel every time they lock their phone.
    ///
    /// `beginBackgroundTask` buys ~30s of runtime for exactly this: the app
    /// stays resident, the QUIC connection stays alive, and both the pass we
    /// drive here *and* any inbound sync a peer is mid-way through get to
    /// finish. It needs no `UIBackgroundModes` entry — it is an assertion, not
    /// a mode.
    ///
    /// Contract, in order of what kills the app if you get it wrong:
    ///
    /// - The assertion MUST be ended exactly once, and the expiration handler
    ///   is the deadline — iOS terminates an app that lets one run out. Both
    ///   paths funnel through [`endFlush`], which is idempotent.
    /// - The full-window FFI (20s cap) is the right one despite the ~30s
    ///   budget: the whole point is to *finish*, a relay pass costs ~20s, and
    ///   the expiration handler is the safety net if this device's budget is
    ///   smaller than usual. The FFI thread unwinds on its own afterwards and
    ///   its late `endFlush()` is a no-op.
    /// - Skipped with zero paired peers (nothing to flush) and when the
    ///   assertion is denied (`.invalid`, i.e. iOS is already suspending us).
    /// - A second backgrounding while one is still in flight keeps the first;
    ///   re-entering would leak the earlier identifier.
    private static func flushOnBackground() {
        flushLock.lock()
        guard flushTaskId == .invalid else {
            flushLock.unlock()
            return
        }
        let id = UIApplication.shared.beginBackgroundTask(withName: "app.outl.sync.flush") {
            // iOS is reclaiming the window — end now or be terminated.
            endFlush()
        }
        guard id != .invalid else {
            flushLock.unlock()
            NSLog("[outl] bg flush: assertion denied, skipping final sync pass")
            return
        }
        // Assigned BEFORE the unlock, and that ordering is the whole reason
        // the expiration handler cannot fire against a still-`.invalid`
        // static and leak this identifier: any handler on another thread
        // blocks on `flushLock` until the id is visible, and one on main
        // cannot run while this function does. Do not reorder these two lines.
        flushTaskId = id
        flushLock.unlock()

        // Size the pass against what iOS ACTUALLY granted, read now that the
        // assertion is held. A fixed 20s is wrong in both directions: it
        // overruns a short budget (the expiration handler then tears down the
        // very exchange we are protecting) and it under-uses a long one.
        //
        // Two out-of-range answers, and it is `min`/`max` that handle both,
        // not `isFinite`. While the app is not yet fully backgrounded,
        // `backgroundTimeRemaining` is `.greatestFiniteMagnitude`, which IS
        // finite — the `min` is what caps it. And a budget already smaller
        // than the reserve makes `usable` negative, which the `max` floors.
        // `isFinite` only catches an actual infinity or NaN.
        let budget = UIApplication.shared.backgroundTimeRemaining
        let usable = budget.isFinite ? budget - handlerReserve : Double(maxFlushSeconds)
        let cap = UInt32(max(Double(minFlushSeconds), min(Double(maxFlushSeconds), usable)))

        DispatchQueue.global(qos: .utility).async {
            // Read the peer list HERE, not before taking the assertion: it
            // hits the disk (and an iCloud-hosted workspace can make that
            // slow), and the main thread is being asked to hand control back
            // to iOS. With no peers there is nothing to flush, so the
            // assertion is released immediately — cheaper than blocking the
            // background transition to find that out.
            if outlIosPeerCount() > 0 {
                let ok = outlIosBackgroundSync(cap)
                NSLog("[outl] bg flush: final pass finished in \(cap)s cap (transport=\(ok))")
            }
            endFlush(matching: id)
        }
    }

    /// End the flush assertion. Idempotent, and safe against every racer:
    /// the sync thread, the expiration handler, and the foreground observer
    /// all call it, while `endBackgroundTask` must never run twice on one
    /// identifier.
    ///
    /// `matching` guards the one case a bare "end whatever is current" gets
    /// wrong. A foreground bounce ends flush A while its worker is still
    /// blocked in the FFI; the next backgrounding starts flush B; A's worker
    /// then finishes and would end **B's** assertion, suspending the device
    /// mid-exchange — the exact failure this whole mechanism exists to
    /// prevent. Pass the identifier you took; pass `nil` (the expiration
    /// handler, the foreground observer) to end whatever is current.
    ///
    /// `UIApplication` is main-thread API, and the expiration handler is
    /// already on main and cannot afford a deferred hop (it *is* the
    /// deadline), so an off-main call dispatches **synchronously**. No lock is
    /// held across that hop.
    private static func endFlush(matching expected: UIBackgroundTaskIdentifier? = nil) {
        flushLock.lock()
        let id = flushTaskId
        if let expected, id != expected {
            // A newer flush owns the assertion — leave it alone.
            flushLock.unlock()
            return
        }
        flushTaskId = .invalid
        flushLock.unlock()
        guard id != .invalid else { return }
        if Thread.isMainThread {
            UIApplication.shared.endBackgroundTask(id)
        } else {
            DispatchQueue.main.sync { UIApplication.shared.endBackgroundTask(id) }
        }
    }

    @available(iOS 13.0, *)
    private static func registerTasks() {
        let refreshOk = BGTaskScheduler.shared.register(
            forTaskWithIdentifier: refreshIdentifier,
            using: nil
        ) { task in
            // The refresh window is ~30s: run the short-capped pass so the
            // sync finishes (or bails) with headroom to report completion.
            handleTask(task, reschedule: scheduleRefresh) {
                outlIosBackgroundSync(refreshWindowSeconds)
            }
        }

        let syncOk = BGTaskScheduler.shared.register(
            forTaskWithIdentifier: syncIdentifier,
            using: nil
        ) { task in
            handleTask(task, reschedule: scheduleSync) {
                outlIosBackgroundSync(processingWindowSeconds)
            }
        }

        if refreshOk { scheduleRefresh() }
        if syncOk { scheduleSync() }
        NSLog("[outl] background tasks registered (refresh=\(refreshOk) sync=\(syncOk))")
    }

    /// Shared BG-task driver: reschedule, run the sync FFI off-thread,
    /// report completion exactly once.
    @available(iOS 13.0, *)
    private static func handleTask(
        _ task: BGTask,
        reschedule: () -> Void,
        sync: @escaping () -> Bool
    ) {
        // Reschedule first so a crash still leaves a future window armed.
        // (The submit inside is peer-gated, so an unpaired device stops
        // rescheduling itself here and re-arms on the next backgrounding
        // after a pair.)
        reschedule()

        // Report completion exactly once — the sync work and the OS
        // expiration handler race, and BGTaskScheduler rejects a double
        // `setTaskCompleted`.
        let lock = NSLock()
        var reported = false
        func complete(_ success: Bool) {
            lock.lock()
            defer { lock.unlock() }
            guard !reported else { return }
            reported = true
            task.setTaskCompleted(success: success)
        }

        // Drive the actual pull/push on a background queue: the FFI blocks
        // while iroh dials every peer and exchanges ops, returning as soon
        // as the forced pass completes (bounded by its cap). The mobile side
        // initiating is NAT-friendly, so this works even when the Mac can't
        // reach the phone directly.
        DispatchQueue.global(qos: .background).async {
            let ok = sync()
            complete(ok)
        }
        task.expirationHandler = {
            // iOS pulled the window — report now; the FFI thread unwinds on
            // its own and its later `complete(_:)` is a no-op.
            complete(false)
        }
    }

    /// Gate: with zero paired peers a background wake boots the whole app
    /// for nothing, so submission is skipped. `outlIosPeerCount()` reads
    /// `<workspace>/.outl/peers.json` through the transport registered by
    /// the Rust side — it returns 0 until that registration happens (early
    /// in launch); the `didEnterBackground` re-arm in `install()` covers
    /// that window and the "first peer paired with the app open" case.
    private static func peersArePaired() -> Bool {
        let count = outlIosPeerCount()
        if count == 0 {
            NSLog("[outl] bg schedule skipped: no paired peers")
        }
        return count > 0
    }

    @available(iOS 13.0, *)
    private static func scheduleRefresh() {
        guard peersArePaired() else { return }
        let req = BGAppRefreshTaskRequest(identifier: refreshIdentifier)
        req.earliestBeginDate = Date(timeIntervalSinceNow: interval)
        submit(req)
    }

    @available(iOS 13.0, *)
    private static func scheduleSync() {
        guard peersArePaired() else { return }
        let req = BGProcessingTaskRequest(identifier: syncIdentifier)
        req.requiresNetworkConnectivity = true
        req.requiresExternalPower = false
        req.earliestBeginDate = Date(timeIntervalSinceNow: interval)
        submit(req)
    }

    @available(iOS 13.0, *)
    private static func submit(_ req: BGTaskRequest) {
        do {
            try BGTaskScheduler.shared.submit(req)
        } catch {
            #if targetEnvironment(simulator)
            // No BGTaskScheduler daemon on the sim — submit always fails;
            // swallow so dev builds stay quiet. Registration still works.
            #else
            NSLog("[outl] schedule \(req.identifier) failed: \(error.localizedDescription)")
            #endif
        }
    }
}
