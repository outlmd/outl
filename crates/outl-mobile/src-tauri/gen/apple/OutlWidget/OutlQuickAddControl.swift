import AppIntents
import OutlKit
import SwiftUI
import WidgetKit

/// Control Center "Quick Add" (iOS 18+). One tap opens `outl://capture`,
/// the same link the Home Screen widget and the Android tile open, so the
/// capture sheet comes up over today's journal with the keyboard raised.
/// It writes nothing, so like the widget it needs no App Group.
///
/// The app and the Home Screen widget still deploy to iOS 17; the control
/// is only registered on systems that have `ControlWidget`
/// (see `OutlWidgetBundle`).
@available(iOS 18.0, *)
struct OpenQuickAddIntent: AppIntent {
    static let title: LocalizedStringResource = "Quick Add"
    static let description = IntentDescription("Open outl on a new line in today's journal.")

    /// Deliberately not `openAppWhenRun`: that only resumes the last screen.
    /// `OpenURLIntent` routes through the `outl` scheme, which is what
    /// shows the capture sheet.
    func perform() async throws -> some IntentResult & OpensIntent {
        .result(opensIntent: OpenURLIntent(CaptureLink.capture))
    }
}

@available(iOS 18.0, *)
struct OutlQuickAddControl: ControlWidget {
    var body: some ControlWidgetConfiguration {
        StaticControlConfiguration(kind: "app.outl.quick-add") {
            ControlWidgetButton(action: OpenQuickAddIntent()) {
                Label("Quick Add", systemImage: "plus.circle")
            }
        }
        .displayName("Quick Add")
        .description("Open outl on a new line in today's journal.")
    }
}
