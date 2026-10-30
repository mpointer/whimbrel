import SwiftUI

/// The approved phone: Today, Ask, Meetings and Routines, with Settings in a sheet. Record
/// joins the tab bar in its own milestone; nothing here is a placeholder control.
struct RootView: View {
    @ObservedObject var sync: SyncModel
    @ObservedObject var enrollment: EnrollmentModel
    @ObservedObject private var themes = ThemeStore.shared
    @State private var showSettings: Bool
    @State private var selection: Int
    @StateObject private var ask: AskModel
    @State private var meetingsPath: [Int64]

    init(sync: SyncModel, enrollment: EnrollmentModel) {
        self.sync = sync
        self.enrollment = enrollment
        _ask = StateObject(wrappedValue: AskModel(backend: PhoneAskBackend(sync: sync)))
        var tab = 0, settings = false, path: [Int64] = []
        #if DEBUG
        // Demo screenshots: `-WhimbrelDemoTab 0|1|2|3`, `-WhimbrelDemoMeeting <id>`, `-WhimbrelDemoSettings`.
        let args = ProcessInfo.processInfo.arguments
        func value(_ flag: String) -> String? { args.firstIndex(of: flag).flatMap { args.indices.contains($0 + 1) ? args[$0 + 1] : nil } }
        tab = value("-WhimbrelDemoTab").flatMap(Int.init) ?? 0
        path = value("-WhimbrelDemoMeeting").flatMap(Int64.init).map { [$0] } ?? []
        if !path.isEmpty { tab = 2 }
        settings = args.contains("-WhimbrelDemoSettings")
        #endif
        _selection = State(initialValue: tab)
        _showSettings = State(initialValue: settings)
        _meetingsPath = State(initialValue: path)
    }

    /// Ask runs only while its tab is the one showing: leaving the tab ends it. (Pushing a
    /// citation's detail screen fires `onDisappear` inside the tab, which must not.)
    static func shouldStopAsk(old: Int, new: Int) -> Bool { old == 1 && new != 1 }

    var body: some View {
        TabView(selection: $selection) {
            NavigationStack { TodayView(sync: sync, labels: enrollment.deviceLabels, pendingDevices: enrollment.pendingDeviceCount, openSettings: { showSettings = true }) }
                .tabItem { Label("Today", systemImage: "sun.max") }.tag(0)
            NavigationStack { AskView(sync: sync, model: ask, labels: enrollment.deviceLabels) }
                .tabItem { Label("Ask", systemImage: "bubble.left.and.text.bubble.right") }.tag(1)
            NavigationStack(path: $meetingsPath) { MeetingsView(sync: sync, labels: enrollment.deviceLabels) }
                .tabItem { Label("Meetings", systemImage: "person.2") }.tag(2)
            NavigationStack { RoutinesView(sync: sync, labels: enrollment.deviceLabels) }
                .tabItem { Label("Routines", systemImage: "bolt") }.tag(3)
        }
        .tint(Color.theme(.accentPrimary))
        .onChange(of: selection) { old, new in if Self.shouldStopAsk(old: old, new: new) { ask.stop() } }
        .sheet(isPresented: $showSettings) { SettingsSheet(sync: sync, enrollment: enrollment) }
        .task(id: sync.readRevision) {
            // Once per sync: also how a new device waiting for approval reaches the Today banner.
            await enrollment.refreshDeviceLabels()
        }
    }
}
