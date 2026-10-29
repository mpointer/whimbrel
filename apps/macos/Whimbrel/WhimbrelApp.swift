import SwiftUI

/// A unit-test host gets an empty app: the real one opens the live database and starts
/// pollers, sync, retention, routines, capture and the MCP server, none of which a test needs.
@main
enum WhimbrelMain {
    static func main() {
        if KeychainAccess.isTestHost { TestHostApp.main() } else { WhimbrelApp.main() }
    }
}

struct TestHostApp: App {
    var body: some Scene { Settings { EmptyView() } }
}

/// Whimbrel > Check for Updates…: runs the check, then opens Settings (General is its
/// default pane) so the result is visible.
struct UpdateCommands: Commands {
    let appState: AppState
    @Environment(\.openSettings) private var openSettings

    var body: some Commands {
        CommandGroup(after: .appInfo) {
            Button("Check for Updates…") {
                openSettings()
                Task { await appState.updateChecker.checkNow() }
            }
        }
    }
}

struct WhimbrelApp: App {
    @StateObject private var appState = AppState()

    init() {
        LegacyDefaultsMigration.run()
    }

    var body: some Scene {
        // Primary, auto-opening window — Timeline is the app's real home now; it
        // doesn't depend on the menu-bar status item being visible/clickable.
        Window("Whimbrel", id: "timeline") {
            ThemedRoot { WhimbrelShellView() }
                .environmentObject(appState)
                // Explicit bounds so `.windowResizability(.automatic)` has a sane range to
                // clamp to. Without this, a corrupted frame from macOS's own window-state
                // restoration (tracked continuously by the window server, independent of
                // this app's own layout code, and observed surviving a force-quit and a
                // full relaunch) can leave the window absurdly wide — seen at 2973-3693pt,
                // wider than either connected display — with no way for a normal resize to
                // recover it, since restoration re-applies the bad frame on every launch.
                .frame(minWidth: 900, idealWidth: 1400, maxWidth: 2400, minHeight: 600, idealHeight: 900, maxHeight: 1600)
        }
        .windowResizability(.automatic)
        .commands {
            UpdateCommands(appState: appState)
            CommandGroup(replacing: .newItem) {
                Button("New Conversation") { appState.requestNewConversation() }
                    .keyboardShortcut("n")
            }
            CommandGroup(after: .textEditing) {
                Button("Search Library") { appState.requestLibrarySearch() }
                    .keyboardShortcut("f")
            }
            CommandGroup(after: .help) {
                Button("Send Feedback…") { NSWorkspace.shared.open(Self.feedbackMailURL()) }
            }
            CommandMenu("Go") {
                ForEach(Array(WhimbrelTab.allCases.enumerated()), id: \.element) { index, tab in
                    Button(tab.rawValue) {
                        appState.openTimeline?()
                        appState.selectedTab = tab
                    }
                    .keyboardShortcut(KeyEquivalent(Character(String(index + 1))))
                }
            }
        }

        MenuBarExtra {
            ThemedRoot { MenuBarContentView() }
                .environmentObject(appState)
        } label: {
            StatusIcon(state: appState)
        }
        .menuBarExtraStyle(.window)

        WindowGroup(id: "meeting-detail", for: Int64.self) { $meetingId in
            if let meetingId {
                ThemedRoot { MeetingDetailView(meetingId: meetingId) }
                    .environmentObject(appState)
            } else {
                // Reachable via the group's automatic "New Window" command (⌘N) or
                // state restoration of a window whose value wasn't persisted — this
                // scene should only ever be opened programmatically with a value, but
                // both paths still produce a nil binding, so degrade gracefully rather
                // than showing an empty, unsized window.
                Text("This meeting is no longer available.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(minWidth: 480, maxWidth: .infinity, minHeight: 400, maxHeight: .infinity)
                    .navigationTitle("Meeting")
            }
        }
        .windowResizability(.automatic)
        .commandsRemoved()

        WindowGroup(id: "capture-detail", for: Int64.self) { $captureItemId in
            if let captureItemId {
                ThemedRoot { CaptureDetailView(captureItemId: captureItemId) }
                    .environmentObject(appState)
            } else {
                Text("This item is no longer available.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(minWidth: 480, maxWidth: .infinity, minHeight: 400, maxHeight: .infinity)
                    .navigationTitle("Capture")
            }
        }
        .windowResizability(.automatic)
        .commandsRemoved()

        WindowGroup(id: "episode-detail", for: Int64.self) { $episodeId in
            if let episodeId {
                ThemedRoot { EpisodeDetailView(episodeId: episodeId) }
                    .environmentObject(appState)
            } else {
                Text("This episode is no longer available.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(minWidth: 480, maxWidth: .infinity, minHeight: 400, maxHeight: .infinity)
                    .navigationTitle("Episode")
            }
        }
        .windowResizability(.automatic)
        .commandsRemoved()

        WindowGroup(id: "thread-detail", for: Int64.self) { $threadId in
            if let threadId {
                ThemedRoot { ThreadDetailView(threadId: threadId) }
                    .environmentObject(appState)
            } else {
                Text("This thread is no longer available.")
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .frame(minWidth: 480, maxWidth: .infinity, minHeight: 400, maxHeight: .infinity)
                    .navigationTitle("Thread")
            }
        }
        .windowResizability(.automatic)
        .commandsRemoved()

        // Native Settings scene (⌘,) — replaces the old standalone "onboarding" window;
        // permissions are one section among several here now (see SettingsView.swift).
        Settings {
            ThemedRoot { SettingsView() }
                .environmentObject(appState)
        }
    }

    /// Help > Send Feedback…: a new email to hello@whimbrelapp.com with the version, macOS
    /// and Mac model filled in, so every report says what it ran on.
    static func feedbackMailURL() -> URL {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "unknown"
        let os = ProcessInfo.processInfo.operatingSystemVersionString
        var size = 0
        sysctlbyname("hw.model", nil, &size, nil, 0)
        var model = [CChar](repeating: 0, count: max(size, 1))
        sysctlbyname("hw.model", &model, &size, nil, 0)
        var parts = URLComponents()
        parts.scheme = "mailto"
        parts.path = "hello@whimbrelapp.com"
        parts.queryItems = [
            URLQueryItem(name: "subject", value: "Whimbrel feedback (Mac \(version))"),
            URLQueryItem(name: "body", value: "\n\n---\nWhimbrel \(version) · macOS \(os) · \(String(cString: model))"),
        ]
        return parts.url ?? URL(string: "mailto:hello@whimbrelapp.com")!
    }
}
