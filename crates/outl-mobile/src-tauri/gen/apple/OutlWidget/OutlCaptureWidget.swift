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
        content.widgetURL(CaptureLink.capture)
    }

    @ViewBuilder private var content: some View {
        switch family {
        case .accessoryCircular:
            ZStack {
                AccessoryWidgetBackground()
                Image(systemName: "plus")
                    .font(.title2.weight(.semibold))
            }
        case .accessoryRectangular:
            Label(CaptureLink.title, systemImage: "plus.circle")
                .font(.headline)
        default:
            VStack(alignment: .leading, spacing: 8) {
                Image(systemName: "plus.circle.fill")
                    .font(.largeTitle)
                Spacer()
                Text(CaptureLink.title)
                    .font(.headline)
                Text("outl")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .leading)
            .padding()
        }
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
