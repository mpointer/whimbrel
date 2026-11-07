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
                case .settings: StatusOverviewView(openSettings: openSettings)
                }
            }
        }
        .background(Color.whimbrelBackgroundPage)
        .tint(Color.whimbrelAccentPrimary)
        .background(HideWindowTitle())
        // Settings is the native `Settings { SettingsView() }` scene (⌘,), never embedded here:
        // nesting its NavigationSplitView in the shell's once ballooned the window past the display.
        .onAppear {
            // `TimelineView` used to set this on `.onAppear` at launch (it was the root
            // view) — now it only runs once the user visits the Library tab. Set it here
            // too so the meeting-alert panel's "Start Recording & Open Timeline" action
            // works from app launch, same target window either way.
            appState.openTimeline = { openWindow(id: "timeline") }
            appState.openSettingsWindow = { openSettings() }
            showFirstRun = FirstRun.shouldShow(
                done: UserDefaults.standard.bool(forKey: FirstRun.doneKey),
                microphoneGranted: appState.permissions.microphoneGranted,
                screenGranted: appState.permissions.screenRecordingGranted)
            if !KeychainAccess.isTestHost {
                LocalModel.configure()
                TranscriptionQuality.migrate(modelExists: FileManager.default.fileExists(atPath: TranscriptionService.modelPath.path))
                Task {
                    await appState.modelBootstrap.start()
                    DispatchQueue.global(qos: .utility).async { EngineBenchmark.runIfNeeded() }
                } }
        }
        .sheet(item: $appState.prepBriefToShow) { request in
            PrepBriefSheet(key: request.key)
                .environmentObject(appState)
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

/// The shell's `.settings` tab: an at-a-glance status page (#595). Each row opens its
/// Settings section in the native Settings window (⌘,); this page never edits anything.
private struct StatusOverviewView: View {
    @EnvironmentObject var appState: AppState
    let openSettings: OpenSettingsAction
    @State private var usageTotals: CoreUsageTotals?
    @State private var lastRun: CoreRoutineRunWithName?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: Spacing.md) {
                Text("Status")
                    .font(.whimbrelDisplay())
                    .foregroundStyle(Color.whimbrelTextPrimary)
                row("Sync", .devices, syncLines)
                row("AI", .ai, aiLines)
                row("Storage", .capture, storageLines)
                row("Updates", .general, updateLines)
                row("Routines", .automations, routineLines)
                Button("Open Settings…") { openSettings() }
                    .buttonStyle(.whimbrelPrimary)
            }
            .frame(maxWidth: 640, alignment: .leading)
            .padding(Spacing.lg)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Color.whimbrelBackgroundPage)
        .onAppear(perform: load)
    }

    private func row(_ title: LocalizedStringKey, _ section: SettingsSection, _ lines: [String]) -> some View {
        Button {
            appState.pendingSettingsSection = section
            openSettings()
        } label: {
            HStack(alignment: .top) {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    Text(title)
                        .font(.whimbrelBody(.headline))
                        .foregroundStyle(Color.whimbrelTextPrimary)
                    ForEach(lines, id: \.self) { line in
                        Text(verbatim: line)
                            .font(.whimbrelBody(.caption))
                            .foregroundStyle(Color.whimbrelTextSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer()
                Image(systemName: "chevron.right").foregroundStyle(Color.whimbrelTextSecondary)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(Rectangle())
            .whimbrelCard()
        }
        .buttonStyle(.plain)
    }

    private var syncLines: [String] {
        guard case .ready = appState.syncState.status else {
            if case .notSignedIn = appState.syncState.status { return [String(localized: "Sync is off.")] }
            return [String(localized: "Sync is setting up.")]
        }
        var lines: [String] = []
        let devices = appState.syncState.devices.filter { $0.status == "approved" }.count
        if devices > 0 { lines.append(String(localized: "Devices approved: \(devices)")) }
        let content = appState.contentSyncCoordinator
        if let error = content.lastSyncError { lines.append(error) }
        else if let at = content.lastSyncedAt { lines.append(String(localized: "Content last synced \(at.formatted(.relative(presentation: .named)))")) }
        let settings = appState.settingsSyncCoordinator
        if let error = settings.lastSyncError { lines.append(error) }
        else if let at = settings.lastSyncedAt { lines.append(String(localized: "Settings last synced \(at.formatted(.relative(presentation: .named)))")) }
        if let backlog = content.outboxBacklog, let oldest = backlog.oldest,
           SyncTiming.backlogIsStalled(pending: backlog.pending, oldest: oldest, now: Date()) {
            lines.append(String(localized: "Changes waiting to send: \(backlog.pending), the oldest from \(oldest.formatted(.dateTime.month().day().hour().minute()))"))
        }
        return lines
    }

    private var aiLines: [String] {
        let routing = appState.routing
        var lines = [String(localized: "Summaries use \(routing.preferredProvider.displayName), \(routing.preferredModel).")]
        if routing.preferredProvider == .ollama, let missing = routing.ollamaMissingChoice {
            lines.append(String(localized: "\(missing.chosen) isn't installed; using \(missing.using)."))
        }
        if let totals = usageTotals {
            for id in RoutingSettings.budgetProviders {
                let spent = totals.monthCents(for: id)
                let budget = routing.monthlyBudgetCents[id].map(Int64.init)
                guard spent > 0 || budget != nil else { continue }
                let name = RoutingSettings.Provider(rawValue: id)?.displayName ?? id
                lines.append("\(name): \(AIProvidersSettingsPane.thisMonthLine(spentCents: spent, budgetCents: budget).text)")
            }
        }
        return lines
    }

    private var storageLines: [String] {
        var lines: [String] = []
        if let usage = appState.mediaRetention.lastPlan?.usage { lines.append(MediaRetentionCopy.usage(usage)) }
        let retention = appState.retention
        lines.append(retention.meetingKeepForever
            ? String(localized: "Meetings are kept forever.")
            : String(localized: "Meetings are kept for \(retention.meetingDays) days."))
        return lines
    }

    private var updateLines: [String] {
        let version = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? ""
        var lines = [String(localized: "Whimbrel \(version)")]
        if let update = appState.updateChecker.downloadedUpdate {
            lines.append(String(localized: "Version \(update.version) is ready. Restart to install it."))
        } else {
            switch appState.updateChecker.status {
            case .checking: lines.append(String(localized: "Checking…"))
            case .downloading(let v): lines.append(String(localized: "Downloading v\(v)…"))
            case .upToDate: lines.append(String(localized: "Up to date."))
            case .failed: lines.append(String(localized: "Couldn't check for updates. Try again later."))
            case .idle, .ready: break
            }
        }
        return lines
    }

    private var routineLines: [String] {
        let host = appState.routineHost.primaryDeviceId
        var lines = [host.isEmpty
            ? String(localized: "Routines run on any device.")
            : String(localized: "Routines run on \(appState.deviceLabel(for: host)).")]
        if let run = lastRun {
            let at = Date(timeIntervalSince1970: TimeInterval(run.ranAtUnixMs) / 1000)
            lines.append(String(localized: "Last run: \(run.routineName), \(at.formatted(.relative(presentation: .named)))"))
        }
        return lines
    }

    private func load() {
        appState.mediaRetention.refreshPlan()
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let totals = store.usageTotals()
            let run = store.recentRoutineRuns(limit: 1)?.first
            DispatchQueue.main.async {
                usageTotals = totals
                lastRun = run
            }
        }
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
