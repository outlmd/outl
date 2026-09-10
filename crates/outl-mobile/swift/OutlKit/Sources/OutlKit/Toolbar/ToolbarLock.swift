import Foundation

/// Toolbar order lock — the user's opt-out from MFU reordering.
///
/// `ToolbarMFU` answers *"which order does usage suggest?"*. This
/// answers *"is the user letting usage decide at all?"*. A locked bar
/// keeps counting taps, it just stops applying the result, so
/// unlocking returns to a live MFU order rather than the cold-start
/// one.
///
/// **A lock stores an order, not a boolean.** Counting keeps running
/// while locked, so a flag alone would leave the row free to jump the
/// moment it is unlocked-and-relocked. Freezing the concrete order is
/// what makes the promise ("these buttons stay where they are") true.
///
/// **This is the one piece of toolbar state iOS does not keep in
/// `UserDefaults`.** The MFU counts can afford a store per bar because
/// only one bar runs per device. The lock cannot: it is written from
/// the web settings sheet on iOS too, so `localStorage` is its single
/// home and `OutlToolbarView` pulls the value out of the webview. A
/// `UserDefaults` copy here would be a second answer to one question.
///
/// Everything in this enum is a pure function over an explicit input,
/// which is what lets it be unit-tested without a `WKWebView`.
public enum ToolbarLock {

    /// `localStorage` key holding the frozen order, as a JSON array of
    /// `ToolbarAction` raw values. Same string as the TS
    /// `LOCK_STORAGE_KEY` in `@outl/shared/toolbar`: a cross-language
    /// contract, so renaming it means renaming it there in the same
    /// commit.
    public static let storageKey = "outl.toolbar.lock.v1"

    /// Reconcile a stored order against the current catalog.
    ///
    /// A frozen order is a snapshot and the catalog moves under it: an
    /// action added in a later release is in no snapshot, one removed
    /// is in every older one. Locking the toolbar must not mean losing
    /// a button that ships afterwards, so unknown raw values (and the
    /// pinned slots, and duplicates) are dropped and every middle
    /// action the snapshot never saw is appended in catalog order.
    ///
    /// Appended rather than inserted at its catalog index on purpose:
    /// the user locked the row to stop things moving, so a new button
    /// joins at the end where it disturbs nothing.
    public static func reconcile(_ saved: [String]) -> [ToolbarAction] {
        let known = Set(ToolbarAction.middleOrder)
        var seen = Set<ToolbarAction>()
        var order: [ToolbarAction] = []
        for raw in saved {
            guard
                let action = ToolbarAction(rawValue: raw),
                known.contains(action),
                !seen.contains(action)
            else { continue }
            seen.insert(action)
            order.append(action)
        }
        for action in ToolbarAction.middleOrder where !seen.contains(action) {
            order.append(action)
        }
        return order
    }

    /// Parse the JSON blob read out of `localStorage`. Tolerant of a
    /// missing key, malformed JSON and non-array shapes — any of which
    /// means "not locked" rather than an error. A stored empty array
    /// still counts as locked; it reconciles back to the full catalog
    /// order.
    public static func parse(_ raw: String?) -> [ToolbarAction]? {
        guard let raw, let data = raw.data(using: .utf8) else { return nil }
        guard
            let parsed = try? JSONSerialization.jsonObject(with: data),
            let array = parsed as? [Any]
        else { return nil }
        return reconcile(array.compactMap { $0 as? String })
    }

    /// The order the middle range should render in: the frozen one when
    /// locked, the live MFU one otherwise. Mirrors the TS
    /// `resolveMiddleOrder`, and is the single place either platform
    /// decides.
    public static func resolveMiddleActions(
        counts: [String: Int],
        locked: [ToolbarAction]?
    ) -> [ToolbarAction] {
        locked ?? ToolbarMFU.orderedMiddleActions(counts: counts)
    }
}
