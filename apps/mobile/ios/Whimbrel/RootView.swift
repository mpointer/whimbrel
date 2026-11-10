import SwiftUI

/// The approved phone: Today, Ask, Meetings, Topics and Routines, with Settings in a sheet. Record
/// joins the tab bar in its own milestone; nothing here is a placeholder control.
struct RootView: View {
    @ObservedObject var sync: SyncModel
    @ObservedObject var enrollment: EnrollmentModel
    @ObservedObject private var themes = ThemeStore.shared
    @State private var showSettings: Bool
    @State private var selection: Int
    @StateObject private var ask: AskModel
    /// Every push in the Today, Meetings and Topics stacks goes through its path, so a topic
    /// opened from a meeting's chip and a meeting opened from that topic stack in order.
    @State private var todayPath = NavigationPath()
    @State private var meetingsPath: NavigationPath
    @State private var topicsPath = NavigationPath()

    init(sync: SyncModel, enrollment: EnrollmentModel) {
        self.sync = sync
        self.enrollment = enrollment
        _ask = StateObject(wrappedValue: AskModel(backend: PhoneAskBackend(sync: sync)))
        var tab = 0, settings = false, path = NavigationPath()
        #if DEBUG
        // Demo screenshots: `-WhimbrelDemoTab 0|1|2|3|4`, `-WhimbrelDemoMeeting <id>`, `-WhimbrelDemoSettings`.
        let args = ProcessInfo.processInfo.arguments
        func value(_ flag: String) -> String? { args.firstIndex(of: flag).flatMap { args.indices.contains($0 + 1) ? args[$0 + 1] : nil } }
        tab = value("-WhimbrelDemoTab").flatMap(Int.init) ?? 0
        if let meeting = value("-WhimbrelDemoMeeting").flatMap(Int64.init) {
            path.append(meeting)
            tab = 2
        }
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
            NavigationStack(path: $todayPath) {
                TodayView(sync: sync, labels: enrollment.deviceLabels, pendingDevices: enrollment.pendingDeviceCount,
                          openSettings: { showSettings = true }, openTopic: { todayPath.append($0) })
            }
                .tabItem { Label("Today", systemImage: "sun.max") }.tag(0)
            NavigationStack { AskView(sync: sync, model: ask, labels: enrollment.deviceLabels) }
                .tabItem { Label("Ask", systemImage: "bubble.left.and.text.bubble.right") }.tag(1)
            NavigationStack(path: $meetingsPath) { MeetingsView(sync: sync, labels: enrollment.deviceLabels, openTopic: { meetingsPath.append($0) }) }
                .tabItem { Label("Meetings", systemImage: "person.2") }.tag(2)
            // Between Meetings and Routines; Routines keeps tag 3 so its demo flag still works.
            NavigationStack(path: $topicsPath) { TopicsView(sync: sync, labels: enrollment.deviceLabels) }
                .tabItem { Label("Topics", systemImage: "tag") }.tag(4)
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
