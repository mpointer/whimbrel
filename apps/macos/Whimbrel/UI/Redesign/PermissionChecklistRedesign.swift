import SwiftUI
import WhimbrelUI

/// `OnboardingView` (the permission checklist in Settings, General) in the redesign.
struct PermissionChecklistRedesignView: View {
    @EnvironmentObject var permissions: PermissionsManager
    @State private var requested: Set<PermissionKind> = []

    var body: some View {
        WPermissionChecklistView(model: model)
            .environment(\.wScrollsContent, false)
            .environment(\.wEmbedded, true)
            .redesignThemed()
            .onAppear { permissions.refresh() }
    }

    private var granted: Int { PermissionKind.allCases.filter { permissions.granted(for: $0) }.count }

    private var model: WPermissionChecklistModel {
        var m = WPermissionChecklistModel()
        m.title = String(localized: "Grant Whimbrel access")
        m.intro = String(localized: "Whimbrel works with only some of these turned on. Each one adds one thing. Turn them on one at a time.")
        func rows(_ kinds: [PermissionKind]) -> [WPermissionRowModel] {
            PermissionRedesign.rows(kinds, permissions: permissions, requested: requested) { requested.insert($0) }
        }
        m.groups = [
            WPermissionGroup(id: "required", heading: PermissionRedesign.requiredHeading, rows: rows(PermissionRedesign.required)),
            WPermissionGroup(id: "optional", heading: PermissionRedesign.optionalHeading, rows: rows(PermissionRedesign.optional)),
        ]
        m.progress = String(localized: "\(granted) of \(PermissionKind.allCases.count) allowed", comment: "Count under the permission list. First placeholder is how many are allowed, second how many there are.")
        m.allSet = permissions.allGranted ? String(localized: "All set") : nil
        m.refreshTitle = String(localized: "Refresh")
        m.onRefresh = { permissions.refresh() }
        return m
    }
}
