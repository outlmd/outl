import Foundation

/// Action identifiers exposed by the keyboard toolbar.
///
/// The `rawValue` is what the iOS side ships to JS via
/// `window.__outlToolbar(action)`. The Solid frontend (`Journal.tsx`)
/// switches on these exact strings, so **renaming a case here breaks
/// the runtime bridge** — keep the raw values in sync with the JS
/// handler (or change them on both sides in the same commit).
public enum ToolbarAction: String, CaseIterable, Sendable {
    case newLine
    case indent
    case outdent
    case insertRef
    case toggleTodo = "todo"
    case bold
    case italic
    case moveUp
    case moveDown
    case undo
    case redo
    case insertHash
    case insertBlock
    case code
    case delete
    case done

    /// Cold-start order. Used until the user has tapped enough times
    /// for MFU to take over, and as the deterministic tiebreak for
    /// actions with equal counts.
    public static let defaultOrder: [ToolbarAction] = [
        .newLine,
        .indent,
        .outdent,
        .insertRef,
        .toggleTodo,
        .bold,
        .italic,
        .moveUp,
        .moveDown,
        .undo,
        .redo,
        .insertHash,
        .insertBlock,
        .code,
        .delete,
        .done,
    ]

    /// Always sits at index 0 in the rendered row — creating a new
    /// block is the outliner's primary act, worth one tap from the
    /// thumb's resting position regardless of MFU stats.
    public static let pinnedFirst: ToolbarAction = .newLine

    /// Always sits at the last index — "hide keyboard" lives where iOS
    /// muscle memory expects "Done".
    public static let pinnedLast: ToolbarAction = .done
}
