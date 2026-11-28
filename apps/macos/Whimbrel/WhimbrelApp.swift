import Combine
import SwiftUI
import os

/// A unit-test host gets an empty app: the real one opens the live database and starts
/// pollers, sync, retention, routines, capture and the MCP server, none of which a test needs.
@main
enum WhimbrelMain {
    static func main() {
        if KeychainAccess.isTestHost { TestHostApp.main(); return }
        // A second copy must not delete, migrate or open anything the running one uses.
        guard SingleInstance.mayStart() else { exit(0) }
        // "Delete everything" removes the data here, before anything opens the database.
        if let result = YourData.runPendingDelete(in: SpikePaths.root, steps: .live()), result.incomplete {
            YourData.reportIncompleteDelete(result)
        }
        // Before the store opens, as it ran (from WhimbrelApp.init) before X4.
        LegacyDefaultsMigration.run()
        // X4: the launch (plain or encrypted open, or a migration) runs before the app's
        // scenes exist. When it needs no screen it is done before the first window draws,
        // so an existing install opens as before; otherwise the window shows its screens.
        MainActor.assumeIsolated {
            let launch = AppLaunch.live()
            AppLaunch.shared = launch
            launch.startAndWait(timeout: 10)
        }
        WhimbrelApp.main()
    }
}

/// Holds the launch until it opened the store, then the `AppState` built on that store
/// (X4 Task 11b-2). Every scene reads `appState` and shows a placeholder while it is nil;
/// the main window shows the launch screens.
@MainActor
final class AppHost: ObservableObject {
    let launch: AppLaunch
    @Published private(set) var appState: AppState?
    private var pendingURLs: [URL] = []
    private var cancellable: AnyCancellable?
    private var mainWindowShown = false

    init(launch: AppLaunch) {
        self.launch = launch
        launch.start()
        // The publisher gives the current value at once: an open that finished before the scenes
        // existed builds AppState here, exactly when `@StateObject` built it before X4. A later
        // one arrives after `result` is stored, so `launchedStore` below reads it.
        cancellable = launch.resultPublisher.sink { [weak self] result in
            MainActor.assumeIsolated { self?.handle(result) }
        }
    }

    private func handle(_ result: AppLaunch.Result?) {
        switch result {
        case nil:
            break
        case .quit:
            DispatchQueue.main.async { NSApp.terminate(nil) }
        case .opened, .openedWithoutStore:
            guard appState == nil, let launched = launch.launchedStore else { return }
            let state = AppState(launched: launched)
            launch.recording = { [weak state] in state?.isRecordingNow ?? false }
            EncryptionSettingsPane.live = Self.encryptionSettings(launch: launch, appState: state)
            appState = state
            for url in pendingURLs { state.handleOpenURL(url) }
            pendingURLs = []
        }
    }

    /// The Settings encryption card, on the launch's environment, coordinator and store.
    private static func encryptionSettings(launch: AppLaunch, appState: AppState) -> EncryptionSettingsLive {
        let env = launch.environment
        let mcp = appState.mcpServerProcess
        let memory = ProcessInfo.processInfo.physicalMemory
        let key = launch.launchedStore?.key
        let storeOpen = appState.coreStore != nil
        let actions = EncryptionSettingsLive.actions(
            keys: env.keys, settings: launch.settings, dbPath: env.dbPath,
            recording: { [weak appState] in appState?.isRecordingNow ?? false },
            precheck: { path, recording in
                await Task.detached { CoreEncryptionMigration.precheck(dbPath: path, recording: recording) }.value
            },
            setup: { EncryptionSetupModel.Actions.live(existingKey: $0, databaseFolder: env.dataRoot) },
            exportEverything: { [weak appState] in appState?.pendingYourDataExport = true },
            // After this process and the MCP server exit, so the next launch finds the
            // database closed and can encrypt it.
            restart: { [weak mcp] in AppRestart.relaunchAfterExit(waitingAlsoFor: mcp?.processIdentifier) })
        return EncryptionSettingsLive(
            outcome: EncryptionSettingsLive.outcome(openedWithKey: key != nil),
            attention: storeOpen ? nil : EncryptionCopy.noteStoreUnavailable,
            pendingNextLaunch: EncryptionSettingsLive.pendingNextLaunch(
                flag: launch.settings.migrateAtNextLaunch, openedWithKey: key != nil, notes: launch.coordinator.notes),
            actions: actions,
            notes: EncryptionNotesModel(coordinator: launch.coordinator),
            fastSearch: FastSearchSetting(defaults: env.defaults, physicalMemory: memory),
            speedCheck: SpeedCheckModel(actions: EncryptionSettingsLive.speedActions(
                store: { [weak appState] in appState?.coreStore }, openMs: { [weak launch] in launch?.openMs },
                physicalMemory: memory, save: { SpeedReportPanel.save($0) },
                recording: { [weak appState] in appState?.isRecordingNow ?? false })))
    }

    func open(_ url: URL) {
        if let appState { appState.handleOpenURL(url) } else { pendingURLs.append(url) }
    }

    /// The main window drew the app (not a launch screen): after a short delay the launch's
    /// good-launch bookkeeping runs (design 3.5 step 7: "until the user has seen a working app").
    func mainWindowAppeared() {
        guard !mainWindowShown else { return }
        mainWindowShown = true
        let launch = launch
        Task { @MainActor in
            try? await Task.sleep(for: .seconds(2))
            await launch.firstWindowRendered()
        }
    }
}

/// The main window before and after the launch: the launch screens, then the app.
struct LaunchGateView<Content: View>: View {
    @ObservedObject var host: AppHost
    @ViewBuilder let content: (AppState) -> Content

    var body: some View {
        if let appState = host.appState {
            content(appState)
                // Running without the memory database: a notice above everything (decision 1B).
                .safeAreaInset(edge: .top, spacing: 0) {
                    VStack(spacing: 0) {
                        NoStoreNoticeHost()
                        // Task 13c item 4: the library vanished and a new empty one opened (once).
                        LibraryMissingNoticeHost(coordinator: host.launch.coordinator)
                    }
                }
                .modifier(RecordingRefusalAlert(refusal: appState.recordingRefusal))
                .environment(\.recordingControls, appState.recordingControls)
                .environmentObject(appState)
                .onAppear { host.mainWindowAppeared() }
        } else {
            LaunchWindowView(presenter: host.launch.presenter)
        }
    }
}

/// A window other than the main one, while the launch has not opened the store yet.
struct LaunchPendingView: View {
    var body: some View {
        Text(EncryptionCopy.starting)
            .font(.whimbrelBody(.callout))
            .foregroundStyle(Color.whimbrelTextSecondary)
            .frame(minWidth: 480, maxWidth: .infinity, minHeight: 300, maxHeight: .infinity)
    }
}

/// Scenes that need `AppState` show `pending` until the launch opened the store.
struct WithAppState<Content: View, Pending: View>: View {
    @ObservedObject var host: AppHost
    @ViewBuilder let content: (AppState) -> Content
    @ViewBuilder let pending: () -> Pending

    var body: some View {
        if let appState = host.appState {
            content(appState).environmentObject(appState)
        } else {
            pending()
        }
    }
}

struct TestHostApp: App {
    var body: some Scene { Settings { EmptyView() } }
}

/// Whimbrel > Check for Updates…: runs the check, then opens Settings (General is its
/// default pane) so the result is visible.
struct UpdateCommands: Commands {
    @ObservedObject var host: AppHost
    @Environment(\.openSettings) private var openSettings

    var body: some Commands {
        CommandGroup(after: .appInfo) {
            Button("Check for Updates…") {
                openSettings()
                Task { await host.appState?.updateChecker.checkNow() }
            }
            .disabled(host.appState == nil)
        }
    }
}

/// The app's own menus; disabled until the launch opened the store.
struct WhimbrelCommands: Commands {
    @ObservedObject var host: AppHost

    var body: some Commands {
        UpdateCommands(host: host)
        CommandGroup(replacing: .newItem) {
            Button("New Conversation") { host.appState?.requestNewConversation() }
                .keyboardShortcut("n")
                .disabled(host.appState == nil)
        }
        CommandGroup(after: .textEditing) {
            Button("Search Library") { host.appState?.requestLibrarySearch() }
                .keyboardShortcut("f")
                .disabled(host.appState == nil)
        }
        // Pause or resume and mark a moment.
        CommandGroup(after: .toolbar) {
            Button(String(localized: "Pause or resume recording")) { host.appState?.toggleRecordingPause() }
                .keyboardShortcut("p", modifiers: [.command, .shift])
            Button(String(localized: "Mark this moment")) { host.appState?.markMoment() }
                .keyboardShortcut("m", modifiers: [.command, .shift])
        }
        CommandGroup(after: .help) {
            Button("Send Feedback…") { NSWorkspace.shared.open(WhimbrelApp.feedbackMailURL()) }
            Button("Join the Whimbrel Discord…") { NSWorkspace.shared.open(WhimbrelApp.discordURL) }
        }
        CommandMenu("Go") {
            ForEach(WhimbrelTab.visibleTabs) { tab in
                Button(tab.rawValue) {
                    host.appState?.openTimeline?()
                    host.appState?.selectedTab = tab
                }
                .keyboardShortcut(KeyEquivalent(tab.goKey))
                .disabled(host.appState == nil)
            }
        }
    }
}

/// The menu bar icon while the launch has not opened the store. It lives as long as the
/// app: when a screen starts waiting for the user it opens the launch window and brings it
/// forward. A window the user closes while a screen is already waiting comes back when the
/// menu bar is clicked (`LaunchMenuRedesignView` opens it on appear, and offers Show Whimbrel).
struct LaunchMenuBarLabel: View {
    @ObservedObject var presenter: LaunchUIPresenter
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Image("StatusGlyph").renderingMode(.template).opacity(presenter.waitingForUser ? 1 : 0.4)
            .onChange(of: presenter.waitingForUser, initial: true) { _, waiting in
                guard waiting else { return }
                NSApp.activate(ignoringOtherApps: true)
                openWindow(id: "timeline")
            }
    }
}

struct WhimbrelApp: App {
    /// Asks before Cmd-Q or the menu bar's Quit would lose recovery material (onboarding).
    @NSApplicationDelegateAdaptor(AppTerminationGuard.self) private var terminationGuard
    @StateObject private var host = AppHost(launch: AppLaunch.shared ?? AppLaunch.live())

    var body: some Scene {
        // Primary, auto-opening window — Timeline is the app's real home now; it
        // doesn't depend on the menu-bar status item being visible/clickable.
        Window("Whimbrel", id: "timeline") {
            ThemedRoot { LaunchGateView(host: host) { _ in WhimbrelShellView() } }
                // whimbrel://meeting/<id>?at=<cs>, from an alert email (held until the launch opened).
                .onOpenURL { host.open($0) }
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
            WhimbrelCommands(host: host)
        }

        MenuBarExtra {
            ThemedRoot {
                WithAppState(host: host) { appState in
                    // The two notices are in the menu's Needs attention section.
                    MenuBarRedesignView(coordinator: host.launch.coordinator, sync: appState.contentSyncCoordinator)
                        .environment(\.recordingControls, appState.recordingControls)
                } pending: {
                    LaunchMenuRedesignView(presenter: host.launch.presenter)
                }
            }
        } label: {
            if let appState = host.appState {
                StatusIcon(state: appState)
            } else {
                LaunchMenuBarLabel(presenter: host.launch.presenter)
            }
        }
        .menuBarExtraStyle(.window)

        WindowGroup(id: "meeting-detail", for: Int64.self) { $meetingId in
            if let meetingId {
                ThemedRoot { WithAppState(host: host) { _ in MeetingDetailView(meetingId: meetingId) } pending: { LaunchPendingView() } }
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
        .handlesExternalEvents(matching: [])

        WindowGroup(id: "capture-detail", for: Int64.self) { $captureItemId in
            if let captureItemId {
                ThemedRoot { WithAppState(host: host) { _ in CaptureDetailView(captureItemId: captureItemId) } pending: { LaunchPendingView() } }
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
        .handlesExternalEvents(matching: [])

        WindowGroup(id: "episode-detail", for: Int64.self) { $episodeId in
            if let episodeId {
                ThemedRoot { WithAppState(host: host) { _ in EpisodeDetailView(episodeId: episodeId) } pending: { LaunchPendingView() } }
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
        .handlesExternalEvents(matching: [])

        WindowGroup(id: "thread-detail", for: Int64.self) { $threadId in
            if let threadId {
                ThemedRoot { WithAppState(host: host) { _ in ThreadDetailView(threadId: threadId) } pending: { LaunchPendingView() } }
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
        .handlesExternalEvents(matching: [])

        // Native Settings scene (⌘,) — replaces the old standalone "onboarding" window;
        // permissions are one section among several here now (see SettingsRedesignView.swift).
        Settings {
            ThemedRoot {
                WithAppState(host: host) { _ in SettingsRedesignView() } pending: {
                    Text(EncryptionCopy.settingsAfterStart).font(.whimbrelBody(.callout))
                        .foregroundStyle(Color.whimbrelTextSecondary).frame(minWidth: 480, minHeight: 200)
                }
            }
        }
    }

    /// Help > Join the Whimbrel Discord…: the community server's invite (#762). Also on the
    /// website footer and the README.
    static let discordURL = URL(string: "https://discord.gg/fetZZA8Ax")!

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
