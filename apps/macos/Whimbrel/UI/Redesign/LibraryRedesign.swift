import AppKit
import Combine
import SwiftUI
import WhimbrelUI

/// Recent, Episodes and Digest of Library: the old modes' loading and actions, drawn as the same cards.
@MainActor
final class LibraryListsController: ObservableObject {
    private weak var appState: AppState?
    @Published private(set) var recent: [WLibraryRow] = []
    @Published private(set) var episodes: [CoreEpisode] = []
    @Published private(set) var hiddenEpisodeIds: Set<Int64> = []
    @Published var showHiddenEpisodes = false
    @Published private(set) var isLoading = false
    @Published private(set) var digest: CoreDailyDigest?
    @Published private(set) var digestError: String?
    @Published private(set) var isGeneratingDigest = false
    @Published var meetingPendingDeletion: CoreMeeting?
    private var meetings: [CoreMeeting] = []
    private var gateToken: UUID?
    /// An ask is in flight on the Find page: a digest then waits, as on the old page where both shared one busy flag.
    var isAskBusy: () -> Bool = { false }

    func attach(_ appState: AppState) { self.appState = appState }

    private func env() -> FindCardEnv? {
        appState.map { FindCardEnv.live(appState: $0, strings: WhimbrelUIStrings.make().find) }
    }

    func refresh(_ view: WLibraryView) {
        guard let appState, let store = appState.coreStore else { return }
        switch view {
        case .recent:
            let includeCaptures = appState.showScreenCaptures
            isLoading = true
            CoreMemoryStore.accessQueue.async {
                let captures = includeCaptures ? store.recent(limit: 50) : []
                let meetings = store.recentMeetingsGrouped(limit: 20)
                DispatchQueue.main.async { [weak self] in
                    guard let self, let env = self.env() else { return }
                    self.meetings = meetings
                    self.recent = FindAdapter.recentRows(captures: captures, meetings: meetings, env: env)
                    self.isLoading = false
                }
            }
        case .episodes:
            let deviceId = appState.deviceId
            isLoading = true
            CoreMemoryStore.accessQueue.async {
                let result = store.recentEpisodes(limit: 50) ?? []
                // Hidden state syncs across devices by episode identity, as the old screen reads it.
                let hidden = Set(store.listHiddenEpisodeIds(deviceId: deviceId))
                DispatchQueue.main.async { [weak self] in
                    self?.episodes = result
                    self?.hiddenEpisodeIds = hidden
                    self?.isLoading = false
                }
            }
        case .digest, .find:
            // A digest already shown stays shown when the tab is chosen again, as on the old page; only Regenerate replaces it.
            break
        }
    }

    func recentModel() -> WLibraryListModel {
        var m = WLibraryListModel()
        m.rows = recent
        m.isLoading = isLoading
        m.emptyTitle = String(localized: "Nothing here yet", comment: "Title of an empty Library list.")
        m.emptyHelp = appState?.showScreenCaptures == true
            ? String(localized: "Nothing yet. Meetings and screen activity appear here as you work.")
            : String(localized: "No meetings yet. Turn on “Show screen captures in Library” in Settings → Capture to see screen activity here too.")
        return m
    }

    func episodesModel() -> WLibraryListModel {
        var m = WLibraryListModel()
        if let env = env() { m.rows = FindAdapter.episodeRows(episodes, hidden: hiddenEpisodeIds, showHidden: showHiddenEpisodes, env: env) }
        m.isLoading = isLoading
        m.emptyTitle = String(localized: "Nothing here yet", comment: "Title of an empty Library list.")
        m.emptyHelp = appState?.coreStore == nil
            ? AskShared.storeUnavailableMessage
            : String(localized: "No episodes yet. They form from screen activity after a few minutes of use.")
        return m
    }

    func digestModel(label: String, phrase: String) -> WLibraryDigestModel {
        var m = WLibraryDigestModel()
        m.canGenerate = !isGeneratingDigest && !isAskBusy()
        if isGeneratingDigest {
            m.state = .generating(line: String(localized: "Generating digest with \(label)…"))
        } else if let digestError {
            m.state = .failed(message: digestError)
        } else if let digest {
            m.state = .ready(summary: digest.summary, openLoops: digest.openLoops)
        } else {
            m.state = .idle(help: String(localized: "Generates an end-of-day digest of what you worked on today and any open loops, from today's captured timeline, written \(phrase)."))
        }
        return m
    }

    func forget(_ meeting: CoreMeeting, then reload: @escaping () -> Void) {
        guard let appState, let store = appState.coreStore, meeting.status != .recording else { return }
        MeetingLifecycle.forgetMeeting(store: store, meetingId: meeting.id) { result in
            if case .failed = result {} else { appState.clearSummaryFailure(meetingId: meeting.id) }
            reload()
        }
    }

    func meeting(forRow id: String) -> CoreMeeting? {
        guard id.hasPrefix("meeting-"), let n = Int64(id.dropFirst("meeting-".count)) else { return nil }
        return meetings.first { $0.id == n }
    }

    func generateDigest(provider: RoutingSettings.Provider, modelLabel: String, ollamaModel: String) {
        guard let appState, let store = appState.slowStore, !isGeneratingDigest, !isAskBusy() else { return }
        digest = nil
        digestError = nil
        let calendar = Calendar.current
        let startOfToday = calendar.startOfDay(for: Date())
        let startOfTomorrow = calendar.date(byAdding: .day, value: 1, to: startOfToday) ?? startOfToday
        let startMs = Int64(startOfToday.timeIntervalSince1970 * 1000), endMs = Int64(startOfTomorrow.timeIntervalSince1970 * 1000)
        isGeneratingDigest = true
        gateToken = UserModelGate.shared.enter()
        let routing = appState.routing.routingConfig(
            preferredProvider: provider, preferredModel: modelLabel, dailyDigestInstructions: appState.promptLibrary.instructions[.dailyDigest] ?? "")
        UserModelGate.runOnSlowLane {
            let outcome = store.dailyDigest(model: ollamaModel, startUnixMs: startMs, endUnixMs: endMs, routing: routing)
            DispatchQueue.main.async { [weak self] in
                guard let self else { return }
                self.isGeneratingDigest = false
                UserModelGate.shared.leave(self.gateToken)
                self.gateToken = nil
                switch outcome {
                case .some(.success(let result)): self.digest = result
                case .some(.failure(let error)): self.digestError = FindAdapter.failureText(error)
                case .none: self.digestError = String(localized: "Couldn't generate the digest. Try again.")
                }
            }
        }
    }
}

/// Library: Find (the default), Recent, Episodes and Digest. The shell's header field drives Find.
///
/// Find's controller belongs to the bridge, which the shell keeps for the whole session, so an answer still arriving when another section is
/// chosen is not lost with this screen: it goes on arriving, is saved, and is on the page when Library is chosen again.
struct LibraryRedesignScreen: View {
    @Environment(\.findBridge) private var injected
    @StateObject private var ownBridge = FindBridge()

    var body: some View {
        let bridge = injected ?? ownBridge
        LibraryRedesignContent(bridge: bridge, find: bridge.controller)
    }
}

private struct LibraryRedesignContent: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openWindow) private var openWindow
    let bridge: FindBridge
    @ObservedObject var find: FindController
    @StateObject private var lists = LibraryListsController()
    @State private var view: WLibraryView = .find
    @State private var headphonesWarningDismissed = false

    var body: some View {
        let model = find.model()
        WLibraryPage(
            selection: view, onSelect: select,
            header: { header },
            panel: { AtAGlancePanelView() },
            content: { content(model) }
        )
        .redesignThemed()
        .onAppear {
            find.attach(appState: appState, bridge: bridge, openWindow: { openWindow(id: $0, value: $1) })
            lists.attach(appState)
            // One thing at a time on the model, as on the old page: a digest holds off an ask, and an ask a digest.
            find.isBusyElsewhere = { [weak lists] in lists?.isGeneratingDigest ?? false }
            lists.isAskBusy = { [weak find] in (find?.isAsking ?? false) || (find?.isSaving ?? false) }
            // A header Ask (Command Return) from Recent, Episodes or Digest comes to Find first.
            find.showFind = { if view != .find { select(.find) } }
            NSApp.activate(ignoringOtherApps: true)
            appState.openTimeline = { openWindow(id: "timeline") }
            appState.setMeetingWindowOpener { openWindow(id: "meeting-detail", value: $0) }
            bridge.showsFilters = view == .find
            handlePending()
        }
        .onDisappear { find.detach() }
        .onChange(of: appState.pendingLibrarySearch) { _, pending in if pending { handlePending() } }
        .onChange(of: appState.pendingNewConversation) { _, pending in if pending { handlePending() } }
        .onChange(of: model.countText) { _, text in bridge.countText = view == .find ? text : nil }
        .onChange(of: appState.showScreenCaptures) { _, _ in if view == .recent { lists.refresh(.recent) } }
        .onReceive(NotificationCenter.default.publisher(for: MeetingLifecycle.meetingsDidChange)) { _ in
            if view == .recent || view == .episodes { lists.refresh(view) }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { notification in
            guard let window = notification.object as? NSWindow, window.title == "Whimbrel" else { return }
            if view == .recent || view == .episodes { lists.refresh(view) }
        }
        .alert("Forget this meeting?", isPresented: Binding(get: { lists.meetingPendingDeletion != nil }, set: { if !$0 { lists.meetingPendingDeletion = nil } })) {
            Button("Forget meeting", role: .destructive) {
                if let meeting = lists.meetingPendingDeletion { lists.forget(meeting) { lists.refresh(.recent) } }
                lists.meetingPendingDeletion = nil
            }
            Button("Cancel", role: .cancel) { lists.meetingPendingDeletion = nil }
        } message: {
            MeetingLifecycle.forgetConfirmationMessage
        }
        .alert("Rename Conversation", isPresented: Binding(get: { find.renaming != nil }, set: { if !$0 { find.renaming = nil } })) {
            TextField("Title", text: $find.renameText)
            Button("Save") { find.commitRename() }
            Button("Cancel", role: .cancel) {}
        }
    }

    private func select(_ new: WLibraryView) {
        view = new
        // The follow-up field exists on Find only; leaving it ends its claim on Command Return.
        if new != .find { find.followUpFocusChanged(false) }
        bridge.showsFilters = new == .find
        bridge.countText = new == .find ? find.countText() : nil
        lists.refresh(new)
    }

    /// Search Library (Command F) and New Conversation (Command N), and what Home's block Open and a header Ask leave for this screen.
    private func handlePending() {
        if appState.pendingNewConversation {
            appState.pendingNewConversation = false
            let prompt = appState.pendingAskPrompt
            appState.pendingAskPrompt = nil
            select(.find)
            find.newConversation(asking: prompt)
            bridge.focusRequest += 1
        } else if appState.pendingLibrarySearch {
            appState.pendingLibrarySearch = false
            select(.find)
            if let pending = appState.pendingLibraryFilter {
                appState.pendingLibraryFilter = nil
                bridge.setFilter(FindAdapter.homeFilter(pending.filter))
                bridge.setQuery(pending.query)
                find.search()
            }
            bridge.focusRequest += 1
        }
    }

    // MARK: Pieces

    private var phrase: String {
        let provider: RoutingSettings.Provider = appState.routing.localOnlyMode ? .ollama : find.selectedProvider
        return provider == .ollama ? "on this Mac with Ollama" : "by \(provider.displayName)"
    }

    private var picker: some View {
        ProviderModelPicker(
            selectedProvider: Binding(get: { find.selectedProvider }, set: { find.selectedProvider = $0; find.providerPickerTouched = true; find.refreshRoute() }),
            ollamaModel: Binding(get: { find.ollamaModel }, set: { find.ollamaModel = $0; find.providerPickerTouched = true; find.refreshRoute() }),
            selectedHostedModel: Binding(get: { find.selectedHostedModel }, set: { find.selectedHostedModel = $0; find.providerPickerTouched = true; find.refreshRoute() })
        )
    }

    @ViewBuilder
    private var header: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            PermissionCard(kind: .accessibility).environmentObject(appState.permissions)
            LibraryStatusHeader(headphonesWarningDismissed: $headphonesWarningDismissed)
            if view == .digest { picker }
        }
    }

    @ViewBuilder
    private func content(_ model: WFindModel) -> some View {
        switch view {
        case .find:
            WFindView(
                model: model, actions: findActions(), stream: find.stream,
                picker: { picker },
                notices: { findNotices },
                setup: { findSetup }
            )
        case .recent:
            WLibraryListView(
                model: lists.recentModel(), actions: listActions(),
                toggleTitle: WhimbrelUIStrings.make().find.showScreenCaptures, toggleOn: Binding(get: { appState.showScreenCaptures }, set: { appState.showScreenCaptures = $0 }))
        case .episodes:
            WLibraryListView(
                model: lists.episodesModel(), actions: listActions(),
                toggleTitle: WhimbrelUIStrings.make().find.showHiddenEpisodes, toggleOn: $lists.showHiddenEpisodes)
        case .digest:
            WLibraryDigestView(model: lists.digestModel(label: find.currentModelLabel, phrase: phrase), actions: listActions())
        }
    }

    @ViewBuilder
    private var findNotices: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            if appState.routing.userDailyCapCents != 0, let spent = find.spendTodayCents {
                let cap = appState.routing.userDailyCapCents.map(Int64.init) ?? RoutingSettings.defaultUserDailyCapCents
                Text("\(spent)¢ / \(cap)¢ today").font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
            }
            if find.held.isEmpty { LocalModelCard(download: appState.localModelDownload) }
            ForEach(find.namedTopics, id: \.topicId) { TopicPageLink(topic: $0) }
        }
    }

    private var findSetup: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            LocalModelCard(download: appState.localModelDownload)
            Button(String(localized: "Set up an AI provider")) { appState.openSettingsWindow?() }.buttonStyle(.whimbrelPrimary)
        }
    }

    private func findActions() -> WFindActions {
        var a = WFindActions()
        a.open = { find.open($0) }
        a.ask = { find.askFromHeader() }
        a.askFollowUp = { find.askFollowUp($0) }
        a.stop = { find.stop() }
        a.copyAnswer = { text in
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
        }
        a.focusExchange = { find.focus($0) }
        a.setOrder = { find.setOrder($0) }
        a.loadMore = { find.loadMore() }
        a.starter = { find.useStarter($0) }
        a.clearFilters = { find.clearFilters() }
        a.retry = { find.search() }
        a.failureButton = { find.openAISettings() }
        a.newConversation = { find.newConversation(asking: nil) }
        a.openConversation = { find.openConversation($0) }
        a.toggleStar = { find.toggleStar($0) }
        a.rename = { find.beginRename($0) }
        a.deleteConversation = { find.deleteConversation($0) }
        a.toggleConversations = {
            find.showConversations.toggle()
            if find.showConversations { find.refreshConversationList() }
        }
        a.followUpFocus = { find.followUpFocusChanged($0) }
        return a
    }

    private func listActions() -> WLibraryActions {
        var a = WLibraryActions()
        a.open = { find.open($0) }
        a.openInNewWindow = { id in if let m = lists.meeting(forRow: id) { openWindow(id: "meeting-detail", value: m.id) } }
        a.forget = { id in lists.meetingPendingDeletion = lists.meeting(forRow: id) }
        a.generateDigest = { lists.generateDigest(provider: find.selectedProvider, modelLabel: find.currentModelLabel, ollamaModel: find.ollamaModel) }
        return a
    }
}

/// Library's status header: the capture toggle and state, the speakers warning, Start Meeting or the notes of the meeting in progress.
struct LibraryStatusHeader: View {
    @EnvironmentObject var appState: AppState
    @Binding var headphonesWarningDismissed: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack {
                Toggle("Activity capture", isOn: Binding(
                    get: { appState.activityCaptureEnabled },
                    // Deferred a turn, as the old header does: see the identical toggle in the menu bar content.
                    set: { _ in Task { @MainActor in appState.toggleActivityCapture() } }
                ))
                .tint(Color.whimbrelAccentPrimary)
                .recordingControl()
                Spacer()
            }
            Text(FindAdapter.statusLine(
                recordingPaused: appState.recordingPaused, meetingTitle: appState.activeMeetingTitle, systemAudio: appState.audio.systemAudioActive,
                mic: appState.audio.micActive, screenText: appState.screenText.isRunning, captureEnabled: appState.activityCaptureEnabled)).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            if let stage = appState.processingStage {
                HStack(spacing: Spacing.xs) {
                    ProgressView().controlSize(.small)
                    Text(stage).font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
                }
            }
            if appState.activeMeetingId != nil, appState.audio.recordingOnBuiltInSpeakers, !headphonesWarningDismissed {
                HStack(alignment: .top, spacing: Spacing.xs) {
                    Text(String(localized: "Recording on speakers. The mic may pick up meeting audio. Headphones reduce this.", comment: "Warning on Library's status header while a meeting records on the built-in speakers."))
                        .font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary).fixedSize(horizontal: false, vertical: true)
                    Spacer()
                    Button("Dismiss") { headphonesWarningDismissed = true }.buttonStyle(.whimbrelSecondary).controlSize(.small)
                }
            }
            if appState.activeMeetingId != nil {
                VStack(alignment: .leading, spacing: Spacing.sm) {
                    HStack {
                        Text("Recording: \(appState.activeMeetingTitle ?? "Meeting")\(appState.activeMeetingIsContinued ? " \u{00B7} Continued" : "")")
                            .font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                        Spacer()
                        Button("End Meeting") { Task { @MainActor in appState.endActiveMeetingIfNeeded() } }.buttonStyle(.whimbrelSecondary)
                    }
                    Text("Notes").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                    TextEditor(text: Binding(get: { appState.meetingNotes }, set: { appState.meetingNotes = $0 }))
                        .font(.whimbrelBody(.callout)).frame(height: 80).padding(Spacing.xs)
                        .background(Color.whimbrelBackgroundCard).clipShape(RoundedRectangle(cornerRadius: Radius.control))
                        .overlay(RoundedRectangle(cornerRadius: Radius.control).stroke(Color.whimbrelBorder, lineWidth: 1))
                }
            } else {
                HStack {
                    RecordingOffCaption()
                    Spacer()
                    Button("Start Meeting") { Task { @MainActor in appState.startManualMeeting() } }.buttonStyle(.whimbrelPrimary).recordingControl()
                }
            }
        }
        .padding(Spacing.md)
        .background(Color.whimbrelBackgroundCard)
        .clipShape(RoundedRectangle(cornerRadius: Radius.control))
    }
}
