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
    @State private var showModelSetup = false
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
                        Text(tab.rawValue)
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
            // Same first-run model-download prompt `TimelineView.onAppear` presents,
            // replicated here since Today (not Library) is now the default landing tab —
            // Today's own "Start note" CTA otherwise invites recording before required
            // local models exist. `TimelineView`'s own copy is untouched and stays
            // idempotent alongside this one: `allModelsPresent` reflects real file
            // presence, so once either sheet completes setup, the other's check simply
            // no-ops if it later runs too.
            if !ModelDownloadService.allModelsPresent {
                // Same next-run-loop-turn deferral `TimelineView` uses — setting this
                // @State synchronously inside .onAppear can silently fail to present the
                // sheet on macOS because the view isn't yet fully attached to its window
                // hierarchy at that exact point in the lifecycle.
                DispatchQueue.main.async {
                    showModelSetup = true
                }
            }
        }
        .sheet(isPresented: $showModelSetup) {
            ModelSetupView()
        }
    }

    /// This Mac's name and whether sync is running, under a hairline.
    private var sidebarFooter: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Divider().overlay(Color.whimbrelBorder)
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
        case .ready: return "Synced"
        case .notSignedIn: return "Sync off"
        case .error: return "Sync error"
        case .waitingForGroupKey: return "Approve this Mac"
        case .signingIn, .waitingForApproval: return "Setting up sync"
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
        guard appState.activeMeetingId == nil else {
            showActiveRecordingWarning = true
            return
        }
        // The staged update lives under FileManager.default.temporaryDirectory (see
        // UpdateChecker.download()), which macOS's periodic cleaner can reap during a
        // long-running session. A user could click "Restart Now" days after seeing
        // "Later" on a path that no longer exists, which would otherwise make the
        // relaunch helper's `ditto` fail mid-swap, after the old app is already renamed
        // aside. Bail out before spawning anything or terminating this process.
        guard FileManager.default.fileExists(atPath: update.appPath.path) else {
            Self.log.error("restartIntoUpdate: staged update at \(update.appPath.path, privacy: .public) no longer exists, aborting restart")
            return
        }
        guard let helperPath = Bundle.main.path(forResource: "RelaunchHelper", ofType: "sh") else { return }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/sh")
        process.arguments = [
            helperPath,
            String(ProcessInfo.processInfo.processIdentifier),
            update.appPath.path,
            Bundle.main.bundleURL.path,
        ]
        do {
            try process.run()
            NSApp.terminate(nil)
        } catch {
            Self.log.error("restartIntoUpdate: failed to launch RelaunchHelper: \(error.localizedDescription, privacy: .public)")
        }
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
