import SwiftUI
import WidgetKit

/// Control Center "Quick Add" (iOS 18+). One tap runs `OpenQuickAddIntent`
/// (`OpenQuickAddIntent.swift`), which opens the app and routes it to
/// `outl://capture`, the same link the Home Screen widget and the Android
/// tile open, so the capture sheet comes up over today's journal with the
/// keyboard raised. It writes nothing, so like the widget it needs no App
/// Group.
///
/// **The icon is a system symbol here, and the outl mark everywhere
/// else.** The `QuickAddMark` symbol draws correctly in the widget and
/// draws an empty circle in a control, and a blank button is worse than
/// a generic one. What was measured and what to try next:
/// `IOS-EXTENSIONS.md` → "The icon is the outl mark".
///
/// The app and the Home Screen widget still deploy to iOS 17; the control
/// is only registered on systems that have `ControlWidget`
/// (see `OutlWidgetBundle`).
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
