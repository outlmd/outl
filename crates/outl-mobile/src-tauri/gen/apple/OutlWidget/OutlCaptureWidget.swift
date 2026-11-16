import OutlKit
import SwiftUI
import WidgetKit

/// Step 1 of RFC 0363: a Home Screen and Lock Screen widget that opens
/// `outl://capture`. It writes nothing; the app's capture sheet does the
/// work, so this needs no App Group.
struct CaptureEntry: TimelineEntry {
    let date: Date
}

struct CaptureProvider: TimelineProvider {
    func placeholder(in context: Context) -> CaptureEntry { CaptureEntry(date: Date()) }

    func getSnapshot(in context: Context, completion: @escaping (CaptureEntry) -> Void) {
        completion(CaptureEntry(date: Date()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<CaptureEntry>) -> Void) {
        // Static content: one entry, never refreshed.
        completion(Timeline(entries: [CaptureEntry(date: Date())], policy: .never))
    }
}

struct CaptureWidgetView: View {
    @Environment(\.widgetFamily) private var family
    let entry: CaptureEntry

    var body: some View {
        content
            .widgetURL(CaptureLink.capture)
            .containerBackground(for: .widget) { background }
    }

    /// **Not optional since the iOS 17 SDK.** A widget that declares no
    /// container background does not fall back to the system material:
    /// WidgetKit paints an opaque white behind it, so the tile reads as a
    /// blank card next to every other widget on the Home Screen and the
    /// gallery preview shows nothing at all. That is the whole of the
    /// "the widget went white" bug, and nothing warns about it, because
    /// the widget still builds, still renders and still opens its link.
    @ViewBuilder private var background: some View {
        switch family {
        case .accessoryCircular, .accessoryRectangular, .accessoryInline:
            // The Lock Screen tints its own content; anything opaque here
            // fights the wallpaper instead of sitting on it.
            AccessoryWidgetBackground()
        default:
            LinearGradient(
                colors: [Color(BrandColor.violet), Color(BrandColor.lavender)],
                startPoint: .bottomLeading,
                endPoint: .topTrailing
            )
        }
    }

    @ViewBuilder private var content: some View {
        switch family {
        case .accessoryCircular:
            icon.padding(7)
        case .accessoryRectangular:
            Label { Text(CaptureLink.title) } icon: { icon }
                .font(.headline)
        default:
            VStack(alignment: .leading, spacing: 8) {
                icon.frame(height: 38)
                Spacer()
                Text(CaptureLink.title)
                    .font(.headline)
                Text("outl")
                    .font(.caption)
                    .opacity(0.75)
            }
            .foregroundStyle(Color(BrandColor.paper))
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
        }
    }

    /// The mark with a `+` knocked out of its body, as a **custom SF
    /// Symbol**: it takes the foreground style, and `scripts/gen-icons.py`
    /// writes it from the same `MARK_BODY` geometry every app icon is
    /// rasterised from, so this and the icon beside it on the Home Screen
    /// cannot drift apart.
    private var icon: some View {
        Image("QuickAddMark")
            .resizable()
            .aspectRatio(contentMode: .fit)
    }
}

extension Color {
    /// A `BrandColor` as SwiftUI sees it. `BrandColor` is the one owner of
    /// these values (`OutlKit`, pinned by `BrandColorTests`).
    init(_ brand: BrandColor) {
        let c = brand.components
        self.init(.sRGB, red: c.red, green: c.green, blue: c.blue, opacity: 1)
    }
}

struct OutlCaptureWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "app.outl.capture", provider: CaptureProvider()) { entry in
            CaptureWidgetView(entry: entry)
        }
        .configurationDisplayName(CaptureLink.title)
        .description("Open outl on a new line in today's journal.")
        .supportedFamilies([.systemSmall, .accessoryCircular, .accessoryRectangular])
    }
}

@main
struct OutlWidgetBundle: WidgetBundle {
    var body: some Widget {
        OutlCaptureWidget()
        if #available(iOS 18.0, *) {
            OutlQuickAddControl()
        }
    }
}
