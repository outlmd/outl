import Foundation

/// Most-frequently-used ordering for the edit toolbar.
///
/// **Ordering only — this type no longer persists anything.** The tap
/// counts live in the webview's `localStorage`, written by
/// `@outl/shared/toolbar`'s `recordToStore` and read back through
/// `ToolbarStore`; see that type for why a `UserDefaults` copy on this
/// side made the settings sheet's Lock and Reset buttons lie.
///
/// Every function here is pure over an explicit `counts` dictionary,
/// which is what lets them be unit-tested with no device state at all.
public enum ToolbarMFU {

    /// `localStorage` key holding the tap counts. Same string as the TS
    /// `MFU_STORAGE_KEY` in `@outl/shared/toolbar`: a cross-language
    /// contract, so renaming it means renaming it there in the same
    /// commit. `OutlToolbarView` interpolates it into the JS it
    /// evaluates.
    public static let storageKey = "outl.toolbar.mfu.v1"

    /// Full ordered row, pinned slots included.
    public static func orderedActions(
        counts: [String: Int]
    ) -> [ToolbarAction] {
        [ToolbarAction.pinnedFirst]
            + orderedMiddleActions(counts: counts)
            + [ToolbarAction.pinnedLast]
    }

    /// Same MFU ordering as `orderedActions`, but **excludes the
    /// pinned slots** — only the middle (scrollable) range comes back.
    /// The view layer uses this to populate the `UIScrollView`'s stack
    /// while keeping `pinnedFirst` / `pinnedLast` as static buttons
    /// outside the scroll, so they stay visible regardless of how far
    /// the user scrolled the middle row.
    public static func orderedMiddleActions(
        counts: [String: Int]
    ) -> [ToolbarAction] {
        ToolbarAction.middleOrder.sorted { a, b in
            let ca = counts[a.rawValue] ?? 0
            let cb = counts[b.rawValue] ?? 0
            if ca != cb { return ca > cb }
            // Stable tiebreak: original `defaultOrder` position.
            let ia = ToolbarAction.defaultOrder.firstIndex(of: a) ?? 0
            let ib = ToolbarAction.defaultOrder.firstIndex(of: b) ?? 0
            return ia < ib
        }
    }
}
