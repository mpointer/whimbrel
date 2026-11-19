import SwiftUI
import WhimbrelUI

/// `FirstRunView` in the redesign: the same two permissions, the same `permissions` calls, and Get started
/// calls the same `done`.
struct FirstRunRedesignView: View {
    @EnvironmentObject var permissions: PermissionsManager
    let done: () -> Void
    @State private var requested: Set<PermissionKind> = []

    var body: some View {
        WFirstRunView(model: model)
            .redesignThemed()
            .frame(width: 680, height: 660)
    }

    private var model: WFirstRunModel {
        var m = WFirstRunModel()
        m.title = String(localized: "Welcome to Whimbrel")
        m.intro = String(localized: "Two permissions and you're ready for your first meeting. No bot joins the call, and your recordings stay on this Mac.")
        m.groups = [WPermissionGroup(
            id: "required", heading: PermissionRedesign.requiredHeading,
            rows: PermissionRedesign.rows(PermissionKind.firstRun, permissions: permissions, requested: requested) { requested.insert($0) })]
        m.finePrint = [
            String(localized: "You can change these later in Settings, General.", comment: "Fine print on the first run screen. Settings, General is the General pane of Whimbrel's Settings."),
            String(localized: "Some places require everyone's consent before a meeting is recorded. Check the rules where you and the other people are."),
            String(localized: "Anything else, Whimbrel asks for when you first use it. Its models are downloading in the background."),
        ]
        m.primaryTitle = String(localized: "Get started", comment: "Button that ends the first run screen.")
        m.onPrimary = done
        return m
    }
}
