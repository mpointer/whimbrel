import AppKit
import SwiftUI
import OSLog

/// The five-tab meeting-first shell replacing TimelineView's mode-strip-first entry
/// point. TimelineView's own 5 search modes are NOT removed — they relocate into
/// `.library` as filters (see TodayView/MeetingsListView/library filter wiring).
enum WhimbrelTab: String, CaseIterable, Identifiable {
    case today = "Today"
    case meetings = "Meetings"
    case library = "Library"
    case automations = "Automations"
    case settings = "Settings"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .today: String(localized: "Today")
        case .meetings: String(localized: "Meetings")
        case .library: String(localized: "Library")
        case .automations: String(localized: "Automations")
        case .settings: String(localized: "Settings")
        }
    }

    /// The skin's icon for the tab; Library has no skin role.
    var icon: Image {
        switch self {
        case .today: return Image(skinRole: .today)
        case .meetings: return Image(skinRole: .meetings)
        case .library: return Image(systemName: "books.vertical")
        case .automations: return Image(skinRole: .routines)
        case .settings: return Image(skinRole: .settings)
        }
    }
}

struct WhimbrelShellView: View {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "shell")

    @EnvironmentObject var appState: AppState
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    @State private var showFirstRun = false
    @State private var showActiveRecordingWarning = false
    // "Later" just hides this banner instance for the version currently staged.
    // updateReadyBanner's `update.version != dismissedUpdateVersion` check below
    // resets this automatically once a *newer* version replaces the staged one --
    // it does NOT re-show on the next 24h check for the same dismissed version:
    // UpdateChecker.shouldDownload() skips re-downloading a version that's
    // already staged, so `downloadedUpdate` never changes and the comparison
    // stays false. No state needed in UpdateChecker itself, which only tracks
    // what's staged on disk, not whether the user has dismissed it.
    @State private var dismissedUpdateVersion: String?

    /// A top-level screen under the skin's header image and band (`SkinHeaderArea`).
    private func banded<Screen: View>(@ViewBuilder _ screen: @escaping () -> Screen) -> some View {
        SkinHeaderArea(screen: screen)
    }

    var body: some View {
        VStack(spacing: 0) {
            // Lives at the shell level (not inside any one tab's view) so it's visible
            // regardless of which tab is active -- it used to live in TimelineView only,
            // which meant a fully-staged, ready-to-install update was invisible to anyone
            // landing on the now-default Today tab instead of Library.
            updateReadyBanner

            NavigationSplitView {
                List(WhimbrelTab.allCases, selection: $appState.selectedTab) { tab in
                    Label {
                        Text(tab.title)
                            .foregroundStyle(Color.whimbrelTextPrimary)
                    } icon: {
                        tab.icon
                            .symbolRenderingMode(.monochrome)
                            .foregroundStyle(appState.selectedTab == tab ? Color.whimbrelTextPrimary : Color.whimbrelTextSecondary)
                    }
                    .tag(tab)
                    .listRowBackground(
                        RoundedRectangle(cornerRadius: Radius.control)
                            .fill(appState.selectedTab == tab ? Color.whimbrelSelection : Color.clear)
                            .padding(.horizontal, Spacing.xs)
                    )
                }
                .listStyle(.sidebar)
                .tint(Color.whimbrelAccentPrimary)
                .safeAreaInset(edge: .bottom, spacing: 0) { sidebarFooter }
                .navigationSplitViewColumnWidth(min: 160, ideal: 180)
            } detail: {
                switch appState.selectedTab {
                case .today: TodayView()
                case .meetings: banded { MeetingsListView() }
                case .library: banded { TimelineView() }
                case .automations: banded { AutomationsView() }
                case .settings: PreferencesRedirectView(openSettings: openSettings)
                }
            }
        }
        .background(Color.whimbrelBackgroundPage)
        .tint(Color.whimbrelAccentPrimary)
        .background(HideWindowTitle())
        // `SettingsView` is its own `NavigationSplitView` (180pt inner sidebar +
        // 640pt minWidth content) and used to render directly as this shell's
        // `.settings` detail — nesting one NavigationSplitView inside another. SwiftUI
        // computes a window's auto-sized ideal width from its content, and that nesting
        // made Settings' own minimums compound with the shell's, which is what
        // ballooned the whole window (observed at 2973-3693pt, wider than either
        // connected display) whenever Settings was the active tab. `WhimbrelApp.swift`
        // already declares a proper native `Settings { SettingsView() }` scene (⌘,) —
        // this route just calls that instead of embedding a second copy in the shell.
        .onChange(of: appState.selectedTab) { _, newValue in
            if newValue == .settings {
                openSettings()
            }
        }
        .onAppear {
            // `TimelineView` used to set this on `.onAppear` at launch (it was the root
            // view) — now it only runs once the user visits the Library tab. Set it here
            // too so the meeting-alert panel's "Start Recording & Open Timeline" action
            // works from app launch, same target window either way.
            appState.openTimeline = { openWindow(id: "timeline") }
            showFirstRun = FirstRun.shouldShow(
                done: UserDefaults.standard.bool(forKey: FirstRun.doneKey),
                microphoneGranted: appState.permissions.microphoneGranted,
                screenGranted: appState.permissions.screenRecordingGranted)
            if !KeychainAccess.isTestHost {
                TranscriptionQuality.migrate(modelExists: FileManager.default.fileExists(atPath: TranscriptionService.modelPath.path))
                Task {
                    await appState.modelBootstrap.start()
                    DispatchQueue.global(qos: .utility).async { EngineBenchmark.runIfNeeded() }
                } }
        }
        .sheet(isPresented: $showFirstRun) {
            FirstRunView {
                UserDefaults.standard.set(true, forKey: FirstRun.doneKey)
                showFirstRun = false
            }
            .environmentObject(appState.permissions)
        }
    }

    /// This Mac's name and whether sync is running, under a hairline.
    private var sidebarFooter: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Divider().overlay(Color.whimbrelBorder)
            ModelStatusLine(bootstrap: appState.modelBootstrap)
            StaleSyncLine(sync: appState.contentSyncCoordinator)
            HStack(spacing: Spacing.sm) {
                DeviceChip(deviceId: appState.deviceId)
                Text(syncLabel)
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .lineLimit(1)
                    .layoutPriority(1)
            }
            .padding(.horizontal, Spacing.md)
            .padding(.bottom, Spacing.sm)
        }
        .accessibilityElement(children: .combine)
    }

    private var syncLabel: String {
        switch appState.syncState.status {
        case .ready: return String(localized: "Synced")
        case .notSignedIn: return String(localized: "Sync off")
        case .error, .rateLimited: return String(localized: "Sync error")
        case .deviceLimit: return String(localized: "Sync limit reached")
        case .waitingForGroupKey: return String(localized: "Approve this Mac")
        case .signingIn, .waitingForApproval: return String(localized: "Setting up sync")
        }
    }

    @ViewBuilder
    private var updateReadyBanner: some View {
        if let update = appState.updateChecker.downloadedUpdate, update.version != dismissedUpdateVersion {
            HStack {
                Text("Update ready: v\(update.version)")
                    .font(.whimbrelBody(.callout))
                Spacer()
                Button("Later") { dismissedUpdateVersion = update.version }
                    .buttonStyle(.whimbrelSecondary)
                Button("Restart Now") { restartIntoUpdate(update) }
                    .buttonStyle(.whimbrelPrimary)
            }
            .padding(Spacing.sm)
            .background(Color.whimbrelAccentPrimary.opacity(0.12))
            .cornerRadius(8)
            .padding(.horizontal, Spacing.md)
            .padding(.top, Spacing.sm)
            .alert("A meeting is currently recording", isPresented: $showActiveRecordingWarning) {
                Button("OK", role: .cancel) {}
            } message: {
                Text("Finish or stop the current recording before restarting to install the update.")
            }
        }
    }

    private func restartIntoUpdate(_ update: UpdateChecker.DownloadedUpdate) {
        if appState.restartIntoUpdate(update) == .recording { showActiveRecordingWarning = true }
    }
}

/// What the shell's `.settings` tab actually shows now — `openSettings()` already opens
/// the real, native Settings window (see `WhimbrelShellView`'s `.onChange(of: selectedTab)`
/// above); this is just the sidebar's landing content so clicking "Settings" isn't a dead
/// end if that window closes or the app is set to not show in the Dock. Deliberately tiny:
/// this is a redirect, not a second copy of Settings.
private struct PreferencesRedirectView: View {
    let openSettings: OpenSettingsAction

    var body: some View {
        VStack(spacing: Spacing.md) {
            Image(skinRole: .settings)
                .font(.system(size: 40))
                .foregroundStyle(Color.whimbrelTextSecondary)
            Text("Settings opens in its own window")
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
            Button("Open Settings…") { openSettings() }
                .buttonStyle(.whimbrelPrimary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.whimbrelBackgroundPage)
    }
}

/// Hides the window's title text ("Whimbrel") but keeps the title string, which other code
/// matches on (`window.title == "Whimbrel"`). The sidebar toggle stays.
private struct HideWindowTitle: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async { view.window?.titleVisibility = .hidden }
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        DispatchQueue.main.async { view.window?.titleVisibility = .hidden }
    }
}

/// Sidebar footer: what the background model download is doing. Nothing once it's done.
/// Says so when content sync has gone quiet. Rechecks on a timer, because a stuck round
/// publishes nothing.
private struct StaleSyncLine: View {
    @ObservedObject var sync: ContentSyncCoordinator

    var body: some View {
        SwiftUI.TimelineView(.periodic(from: .now, by: 300)) { context in
            if sync.isStale(now: context.date), let last = sync.lastSuccess {
                Text("Sync hasn't worked since \(last.formatted(date: .abbreviated, time: .shortened)). Whimbrel keeps trying.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelError)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, Spacing.md)
            }
        }
    }
}

private struct ModelStatusLine: View {
    @ObservedObject var bootstrap: ModelBootstrap

    var body: some View {
        switch bootstrap.phase {
        case .idle, .ready:
            EmptyView()
        case .downloading(let fraction, _):
            VStack(alignment: .leading, spacing: Spacing.xs) {
                ProgressView(value: fraction)
                Text("Downloading models. Keyword search works now; search by meaning starts when this finishes.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .padding(.horizontal, Spacing.md)
        case .failed:
            HStack(spacing: Spacing.sm) {
                Text("Couldn't download models.")
                    .font(.whimbrelBody(.caption2))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Button("Try Again") { Task { await bootstrap.start() } }
                    .font(.whimbrelBody(.caption2))
                    .buttonStyle(.link)
            }
            .padding(.horizontal, Spacing.md)
        }
    }
}
