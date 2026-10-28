import AppKit
import SwiftUI

/// Default landing surface: a greeting, then the live call or an Ask composer (and an empty
/// state when there is nothing to show), today's schedule card (events with Start and prep
/// status, plus meetings already held) and an "Earlier" catch-up list, with approvals, open
/// loops, the daily recap and a device footer below. Only the live call is
/// embedded (`ActiveMeetingView`); finished meetings open the meeting-detail window.
///
/// Every `CoreMemoryStore` call here (`refresh()`/`loadLiveDraft`) is dispatched via
/// `CoreMemoryStore.accessQueue.async { ... }` + `DispatchQueue.main.async { ... }`, never
/// synchronously from the view body. `hero` and `agenda` are pure computed properties
/// over `TodayLogic`.
struct TodayView: View {
    @EnvironmentObject var appState: AppState
    @Environment(\.openSettings) private var openSettings
    @Environment(\.openWindow) private var openWindow
    @State private var deviceActivity: [CoreDeviceActivity] = []

    @State private var recentMeetings: [CoreMeeting] = []
    /// Until the first `refresh()` lands, an active meeting holds the live slot (a spinner) instead of
    /// the empty state, so a rebuild mid-meeting (a skin switch) does not flash "Start note".
    @State private var recentLoaded = false
    /// Seeds `ActiveMeetingView` for the live call only; finished meetings are not embedded.
    @State private var liveDraft: CoreMeetingNoteDraft?
    /// The meeting id `liveDraft` belongs to, nil until `loadLiveDraft` finishes.
    @State private var liveDraftMeetingId: Int64?
    @State private var digest: CoreDailyDigest?
    @State private var isGeneratingDigest = false
    @State private var digestError: String?
    @State private var briefRecap: CoreRoutineRunWithName?
    @State private var attentionThreads: [CoreThread] = []
    @State private var followups: [CoreMeetingFollowup] = []
    @State private var pendingApprovalCount: Int = 0
    /// The prep brief for the next upcoming meeting, when one has been prepared.
    @State private var nextPrep: CoreMeetingPrep?

    /// Everything Today shows, read from this view's state and `AppState` at render time.
    private var content: TodayContent {
        let rows = deviceRows
        var labels = Dictionary(uniqueKeysWithValues: rows.map { ($0.deviceId, appState.deviceLabel(for: $0.deviceId)) })
        labels[appState.deviceId] = appState.deviceLabel(for: appState.deviceId)
        if let origin = briefRecap?.originDeviceId { labels[origin] = appState.deviceLabel(for: origin) }
        return TodayContent(
            name: appState.profile.name, now: Date(), pinnedNow: nil, events: appState.allUpcomingMeetings,
            recentMeetings: recentMeetings, recentLoaded: recentLoaded, calendarGranted: appState.permissions.calendarGranted,
            activeMeetingId: appState.activeMeetingId, processingMeetingId: appState.processingMeetingId,
            nextPrep: nextPrep, pendingApprovalCount: pendingApprovalCount, attentionThreads: attentionThreads,
            briefRecap: briefRecap, digest: digest, digestError: digestError, isGeneratingDigest: isGeneratingDigest,
            deviceId: appState.deviceId, deviceRows: rows, deviceLabels: labels, followups: followups
        )
    }

    private var actions: TodayActions {
        TodayActions(
            openMeeting: { openWindow(id: "meeting-detail", value: $0) },
            openThread: { openWindow(id: "thread-detail", value: $0) },
            ask: { appState.requestNewConversation(prompt: $0) },
            openSettings: { openSettings() },
            generateDigest: { generateDigest() },
            reviewApprovals: { appState.selectedTab = .automations },
            startMeeting: { event in
                if let source = appState.allUpcomingMeetings.first(where: { $0.id == event.id }) {
                    appState.startCapture(for: source)
                } else {
                    appState.startManualMeeting()
                }
            },
            startNote: { appState.startManualMeeting() },
            completeFollowup: { id in completeFollowup(id) }
        )
    }

    /// The live call's pane, once its draft has loaded; `nil` until then.
    private func liveView(_ content: TodayContent) -> AnyView? {
        guard case let .live(meetingId) = content.hero,
              let meeting = recentMeetings.first(where: { $0.id == meetingId }), liveDraftMeetingId == meetingId,
              case let .during(model) = deriveMeetingWorkspaceState(
                  meeting: meeting, activeMeetingId: appState.activeMeetingId,
                  processingMeetingId: appState.processingMeetingId, processingStage: appState.processingStage,
                  draft: liveDraft, actionItems: []
              )
        else { return nil }
        return AnyView(
            ActiveMeetingView(
                model: model, initialNotes: liveDraft?.personalNotes ?? "",
                onSaveNotes: { text, now in saveLiveNotes(meetingId: meetingId, text: text, nowUnixMs: now) },
                onStop: { appState.endActiveMeetingIfNeeded() },
                liveSection: appState.liveTranscriptSettings.enabled
                    ? AnyView(LiveMeetingSection(live: appState.liveTranscript, meeting: meeting))
                    : nil
            )
            .id(meetingId)
        )
    }

    var body: some View {
        let content = content
        let actions = actions
        TodayChrome(
            content: content, actions: actions,
            slots: TodaySlots(
                live: liveView(content), decisionCount: content.pendingApprovalCount, actions: actions,
                composer: AnyView(TodayComposer(placeholder: SkinStore.shared.selected.voice.text(.askPlaceholder, name: "")) { actions.ask($0) })
            )
        )
        .onAppear { refresh() }
        .onReceive(NotificationCenter.default.publisher(for: NSWindow.didBecomeKeyNotification)) { notification in
            guard let window = notification.object as? NSWindow, window.title == "Whimbrel" else { return }
            // Forgetting a meeting from the standalone meeting window hands focus back here.
            refresh()
        }
        .onChange(of: appState.activeMeetingId) { _, _ in
            // Starting or ending a meeting while sitting on Today must show up at once.
            refresh()
        }
        .onChange(of: content.upcoming.first?.id) { _, _ in
            // The prep brief belongs to the next unfinished event; refetch when that changes.
            refresh()
        }
        .onChange(of: content.liveMeetingId) { _, newValue in
            guard let newValue else {
                liveDraft = nil
                liveDraftMeetingId = nil
                return
            }
            loadLiveDraft(meetingId: newValue)
        }
    }

    /// This Mac first, listed even with nothing captured today, then the other devices
    /// by activity. Mirrors the Windows Today card.
    private var deviceRows: [CoreDeviceActivity] {
        var rows = deviceActivity
        if !rows.contains(where: { $0.deviceId == appState.deviceId }) {
            rows.append(CoreDeviceActivity(deviceId: appState.deviceId, captures: 0, lastTsUnixMs: 0))
        }
        return rows.sorted {
            ($0.deviceId == appState.deviceId ? 0 : 1, -$0.captures) < ($1.deviceId == appState.deviceId ? 0 : 1, -$1.captures)
        }
    }


    private func completeFollowup(_ id: Int64) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.completeFollowup(id: id)
            DispatchQueue.main.async { refresh() }
        }
    }

    private func refresh() {
        guard let store = appState.coreStore else { recentLoaded = true; loadDigestIfNeeded(); return }
        let nextEventId = content.upcoming.first?.id
        CoreMemoryStore.accessQueue.async {
            let prep = nextEventId.flatMap { store.meetingPrep(eventId: $0) }
            let meetings = store.recentMeetingsGrouped(limit: 10)
            let threads = store.threadsNeedingAttention(limit: 5) ?? []
            let pending = store.pendingApprovals() ?? []
            let open = store.openFollowups()
            let brief = TodayLogic.recapRun(from: store.recentRoutineRuns(limit: 50) ?? [], now: Date(), calendar: .current)
            let startOfToday = Calendar.current.startOfDay(for: Date())
            let startOfTomorrow = Calendar.current.date(byAdding: .day, value: 1, to: startOfToday) ?? startOfToday
            let activity = store.captureActivityByDevice(
                startUnixMs: Int64(startOfToday.timeIntervalSince1970 * 1000),
                endUnixMs: Int64(startOfTomorrow.timeIntervalSince1970 * 1000)
            )
            DispatchQueue.main.async {
                briefRecap = brief
                if brief == nil { loadDigestIfNeeded() }
                nextPrep = prep
                recentMeetings = meetings
                // A Today built mid-meeting is live from its first render, so `liveMeetingId` never changes
                // and the draft is loaded here. Here, not on appear: the replaced pane's `onDisappear` flush
                // is already queued on `accessQueue` ahead of this read, so the draft holds its last notes.
                if !recentLoaded, let id = appState.activeMeetingId, meetings.contains(where: { $0.id == id }) {
                    loadLiveDraft(meetingId: id)
                }
                recentLoaded = true
                attentionThreads = threads
                followups = open
                pendingApprovalCount = pending.count
                deviceActivity = activity
            }
        }
    }

    private func loadLiveDraft(meetingId: Int64) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let draft = store.getMeetingNoteDraft(meetingId: meetingId)
            DispatchQueue.main.async {
                liveDraft = draft
                liveDraftMeetingId = meetingId
            }
        }
    }

    /// Saves the live call's personal notes. `ActiveMeetingView.onSaveNotes` must not call `CoreMemoryStore` synchronously off
    /// the view body/closure — dispatched onto `CoreMemoryStore.accessQueue` like every
    /// other store write here, per this task's boundary. Returns `true` optimistically
    /// (matching `ActiveNoteAutosaveController`'s save-state contract) since the actual
    /// write result isn't observed synchronously; `false` only when there's no store to
    /// write to at all.
    private func saveLiveNotes(meetingId: Int64, text: String, nowUnixMs: Int64) -> Bool {
        guard let store = appState.coreStore else { return false }
        CoreMemoryStore.accessQueue.async {
            _ = store.savePersonalNotes(meetingId: meetingId, notes: text, nowUnixMs: nowUnixMs)
        }
        return true
    }

    // MARK: - Digest (verbatim from `AtAGlancePanelView`'s `loadDigestIfNeeded`/`generateDigest`)

    private func loadDigestIfNeeded() {
        if let cached = DailyDigestCache.loadIfFresh() {
            digest = cached
            return
        }
        generateDigest()
    }

    private func generateDigest() {
        guard let store = appState.slowStore, !isGeneratingDigest else { return }
        isGeneratingDigest = true
        let calendar = Calendar.current
        let startOfToday = calendar.startOfDay(for: Date())
        let startOfTomorrow = calendar.date(byAdding: .day, value: 1, to: startOfToday) ?? startOfToday
        let startMs = Int64(startOfToday.timeIntervalSince1970 * 1000)
        let endMs = Int64(startOfTomorrow.timeIntervalSince1970 * 1000)
        let ollamaModel = appState.routing.ollamaDefaultModel
        let routing = appState.routing.routingConfig(
            preferredProvider: appState.routing.preferredProvider, preferredModel: appState.routing.preferredModel,
            dailyDigestInstructions: appState.promptLibrary.instructions[.dailyDigest] ?? ""
        )
        CoreMemoryStore.slowLaneQueue.async {
            let outcome = store.dailyDigest(model: ollamaModel, startUnixMs: startMs, endUnixMs: endMs, routing: routing)
            DispatchQueue.main.async {
                isGeneratingDigest = false
                switch outcome {
                case .some(.success(let result)):
                    digest = result
                    digestError = nil
                    DailyDigestCache.store(result)
                case .some(.failure(let error)):
                    digestError = error.userFacingMessage
                case .none:
                    digestError = "Couldn't write today's recap. Try again."
                }
            }
        }
    }
}
