import Foundation

/// Where a capture came from. Stored in the file; the app does not branch
/// on it today, it is there for the log and for later per-source rules.
///
/// **Writer-side only.** This is a closed set, and the format has more
/// values than it lists: `outl capture` writes `cli`, and a newer build
/// may add another. The Rust reader takes `source` as a plain `String`
/// for that reason. Decoding a `CaptureRecord` with this enum would
/// throw `DecodingError` on a value it does not know and lose that
/// capture — so if a reader is ever written here, widen this to a
/// `String` (or add an unknown case) first.
///
/// `widget` is emitted by nobody: both platforms' widgets open
/// `outl://capture` rather than writing a file.
public enum CaptureSource: String, Codable {
    case widget
    case intent
    case share
}

/// One capture as written to the inbox. Field names match the Rust
/// reader (`outl_actions::capture::CaptureRecord`).
public struct CaptureRecord: Codable, Equatable {
    public let text: String
    public let capturedAtMs: Int64
    public let source: CaptureSource

    public init(text: String, capturedAtMs: Int64, source: CaptureSource) {
        self.text = text
        self.capturedAtMs = capturedAtMs
        self.source = source
    }

    enum CodingKeys: String, CodingKey {
        case text
        case capturedAtMs = "captured_at_ms"
        case source
    }
}

/// The capture inbox (RFC 0363). Extensions append one `<ULID>.json` per
/// capture, written create-temp, fsync, rename, so the app sees a whole
/// capture or none of it. The app drains it (`drain_capture_inbox`);
/// nothing here touches the workspace.
public struct CaptureInbox {
    /// The App Group the app and its extensions share.
    ///
    /// Resolved per flavor: a preview build installed beside the real app
    /// has its own container, so a capture written by one is never drained
    /// by the other. See `OutlFlavor`.
    public static let appGroup = OutlFlavor.current.appGroup
    /// Darwin notification an extension posts after a write, so an app
    /// already on screen drains without waiting for the next foreground.
    /// Machine-global, so it carries the flavor too — see `OutlFlavor`.
    public static let arrivedNotification = OutlFlavor.current.captureNotification
    /// Env var the app sets so the Rust side finds the inbox.
    public static let envVar = "OUTL_CAPTURE_INBOX"
    /// Folder name inside the App Group container.
    public static let folderName = "captures"

    public enum WriteError: Error, Equatable {
        case empty
    }

    public let directory: URL

    public init(directory: URL) {
        self.directory = directory
    }

    #if canImport(Darwin)
    /// The inbox in the App Group container, or `nil` when the App Group
    /// entitlement is missing (then no extension can reach the app).
    public static func shared(fileManager: FileManager = .default) -> CaptureInbox? {
        guard let root = fileManager.containerURL(forSecurityApplicationGroupIdentifier: appGroup) else {
            return nil
        }
        return CaptureInbox(directory: root.appendingPathComponent(folderName, isDirectory: true))
    }
    #endif

    /// Write one capture and return its final URL. Whitespace-only text
    /// is refused so a stray tap never adds an empty block.
    @discardableResult
    public func write(
        text: String,
        source: CaptureSource,
        now: Date = Date(),
        random: () -> UInt64 = { UInt64.random(in: 0...UInt64.max) }
    ) throws -> URL {
        let body = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !body.isEmpty else { throw WriteError.empty }
        let ms = Int64((now.timeIntervalSince1970 * 1000).rounded(.down))
        let id = ULID.make(ms: UInt64(ms), random: random)
        let record = CaptureRecord(text: body, capturedAtMs: ms, source: source)
        let data = try JSONEncoder().encode(record)

        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let temp = directory.appendingPathComponent(".\(id).tmp")
        let final = directory.appendingPathComponent("\(id).json")
        FileManager.default.createFile(atPath: temp.path, contents: nil)
        let handle = try FileHandle(forWritingTo: temp)
        do {
            try handle.write(contentsOf: data)
            try handle.synchronize()
            try handle.close()
        } catch {
            try? handle.close()
            try? FileManager.default.removeItem(at: temp)
            throw error
        }
        try FileManager.default.moveItem(at: temp, to: final)
        return final
    }
}

/// Crockford base32 ULIDs: 48-bit ms time + 80 random bits, 26 chars.
/// Names sort in time order, which is the drain order.
public enum ULID {
    static let alphabet = Array("0123456789ABCDEFGHJKMNPQRSTVWXYZ")

    public static func make(ms: UInt64, random: () -> UInt64) -> String {
        var out = [Character](repeating: "0", count: 26)
        var time = ms & 0xFFFF_FFFF_FFFF
        for i in stride(from: 9, through: 0, by: -1) {
            out[i] = alphabet[Int(time & 31)]
            time >>= 5
        }
        // 80 random bits as 16 base32 digits: 64 bits plus 16 more.
        var hi = random() & 0xFFFF
        var lo = random()
        for i in stride(from: 25, through: 10, by: -1) {
            out[i] = alphabet[Int(lo & 31)]
            lo = (lo >> 5) | ((hi & 31) << 59)
            hi >>= 5
        }
        return String(out)
    }
}
