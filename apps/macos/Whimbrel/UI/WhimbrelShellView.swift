import AppKit
import SwiftUI
import OSLog
import WhimbrelUI

/// The meeting-first shell's seven sections.
enum WhimbrelTab: String, CaseIterable, Identifiable {
    case today = "Today"
    case meetings = "Meetings"
    case library = "Library"
    case topics = "Topics"
    case conversations = "Conversations"
    case automations = "Automations"
    case settings = "Settings"

    var id: String { rawValue }

    var title: String {
        switch self {
        case .today: String(localized: "Today")
        case .meetings: String(localized: "Meetings")
        case .library: String(localized: "Library")
        case .topics: String(localized: "Topics")
        case .conversations: String(localized: "Conversations")
        case .automations: String(localized: "Automations")
        case .settings: String(localized: "Settings")
        }
    }

    /// The Go menu's ⌘ number. Topics came later, so it takes the next free one.
    var goKey: Character {
        switch self {
        case .today: "1"
        case .meetings: "2"
        case .library: "3"
        case .automations: "4"
        case .settings: "5"
        case .topics: "6"
        case .conversations: "7"
        }
    }

    /// The sections the sidebar and the Go menu list, in order.
    static let visibleTabs: [WhimbrelTab] = [.today, .meetings, .library, .topics, .conversations, .automations, .settings]

    /// Where a question from the shell's Ask bar goes (no search filter set).
    enum AskTarget: Equatable {
        /// Answered under the bar on Today.
        case today
        /// A new conversation with this scope, its row made once the first answer is in.
        case conversation(CoreConversationScope)
        /// The Library's Ask, as before.
        case library
    }

    /// Spec part 4, scope by where you ask: Today answers under the bar; the Conversations section asks about
    /// everything; a topic page (`topicId`, the topic shown) asks about that topic. Elsewhere the Library's Ask.
    func askTarget(topicId: Int64?) -> AskTarget {
        switch self {
        case .today: return .today
        case .conversations: return .conversation(.everything)
        case .topics: return topicId.map { .conversation(.topic($0)) } ?? .library
        case .meetings, .library, .automations, .settings: return .library
        }
    }

}

struct WhimbrelShellView: View {
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "shell")

    @EnvironmentObject var appState: AppState
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    @State private var showFirstRun = false
    @StateObject private var homeBridge = HomeRedesignBridge()
    /// The header field and chips Library's Find page shares with the shell.
    @StateObject private var findBridge = FindBridge()
    @State private var homeApps: [CoreCaptureApp] = []
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

            shell
        }
        .background(Color.whimbrelBackgroundPage)
        .tint(Color.whimbrelAccentPrimary)
        .background(HideWindowTitle())
        // Settings is the native `Settings { SettingsRedesignView() }` scene (⌘,), never embedded here:
        // nesting its NavigationSplitView in the shell's once ballooned the window past the display.
        .onAppear {
            // `TimelineView` used to set this on `.onAppear` at launch (it was the root
            // view) — now it only runs once the user visits the Library tab. Set it here
            // too so the meeting-alert panel's "Start Recording & Open Timeline" action
            // works from app launch, same target window either way.
            appState.openTimeline = { openWindow(id: "timeline") }
            appState.openSettingsWindow = { openSettings() }
            showFirstRun = FirstRun.shouldShow(
                defaults: .standard,
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
        .plusAiPrompt(syncState: appState.syncState, routing: appState.routing)
        .sheet(item: $appState.prepBriefToShow) { request in
            PrepBriefSheet(key: request.key)
                .environmentObject(appState)
        }
        .sheet(isPresented: $showFirstRun) {
            FirstRunRedesignView {
                FirstRun.markDone(.standard)
                showFirstRun = false
            }
            .environmentObject(appState.permissions)
        }
    }

    /// The selected section's screen.
    @ViewBuilder
    private var detailView: some View {
        switch appState.selectedTab {
        case .today: TodayView()
        case .meetings: banded { MeetingsListView() }
        case .library: banded { LibraryRedesignScreen() }
        case .topics: banded { TopicsView() }
        case .conversations: banded { ConversationsHostView() }
        case .automations: banded { AutomationsView() }
        case .settings: StatusOverviewView(openSettings: openSettings)
        }
    }

    // MARK: Shell

    /// The sidebar icon for a section: the selected skin's icon where the skin has a role for it, else the Thread symbol.
    private func glyph(_ tab: WhimbrelTab) -> String {
        let icons = SkinStore.shared.selected.icons
        switch tab {
        case .today: return icons[.today]
        case .meetings: return icons[.meetings]
        case .library: return "books.vertical"
        case .topics: return "tag"
        case .conversations: return "bubble.left.and.bubble.right"
        case .automations: return icons[.routines]
        case .settings: return icons[.settings]
        }
    }

    private var shell: some View {
        SwiftUI.TimelineView(.periodic(from: .now, by: appState.activeMeetingId == nil ? 30 : 1)) { context in
            let onHome = appState.selectedTab == .today
            WShell(
                items: WhimbrelTab.visibleTabs.map {
                    WShellItem(id: $0.rawValue, glyph: glyph($0), title: $0.title, shortcut: "\u{2318}" + String($0.goKey))
                },
                selection: appState.selectedTab.rawValue,
                onSelect: {
                    let tab = WhimbrelTab(rawValue: $0) ?? .today
                    // Choosing Conversations again from one of its pages goes back to the list.
                    if tab == .conversations, appState.selectedTab == .conversations { appState.showConversationList() }
                    appState.selectedTab = tab
                },
                header: WShellHeader(
                    title: appState.selectedTab.title, subtitle: onHome ? HomeRedesignAdapter.dateLine(context.date) : nil,
                    ask: HomeRedesignAdapter.ask(appState)),
                elements: WHomeElements.resolve(options: SkinStore.shared.selected.archetypeOptions, archetype: SkinStore.shared.selected.archetype.rawValue),
                people: homeBridge.people, appChoices: homeApps.map { WHomeAppChoice(id: $0.appBundleId, name: HomeRedesignAdapter.appName($0.appBundleId)) },
                showsFilters: onHome || (appState.selectedTab == .library && findBridge.showsFilters),
                onSubmit: { query, _ in
                    // Enter searches this device, from any section, and never sends a question to a model.
                    findBridge.shellQueryChanged(query)
                    appState.selectedTab = .library
                    findBridge.searchRequest += 1
                },
                onSetUpAsk: { openSettings() },
                thread: onHome ? homeBridge.thread : [], onThread: { homeBridge.scrollTo($0) },
                memory: HomeRedesignAdapter.memoryState(enabled: appState.activityCaptureEnabled, resumeAt: appState.captureResumeAt, now: context.date, capturing: appState.screenText.isRunning, since: appState.memoryOnSince),
                onWhatIsCaptured: {
                    appState.pendingSettingsSection = .capture
                    openSettings()
                },
                recording: recording(at: context.date),
                level: appState.audio.inputLevel,
                onPause: appState.canPauseRecording ? { appState.pauseRecording() } : nil,
                onResume: appState.recordingPaused ? { appState.resumeRecording() } : nil,
                // Also offered for a start still waiting on its meeting row with its audio on (`canStopStartingRecording`): the same
                // action the menu's End Meeting runs, which stops the audio at once and saves the meeting when its row lands.
                onStop: appState.activeMeetingId == nil && !appState.canStopStartingRecording ? nil : { appState.endActiveMeetingIfNeeded() },
                pauseChoices: {
                    let state = CapturePauseModel.state(captureOn: appState.activityCaptureEnabled, resumeAt: appState.captureResumeAt)
                    if state != .off {
                        WPauseChoices(isPaused: state == .paused, onPause: { appState.pauseCapture($0) }, onResume: { appState.resumeCaptureNow() })
                        Divider()
                    }
                    Button(state == .off ? String(localized: "Turn on memory") : String(localized: "Turn off memory")) {
                        appState.toggleActivityCapture()
                    }
                },
                footer: { sidebarFooter },
                banner: { EmptyView() },
                content: {
                    detailView
                        .environment(\.homeBridge, homeBridge)
                        .environment(\.homeScroll, homeBridge.scroll)
                        .environment(\.findBridge, findBridge)
                },
                initialQuery: findBridge.query, initialFilter: findBridge.filter,
                onQueryChange: { findBridge.shellQueryChanged($0) }, onFilterChange: { findBridge.shellFilterChanged($0) }, push: findBridge.push,
                onAsk: { query, _ in
                    // Command Return asks. From another section the answer goes where that section sends it (below).
                    findBridge.shellQueryChanged(query)
                    if appState.selectedTab == .library {
                        // The page decides: it brings Find forward, sends a follow-up when that field has the keyboard, and sends the
                        // header's text otherwise.
                        findBridge.askRequest += 1
                    } else if !query.isEmpty {
                        if !HomeRedesignAdapter.ask(appState).isAvailable {
                            // No provider is set up: the set-up the field's own button leads to.
                            openSettings()
                        } else {
                            // Where the answer goes, per section (screen 6): under the bar on Today, a new conversation from
                            // Conversations or a topic, and Find on a new conversation from anywhere else.
                            switch appState.selectedTab.askTarget(topicId: appState.selectedTopicId) {
                            case .today where !findBridge.filter.isEmpty:
                                // Asked on Today with People, Apps or Dates chips on: Today's answer does not read them, so the
                                // question goes to Find, which asks under those filters (as Windows does). Never asked without them.
                                // A busy Find keeps it in its field and says why (`newConversation(asking:)`).
                                appState.requestNewConversation(prompt: query)
                            case .today:
                                // Asked on Today: the answer shows under the bar (`TodayAnswerArea`), never in Library.
                                appState.askOnToday(query)
                            case .conversation(let scope):
                                appState.startConversation(scope: scope, question: query)
                            case .library:
                                if !findBridge.askBusy { appState.requestNewConversation(prompt: query) }
                            }
                        }
                    }
                },
                focusRequest: findBridge.focusRequest, filterTrailing: appState.selectedTab == .library ? findBridge.countText : nil,
                // Command Return belongs to the header field while it has the keyboard, and to the Find page while that shows; anywhere else (an
                // editor, a sheet on Meetings) it is left alone. Off the Library tab an Ask button beside the field says that asking is possible.
                askKeyActive: appState.selectedTab == .library && findBridge.showsFilters,
                showsAskButton: appState.selectedTab != .library,
                askBusy: findBridge.askBusy
            )
            .redesignThemed()
            .onAppear { loadHomeApps() }
        }
    }

    /// The apps seen in the last 30 days, for the Apps chip. Read on the store's queue.
    private func loadHomeApps() {
        guard let store = appState.coreStore else { return }
        let end = Int64(Date().timeIntervalSince1970 * 1000), start = end - 30 * 24 * 3600 * 1000
        CoreMemoryStore.accessQueue.async {
            let apps = store.captureApps(startUnixMs: start, endUnixMs: end)
            DispatchQueue.main.async { if apps.map(\.appBundleId) != homeApps.map(\.appBundleId) { homeApps = apps } }
        }
    }

    /// The bar shows while a meeting is recording or being processed, on every section. It offers Pause or Resume and Stop, with the level, and says Paused while the recording is paused.
    private func recording(at now: Date) -> WShellRecording? {
        // The live meeting screen draws the bar itself, at the top of its column: never a second time.
        if appState.activeMeetingId != nil, appState.selectedTab == .today, appState.liveScreenShowing { return nil }
        if appState.activeMeetingId != nil {
            let state = LiveMeetingAdapter.state(
                isProcessing: false, stage: nil, isStarting: false, pause: appState.recordingPause,
                resuming: appState.isResumingRecording, problem: appState.recordingProblem).barState ?? .recording
            return WShellRecording(
                state: state, elapsedSeconds: LiveMeetingAdapter.recordedSeconds(now: now, startedAt: appState.activeMeetingStartedAt, pause: appState.recordingPause))
        }
        // A detected or calendar start whose meeting row is not saved yet: the bar says it is starting and offers Stop (or says Saving
        // once Stop was asked for). A start that is neither (nothing recording yet) draws no bar, as before.
        let stopOffer = MenuBarAdapter.stopOffer(
            hasActiveMeeting: false, isStarting: appState.isStartingMeetingNow, audioLive: false,
            canStopStarting: appState.canStopStartingRecording, stopPendingForRow: appState.isStopPendingForRow)
        if appState.isStartingMeetingNow, stopOffer != .unavailable { return WShellRecording(state: .starting, startingStop: stopOffer) }
        if appState.processingMeetingId != nil { return WShellRecording(state: .processing) }
        return nil
    }

    /// This Mac's name and whether sync is running, under a hairline.
    private var sidebarFooter: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Divider().overlay(Color.whimbrelBorder)
            ModelStatusLine(bootstrap: appState.modelBootstrap)
            StaleSyncLine(sync: appState.contentSyncCoordinator, offersSyncNow: true)
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
                if !planLines.isEmpty { row("Plan", .plan, planLines) }
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
            // A sync that worked and then stopped isn't "setting up" (#590).
            if let line = appState.contentSyncCoordinator.stoppedLine() { return [line] }
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

    /// Empty (row hidden) unless sync is ready or at its device limit, or while the plan is unknown. A balance shows only while sync
    /// is ready, so a stale one never appears.
    private var planLines: [String] {
        let state = appState.syncState
        var balance: AIBalance?
        switch state.status {
        case .ready: balance = state.aiBalance
        case .deviceLimit: break
        default: return []
        }
        return PlanStatusLines.lines(plan: state.plan, balance: balance)
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
        Task { await appState.syncState.refreshAIBalance() }
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
struct StaleSyncLine: View {
    @ObservedObject var sync: ContentSyncCoordinator
    var inset: CGFloat = Spacing.md
    /// Adds the "Sync now" link under the warning (the sidebar). The Devices pane has its own button.
    var offersSyncNow = false

    var body: some View {
        SwiftUI.TimelineView(.periodic(from: .now, by: 300)) { context in
            if sync.isStale(now: context.date), let last = sync.lastSuccess {
                VStack(alignment: .leading, spacing: Spacing.xs) {
                    Text(sync.retryPossible
                         ? String(localized: "Sync hasn't worked since \(last.formatted(date: .abbreviated, time: .shortened)). Whimbrel keeps trying.")
                         : String(localized: "Sync hasn't worked since \(last.formatted(date: .abbreviated, time: .shortened))."))
                        .font(.whimbrelBody(.caption2))
                        .foregroundStyle(Color.whimbrelError)
                        // A line limit, never `fixedSize(vertical:)`: while the sidebar collapses its width nears zero,
                        // and unbounded wrapping then asked for ~800 pt. The window could not fit that and AppKit looped
                        // on Update Constraints until it aborted (Nov 18 2026, every sidebar toggle while sync was stale).
                        .lineLimit(4)
                    if offersSyncNow {
                        // Compact: one line, link style, so the footer's height stays bounded.
                        SyncNowControl(sync: sync, compact: true)
                    }
                }
                .padding(.horizontal, inset)
            }
        }
    }
}

/// "Sync now": one content sync round on request, with its outcome. Disabled when sync is off or not
/// signed in. The round itself (and its one-at-a-time guard) is `ContentSyncCoordinator.syncNow`.
struct SyncNowControl: View {
    @ObservedObject var sync: ContentSyncCoordinator
    /// Sidebar footer form: the label and outcome share one line and never wrap past it.
    var compact = false
    @StateObject private var model: SyncNowModel

    init(sync: ContentSyncCoordinator, compact: Bool = false) {
        self.sync = sync
        self.compact = compact
        _model = StateObject(wrappedValue: SyncNowModel(
            canSync: { [weak sync] in sync?.canSyncNow ?? false },
            run: { [weak sync] in await sync?.syncNowManually() ?? .unavailable }
        ))
    }

    var body: some View {
        let state = SyncNowModel.buttonState(status: sync.syncStatus, roundRunning: sync.roundInFlight, phase: model.phase)
        HStack(spacing: compact ? Spacing.sm : Spacing.md) {
            Button {
                Task { await model.start() }
            } label: {
                if state == .syncing {
                    HStack(spacing: Spacing.xs) {
                        ProgressView().controlSize(.small)
                        Text("Syncing…")
                    }
                } else {
                    Text("Sync now")
                }
            }
            .buttonStyle(.link)
            .disabled(state != .enabled)
            .accessibilityIdentifier("sync-now-button")
            if let message = model.message {
                Text(message)
                    .foregroundStyle(model.phase == .synced ? Color.whimbrelTextSecondary : Color.whimbrelError)
                    // Bounded, never `fixedSize`: see `StaleSyncLine`.
                    .lineLimit(compact ? 1 : 3)
            }
        }
        .font(.whimbrelBody(compact ? .caption2 : .caption))
        .lineLimit(compact ? 1 : 3)
        // The "Synced" line goes away by itself; an error stays until the next try.
        .task(id: model.phase) {
            guard model.phase == .synced else { return }
            try? await Task.sleep(nanoseconds: 5 * 1_000_000_000)
            if !Task.isCancelled { model.clearOutcome() }
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
                    // Bounded for the same reason as `StaleSyncLine`: the sidebar's width nears zero as it collapses.
                    .lineLimit(5)
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
