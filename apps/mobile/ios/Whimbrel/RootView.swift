import SwiftUI

/// The approved phone: Today, Meetings and Routines, with Settings in a sheet. Ask and Record
/// join the tab bar in their own milestones; nothing here is a placeholder control.
struct RootView: View {
    @ObservedObject var sync: SyncModel
    @ObservedObject var enrollment: EnrollmentModel
    @ObservedObject private var themes = ThemeStore.shared
    @State private var showSettings: Bool
    @State private var selection: Int
    @State private var meetingsPath: [Int64]

    init(sync: SyncModel, enrollment: EnrollmentModel) {
        self.sync = sync
        self.enrollment = enrollment
        var tab = 0, settings = false, path: [Int64] = []
        #if DEBUG
        // Demo screenshots: `-WhimbrelDemoTab 0|1|2`, `-WhimbrelDemoMeeting <id>`, `-WhimbrelDemoSettings`.
        let args = ProcessInfo.processInfo.arguments
        func value(_ flag: String) -> String? { args.firstIndex(of: flag).flatMap { args.indices.contains($0 + 1) ? args[$0 + 1] : nil } }
        tab = value("-WhimbrelDemoTab").flatMap(Int.init) ?? 0
        path = value("-WhimbrelDemoMeeting").flatMap(Int64.init).map { [$0] } ?? []
        if !path.isEmpty { tab = 1 }
        settings = args.contains("-WhimbrelDemoSettings")
        #endif
        _selection = State(initialValue: tab)
        _showSettings = State(initialValue: settings)
        _meetingsPath = State(initialValue: path)
    }

    var body: some View {
        TabView(selection: $selection) {
            NavigationStack { TodayView(sync: sync, labels: enrollment.deviceLabels, openSettings: { showSettings = true }) }
                .tabItem { Label("Today", systemImage: "sun.max") }.tag(0)
            NavigationStack(path: $meetingsPath) { MeetingsView(sync: sync, labels: enrollment.deviceLabels) }
                .tabItem { Label("Meetings", systemImage: "person.2") }.tag(1)
            NavigationStack { RoutinesView(sync: sync, labels: enrollment.deviceLabels) }
                .tabItem { Label("Routines", systemImage: "bolt") }.tag(2)
        }
        .tint(Color.theme(.accentPrimary))
        .sheet(isPresented: $showSettings) { SettingsSheet(sync: sync) }
        .task(id: sync.readRevision) {
            // The device list changes rarely: fetch it once, and again only while names are missing.
            if enrollment.deviceLabels.names.isEmpty { await enrollment.refreshDeviceLabels() }
        }
    }
}
