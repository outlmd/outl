import Foundation

/// Reads the toolbar's stored state out of the webview's
/// `localStorage`, which is where **both** the tap counts and the
/// order lock live.
///
/// The bar on iOS is native, but the settings sheet that locks and
/// resets it is web. A `UserDefaults` store on this side would be
/// invisible to that sheet: "Lock button order" would freeze a
/// cold-start row rather than the user's own, and "Reset button order"
/// would do nothing at all. One store, read from here, is what makes
/// those two buttons mean what they say.
///
/// `OutlToolbarView` asks for both keys in a single
/// `evaluateJavaScript`, so this parses the pair together — reading
/// them separately would let the lock come from one moment and the
/// counts from another.
public enum ToolbarStore {

    /// What one read of `localStorage` yielded.
    public struct Pair: Equatable, Sendable {
        /// Tap counts, empty when the key is missing or unreadable.
        public let counts: [String: Int]
        /// The frozen order, or `nil` when the toolbar is not locked.
        public let locked: [ToolbarAction]?

        public init(counts: [String: Int], locked: [ToolbarAction]?) {
            self.counts = counts
            self.locked = locked
        }
    }

    /// Parse the `JSON.stringify([mfuRaw, lockRaw])` payload the bar
    /// evaluates. Both slots are independently nullable: a device that
    /// has never tapped a button has no counts, and one that never
    /// locked the bar has no lock, and neither is an error.
    ///
    /// Returns `nil` only when the payload itself is unusable, which
    /// the caller treats as "keep what you had" rather than as empty —
    /// clearing on a failed read is how a locked bar silently unlocks.
    public static func parsePair(_ raw: String?) -> Pair? {
        guard let raw, let data = raw.data(using: .utf8) else { return nil }
        guard
            let parsed = try? JSONSerialization.jsonObject(with: data),
            let slots = parsed as? [Any],
            slots.count == 2
        else { return nil }
        return Pair(
            counts: parseCounts(slots[0] as? String),
            locked: ToolbarLock.parse(slots[1] as? String)
        )
    }

    /// Parse the counts blob written by `@outl/shared/toolbar`'s
    /// `recordToStore`. Tolerant of a missing key, malformed JSON,
    /// non-object shapes, unknown action ids and non-integer values —
    /// any of which contributes nothing rather than failing the read.
    ///
    /// Mirrors the TS `parseCounts`, including truncating a
    /// non-integer number rather than dropping it: JSON has one number
    /// type, so a count that round-tripped through a float must not
    /// vanish.
    public static func parseCounts(_ raw: String?) -> [String: Int] {
        guard let raw, let data = raw.data(using: .utf8) else { return [:] }
        guard
            let parsed = try? JSONSerialization.jsonObject(with: data),
            let object = parsed as? [String: Any]
        else { return [:] }
        var counts: [String: Int] = [:]
        for (key, value) in object {
            guard
                ToolbarAction(rawValue: key) != nil,
                let number = value as? NSNumber,
                // `JSONSerialization` hands back `NSNumber` for both
                // numbers and booleans, and `is Bool` does NOT separate
                // them: an `NSNumber` holding 1 bridges to `true`, so
                // that test would have thrown away every count of 1 —
                // which is every button's first tap. Compare the
                // CoreFoundation type instead, the only check that
                // actually distinguishes the two.
                CFGetTypeID(number) != CFBooleanGetTypeID(),
                number.doubleValue.isFinite,
                // `Int(Double)` traps when the value is outside `Int`'s
                // range, and this is `localStorage` — a corrupted or
                // hand-edited blob must cost the bar one count, not a
                // crash. `Int(exactly:)` returns `nil` instead.
                let count = Int(exactly: number.doubleValue.rounded(.towardZero))
            else { continue }
            counts[key] = count
        }
        return counts
    }
}
